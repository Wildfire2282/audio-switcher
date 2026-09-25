//! Logon scheduled task management (`schtasks.exe`) and elevated helper runner.

use std::sync::atomic::{AtomicU8, Ordering};

use super::{AutostartError, HELPER_ARG_PREFIX, autostart_key_name, get_exe_path};

/// Helper action: create the elevated logon task.
pub(super) const HELPER_TASK_ON: &str = "admin-on";
/// Helper action: delete the elevated logon task.
pub(super) const HELPER_TASK_OFF: &str = "admin-off";

/// Cached presence of the scheduled task: see [`admin_task_cached`].
const TASK_ABSENT: u8 = 0;
const TASK_PRESENT: u8 = 1;
static ADMIN_TASK: AtomicU8 = AtomicU8::new(TASK_ABSENT);

/// Whether the scheduled task is present, per the cache.
pub(super) fn admin_task_cached() -> bool {
    ADMIN_TASK.load(Ordering::Acquire) == TASK_PRESENT
}

/// Mark the task cache state directly (e.g. after successful task operations).
pub(super) fn set_admin_task_cached(present: bool) {
    ADMIN_TASK.store(
        if present { TASK_PRESENT } else { TASK_ABSENT },
        Ordering::Release,
    );
}

/// Query `schtasks` for the task, refreshing the cache.
///
/// Never fails: a query error falls back to the cached state, because the caller
/// is deciding whether to delete something and neither "delete blindly" nor
/// "skip blindly" beats the cache.
pub(super) fn admin_task_live() -> bool {
    match task_exists() {
        Ok(present) => {
            set_admin_task_cached(present);
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
        Ok(present) => set_admin_task_cached(present),
        // The stale cache only mis-reports the tray menu's autostart group, so
        // this is a warning rather than a failure — but a silent one left the
        // group permanently grayed with nothing to read.
        Err(e) => tracing::warn!("scheduled task query failed ({e}); cache unchanged"),
    }
}

/// Delete the scheduled task through one elevated helper instance, unless it is
/// already known to be absent.
pub(super) fn remove_admin_task() -> Result<(), AutostartError> {
    if !admin_task_live() {
        return Ok(());
    }
    run_helper(HELPER_TASK_OFF)?;
    set_admin_task_cached(false);
    Ok(())
}

/// Re-launch ourselves elevated with the helper switch. Blocks until the prompt
/// is answered, so callers run it off the message loop's hot path.
pub(super) fn run_helper(action: &str) -> Result<(), AutostartError> {
    #[cfg(windows)]
    {
        let exe = get_exe_path().ok_or(AutostartError::NoExePath)?;
        let target = crate::platform::utf16::wide_z(&exe.to_string_lossy());
        let params = crate::platform::utf16::wide_z(&format!("{HELPER_ARG_PREFIX}{action}"));
        crate::platform::shell::execute_runas(&target, &params).map_err(|e| match e {
            crate::platform::shell::ShellError::Execute { code, .. } => {
                AutostartError::Elevation { code }
            }
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

/// Create the logon task for the current exe. Runs in the elevated helper.
pub(super) fn create_task() -> Result<(), AutostartError> {
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
pub(super) fn delete_task() -> Result<(), AutostartError> {
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
pub(super) fn task_exists() -> Result<bool, AutostartError> {
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
pub(super) fn describe(out: &std::process::Output) -> String {
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
