//! The system's own share sheet, for a file: Windows' Share panel and the
//! macOS Share menu. It reaches the apps on this computer that take an image
//! (WhatsApp, Mail, Teams, Messages, AirDrop and Nearby Share to a phone),
//! which no link can: Instagram and Facebook take no image from a web link.
//! Linux has no such sheet, so the page does not offer it there.

use serde::Deserialize;
use specta::Type;
use std::path::Path;

/// Where the button that opened the sheet is, in the webview's CSS pixels
/// from its top left. The macOS menu opens beside it; Windows places its
/// panel itself.
#[derive(Debug, Clone, Copy, Deserialize, Type)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct ShareAnchor {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[cfg(windows)]
pub fn show(
    window: &tauri::WebviewWindow,
    file: &Path,
    title: &str,
    _anchor: ShareAnchor,
) -> Result<(), String> {
    use std::cell::RefCell;
    use windows::core::{factory, Interface, Ref, HSTRING};
    use windows::ApplicationModel::DataTransfer::{DataRequestedEventArgs, DataTransferManager};
    use windows::Foundation::TypedEventHandler;
    use windows::Storage::Streams::RandomAccessStreamReference;
    use windows::Storage::{IStorageItem, StorageFile};
    use windows::Win32::Foundation::E_FAIL;
    use windows::Win32::UI::Shell::IDataTransferManagerInterop;
    use windows_collections::IIterable;
    use windows_future::{AsyncOperationCompletedHandler, IAsyncOperation};

    thread_local! {
        /// The handler the last share registered on this (the window's)
        /// thread, removed before the next one so a second share does not
        /// hand over the first one's file as well.
        static REGISTERED: RefCell<Option<(DataTransferManager, i64)>> =
            const { RefCell::new(None) };
    }

    let path = HSTRING::from(file.as_os_str());
    let title = HSTRING::from(title);
    let (done_tx, done_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| -> windows::core::Result<()> {
                let hwnd = target
                    .hwnd()
                    .map_err(|e| windows::core::Error::new(E_FAIL, e.to_string()))?;
                let interop = factory::<DataTransferManager, IDataTransferManagerInterop>()?;
                let manager: DataTransferManager = unsafe { interop.GetForWindow(hwnd)? };
                // The panel asks for the data once it is open. Opening the
                // file is asynchronous, so the request is held open with a
                // deferral until it is, rather than waiting on this thread.
                let handler = TypedEventHandler::<DataTransferManager, DataRequestedEventArgs>::new(
                    move |_, args| {
                        let Some(args) = args.as_ref() else {
                            return Ok(());
                        };
                        let request = args.Request()?;
                        let data = request.Data()?;
                        data.Properties()?.SetTitle(&title)?;
                        let deferral = request.GetDeferral()?;
                        StorageFile::GetFileFromPathAsync(&path)?.SetCompleted(
                            &AsyncOperationCompletedHandler::new(
                                move |operation: Ref<'_, IAsyncOperation<StorageFile>>, _| {
                                    let filled = (|| -> windows::core::Result<()> {
                                        let file = operation.ok()?.GetResults()?;
                                        let item: IStorageItem = file.cast()?;
                                        data.SetStorageItemsReadOnly(
                                            &IIterable::<IStorageItem>::from(vec![Some(item)]),
                                        )?;
                                        data.SetBitmap(
                                            &RandomAccessStreamReference::CreateFromFile(&file)?,
                                        )?;
                                        Ok(())
                                    })();
                                    if let Err(error) = &filled {
                                        log::warn!(
                                            "Share: the image could not be attached: {error}"
                                        );
                                    }
                                    deferral.Complete()
                                },
                            ),
                        )
                    },
                );
                let token = manager.DataRequested(&handler)?;
                REGISTERED.with(|slot| {
                    if let Some((previous, token)) = slot.borrow_mut().replace((manager, token)) {
                        let _ = previous.RemoveDataRequested(token);
                    }
                });
                unsafe { interop.ShowShareUIForWindow(hwnd) }
            })();
            let _ = done_tx.send(result.map_err(|e| format!("Could not open Share: {e}")));
        })
        .map_err(|e| e.to_string())?;
    done_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .map_err(|_| "Share did not open".to_string())?
}

#[cfg(target_os = "macos")]
pub fn show(
    window: &tauri::WebviewWindow,
    file: &Path,
    _title: &str,
    anchor: ShareAnchor,
) -> Result<(), String> {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSSharingServicePicker, NSView};
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSURL};
    use std::cell::RefCell;

    thread_local! {
        /// The open picker, kept alive while its menu is up. AppKit does not
        /// promise to retain it.
        static PICKER: RefCell<Option<Retained<NSSharingServicePicker>>> =
            const { RefCell::new(None) };
    }

    let path = file.to_string_lossy().into_owned();
    let view_ptr = window.ns_view().map_err(|e| e.to_string())? as usize;
    let (done_tx, done_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    window
        .run_on_main_thread(move || {
            let result = (|| -> Result<(), String> {
                let mtm = MainThreadMarker::new().ok_or("Not on the main thread")?;
                // SAFETY: `ns_view` is the window's content view, which lives as
                // long as the window, and this runs on the main thread.
                let view: &NSView = unsafe { &*(view_ptr as *const NSView) };
                let url = NSURL::fileURLWithPath(&NSString::from_str(&path));
                let item: Retained<AnyObject> = Retained::into_super(Retained::into_super(url));
                let items = NSArray::from_retained_slice(&[item]);
                let picker = unsafe { NSSharingServicePicker::initWithItems(mtm.alloc(), &items) };
                // The content view counts from its bottom left; the page from
                // its top left.
                let bounds = view.bounds();
                let flipped = view.isFlipped();
                let y = if flipped {
                    anchor.y
                } else {
                    bounds.size.height - anchor.y - anchor.height
                };
                let rect = NSRect::new(
                    NSPoint::new(anchor.x, y),
                    NSSize::new(anchor.width.max(1.0), anchor.height.max(1.0)),
                );
                let edge = if flipped {
                    NSRectEdge::MaxY
                } else {
                    NSRectEdge::MinY
                };
                picker.showRelativeToRect_ofView_preferredEdge(rect, view, edge);
                PICKER.with(|slot| *slot.borrow_mut() = Some(picker));
                Ok(())
            })();
            let _ = done_tx.send(result);
        })
        .map_err(|e| e.to_string())?;
    done_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .map_err(|_| "Share did not open".to_string())?
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn show(
    _window: &tauri::WebviewWindow,
    _file: &Path,
    _title: &str,
    _anchor: ShareAnchor,
) -> Result<(), String> {
    Err("This system has no share sheet".into())
}
