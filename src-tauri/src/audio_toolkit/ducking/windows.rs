//! Windows: the default playback device's master volume
//! (`IAudioEndpointVolume`), the control the taskbar's volume slider moves.
//!
//! The endpoint is held from the moment it is lowered, so the recording
//! restores that device even if the default changes (headphones plugged in)
//! before it ends. COM objects stay on the worker thread that made them.

use super::{OutputVolume, Reading};
use windows::Win32::Media::Audio::{
    eMultimedia, eRender, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};

/// Join COM's multithreaded apartment, once, before the worker touches a
/// device. The worker lives as long as the app, so it never leaves.
pub(super) fn prepare_thread() {
    // SAFETY: called once at the start of the worker thread, before any COM
    // call on it.
    let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if result.is_err() {
        log::warn!("Could not initialise COM for audio ducking: {result:?}");
    }
}

struct Endpoint(IAudioEndpointVolume);

pub(super) fn default_output() -> Option<Box<dyn OutputVolume>> {
    // SAFETY: plain COM calls on the worker thread, which joined the
    // multithreaded apartment in `prepare_thread`.
    let volume = unsafe {
        let devices: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = devices.GetDefaultAudioEndpoint(eRender, eMultimedia).ok()?;
        device
            .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
            .ok()?
    };
    Some(Box::new(Endpoint(volume)))
}

impl OutputVolume for Endpoint {
    fn read(&self) -> Reading {
        // SAFETY: the interface is valid for as long as `self` holds it.
        unsafe {
            Reading {
                level: self.0.GetMasterVolumeLevelScalar().ok(),
                muted: self.0.GetMute().ok().map(|muted| muted.as_bool()),
            }
        }
    }

    fn set_level(&self, level: f32) -> bool {
        // SAFETY: as above; a null event context is allowed.
        unsafe {
            self.0
                .SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null())
                .is_ok()
        }
    }

    fn set_muted(&self, muted: bool) -> bool {
        // SAFETY: as above.
        unsafe { self.0.SetMute(muted, std::ptr::null()).is_ok() }
    }
}
