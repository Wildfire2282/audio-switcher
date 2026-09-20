//! Concrete WASAPI backend for the Windows audio stack.
//!
//! Split by seam: [`default_device`] owns the raw `IPolicyConfig` switch, [`notify`]
//! owns the COM callbacks and the self-change suppression window, and this
//! file owns the backend itself - device cache, snapshot, volume/mute IO -
//! plus the non-Windows stub.
//!
//! Threading: the backend is used from the message-loop thread only.

use super::{AudioBackend, AudioDevice, AudioError, AudioSnapshot};
use crate::config::{AppConfig, clamp_volume};
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
#[cfg(windows)]
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
#[cfg(windows)]
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
#[cfg(windows)]
use windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, EDataFlow, IMMDevice, IMMDeviceCollection, IMMDeviceEnumerator,
    MMDeviceEnumerator, eCapture, eMultimedia, eRender,
};
#[cfg(windows)]
use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
#[cfg(windows)]
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ};
#[cfg(windows)]
use windows::Win32::System::Variant::VT_LPWSTR;
#[cfg(windows)]
mod default_device;
#[cfg(windows)]
mod notify;

#[cfg(windows)]
use default_device::set_default_endpoint_raw;
#[cfg(windows)]
use notify::{
    SUPPRESS_WINDOW_MS, register_notification_client, spawn_volume_notify_worker,
    suppress_self_changes_for, take_device_changed, take_volume_changed,
};

/// The Windows WASAPI backend.
#[cfg(windows)]
pub struct WasapiBackend {
    cached: Option<Vec<AudioDevice>>,
    cache_time: Option<Instant>,
    input_cached: Option<Vec<AudioDevice>>,
    input_cache_time: Option<Instant>,
    // Reuse the enumerator/endpoint across startup batch queries (fewer CoCreateInstance calls).
    cached_enumerator: Option<windows::Win32::Media::Audio::IMMDeviceEnumerator>,
}

#[cfg(windows)]
impl WasapiBackend {
    /// Create a backend and register the endpoint notification client once.
    pub fn new() -> Self {
        let s = Self {
            cached: None,
            cache_time: None,
            input_cached: None,
            input_cache_time: None,
            cached_enumerator: None,
        };
        // register once per process
        static REGISTERED: AtomicBool = AtomicBool::new(false);
        if !REGISTERED.swap(true, AtomicOrdering::AcqRel) {
            register_notification_client();
            spawn_volume_notify_worker();
        }
        s
    }

    /// Invalidate the device enumeration caches (render and capture).
    pub fn clear_cache(&mut self) {
        self.cached = None;
        self.cache_time = None;
        self.input_cached = None;
        self.input_cache_time = None;
        // The enumerator stays valid; only the device-list caches expire.
    }

    /// Get or create the cached IMMDeviceEnumerator (fewer CoCreateInstance calls).
    fn enumerator_mut(
        &mut self,
    ) -> windows::core::Result<windows::Win32::Media::Audio::IMMDeviceEnumerator> {
        if let Some(e) = &self.cached_enumerator {
            return Ok(e.clone());
        }
        let e = Self::get_enumerator()?;
        self.cached_enumerator = Some(e.clone());
        Ok(e)
    }

    /// Batch-fetch startup state (devices + default + volume + mute) sharing one
    /// enumerator/endpoint, with the limit applied inline so callers skip a
    /// second `get_volume_and_mute`.
    pub fn fetch_snapshot_clamped(&mut self, cfg: &AppConfig) -> AudioSnapshot {
        let mut snap = AudioSnapshot::default();
        // Prefer the cached enumerator.
        let Ok(enumerator) = self.enumerator_mut() else {
            return snap;
        };
        // Devices (3000ms cache; notifications already cleared it, so hits last longer).
        if let Ok(devs) = self.enumerate_devices_inner(&enumerator, eRender) {
            snap.devices = devs;
        }
        // Default + volume/mute share one endpoint.
        // SAFETY: the `windows` crate marks COM methods unsafe. Every call here
        // runs on an interface this scope owns and keeps alive (`enumerator` is
        // borrowed for the block; `dev` and `vol` are owned by it), and the null
        // event-context argument is the documented "no callback" form.
        unsafe {
            if let Ok(dev) = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) {
                if let Ok(id) = Self::device_id(&dev) {
                    let name = Self::device_friendly_name(&dev);
                    snap.default_device = Some(AudioDevice { id, name });
                }
                if let Ok(vol) = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                    if let Ok(scalar) = vol.GetMasterVolumeLevelScalar() {
                        // Contractually 0.0..=1.0; clamp so a lying driver cannot
                        // break the 0..=100 invariant (NaN folds to 0 via the
                        // saturating float-to-int cast).
                        snap.volume = (scalar.clamp(0.0, 1.0) * 100.0).round() as u32;
                    }
                    if let Ok(m) = vol.GetMute() {
                        snap.mute = m.as_bool();
                    }
                    // Clamp inline so callers skip a second get_volume_and_mute.
                    if cfg.volume_limit_enabled {
                        let clamped = clamp_volume(snap.volume, cfg);
                        if clamped != snap.volume {
                            suppress_self_changes_for(SUPPRESS_WINDOW_MS);
                            let v = clamped.min(100) as f32 / 100.0;
                            if vol.SetMasterVolumeLevelScalar(v, std::ptr::null()).is_ok() {
                                snap.volume = clamped;
                            }
                        }
                    }
                }
            }
        }
        // capture devices share the cached enumerator; render-only volume/mute untouched.
        if let Ok(inputs) = self.enumerate_devices_inner(&enumerator, eCapture) {
            snap.input_devices = inputs;
        }
        snap.default_input_device =
            Self::default_device_with(self.cached_enumerator.as_ref(), eCapture);
        snap
    }

    /// Fetch volume+mute with one endpoint activation (one fewer CoCreateInstance+Activate).
    /// Reuses cached_enumerator instead of creating a new one.
    pub fn get_volume_and_mute(&self) -> Result<(u32, bool), AudioError> {
        Self::volume_and_mute_with(self.cached_enumerator.as_ref())
    }

    /// Shared single-Activate core: trait and inherent methods both route here
    /// instead of triplicating the `get_volume_and_mute` body.
    fn volume_and_mute_with(
        cached: Option<&windows::Win32::Media::Audio::IMMDeviceEnumerator>,
    ) -> Result<(u32, bool), AudioError> {
        // SAFETY: COM methods are unsafe in the `windows` crate; each one here
        // is called on an interface this scope owns (the `enumerator` clone,
        // then the endpoint it returns), so every vtable pointer is live.
        unsafe {
            let enumerator = if let Some(e) = cached {
                e.clone()
            } else {
                Self::get_enumerator().map_err(AudioError::from)?
            };
            let dev = enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .map_err(AudioError::from)?;
            let vol: IAudioEndpointVolume =
                dev.Activate(CLSCTX_ALL, None).map_err(AudioError::from)?;
            let scalar = vol.GetMasterVolumeLevelScalar().map_err(AudioError::from)?;
            let m = vol.GetMute().map_err(AudioError::from)?;
            // Same driver-scalar clamp as the snapshot path (see above).
            Ok(((scalar.clamp(0.0, 1.0) * 100.0).round() as u32, m.as_bool()))
        }
    }

    fn enumerate_devices_inner(
        &mut self,
        enumerator: &windows::Win32::Media::Audio::IMMDeviceEnumerator,
        flow: EDataFlow,
    ) -> Result<Vec<AudioDevice>, AudioError> {
        if take_device_changed() {
            self.clear_cache();
        }
        let (cache, cache_time) = if flow == eCapture {
            (&self.input_cached, &self.input_cache_time)
        } else {
            (&self.cached, &self.cache_time)
        };
        if let Some(cached) = cache {
            if let Some(t) = cache_time {
                if t.elapsed() < Duration::from_millis(3000) {
                    return Ok(cached.clone());
                }
            }
        }
        // SAFETY: COM methods are unsafe in the `windows` crate; `enumerator` is
        // borrowed live by the caller and `collection`/`dev` are owned here, so
        // no call outlives the interface it runs on.
        unsafe {
            let collection: IMMDeviceCollection = enumerator
                .EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)
                .map_err(AudioError::from)?;
            let count = collection.GetCount().map_err(AudioError::from)?;
            // `GetCount` just returned it, so the capacity is exact on re-walk.
            let mut devices = Vec::with_capacity(count as usize);
            for i in 0..count {
                if let Ok(dev) = collection.Item(i) {
                    if let Ok(id) = Self::device_id(&dev) {
                        let name = Self::device_friendly_name(&dev);
                        devices.push(AudioDevice { id, name });
                    }
                }
            }
            // Always update cache, even if empty — UI must see removal vs stale list.
            if flow == eCapture {
                self.input_cached = Some(devices.clone());
                self.input_cache_time = Some(Instant::now());
            } else {
                self.cached = Some(devices.clone());
                self.cache_time = Some(Instant::now());
            }
            Ok(devices)
        }
    }

    /// Polls the notification flag and clears cache if a device changed.
    /// Single helper shared by the inherent method and the trait impl.
    fn take_notification(&mut self) -> bool {
        if take_device_changed() {
            self.clear_cache();
            return true;
        }
        false
    }

    /// Inherent alias kept for callers using `WasapiBackend` directly.
    pub fn poll_device_changed(&mut self) -> bool {
        self.take_notification()
    }

    fn get_enumerator() -> windows::core::Result<IMMDeviceEnumerator> {
        // SAFETY: a null outer object plus the `windows`-crate class id and IID
        // constants; on success the written pointer is an owned interface
        // returned in the `Result`, so no unowned reference is left behind.
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
    }

    /// Default endpoint for `flow` using the cached enumerator when available.
    fn default_device_with(
        cached: Option<&windows::Win32::Media::Audio::IMMDeviceEnumerator>,
        flow: EDataFlow,
    ) -> Option<AudioDevice> {
        // SAFETY: COM methods are unsafe in the `windows` crate; the enumerator
        // is either the caller's cached interface or one created here, and both
        // it and the endpoint it returns outlive every call in the block.
        unsafe {
            let enumerator = if let Some(e) = cached {
                e.clone()
            } else {
                Self::get_enumerator().ok()?
            };
            let dev = enumerator.GetDefaultAudioEndpoint(flow, eMultimedia).ok()?;
            let id = Self::device_id(&dev).ok()?;
            let name = Self::device_friendly_name(&dev);
            Some(AudioDevice { id, name })
        }
    }

    /// Shared validation + `IPolicyConfig` switch for both flows; roles are
    /// orthogonal to direction, so capture reuses the render role sequence.
    fn set_default_inner(&mut self, id: &str) -> Result<(), AudioError> {
        if id.is_empty() || id.contains('\0') {
            return Err(AudioError::Failed("invalid device id".into()));
        }
        // SAFETY: `set_default_endpoint_raw` requires a NUL-free id, a valid
        // role and COM initialized on this thread; the check above rejects the
        // ids that would break its UTF-16 encoding, and every role passed below
        // comes from `ERole`.
        unsafe {
            // Primary role: eMultimedia (1), must succeed.
            set_default_endpoint_raw(id, eMultimedia.0)
                .map_err(|e| AudioError::Failed(e.to_string()))?;
            // Secondary roles: best-effort but log failures (do not hide).
            for role in [0i32, 2i32] {
                if let Err(e) = set_default_endpoint_raw(id, role) {
                    // 0x80070490 = not found, 0x80070057 = invalid arg — don't retry, just warn.
                    tracing::warn!("set_default role {role} failed: {e}");
                }
            }
        }
        self.clear_cache();
        Ok(())
    }

    fn device_id(device: &IMMDevice) -> windows::core::Result<String> {
        // SAFETY: `GetId` hands back a string the shell allocated with
        // `CoTaskMemAlloc`; copying it and then freeing that exact pointer once
        // is the ownership transfer the API documents.
        unsafe {
            let pw = device.GetId()?;
            let s = pw.to_string().unwrap_or_default();
            CoTaskMemFree(Some(pw.0 as *const std::ffi::c_void));
            Ok(s)
        }
    }

    fn device_friendly_name(device: &IMMDevice) -> String {
        // SAFETY: the `PROPVARIANT` union arm is read only after `vt` selects
        // `VT_LPWSTR`, and the same value is cleared once with
        // `PropVariantClear` while it is still owned by this scope.
        unsafe {
            if let Ok(store) = device.OpenPropertyStore(STGM_READ) {
                if let Ok(mut pv) = store.GetValue(&PKEY_Device_FriendlyName) {
                    let vt = pv.Anonymous.Anonymous.vt;
                    let s = if vt == VT_LPWSTR {
                        let pw = pv.Anonymous.Anonymous.Anonymous.pwszVal;
                        if pw.0.is_null() {
                            String::new()
                        } else {
                            pw.to_string().unwrap_or_default()
                        }
                    } else {
                        String::new()
                    };
                    let _ = PropVariantClear(&raw mut pv);
                    if !s.is_empty() {
                        return s.chars().take(80).collect();
                    }
                }
            }
            // Fall back to a shortened endpoint id.
            if let Ok(id) = Self::device_id(device) {
                let short = id.split('\\').next_back().unwrap_or(&id);
                let truncated: String = short.chars().take(40).collect();
                if !truncated.is_empty() {
                    return truncated;
                }
                return id.chars().take(40).collect();
            }
            "Unknown".to_string()
        }
    }
}

#[cfg(windows)]
impl AudioBackend for WasapiBackend {
    fn fetch_snapshot_clamped(&mut self, cfg: &AppConfig) -> AudioSnapshot {
        WasapiBackend::fetch_snapshot_clamped(self, cfg)
    }
    fn get_volume_and_mute(&self) -> Result<(u32, bool), AudioError> {
        Self::volume_and_mute_with(self.cached_enumerator.as_ref())
    }
    fn clear_cache(&mut self) {
        WasapiBackend::clear_cache(self);
    }
    fn poll_device_changed(&mut self) -> bool {
        self.take_notification()
    }
    fn take_volume_changed(&mut self) -> bool {
        take_volume_changed()
    }
    fn enumerate_devices(&mut self) -> Result<Vec<AudioDevice>, AudioError> {
        let enumerator = self.enumerator_mut().map_err(AudioError::from)?;
        self.enumerate_devices_inner(&enumerator, eRender)
    }

    fn enumerate_input_devices(&mut self) -> Result<Vec<AudioDevice>, AudioError> {
        let enumerator = self.enumerator_mut().map_err(AudioError::from)?;
        self.enumerate_devices_inner(&enumerator, eCapture)
    }

    fn get_default_device(&self) -> Option<AudioDevice> {
        Self::default_device_with(self.cached_enumerator.as_ref(), eRender)
    }

    fn get_default_input_device(&self) -> Option<AudioDevice> {
        Self::default_device_with(self.cached_enumerator.as_ref(), eCapture)
    }

    fn set_default_device(&mut self, id: &str) -> Result<(), AudioError> {
        self.set_default_inner(id)
    }

    fn set_default_input_device(&mut self, id: &str) -> Result<(), AudioError> {
        self.set_default_inner(id)
    }

    fn get_volume(&self) -> Result<u32, AudioError> {
        self.get_volume_and_mute().map(|(v, _)| v)
    }

    fn set_volume(&mut self, volume: u32) -> Result<(), AudioError> {
        let v = volume.min(100) as f32 / 100.0;
        // SAFETY: COM methods are unsafe in the `windows` crate; `enumerator`,
        // `dev` and `vol` are all owned by this scope for the whole block, and
        // the null event-context argument is the documented "no callback" form.
        unsafe {
            let enumerator = self.enumerator_mut().map_err(AudioError::from)?;
            let dev = enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .map_err(AudioError::from)?;
            let vol: IAudioEndpointVolume =
                dev.Activate(CLSCTX_ALL, None).map_err(AudioError::from)?;
            // Self-initiated change: suppress the asynchronously delivered
            // notification of our own write.
            suppress_self_changes_for(SUPPRESS_WINDOW_MS);
            match vol.SetMasterVolumeLevelScalar(v, std::ptr::null()) {
                Ok(()) => Ok(()),
                Err(e) => {
                    // Only retry on busy/timeout HRESULTs; invalid arg is permanent.
                    let hr = e.code().0 as u32;
                    if (hr == 0x8007_001E || hr == 0x8007_04D4)
                        && vol.SetMasterVolumeLevelScalar(v, std::ptr::null()).is_ok()
                    {
                        return Ok(());
                    }
                    Err(AudioError::Failed(e.to_string()))
                }
            }
        }
    }

    fn get_mute(&self) -> Result<bool, AudioError> {
        self.get_volume_and_mute().map(|(_, m)| m)
    }

    fn set_mute(&mut self, mute: bool) -> Result<(), AudioError> {
        // SAFETY: COM methods are unsafe in the `windows` crate; `enumerator`,
        // `dev` and `vol` are owned here, and the null event-context argument
        // means "no completion callback", which this call does not need.
        unsafe {
            let enumerator = self.enumerator_mut().map_err(AudioError::from)?;
            let dev = enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .map_err(AudioError::from)?;
            let vol: IAudioEndpointVolume =
                dev.Activate(CLSCTX_ALL, None).map_err(AudioError::from)?;
            // Self-initiated change: suppress our own notification.
            suppress_self_changes_for(SUPPRESS_WINDOW_MS);
            vol.SetMute(mute, std::ptr::null())
                .map_err(|e| AudioError::Failed(e.to_string()))
        }
    }

    fn clamp_volume_if_needed(&mut self, cfg: &AppConfig) -> Result<(), AudioError> {
        if !cfg.volume_limit_enabled {
            return Ok(());
        }
        let (vol, _) = self.get_volume_and_mute()?;
        let clamped = clamp_volume(vol, cfg);
        if clamped != vol {
            suppress_self_changes_for(SUPPRESS_WINDOW_MS);
            return self.set_volume(clamped);
        }
        Ok(())
    }
}

/// Real backend stub for non-Windows (compilation only).
#[cfg(not(windows))]
pub struct WasapiBackend {
    cached: Option<Vec<AudioDevice>>,
    cache_time: Option<Instant>,
}
#[cfg(not(windows))]
impl WasapiBackend {
    pub fn new() -> Self {
        Self {
            cached: None,
            cache_time: None,
        }
    }
    pub fn clear_cache(&mut self) {
        self.cached = None;
        self.cache_time = None;
    }
    pub fn poll_device_changed(&mut self) -> bool {
        false
    }
    pub fn fetch_snapshot_clamped(&mut self, _cfg: &AppConfig) -> AudioSnapshot {
        AudioSnapshot::default()
    }
    pub fn get_volume_and_mute(&self) -> Result<(u32, bool), AudioError> {
        Ok((50, false))
    }
}
#[cfg(not(windows))]
impl AudioBackend for WasapiBackend {
    fn enumerate_devices(&mut self) -> Result<Vec<AudioDevice>, AudioError> {
        Ok(vec![])
    }
    fn get_default_device(&self) -> Option<AudioDevice> {
        None
    }
    fn set_default_device(&mut self, _id: &str) -> Result<(), AudioError> {
        Ok(())
    }
    fn enumerate_input_devices(&mut self) -> Result<Vec<AudioDevice>, AudioError> {
        Ok(vec![])
    }
    fn get_default_input_device(&self) -> Option<AudioDevice> {
        None
    }
    fn set_default_input_device(&mut self, _id: &str) -> Result<(), AudioError> {
        Ok(())
    }
    fn get_volume(&self) -> Result<u32, AudioError> {
        Ok(50)
    }
    fn set_volume(&mut self, _volume: u32) -> Result<(), AudioError> {
        Ok(())
    }
    fn get_mute(&self) -> Result<bool, AudioError> {
        Ok(false)
    }
    fn set_mute(&mut self, _mute: bool) -> Result<(), AudioError> {
        Ok(())
    }
    fn clamp_volume_if_needed(&mut self, _cfg: &AppConfig) -> Result<(), AudioError> {
        Ok(())
    }
    fn clear_cache(&mut self) {
        WasapiBackend::clear_cache(self);
    }
}
#[cfg(not(windows))]
pub fn take_device_changed() -> bool {
    false
}

#[cfg(all(test, windows))]
mod tests;
