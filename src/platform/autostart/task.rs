//! Logon scheduled task management (`schtasks.exe`) and elevated helper runner.

use std::sync::atomic::{AtomicU8, Ordering};

use super::{AutostartError, HELPER_ARG_PREFIX, autostart_key_name, get_exe_path};

/// Helper action: create the elevated logon task.
pub(super) const HELPER_TASK_ON: &str = "admin-on";
/// Helper action: delete the elevated logon task.
pub(super) const HELPER_TASK_OFF: &str = "admin-off";

/// No successful query has established the task's current presence.
const TASK_UNKNOWN: u8 = 0;
const TASK_ABSENT: u8 = 1;
const TASK_PRESENT: u8 = 2;
static ADMIN_TASK: AtomicU8 = AtomicU8::new(TASK_UNKNOWN);

/// Cached task presence; a failed query invalidates the last observation.
pub(super) fn admin_task_cached() -> Option<bool> {
    match ADMIN_TASK.load(Ordering::Acquire) {
        TASK_ABSENT => Some(false),
        TASK_PRESENT => Some(true),
        _ => None,
    }
}

pub(super) fn set_admin_task_cached(present: Option<bool>) {
    ADMIN_TASK.store(
        match present {
            Some(false) => TASK_ABSENT,
            Some(true) => TASK_PRESENT,
            None => TASK_UNKNOWN,
        },
        Ordering::Release,
    );
}

/// How long the parent waits for the elevated helper's task write to land.
///
/// The helper is a second process: `ShellExecuteW` only reports that it started,
/// so the task's presence is the success signal the parent can actually see.
const TASK_CONFIRM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Poll interval while confirming; every poll spawns `schtasks`.
const TASK_CONFIRM_POLL: std::time::Duration = std::time::Duration::from_millis(250);

/// Confirm the helper's write using successful live queries only.
pub(super) fn wait_for_task(present: bool) -> Result<(), AutostartError> {
    let deadline = std::time::Instant::now() + TASK_CONFIRM_TIMEOUT;
    loop {
        if admin_task_live()? == present {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            set_admin_task_cached(None);
            return Err(AutostartError::Task {
                action: if present { "create" } else { "delete" },
                detail: "the elevated helper's task change was not confirmed".to_string(),
            });
        }
        std::thread::sleep(TASK_CONFIRM_POLL);
    }
}

/// Refresh the cache without turning a failed live query into write permission.
pub(super) fn admin_task_live() -> Result<bool, AutostartError> {
    let result = task_exists();
    set_admin_task_cached(result.as_ref().ok().copied());
    result
}

/// Refresh the scheduled-task cache.
///
/// Runs once at startup, on the message-loop thread before the loop starts:
/// `schtasks` costs a process launch, which is why the menu reads this cache
/// instead of querying. Callers must not add it to a per-frame path.
pub(crate) fn refresh_admin_task_cache() {
    if let Err(e) = admin_task_live() {
        tracing::warn!("scheduled task query failed ({e}); state unknown");
    }
}

/// Delete the scheduled task through one elevated helper instance, unless it is
/// already known to be absent.
pub(super) fn remove_admin_task() -> Result<(), AutostartError> {
    if !admin_task_live()? {
        return Ok(());
    }
    run_helper(HELPER_TASK_OFF)?;
    set_admin_task_cached(None);
    wait_for_task(false)?;
    Ok(())
}

/// Re-launch ourselves elevated with the helper switch. Blocks until the prompt
/// is answered, so callers run it off the message loop's hot path.
pub(super) fn run_helper(action: &str) -> Result<(), AutostartError> {
    #[cfg(test)]
    if let Some(result) = super::tests::helper_result(action) {
        return result;
    }
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

/// Query one task with locale-independent HRESULT failure classification.
pub(super) fn task_exists() -> Result<bool, AutostartError> {
    #[cfg(test)]
    if let Some(result) = super::tests::task_query_result() {
        return result;
    }
    let out = run_schtasks(&[
        "/Query",
        "/TN",
        autostart_key_name(),
        "/FO",
        "CSV",
        "/NH",
        "/HRESULT",
    ])
    .map_err(|e| AutostartError::Task {
        action: "query",
        detail: e.to_string(),
    })?;
    classify_query_result(&out, out.status.code())
}

pub(super) fn classify_query_result(
    out: &std::process::Output,
    code: Option<i32>,
) -> Result<bool, AutostartError> {
    // /HRESULT preserves ERROR_FILE_NOT_FOUND instead of collapsing all errors
    // to exit code 1; access denied and unavailable services remain errors.
    const TASK_NOT_FOUND: i32 = 0x8007_0002u32 as i32;
    if code == Some(TASK_NOT_FOUND) {
        return Ok(false);
    }
    if code != Some(0) || !out.stderr.is_empty() {
        return Err(AutostartError::Task {
            action: "query",
            detail: describe(out),
        });
    }
    if !valid_task_rows(&out.stdout) {
        return Err(AutostartError::Task {
            action: "query",
            detail: "unexpected schtasks CSV response".to_string(),
        });
    }
    Ok(true)
}

/// /FO CSV /NH has three quoted columns; localized field bytes stay opaque.
fn valid_task_rows(stdout: &[u8]) -> bool {
    let mut found = false;
    for row in stdout.split(|&byte| byte == b'\n') {
        let row = row.strip_suffix(b"\r").unwrap_or(row);
        if row.is_empty() {
            continue;
        }
        let Some((name, rest)) = csv_field(row) else {
            return false;
        };
        if !name
            .strip_prefix(b"\\")
            .is_some_and(|name| name.eq_ignore_ascii_case(autostart_key_name().as_bytes()))
        {
            return false;
        }
        let Some((_, rest)) = rest.strip_prefix(b",").and_then(csv_field) else {
            return false;
        };
        let Some((_, rest)) = rest.strip_prefix(b",").and_then(csv_field) else {
            return false;
        };
        if !rest.is_empty() {
            return false;
        }
        found = true;
    }
    found
}

fn csv_field(input: &[u8]) -> Option<(&[u8], &[u8])> {
    let input = input.strip_prefix(b"\"")?;
    let mut index = 0;
    while index < input.len() {
        if input[index] == b'"' {
            if input.get(index + 1) != Some(&b'"') {
                return Some((&input[..index], &input[index + 1..]));
            }
            index += 1;
        }
        index += 1;
    }
    None
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
        // `/HRESULT` failures arrive as negative HRESULT values; hex is the
        // form every Windows diagnostic uses for them.
        |c| {
            if c < 0 {
                format!("exit code 0x{:08X}", c as u32)
            } else {
                format!("exit code {c}")
            }
        },
    );
    if text.is_empty() {
        code
    } else {
        format!("{code}: {text}")
    }
}

/// Run `schtasks` with no console window, capturing its output.
///
/// Resolved under `System32` rather than by bare name: `CreateProcessW` searches
/// this process's own (writable, portable) folder first, and the elevated helper
/// would run a planted `schtasks.exe` with administrator rights.
#[cfg(windows)]
fn run_schtasks(args: &[&str]) -> std::io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;
    /// Keep the console app from flashing a window over the tray.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = crate::platform::shell::system32_exe("schtasks.exe").ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "cannot resolve %SystemRoot%\\System32",
        )
    })?;
    std::process::Command::new(exe)
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
