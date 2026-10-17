//! Linux: the text around the caret, through AT-SPI.
//!
//! AT-SPI runs over D-Bus, so the same code reads fields on X11 and Wayland.
//! One dedicated thread runs a small tokio runtime that owns the connection to
//! the accessibility bus. It keeps track of which object last took focus, from
//! the bus's own focus events, so a read goes straight to the right field
//! instead of searching every window. The connection, and with it the focus
//! tracking, only exists once Smart spacing has been used.
//!
//! What the read does, and the bug each step avoids:
//!
//! * Password fields are never read; terminals are never read (their text is
//!   the scrollback).
//! * Our own windows are never read.
//! * Only a small window around the caret is copied, never the whole text: in a
//!   terminal or a long document that would be the entire buffer.
//! * Chromium hands a rich editor (Gmail, Slack, claude.ai) over as one object
//!   per paragraph, with U+FFFC standing in for each one. The read follows the
//!   link into the paragraph that holds the caret instead of taking the
//!   placeholder for text.
//! * Every call is bounded by one deadline: a busy app would otherwise hold the
//!   read until the D-Bus timeout.
//!
//! Chromium, Electron and Qt apps only expose their text when the desktop's
//! accessibility setting is on. SpeakoFlow does not switch it on by itself: it
//! is a system-wide setting that changes how other apps behave, and may be the
//! user's own assistive-technology choice. Where text is not exposed, the
//! last-paste memory still handles stopping and continuing.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self as std_mpsc, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use atspi::events::object::StateChangedEvent;
use atspi::proxy::accessible::AccessibleProxy;
use atspi::proxy::hyperlink::HyperlinkProxy;
use atspi::proxy::proxy_ext::ProxyExt;
use atspi::proxy::text::TextProxy;
use atspi::{AccessibilityConnection, Role, State};
use futures_util::StreamExt;
use log::debug;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use super::reading::{self, Reading, Snapshot, AFTER_CHARS, BEFORE_CHARS};

/// Deadline for one whole read, inside the caller's budget.
const READ_LIMIT: Duration = Duration::from_millis(250);

/// After failing to reach the accessibility bus, wait this long before trying
/// again (the bus may simply not be running).
const RETRY_AFTER: Duration = Duration::from_secs(30);

/// Nodes visited when no focus event has been seen yet and the focused field
/// has to be searched for.
const SEARCH_NODES: usize = 800;
const SEARCH_DEPTH: usize = 24;

/// Paragraph placeholders followed into a rich editor, at most.
const MAX_DESCENT: usize = 8;

struct Job {
    reply: SyncSender<Snapshot>,
}

static BUSY: AtomicBool = AtomicBool::new(false);
static JOBS: OnceLock<Mutex<Option<UnboundedSender<Job>>>> = OnceLock::new();

fn jobs() -> Option<UnboundedSender<Job>> {
    let slot = JOBS.get_or_init(|| {
        let (tx, rx) = mpsc::unbounded_channel::<Job>();
        let spawned = std::thread::Builder::new()
            .name("smart-spacing-atspi".into())
            .spawn(move || worker(rx));
        Mutex::new(spawned.ok().map(|_| tx))
    });
    slot.lock().ok().and_then(|guard| guard.clone())
}

/// Read the focused field, waiting at most `budget`.
pub fn read(budget: Duration) -> Snapshot {
    if BUSY.swap(true, Ordering::SeqCst) {
        debug!("Smart spacing: the previous read is still running; skipping this one");
        return Snapshot::unknown(None);
    }
    let Some(sender) = jobs() else {
        BUSY.store(false, Ordering::SeqCst);
        return Snapshot::unknown(None);
    };
    let (reply, answer) = std_mpsc::sync_channel(1);
    if sender.send(Job { reply }).is_err() {
        BUSY.store(false, Ordering::SeqCst);
        return Snapshot::unknown(None);
    }
    answer
        .recv_timeout(budget)
        .unwrap_or_else(|_| Snapshot::unknown(None))
}

/// The accessible object that most recently took focus.
#[derive(Clone, Debug)]
struct FocusRef {
    bus: String,
    path: String,
}

type FocusSlot = Arc<Mutex<Option<FocusRef>>>;

fn worker(mut jobs: UnboundedReceiver<Job>) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            log::warn!("Smart spacing: could not start the AT-SPI runtime: {error}");
            while let Some(job) = jobs.blocking_recv() {
                BUSY.store(false, Ordering::SeqCst);
                let _ = job.reply.try_send(Snapshot::unknown(None));
            }
            return;
        }
    };
    runtime.block_on(async move {
        let focus: FocusSlot = Arc::new(Mutex::new(None));
        let mut connection: Option<Arc<AccessibilityConnection>> = None;
        let mut retry_at = Instant::now();
        let mut pids: HashMap<String, Option<u32>> = HashMap::new();

        while let Some(job) = jobs.recv().await {
            if connection.is_none() && Instant::now() >= retry_at {
                match tokio::time::timeout(
                    Duration::from_millis(500),
                    AccessibilityConnection::new(),
                )
                .await
                {
                    Ok(Ok(conn)) => {
                        let conn = Arc::new(conn);
                        tokio::spawn(track_focus(Arc::clone(&conn), Arc::clone(&focus)));
                        connection = Some(conn);
                    }
                    _ => {
                        debug!("Smart spacing: the accessibility bus is not reachable");
                        retry_at = Instant::now() + RETRY_AFTER;
                    }
                }
            }
            let snapshot = match &connection {
                Some(conn) => {
                    tokio::time::timeout(READ_LIMIT, read_focused(conn, &focus, &mut pids))
                        .await
                        .unwrap_or_else(|_| Snapshot::unknown(None))
                }
                None => Snapshot::unknown(None),
            };
            BUSY.store(false, Ordering::SeqCst);
            let _ = job.reply.try_send(snapshot);
        }
    });
}

/// Follow focus changes on the bus. Only state changes are subscribed to, and
/// only "focused" ones are kept.
async fn track_focus(conn: Arc<AccessibilityConnection>, focus: FocusSlot) {
    if let Err(error) = conn.register_event::<StateChangedEvent>().await {
        debug!("Smart spacing: could not subscribe to focus changes: {error}");
        return;
    }
    let stream = conn.event_stream();
    futures_util::pin_mut!(stream);
    while let Some(event) = stream.next().await {
        let Ok(event) = event else {
            continue;
        };
        let Ok(changed) = StateChangedEvent::try_from(event) else {
            continue;
        };
        if changed.state != State::Focused || !changed.enabled {
            continue;
        }
        let Some(bus) = changed.item.name_as_str() else {
            continue;
        };
        let record = FocusRef {
            bus: bus.to_string(),
            path: changed.item.path_as_str().to_string(),
        };
        if let Ok(mut slot) = focus.lock() {
            *slot = Some(record);
        }
    }
}

async fn accessible<'a>(
    conn: &'a AccessibilityConnection,
    bus: &str,
    path: &str,
) -> Option<AccessibleProxy<'a>> {
    AccessibleProxy::builder(conn.connection())
        .destination(bus.to_string())
        .ok()?
        .path(path.to_string())
        .ok()?
        .cache_properties(atspi::zbus::proxy::CacheProperties::No)
        .build()
        .await
        .ok()
}

/// The process behind a bus name, cached per name.
async fn process_of(
    conn: &AccessibilityConnection,
    bus: &str,
    pids: &mut HashMap<String, Option<u32>>,
) -> Option<u32> {
    if let Some(pid) = pids.get(bus) {
        return *pid;
    }
    let pid = async {
        let dbus = atspi::zbus::fdo::DBusProxy::new(conn.connection())
            .await
            .ok()?;
        let name = atspi::zbus::names::BusName::try_from(bus.to_string()).ok()?;
        dbus.get_connection_unix_process_id(name).await.ok()
    }
    .await;
    pids.insert(bus.to_string(), pid);
    pid
}

async fn read_focused(
    conn: &AccessibilityConnection,
    focus: &FocusSlot,
    pids: &mut HashMap<String, Option<u32>>,
) -> Snapshot {
    let recorded = focus.lock().ok().and_then(|slot| slot.clone());
    let found = match recorded {
        Some(record) => Some(record),
        None => search_focused(conn).await,
    };
    let Some(found) = found else {
        return Snapshot::unknown(None);
    };
    if process_of(conn, &found.bus, pids).await == Some(std::process::id()) {
        return Snapshot::unknown(None);
    }
    let key = Some(format!("atspi:{}", found.bus));
    let Some(object) = accessible(conn, &found.bus, &found.path).await else {
        return Snapshot::unknown(key);
    };
    let Ok(state) = object.get_state().await else {
        return Snapshot::unknown(key);
    };
    if !state.contains(State::Focused) {
        // Focus moved somewhere that sent no event (or to a non-text widget).
        return Snapshot::unknown(key);
    }
    let role = object.get_role().await.unwrap_or(Role::Invalid);
    if role == Role::PasswordText {
        return Snapshot::new(Reading::Secret, key);
    }
    if role == Role::Terminal {
        return Snapshot::new(Reading::Terminal, key);
    }
    if !state.contains(State::Editable) {
        return Snapshot::unknown(key);
    }
    Snapshot::new(read_text(conn, object).await, key)
}

/// Look for the focused, editable object inside an active window, for the
/// first read before any focus event has arrived.
async fn search_focused(conn: &AccessibilityConnection) -> Option<FocusRef> {
    let root = conn.root_accessible_on_registry().await.ok()?;
    let apps = root.get_children().await.ok()?;
    let mut visited = 0usize;
    for app in apps {
        let (Some(bus), path) = (app.name_as_str(), app.path_as_str()) else {
            continue;
        };
        let Some(app) = accessible(conn, bus, path).await else {
            continue;
        };
        let Ok(windows) = app.get_children().await else {
            continue;
        };
        for window in windows {
            let Some(window_bus) = window.name_as_str() else {
                continue;
            };
            let Some(window) = accessible(conn, window_bus, window.path_as_str()).await else {
                continue;
            };
            let active = window
                .get_state()
                .await
                .map(|s| s.contains(State::Active))
                .unwrap_or(false);
            if !active {
                continue;
            }
            // Breadth-first, bounded.
            let mut queue = vec![(window, 0usize)];
            while let Some((node, depth)) = queue.pop() {
                visited += 1;
                if visited > SEARCH_NODES {
                    return None;
                }
                if let Ok(state) = node.get_state().await {
                    if state.contains(State::Focused) && state.contains(State::Editable) {
                        let reference = atspi::ObjectRefOwned::try_from(&node).ok()?;
                        return Some(FocusRef {
                            bus: reference.name_as_str()?.to_string(),
                            path: reference.path_as_str().to_string(),
                        });
                    }
                    if !state.contains(State::Showing) {
                        continue;
                    }
                }
                if depth >= SEARCH_DEPTH {
                    continue;
                }
                if let Ok(children) = node.get_children().await {
                    for child in children.into_iter().rev() {
                        let Some(child_bus) = child.name_as_str() else {
                            continue;
                        };
                        if let Some(child) = accessible(conn, child_bus, child.path_as_str()).await
                        {
                            queue.insert(0, (child, depth + 1));
                        }
                    }
                }
            }
        }
    }
    None
}

async fn text_of<'a>(object: &AccessibleProxy<'a>) -> Option<TextProxy<'a>> {
    object.proxies().await.ok()?.text().await.ok()
}

async fn read_text<'a>(
    conn: &'a AccessibilityConnection,
    mut object: AccessibleProxy<'a>,
) -> Reading {
    for _ in 0..=MAX_DESCENT {
        let Some(text) = text_of(&object).await else {
            return Reading::Unknown;
        };
        let Ok(count) = text.character_count().await else {
            return Reading::Unknown;
        };
        if count == 0 {
            return Reading::Empty;
        }
        let Ok(caret) = text.caret_offset().await else {
            return Reading::Unknown;
        };
        if caret < 0 || caret > count {
            return Reading::Unknown;
        }

        // A paragraph placeholder next to the caret: follow it into the
        // paragraph that really holds the caret.
        if let Some(child) = paragraph_at_caret(conn, &object, &text, caret, count).await {
            object = child;
            continue;
        }

        let (start, end) = match text.get_n_selections().await {
            Ok(n) if n > 0 => match text.get_selection(0).await {
                Ok((a, b)) => (a.min(b).clamp(0, count), a.max(b).clamp(0, count)),
                Err(_) => (caret, caret),
            },
            _ => (caret, caret),
        };
        let before_from = (start - BEFORE_CHARS as i32).max(0);
        let after_to = (end + AFTER_CHARS as i32).min(count);
        let Ok(before) = text.get_text(before_from, start).await else {
            return Reading::Unknown;
        };
        let after = text.get_text(end, after_to).await.unwrap_or_default();
        let selected = if end > start {
            text.get_text(start, end).await.unwrap_or_default()
        } else {
            String::new()
        };
        return Reading::Text {
            before: reading::tail_chars(&reading::normalize(&before), BEFORE_CHARS),
            after: reading::head_chars(&reading::normalize(&after), AFTER_CHARS),
            selected: reading::normalize(&selected),
        };
    }
    Reading::Unknown
}

async fn paragraph_at_caret<'a>(
    conn: &'a AccessibilityConnection,
    object: &AccessibleProxy<'a>,
    text: &TextProxy<'a>,
    caret: i32,
    count: i32,
) -> Option<AccessibleProxy<'a>> {
    let from = (caret - 1).max(0);
    let to = (caret + 1).min(count);
    let around: Vec<char> = text.get_text(from, to).await.ok()?.chars().collect();
    let at = |offset: i32| -> Option<char> { around.get((offset - from) as usize).copied() };
    let offset = if at(caret) == Some('\u{FFFC}') {
        caret
    } else if caret > 0 && at(caret - 1) == Some('\u{FFFC}') {
        caret - 1
    } else {
        return None;
    };
    let hypertext = object.proxies().await.ok()?.hypertext().await.ok()?;
    let index = hypertext.get_link_index(offset).await.ok()?;
    if index < 0 {
        return None;
    }
    let link = hypertext.get_link(index).await.ok()?;
    let link = HyperlinkProxy::builder(conn.connection())
        .destination(link.name_as_str()?.to_string())
        .ok()?
        .path(link.path_as_str().to_string())
        .ok()?
        .cache_properties(atspi::zbus::proxy::CacheProperties::No)
        .build()
        .await
        .ok()?;
    let child = link.get_object(0).await.ok()?;
    let child = accessible(conn, child.name_as_str()?, child.path_as_str()).await?;
    // Only a child that actually has the caret is worth going into.
    let child_text = text_of(&child).await?;
    (child_text.caret_offset().await.ok()? >= 0).then_some(child)
}
