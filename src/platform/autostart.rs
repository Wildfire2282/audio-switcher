//! Auto-launch (autostart) helpers.
//!
//! The typed trio is the whole surface: [`get_exe_path`],
//! [`set_autostart`], [`autostart_state`]. A read failure is
//! [`AutostartState::Unknown`]: the UI grays the item out and reports, it
//! never pretends the setting is off, and no write ever happens on `Unknown`.

use std::path::PathBuf;

use auto_launch::{AutoLaunch, WindowsEnableMode};
use thiserror::Error;

/// Autostart failure with the underlying cause chained.
#[derive(Debug, Error)]
pub enum AutostartError {
    /// The current executable path could not be determined.
    #[error("cannot determine executable path")]
    NoExePath,
    /// Enabling autostart failed.
    #[error("failed to enable autostart")]
    Enable(#[source] auto_launch::Error),
    /// Disabling autostart failed.
    #[error("failed to disable autostart")]
    Disable(#[source] auto_launch::Error),
}

/// Read-back of the autostart switch. `Unknown` carries the reason; the menu
/// renders it grayed and never writes on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutostartState {
    /// Registry value present.
    Enabled,
    /// Registry value explicitly absent.
    Disabled,
    /// Read failed (reason); UI reports, never writes.
    Unknown(String),
}

/// Returns the current executable path, if determinable.
#[must_use]
pub fn get_exe_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Registry value name for autostart, derived from the PascalCase display
/// name. Locked by test: renaming orphans existing installs.
#[must_use]
pub fn autostart_key_name() -> String {
    crate::TOOL_DISPLAY_NAME.to_string()
}

/// Pre-scheme Run-value names that must be removed when the autostart scheme
/// is (re-)applied. Locked by test: the cleanup must not miss a legacy name
/// and must never include the canonical [`autostart_key_name`].
pub const LEGACY_AUTOSTART_KEYS: &[&str] = &["audio-switcher", "Audio Switcher"];

fn autolaunch_for_current_exe() -> Option<AutoLaunch> {
    let exe = get_exe_path()?;
    let exe_str = exe.to_string_lossy().to_string();
    Some(AutoLaunch::new(
        &autostart_key_name(),
        &exe_str,
        WindowsEnableMode::CurrentUser,
        &[] as &[&str],
    ))
}

/// Enable or disable auto-launch at login.
///
/// Cleans up [`LEGACY_AUTOSTART_KEYS`] on success (best effort, failures only
/// logged). Unknown-state callers must not reach here; see
/// [`autostart_state`].
///
/// # Errors
///
/// Returns [`AutostartError`] when the exe path is unknown or the underlying
/// registry operation fails.
pub fn set_autostart(enable: bool) -> Result<(), AutostartError> {
    let auto = autolaunch_for_current_exe().ok_or(AutostartError::NoExePath)?;
    let result = if enable {
        auto.enable().map_err(AutostartError::Enable)
    } else {
        auto.disable().map_err(AutostartError::Disable)
    };
    if result.is_ok() {
        cleanup_legacy_keys();
    }
    result
}

/// Read the current autostart state. A read failure is `Unknown`, never
/// `Disabled`: the caller grays the UI out and must not write.
#[must_use]
pub fn autostart_state() -> AutostartState {
    let Some(auto) = autolaunch_for_current_exe() else {
        return AutostartState::Unknown("exe path unavailable".to_string());
    };
    match auto.is_enabled() {
        Ok(true) => AutostartState::Enabled,
        Ok(false) => AutostartState::Disabled,
        Err(e) => AutostartState::Unknown(e.to_string()),
    }
}

/// Delete pre-scheme Run values. Best effort: failures are logged, never fatal.
fn cleanup_legacy_keys() {
    #[cfg(windows)]
    {
        use windows::Win32::System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
        };
        use windows::core::PCWSTR;

        const RUN_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
        let run_w: Vec<u16> = RUN_KEY.encode_utf16().chain(std::iter::once(0)).collect();
        let mut hkey = HKEY(std::ptr::null_mut());
        // SAFETY: RegOpenKeyExW with a subkey string living through the call;
        // `hkey` is written only on success.
        let opened = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(run_w.as_ptr()),
                None,
                KEY_SET_VALUE,
                &raw mut hkey,
            )
        };
        if opened.is_err() {
            tracing::warn!("legacy autostart cleanup: cannot open Run key");
            return;
        }
        for name in LEGACY_AUTOSTART_KEYS {
            let name_w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: key handle valid, name string alive through the call.
            let status = unsafe { RegDeleteValueW(hkey, PCWSTR(name_w.as_ptr())) };
            if status.is_ok() {
                tracing::debug!("legacy autostart cleanup: removed {name}");
            }
        }
        // SAFETY: balances the successful RegOpenKeyExW above, exactly once.
        unsafe {
            let _ = RegCloseKey(hkey);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_locked() {
        // Canonical value derives from the PascalCase display name; renaming
        // orphans installed Run values.
        assert_eq!(autostart_key_name(), "AudioSwitcher");
        assert_eq!(
            autostart_key_name(),
            crate::display_name_for(crate::TOOL_ID)
        );
        // Legacy predecessors the cleanup must catch — and it must never
        // delete the canonical value itself.
        assert_eq!(LEGACY_AUTOSTART_KEYS, &["audio-switcher", "Audio Switcher"]);
        assert!(!LEGACY_AUTOSTART_KEYS.contains(&autostart_key_name().as_str()));
    }

    #[test]
    fn unknown_never_implies_off() {
        // The type forces callers to handle Unknown distinctly from Disabled.
        let unknown = AutostartState::Unknown("test".to_string());
        assert_ne!(unknown, AutostartState::Disabled);
        assert_ne!(unknown, AutostartState::Enabled);
    }

    #[test]
    fn exe_path_available_in_tests() {
        assert!(get_exe_path().is_some());
    }
}
