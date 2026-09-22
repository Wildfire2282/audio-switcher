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
    Enable(#[source] std::io::Error),
    #[error("failed to disable autostart")]
    Disable(#[source] std::io::Error),
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

/// `HKCU` subkey holding the current-user logon `Run` values.
const RUN_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";

/// `HKCU` subkey where the shell records whether a startup entry is enabled.
///
/// An entry switched off in Task Manager's Startup tab stays in `Run`; this
/// marker is what says it must not start.
const RUN_APPROVED_KEY: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

/// `StartupApproved` value meaning "enabled": `0x02` then eight zero bytes (the
/// disabled form carries a timestamp there instead).
#[cfg(windows)]
const STARTUP_APPROVED_ENABLED: [u8; 12] = [0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

#[must_use]
pub fn get_exe_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// A `WIN32_ERROR` as an IO error: it already renders the OS message, and
/// `AutostartError`'s wording stays the same.
#[cfg(windows)]
fn win32_err(status: windows::Win32::Foundation::WIN32_ERROR) -> std::io::Error {
    std::io::Error::from_raw_os_error(status.0 as i32)
}

/// Whether the status is `ERROR_FILE_NOT_FOUND`, which for our calls means the
/// key or value is simply absent.
#[cfg(windows)]
fn is_absent(status: windows::Win32::Foundation::WIN32_ERROR) -> bool {
    status == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND
}

/// A registry operation that had to succeed.
#[cfg(windows)]
fn require_ok(
    status: windows::Win32::Foundation::WIN32_ERROR,
    map: fn(std::io::Error) -> AutostartError,
) -> Result<(), AutostartError> {
    if status.is_ok() {
        Ok(())
    } else {
        Err(map(win32_err(status)))
    }
}

/// Write the current-user `Run` command and clear the Task Manager override.
///
/// The `Run` value is the quoted executable path: Windows splits an unquoted
/// value at its first space, so `C:\Program Files\...` would never start. The
/// read-back only asks whether a value with our name exists, so an entry
/// written earlier without quotes still counts as installed.
#[cfg(windows)]
fn enable_run_value() -> Result<(), AutostartError> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, REG_SZ, RegCloseKey, RegCreateKeyW, RegSetValueExW,
    };
    use windows::core::PCWSTR;

    let exe = get_exe_path().ok_or(AutostartError::NoExePath)?;
    let command = super::utf16::wide_z(&format!("\"{}\"", exe.display()));
    let value: Vec<u8> = command.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    let name = super::utf16::wide_z(autostart_key_name());
    let subkey = super::utf16::wide_z(RUN_KEY);
    let mut hkey = HKEY(std::ptr::null_mut());
    // SAFETY: the subkey string outlives the call; `hkey` is written only on
    // success and closed below. `RegCreateKeyW` creates the key when it is
    // missing, which is the reason the extended form (and with it the
    // `Win32_Security` feature) is not needed.
    let created =
        unsafe { RegCreateKeyW(HKEY_CURRENT_USER, PCWSTR(subkey.as_ptr()), &raw mut hkey) };
    require_ok(created, AutostartError::Enable)?;
    // SAFETY: `hkey` is open for writing; the value name and its data outlive
    // the call.
    let written =
        unsafe { RegSetValueExW(hkey, PCWSTR(name.as_ptr()), None, REG_SZ, Some(&value)) };
    // SAFETY: closes the handle opened above, exactly once.
    unsafe {
        let _ = RegCloseKey(hkey);
    };
    require_ok(written, AutostartError::Enable)?;

    enable_startup_approved();
    Ok(())
}

/// Clear the shell's "entry disabled" marker for our `Run` value.
///
/// Best effort: `StartupApproved` exists wherever there is a Startup tab, and a
/// missing marker already means enabled. Without this, re-enabling autostart
/// here would leave the entry switched off in the shell while the menu claimed
/// otherwise.
#[cfg(windows)]
fn enable_startup_approved() {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_BINARY, RegCloseKey, RegOpenKeyExW,
        RegSetValueExW,
    };
    use windows::core::PCWSTR;

    let subkey = super::utf16::wide_z(RUN_APPROVED_KEY);
    let name = super::utf16::wide_z(autostart_key_name());
    let mut hkey = HKEY(std::ptr::null_mut());
    // SAFETY: the subkey string outlives the call; `hkey` is written only on
    // success.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_SET_VALUE,
            &raw mut hkey,
        )
    };
    if opened.is_err() {
        tracing::debug!("startup-approved key unavailable ({opened:?}); entry stays enabled");
        return;
    }
    // SAFETY: `hkey` is open for writing; the value name and its data outlive
    // the call.
    let written = unsafe {
        RegSetValueExW(
            hkey,
            PCWSTR(name.as_ptr()),
            None,
            REG_BINARY,
            Some(&STARTUP_APPROVED_ENABLED),
        )
    };
    if written.is_err() {
        tracing::warn!("startup-approved marker not written: {written:?}");
    }
    // SAFETY: closes the handle opened above, exactly once.
    unsafe {
        let _ = RegCloseKey(hkey);
    };
}

/// Remove the current-user `Run` value; an absent key or value is success.
#[cfg(windows)]
fn disable_run_value() -> Result<(), AutostartError> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
    };
    use windows::core::PCWSTR;

    let subkey = super::utf16::wide_z(RUN_KEY);
    let name = super::utf16::wide_z(autostart_key_name());
    let mut hkey = HKEY(std::ptr::null_mut());
    // SAFETY: the subkey string outlives the call; `hkey` is written only on
    // success.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_SET_VALUE,
            &raw mut hkey,
        )
    };
    // Absent: no `Run` value of ours can exist without the key.
    if is_absent(opened) {
        return Ok(());
    }
    require_ok(opened, AutostartError::Disable)?;
    // SAFETY: `hkey` is open; the value name outlives the call.
    let removed = unsafe { RegDeleteValueW(hkey, PCWSTR(name.as_ptr())) };
    // SAFETY: closes the handle opened above, exactly once.
    unsafe {
        let _ = RegCloseKey(hkey);
    };
    // Absent: the caller asked for it to be gone, and it is.
    if is_absent(removed) {
        return Ok(());
    }
    require_ok(removed, AutostartError::Disable)
}

/// Whether the current-user `Run` entry exists and is still enabled.
///
/// `Err` is a read failure: the menu grays the group out rather than guessing,
/// so a key it cannot open must not read as "off".
#[cfg(windows)]
fn run_value_enabled() -> Result<bool, std::io::Error> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    use windows::core::PCWSTR;

    let subkey = super::utf16::wide_z(RUN_KEY);
    let name = super::utf16::wide_z(autostart_key_name());
    let mut hkey = HKEY(std::ptr::null_mut());
    // SAFETY: the subkey string outlives the call; `hkey` is written only on
    // success.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_QUERY_VALUE,
            &raw mut hkey,
        )
    };
    if is_absent(opened) {
        return Ok(false);
    }
    if opened.is_err() {
        return Err(win32_err(opened));
    }
    // SAFETY: `hkey` is open; the value name outlives the call; null data and
    // size pointers ask only "is there a value with this name?".
    let present = unsafe { RegQueryValueExW(hkey, PCWSTR(name.as_ptr()), None, None, None, None) };
    // SAFETY: closes the handle opened above, exactly once.
    unsafe {
        let _ = RegCloseKey(hkey);
    };
    if is_absent(present) {
        return Ok(false);
    }
    if present.is_err() {
        return Err(win32_err(present));
    }
    startup_approved_enabled()
}

/// Whether Task Manager's Startup tab still has our entry switched on.
///
/// A missing key, a missing value, or a value too short to hold the timestamp
/// all read as enabled: that is the shell's own default when there is nothing
/// to remember.
#[cfg(windows)]
fn startup_approved_enabled() -> Result<bool, std::io::Error> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    use windows::core::PCWSTR;

    let subkey = super::utf16::wide_z(RUN_APPROVED_KEY);
    let name = super::utf16::wide_z(autostart_key_name());
    let mut hkey = HKEY(std::ptr::null_mut());
    // SAFETY: the subkey string outlives the call; `hkey` is written only on
    // success.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_QUERY_VALUE,
            &raw mut hkey,
        )
    };
    if is_absent(opened) {
        return Ok(true);
    }
    if opened.is_err() {
        return Err(win32_err(opened));
    }
    let mut len = 0u32;
    // SAFETY: `hkey` is open; a null data pointer with a size pointer asks for
    // the size only.
    let sized = unsafe {
        RegQueryValueExW(
            hkey,
            PCWSTR(name.as_ptr()),
            None,
            None,
            None,
            Some(&raw mut len),
        )
    };
    if is_absent(sized) {
        // SAFETY: closes the handle opened above, exactly once.
        unsafe {
            let _ = RegCloseKey(hkey);
        };
        return Ok(true);
    }
    if sized.is_err() {
        // SAFETY: as above.
        unsafe {
            let _ = RegCloseKey(hkey);
        };
        return Err(win32_err(sized));
    }
    let mut data = vec![0u8; usize::try_from(len).unwrap_or(0)];
    // SAFETY: `data` is exactly `len` bytes; the value name outlives the call.
    let read = unsafe {
        RegQueryValueExW(
            hkey,
            PCWSTR(name.as_ptr()),
            None,
            None,
            Some(data.as_mut_ptr()),
            Some(&raw mut len),
        )
    };
    // SAFETY: closes the handle opened above, exactly once.
    unsafe {
        let _ = RegCloseKey(hkey);
    };
    if read.is_err() {
        return Err(win32_err(read));
    }
    Ok(data
        .last_chunk::<8>()
        .is_none_or(|tail| tail.iter().all(|byte| *byte == 0)))
}

#[cfg(not(windows))]
fn enable_run_value() -> Result<(), AutostartError> {
    Err(AutostartError::Enable(std::io::Error::other(
        "the logon Run value is Windows-only",
    )))
}

#[cfg(not(windows))]
fn disable_run_value() -> Result<(), AutostartError> {
    Err(AutostartError::Disable(std::io::Error::other(
        "the logon Run value is Windows-only",
    )))
}

#[cfg(not(windows))]
fn run_value_enabled() -> Result<bool, std::io::Error> {
    Err(std::io::Error::other("the logon Run value is Windows-only"))
}

/// Apply the current-user `Run` value, then drop the pre-scheme names.
///
/// [`enable_run_value`]/[`disable_run_value`] replace what `auto-launch` did for
/// this one call site: the `Run` command plus the `StartupApproved` marker. The
/// dependency also carried OS detection and a macOS service crate for the two
/// platforms this tool does not build for.
fn set_run_value(enable: bool) -> Result<(), AutostartError> {
    let result = if enable {
        enable_run_value()
    } else {
        disable_run_value()
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
    match run_value_enabled() {
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
