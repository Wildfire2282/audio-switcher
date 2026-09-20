//! In-memory `AudioBackend` used by tests and by `App` construction tests.
//!
//! Deliberately dumb: device state lives in a `Vec` and the error injection
//! switch (`fail_next`) exists so callers can assert the failure paths without
//! hardware. It never touches WASAPI, which is why it is the only backend that
//! builds on every platform.
use std::time::{Duration, Instant};

use crate::config::{AppConfig, clamp_volume};

use super::{AudioBackend, AudioDevice, AudioError};

#[derive(Debug, Clone)]
pub struct MockBackend {
    pub(crate) devices: Vec<AudioDevice>,
    pub(crate) default_id: Option<String>,
    pub(crate) input_devices: Vec<AudioDevice>,
    pub(crate) default_input_id: Option<String>,
    pub(crate) volume: u32,
    pub(crate) mute: bool,
    /// When true the next fallible operation returns an error.
    pub fail_next: bool,
    pub(crate) enumerate_count: usize,
    cached: Option<Vec<AudioDevice>>,
    cache_time: Option<Instant>,
}

impl MockBackend {
    pub fn new(devices: Vec<AudioDevice>, default_id: Option<String>) -> Self {
        Self {
            devices,
            default_id,
            input_devices: Vec::new(),
            default_input_id: None,
            volume: 50,
            mute: false,
            fail_next: false,
            enumerate_count: 0,
            cached: None,
            cache_time: None,
        }
    }

    /// Attach capture (input) devices for tests.
    #[must_use]
    pub fn with_inputs(mut self, devices: Vec<AudioDevice>, default_id: Option<String>) -> Self {
        self.input_devices = devices;
        self.default_input_id = default_id;
        self
    }

    fn maybe_fail(&mut self) -> Option<AudioError> {
        if self.fail_next {
            self.fail_next = false;
            return Some(AudioError::Failed("mock failure".into()));
        }
        None
    }

    pub fn set_volume_impl(&mut self, volume: u32) -> Result<(), AudioError> {
        if let Some(e) = self.maybe_fail() {
            return Err(e);
        }
        self.volume = volume.min(100);
        Ok(())
    }
}

impl AudioBackend for MockBackend {
    fn clear_cache(&mut self) {
        self.cached = None;
        self.cache_time = None;
    }
    fn enumerate_devices(&mut self) -> Result<Vec<AudioDevice>, AudioError> {
        if let Some(e) = self.maybe_fail() {
            return Err(e);
        }
        if let Some(cached) = &self.cached {
            if let Some(t) = &self.cache_time {
                if t.elapsed() < Duration::from_millis(800) {
                    return Ok(cached.clone());
                }
            }
        }
        self.enumerate_count += 1;
        let v = self.devices.clone();
        self.cached = Some(v.clone());
        self.cache_time = Some(Instant::now());
        // Return empty when no devices, do not keep old cache — UI should show empty state.
        Ok(v)
    }

    fn get_default_device(&self) -> Option<AudioDevice> {
        let id = self.default_id.as_ref()?;
        self.devices.iter().find(|d| &d.id == id).cloned()
    }

    fn enumerate_input_devices(&mut self) -> Result<Vec<AudioDevice>, AudioError> {
        if let Some(e) = self.maybe_fail() {
            return Err(e);
        }
        // No cache by design: inputs are fixed per-test via `with_inputs`,
        // while the render-side cache exists to emulate WASAPI round-trips
        // exercised by `mock_enumerate_cache`.
        Ok(self.input_devices.clone())
    }

    fn get_default_input_device(&self) -> Option<AudioDevice> {
        let id = self.default_input_id.as_ref()?;
        self.input_devices.iter().find(|d| &d.id == id).cloned()
    }

    fn set_default_input_device(&mut self, id: &str) -> Result<(), AudioError> {
        if let Some(e) = self.maybe_fail() {
            return Err(e);
        }
        if id.is_empty() {
            return Err(AudioError::Failed("empty device id".into()));
        }
        if self.input_devices.iter().any(|d| d.id == id) {
            self.default_input_id = Some(id.to_string());
            Ok(())
        } else {
            Err(AudioError::Failed(format!("not found: {id}")))
        }
    }

    fn set_default_device(&mut self, id: &str) -> Result<(), AudioError> {
        if let Some(e) = self.maybe_fail() {
            return Err(e);
        }
        if id.is_empty() {
            return Err(AudioError::Failed("empty device id".into()));
        }
        if self.devices.iter().any(|d| d.id == id) {
            self.default_id = Some(id.to_string());
            self.clear_cache();
            Ok(())
        } else {
            Err(AudioError::Failed(format!("not found: {id}")))
        }
    }

    fn get_volume(&self) -> Result<u32, AudioError> {
        Ok(self.volume)
    }

    fn set_volume(&mut self, volume: u32) -> Result<(), AudioError> {
        self.set_volume_impl(volume)
    }

    fn get_mute(&self) -> Result<bool, AudioError> {
        Ok(self.mute)
    }

    fn set_mute(&mut self, mute: bool) -> Result<(), AudioError> {
        if let Some(e) = self.maybe_fail() {
            return Err(e);
        }
        self.mute = mute;
        Ok(())
    }

    fn clamp_volume_if_needed(&mut self, cfg: &AppConfig) -> Result<(), AudioError> {
        let clamped = clamp_volume(self.volume, cfg);
        if clamped != self.volume {
            self.volume = clamped;
        }
        Ok(())
    }
}
