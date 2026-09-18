//! Single-instance guard — prevents multiple tray processes.
//!
//! [`SingleInstanceGuard::acquire`] separates "already running" (`Ok(None)`,
//! the caller exits silently with 0) from "creation failed" (`Err`, the
//! caller dialogs and exits with 1).

use single_instance::SingleInstance;
use thiserror::Error;

/// Failure to create the single-instance primitive (ACL/namespace error).
#[derive(Debug, Error)]
pub enum InstanceError {
    /// The OS primitive itself could not be created.
    #[error("single-instance guard creation failed: {0}")]
    CreateFailed(String),
}

/// RAII guard ensuring only one instance runs with the given `name`.
pub struct SingleInstanceGuard {
    _instance: SingleInstance,
}

impl SingleInstanceGuard {
    /// Try to acquire the single-instance lock.
    ///
    /// Returns `Ok(None)` when another instance already holds the lock,
    /// `Err` when the underlying OS primitive cannot be created.
    ///
    /// # Errors
    ///
    /// Returns [`InstanceError::CreateFailed`] when the OS primitive fails.
    pub fn acquire(name: &str) -> Result<Option<Self>, InstanceError> {
        let instance =
            SingleInstance::new(name).map_err(|e| InstanceError::CreateFailed(e.to_string()))?;
        if !instance.is_single() {
            return Ok(None);
        }
        Ok(Some(Self {
            _instance: instance,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_unique_name() {
        let g = SingleInstanceGuard::acquire("audio-switcher-test-single-instance-unique-12345");
        assert!(matches!(g, Ok(Some(_))));
    }

    #[test]
    fn second_acquire_reports_running() {
        let _first =
            SingleInstanceGuard::acquire("audio-switcher-test-single-instance-twice-12345");
        let second =
            SingleInstanceGuard::acquire("audio-switcher-test-single-instance-twice-12345");
        assert!(matches!(second, Ok(None)));
    }
}
