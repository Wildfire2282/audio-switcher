//! Audio backend abstraction: the port for device enumeration, default
//! switching and volume/mute control, generic over `App` so tests inject
//! `MockBackend` and Windows uses `WasapiBackend`.
//!
//! The cast lints are allowed for the whole layer, not per file: every flagged
//! conversion is at the COM boundary, where the alternative is a `try_from`
//! branch for a range the API already guarantees.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::ptr_as_ptr,
    clippy::borrow_as_ptr
)]

use thiserror::Error;

use crate::config::AppConfig;

/// An audio endpoint discovered via WASAPI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDevice {
    /// WASAPI endpoint ID (`IMMDevice::GetId`).
    pub id: String,
    /// Friendly name (`PKEY_Device_FriendlyName`).
    pub name: String,
}

/// Errors from the audio subsystem. The `#[error]` text is the user-visible
/// message.
#[derive(Debug, Clone, Error)]
pub enum AudioError {
    /// `hr` is preserved for diagnostics.
    #[error("COM 0x{hr:08X}: {msg}")]
    Com { hr: i32, msg: String },
    #[error("audio failed: {0}")]
    Failed(String),
}

#[cfg(windows)]
impl From<windows::core::Error> for AudioError {
    fn from(e: windows::core::Error) -> Self {
        Self::Com {
            hr: e.code().0,
            msg: e.to_string(),
        }
    }
}

/// Every fallible method reports one of two failures: `AudioError::Com` when
/// WASAPI returns an HRESULT, `AudioError::Failed` when the argument names an
/// endpoint that does not exist.
///
/// Must be `Send` where possible: `WasapiBackend` registers a COM notification
/// client on the STA thread and keeps it via `OnceLock`.
pub trait AudioBackend {
    fn enumerate_devices(&mut self) -> Result<Vec<AudioDevice>, AudioError>;

    fn get_default_device(&self) -> Option<AudioDevice>;

    fn set_default_device(&mut self, id: &str) -> Result<(), AudioError>;

    fn enumerate_input_devices(&mut self) -> Result<Vec<AudioDevice>, AudioError>;

    fn get_default_input_device(&self) -> Option<AudioDevice>;

    fn set_default_input_device(&mut self, id: &str) -> Result<(), AudioError>;

    /// `0..=100`.
    fn get_volume(&self) -> Result<u32, AudioError>;

    /// Values outside `0..=100` are clamped by the caller.
    fn set_volume(&mut self, volume: u32) -> Result<(), AudioError>;

    fn get_mute(&self) -> Result<bool, AudioError>;

    fn set_mute(&mut self, mute: bool) -> Result<(), AudioError>;

    fn clamp_volume_if_needed(&mut self, cfg: &AppConfig) -> Result<(), AudioError>;

    /// Batched so `WasapiBackend` can do one `Activate` instead of two calls.
    fn get_volume_and_mute(&self) -> Result<(u32, bool), AudioError> {
        Ok((self.get_volume()?, self.get_mute()?))
    }

    fn poll_device_changed(&mut self) -> bool {
        false
    }

    /// Take and clear the "external volume/mute changed" flag (media keys,
    /// other apps, the system mixer); it drives an icon refresh without polling.
    fn take_volume_changed(&mut self) -> bool {
        false
    }

    /// Uncached backends implement the empty body explicitly — there is
    /// deliberately no `noop` default to inherit.
    fn clear_cache(&mut self);

    fn fetch_snapshot_clamped(&mut self, cfg: &AppConfig) -> AudioSnapshot {
        let devices = self.enumerate_devices().unwrap_or_default();
        let default_device = self.get_default_device();
        let input_devices = self.enumerate_input_devices().unwrap_or_default();
        let default_input_device = self.get_default_input_device();
        let (volume, mute) = self.get_volume_and_mute().unwrap_or((50, false));
        let volume = crate::config::clamp_volume(volume, cfg);
        AudioSnapshot {
            devices,
            default_device,
            input_devices,
            default_input_device,
            volume,
            mute,
        }
    }
}

/// Snapshot of the current audio state — fetched once per UI refresh to avoid
/// repeated `CoCreateInstance` calls.
#[derive(Debug, Clone)]
pub struct AudioSnapshot {
    pub devices: Vec<AudioDevice>,
    pub default_device: Option<AudioDevice>,
    pub input_devices: Vec<AudioDevice>,
    pub default_input_device: Option<AudioDevice>,
    /// `0..=100`.
    pub volume: u32,
    pub mute: bool,
}

impl Default for AudioSnapshot {
    fn default() -> Self {
        Self {
            devices: Vec::new(),
            default_device: None,
            input_devices: Vec::new(),
            default_input_device: None,
            volume: 50,
            mute: false,
        }
    }
}

#[cfg(test)]
pub mod mock;
#[cfg(test)]
pub use mock::MockBackend;

/// Real Windows WASAPI backend.
pub mod wasapi;
pub use wasapi::WasapiBackend;

#[cfg(test)]
mod tests;
