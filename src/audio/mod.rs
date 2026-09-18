//! Audio backend abstraction.
//!
//! `AudioBackend` is the port for device enumeration, default switching and
//! volume/mute control. The trait is generic over `App` for test injection
//! (`MockBackend`) and for the Windows `WasapiBackend`.
//!
//! The cast lints are allowed for the whole layer, not per file: every
//! conversion they would flag is at the COM boundary, where the alternative is
//! a `try_from` branch for a range the API already guarantees.
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

/// Errors from the audio subsystem.
#[derive(Debug, Clone, Error)]
pub enum AudioError {
    /// COM / HRESULT failure; `hr` preserved for diagnostics.
    #[error("COM 0x{hr:08X}: {msg}")]
    Com {
        /// Raw HRESULT value.
        hr: i32,
        /// Human-readable message.
        msg: String,
    },

    /// Generic failure with context.
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

/// Backend for audio operations.
///
/// Must be `Send` where possible; `WasapiBackend` registers a COM notification
/// client on the STA thread and keeps it via `OnceLock`.
pub trait AudioBackend {
    /// Enumerate active render endpoints.
    ///
    /// # Errors
    ///
    /// Returns `AudioError::Com` on WASAPI failure.
    fn enumerate_devices(&mut self) -> Result<Vec<AudioDevice>, AudioError>;

    /// Current default render device, if any.
    fn get_default_device(&self) -> Option<AudioDevice>;

    /// Set the default render device by `id`.
    ///
    /// # Errors
    ///
    /// Returns `AudioError::Failed` if `id` is unknown, `Com` on WASAPI failure.
    fn set_default_device(&mut self, id: &str) -> Result<(), AudioError>;

    /// Enumerate active capture (input) endpoints.
    ///
    /// # Errors
    ///
    /// Returns `AudioError::Com` on WASAPI failure.
    fn enumerate_input_devices(&mut self) -> Result<Vec<AudioDevice>, AudioError>;

    /// Current default capture device, if any.
    fn get_default_input_device(&self) -> Option<AudioDevice>;

    /// Set the default capture device by `id`.
    ///
    /// # Errors
    ///
    /// Returns `AudioError::Failed` if `id` is unknown, `Com` on WASAPI failure.
    fn set_default_input_device(&mut self, id: &str) -> Result<(), AudioError>;

    /// Master volume `0..=100`.
    ///
    /// # Errors
    ///
    /// Returns `AudioError::Com` on WASAPI failure.
    fn get_volume(&self) -> Result<u32, AudioError>;

    /// Set master volume `0..=100` (values outside are clamped by caller).
    ///
    /// # Errors
    ///
    /// Returns `AudioError::Com` on WASAPI failure.
    fn set_volume(&mut self, volume: u32) -> Result<(), AudioError>;

    /// Whether the endpoint is muted.
    ///
    /// # Errors
    ///
    /// Returns `AudioError::Com` on WASAPI failure.
    fn get_mute(&self) -> Result<bool, AudioError>;

    /// Set mute state.
    ///
    /// # Errors
    ///
    /// Returns `AudioError::Com` on WASAPI failure.
    fn set_mute(&mut self, mute: bool) -> Result<(), AudioError>;

    /// Clamp volume to `cfg` if limiting is enabled.
    ///
    /// # Errors
    ///
    /// Returns `AudioError::Com` if current volume cannot be read.
    fn clamp_volume_if_needed(&mut self, cfg: &AppConfig) -> Result<(), AudioError>;

    /// Batch fetch `(volume, mute)`; default impl does two calls.
    ///
    /// `WasapiBackend` overrides with a single `Activate`.
    ///
    /// # Errors
    ///
    /// Returns `AudioError::Com` on WASAPI failure.
    fn get_volume_and_mute(&self) -> Result<(u32, bool), AudioError> {
        Ok((self.get_volume()?, self.get_mute()?))
    }

    /// Whether an external device change notification fired.
    ///
    /// Default `false` for mocks.
    fn poll_device_changed(&mut self) -> bool {
        false
    }

    /// Take and clear the "external volume/mute changed" flag (keyboard media
    /// keys, other apps, system mixer). Drives a tray-icon refresh without
    /// polling. Default `false` for mocks.
    fn take_volume_changed(&mut self) -> bool {
        false
    }

    /// Invalidate enumeration caches. Caching backends re-enumerate on the
    /// next call; uncached backends implement the empty body explicitly —
    /// there is deliberately no `noop` default to inherit.
    fn clear_cache(&mut self);

    /// Fetch a clamped snapshot in one round-trip; default builds from the
    /// methods above.
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
    /// Enumerated devices.
    pub devices: Vec<AudioDevice>,
    /// Default device, if known.
    pub default_device: Option<AudioDevice>,
    /// Enumerated capture (input) devices.
    pub input_devices: Vec<AudioDevice>,
    /// Default capture device, if known.
    pub default_input_device: Option<AudioDevice>,
    /// Current volume `0..=100`.
    pub volume: u32,
    /// Mute state.
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
