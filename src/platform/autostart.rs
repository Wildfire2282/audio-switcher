//! Auto-launch (autostart) helpers.
//!
//! [`AutostartMode`] picks the mechanism: none, the current-user `Run` value, or
//! a logon scheduled task with highest privileges.
//!
//! The elevated mode exists because of UIPI: while a HIGH-integrity process owns
//! the foreground window (Tencent Androws does, permanently), Windows stops
//! delivering mouse input to a medium-integrity low-level hook, so hover-wheel
//! goes dead over that window. An equally elevated process keeps receiving it.
//!
//! Writing that task needs elevation: `schtasks /Create /RL HIGHEST` from a
//! medium-integrity process fails with "Access is denied". The menu path
//! therefore re-launches this exe through `ShellExecuteW(.., "runas", ..)` with
//! [`HELPER_ARG_PREFIX`], and that child — which starts before the
//! single-instance guard — performs the write. One consent prompt per switch; an
//! already-elevated process sees none. (An earlier revision believed no
//! elevation was needed and deleted the helper; that measurement was taken in an
//! already-elevated shell, which hid the requirement.)
//!
//! Read failures are [`AutostartState::Unknown`]: the UI grays the group out, it
//! never pretends the setting is off, and no write happens on `Unknown`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};

use auto_launch::{AutoLaunch, WindowsEnableMode};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Which autostart mechanism the user picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutostartMode {
    Off,
    /// Current-user `Run` value; starts at the caller's integrity level.
    #[default]
    User,
    /// Logon scheduled task with highest privileges, so input aimed at elevated
    /// foreground windows keeps reaching our low-level mouse hook.
    Admin,
}

/// Autostart failure. The `#[error]` text is the user-visible message.
#[derive(Debug, Error)]
pub enum AutostartError {
    #[error("cannot determine executable path")]
    NoExePath,
    #[error("failed to enable autostart")]
    Enable(#[source] auto_launch::Error),
    #[error("failed to disable autostart")]
    Disable(#[source] auto_launch::Error),
    /// The elevated helper never started: the user declined the consent prompt
    /// or the launch itself failed (`ShellExecuteW` returned `<= 32`).
    #[error("elevation declined or unavailable (code {code})")]
    Elevation {
        /// Raw `ShellExecuteW` return code.
        code: usize,
    },
    #[error("scheduled task {action} failed: {detail}")]
    Task {
        /// One of `create`, `delete`, `query`.
        action: &'static str,
        /// Spawn error or exit code, plus what the tool printed.
        detail: String,
    },
}

/// Read-back of the autostart setting; the menu renders it and never writes on
/// `Unknown`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutostartState {
    Off,
    User,
    Admin,
    /// Read failed (reason); UI reports, never writes.
    Unknown(String),
}

impl AutostartState {
    /// The installed mode, or `None` when the read failed.
    ///
    /// The single place the read-back becomes a mode: `ui` renders `None` as the
    /// grayed group and never guesses one.
    #[must_use]
    pub fn mode(&self) -> Option<AutostartMode> {
        match self {
            Self::Off => Some(AutostartMode::Off),
            Self::User => Some(AutostartMode::User),
            Self::Admin => Some(AutostartMode::Admin),
            Self::Unknown(_) => None,
        }
    }
}

/// `Run` value name and scheduled-task name, both derived from the PascalCase
/// display name. Locked by test: renaming orphans existing installs.
///
/// Borrowed, not owned: [`autostart_state`] runs on every menu refresh and the
/// name is a `'static` constant, so allocating here would be pure churn.
#[must_use]
pub fn autostart_key_name() -> &'static str {
    crate::TOOL_DISPLAY_NAME
}

/// Pre-scheme Run-value names that must be removed when the autostart scheme is
/// (re-)applied. Locked by test: the cleanup must not miss a legacy name and
/// must never include the canonical [`autostart_key_name`].
pub const LEGACY_AUTOSTART_KEYS: &[&str] = &["audio-switcher", "Audio Switcher"];

/// Command-line switch that turns this process into the elevated helper.
pub const HELPER_ARG_PREFIX: &str = "--autostart-helper=";

/// Helper action: create the elevated logon task.
const HELPER_TASK_ON: &str = "admin-on";
/// Helper action: delete the elevated logon task.
const HELPER_TASK_OFF: &str = "admin-off";

/// Cached presence of the scheduled task: see [`admin_task_cached`].
const TASK_ABSENT: u8 = 0;
const TASK_PRESENT: u8 = 1;
static ADMIN_TASK: AtomicU8 = AtomicU8::new(TASK_ABSENT);

#[must_use]
pub fn get_exe_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

fn autolaunch_for_current_exe() -> Option<AutoLaunch> {
    let exe = get_exe_path()?;
    // `Cow` avoids an allocation when the path is valid UTF-8 (the usual case).
    let exe_str = exe.to_string_lossy();
    Some(AutoLaunch::new(
        autostart_key_name(),
        &exe_str,
        WindowsEnableMode::CurrentUser,
        &[] as &[&str],
    ))
}

/// Cleans up [`LEGACY_AUTOSTART_KEYS`] on success (best effort, failures only
/// logged).
fn set_run_value(enable: bool) -> Result<(), AutostartError> {
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

/// The scheduled-task half comes from a cache (see [`refresh_admin_task_cache`])
/// because `schtasks` costs a process launch and this runs on every menu refresh.
/// A read failure is `Unknown`, never `Off`: the caller grays the UI out and must
/// not write.
#[must_use]
pub fn autostart_state() -> AutostartState {
    let Some(auto) = autolaunch_for_current_exe() else {
        return AutostartState::Unknown("exe path unavailable".to_string());
    };
    match auto.is_enabled() {
        Ok(true) => AutostartState::User,
        Ok(false) => {
            if admin_task_cached() {
                AutostartState::Admin
            } else {
                AutostartState::Off
            }
        }
        Err(e) => AutostartState::Unknown(e.to_string()),
    }
}

/// Apply `mode`: clear the mechanism `mode` does not use, then set the one it
/// does.
///
/// Switching into or out of [`AutostartMode::Admin`] raises one consent prompt
/// (see the module docs). The helper is launched asynchronously, so a successful
/// return means "the helper started", not "the task exists" — the cache is
/// updated optimistically and the next startup query corrects it.
///
/// # Errors
///
/// [`AutostartError`] when the exe path is unknown, a registry write fails,
/// elevation was declined, or `schtasks` fails inside the helper.
pub fn set_autostart_mode(mode: AutostartMode) -> Result<(), AutostartError> {
    match mode {
        AutostartMode::Off => {
            remove_admin_task()?;
            set_run_value(false)
        }
        AutostartMode::User => {
            remove_admin_task()?;
            set_run_value(true)
        }
        AutostartMode::Admin => {
            // Create first: a failure here must leave the existing `Run` entry
            // alone rather than dropping the user to "no autostart at all".
            run_helper(HELPER_TASK_ON)?;
            ADMIN_TASK.store(TASK_PRESENT, Ordering::Release);
            set_run_value(false)
        }
    }
}

/// Delete the scheduled task through one elevated helper instance, unless it is
/// already known to be absent.
fn remove_admin_task() -> Result<(), AutostartError> {
    if !admin_task_live() {
        return Ok(());
    }
    run_helper(HELPER_TASK_OFF)?;
    ADMIN_TASK.store(TASK_ABSENT, Ordering::Release);
    Ok(())
}

/// Re-launch ourselves elevated with the helper switch. Blocks until the prompt
/// is answered, so callers run it off the message loop's hot path.
fn run_helper(action: &str) -> Result<(), AutostartError> {
    #[cfg(windows)]
    {
        let exe = get_exe_path().ok_or(AutostartError::NoExePath)?;
        let target = super::utf16::wide_z(&exe.to_string_lossy());
        let params = super::utf16::wide_z(&format!("{HELPER_ARG_PREFIX}{action}"));
        super::shell::execute_runas(&target, &params).map_err(|e| match e {
            super::shell::ShellError::Execute { code, .. } => AutostartError::Elevation { code },
            _ => AutostartError::NoExePath,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = action;
        Err(AutostartError::Task {
            action: "helper",
            detail: "unsupported platform".to_string(),
        })
    }
}

/// Handle the elevated-helper invocation, when this process is one.
///
/// Returns `Some(exit_code)` when the process was started with the helper switch
/// and must exit without touching the tray, the single-instance guard or COM.
/// Returns `None` for a normal start.
#[must_use]
pub fn run_autostart_helper_if_requested() -> Option<i32> {
    let action =
        std::env::args().find_map(|arg| arg.strip_prefix(HELPER_ARG_PREFIX).map(str::to_string))?;
    let outcome = match action.as_str() {
        HELPER_TASK_ON => create_task(),
        HELPER_TASK_OFF => delete_task(),
        other => {
            tracing::warn!("unknown autostart helper action: {other}");
            return Some(2);
        }
    };
    match outcome {
        Ok(()) => Some(0),
        Err(e) => {
            // The dialog is the only channel back to the user: this child has no
            // console and the parent does not wait for it.
            tracing::warn!("autostart helper failed: {e}");
            super::dialog::show_msgbox(&format!(
                "{} could not change the startup task.\n\n{e}",
                crate::TOOL_DISPLAY_NAME
            ));
            Some(1)
        }
    }
}

/// Whether the scheduled task is present, per the cache.
fn admin_task_cached() -> bool {
    ADMIN_TASK.load(Ordering::Acquire) == TASK_PRESENT
}

/// Query `schtasks` for the task, refreshing the cache.
///
/// Never fails: a query error falls back to the cached state, because the caller
/// is deciding whether to delete something and neither "delete blindly" nor
/// "skip blindly" beats the cache.
fn admin_task_live() -> bool {
    match task_exists() {
        Ok(present) => {
            ADMIN_TASK.store(
                if present { TASK_PRESENT } else { TASK_ABSENT },
                Ordering::Release,
            );
            present
        }
        Err(e) => {
            tracing::warn!("scheduled task query failed ({e}); using cached state");
            admin_task_cached()
        }
    }
}

/// Refresh the scheduled-task cache. Called once at startup from a worker
/// thread; `schtasks` is too slow for the menu's refresh path.
pub fn refresh_admin_task_cache() {
    match task_exists() {
        Ok(present) => ADMIN_TASK.store(
            if present { TASK_PRESENT } else { TASK_ABSENT },
            Ordering::Release,
        ),
        // The stale cache only mis-reports the tray menu's autostart group, so
        // this is a warning rather than a failure — but a silent one left the
        // group permanently grayed with nothing to read.
        Err(e) => tracing::warn!("scheduled task query failed ({e}); cache unchanged"),
    }
}

/// Create the logon task for the current exe. Runs in the elevated helper.
fn create_task() -> Result<(), AutostartError> {
    let exe = get_exe_path().ok_or(AutostartError::NoExePath)?;
    let name = autostart_key_name();
    // `/TR` takes a command line, so a path with spaces must carry its own
    // quotes through to `schtasks`; without them the tool splits the path at the
    // first space. Verified against a spaced path.
    let target = format!("\"{}\"", exe.to_string_lossy());
    let out = run_schtasks(&[
        "/Create", "/F", "/TN", name, "/TR", &target, "/SC", "ONLOGON", "/RL", "HIGHEST",
    ])
    .map_err(|e| AutostartError::Task {
        action: "create",
        detail: e.to_string(),
    })?;
    if out.status.success() {
        Ok(())
    } else {
        Err(AutostartError::Task {
            action: "create",
            detail: describe(&out),
        })
    }
}

/// Delete the logon task by name. Runs in the elevated helper.
fn delete_task() -> Result<(), AutostartError> {
    let name = autostart_key_name();
    let out = run_schtasks(&["/Delete", "/F", "/TN", name]).map_err(|e| AutostartError::Task {
        action: "delete",
        detail: e.to_string(),
    })?;
    if out.status.success() {
        Ok(())
    } else {
        Err(AutostartError::Task {
            action: "delete",
            detail: describe(&out),
        })
    }
}

/// Whether the logon task exists.
fn task_exists() -> Result<bool, AutostartError> {
    let name = autostart_key_name();
    run_schtasks(&["/Query", "/TN", name])
        .map(|out| out.status.success())
        .map_err(|e| AutostartError::Task {
            action: "query",
            detail: e.to_string(),
        })
}

/// Failure detail for [`AutostartError::Task`]: the exit code plus whatever
/// `schtasks` printed. The text is localized and never parsed, but discarding it
/// made a real failure undiagnosable — the user saw only `exit code: 1`.
fn describe(out: &std::process::Output) -> String {
    let raw = if out.stderr.is_empty() {
        &out.stdout
    } else {
        &out.stderr
    };
    let text = String::from_utf8_lossy(raw);
    let text = text.trim();
    let code = out.status.code().map_or_else(
        || "terminated without an exit code".to_string(),
        |c| format!("exit code {c}"),
    );
    if text.is_empty() {
        code
    } else {
        format!("{code}: {text}")
    }
}

/// Run `schtasks` with no console window, capturing its output.
#[cfg(windows)]
fn run_schtasks(args: &[&str]) -> std::io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;
    /// Keep the console app from flashing a window over the tray.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("schtasks.exe")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
}

#[cfg(not(windows))]
fn run_schtasks(_args: &[&str]) -> std::io::Result<std::process::Output> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "schtasks is Windows-only",
    ))
}

/// Best effort: failures are logged, never fatal.
fn cleanup_legacy_keys() {
    #[cfg(windows)]
    {
        use super::utf16::wide_z;
        use windows::Win32::System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
        };
        use windows::core::PCWSTR;

        const RUN_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
        let run_w = wide_z(RUN_KEY);
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
            let name_w = wide_z(name);
            // SAFETY: key handle valid, name string alive through the call.
            let status = unsafe { RegDeleteValueW(hkey, PCWSTR(name_w.as_ptr())) };
            if status.is_ok() {
                tracing::debug!("legacy autostart cleanup: removed {name}");
            } else {
                // Usually "no such value", which is the normal case.
                tracing::debug!("legacy autostart cleanup: {name} not removed ({status:?})");
            }
        }
        // SAFETY: balances the successful RegOpenKeyExW above, exactly once.
        unsafe {
            let _ = RegCloseKey(hkey);
        }
    }
}

#[cfg(test)]
mod tests;
