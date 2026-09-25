//! Current-user logon `Run` value and `StartupApproved` management.

use super::{AutostartError, autostart_key_name, get_exe_path};

/// Pre-scheme Run-value names that must be removed when the autostart scheme is
/// (re-)applied. Locked by test: the cleanup must not miss a legacy name and
/// must never include the canonical [`autostart_key_name`].
pub const LEGACY_AUTOSTART_KEYS: &[&str] = &["audio-switcher", "Audio Switcher"];

/// `HKCU` subkey holding the current-user logon `Run` values.
pub(super) const RUN_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";

/// `HKCU` subkey where the shell records whether a startup entry is enabled.
///
/// An entry switched off in Task Manager's Startup tab stays in `Run`; this
/// marker is what says it must not start.
pub(super) const RUN_APPROVED_KEY: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

/// `StartupApproved` value meaning "enabled": the state byte `0x02` followed by
/// a zeroed timestamp, which is what the shell writes for an entry it has never
/// been asked to disable.
#[cfg(windows)]
pub(super) const STARTUP_APPROVED_ENABLED: [u8; 12] = [0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

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
pub(super) fn enable_run_value() -> Result<(), AutostartError> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, REG_SZ, RegCloseKey, RegCreateKeyW, RegSetValueExW,
    };
    use windows::core::PCWSTR;

    let exe = get_exe_path().ok_or(AutostartError::NoExePath)?;
    let command = crate::platform::utf16::wide_z(&format!("\"{}\"", exe.display()));
    let value: Vec<u8> = command.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    let name = crate::platform::utf16::wide_z(autostart_key_name());
    let subkey = crate::platform::utf16::wide_z(RUN_KEY);
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

    let subkey = crate::platform::utf16::wide_z(RUN_APPROVED_KEY);
    let name = crate::platform::utf16::wide_z(autostart_key_name());
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
pub(super) fn disable_run_value() -> Result<(), AutostartError> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
    };
    use windows::core::PCWSTR;

    let subkey = crate::platform::utf16::wide_z(RUN_KEY);
    let name = crate::platform::utf16::wide_z(autostart_key_name());
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

/// State of the current-user `Run` entry.
///
/// `Disabled` is not `Absent`: the value is there, with the shell's own marker
/// saying it must not start. The difference decides whether the startup
/// self-heal may write — rewriting an entry the user switched off in Task
/// Manager's Startup tab would undo that choice on every launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RunEntry {
    /// No value of ours: autostart was never installed here.
    Absent,
    /// Present and the shell still starts it.
    Enabled,
    /// Present, switched off in the shell's Startup tab.
    Disabled,
}

/// The current-user `Run` entry's state, presence and shell marker together.
///
/// `Err` is a read failure: the menu grays the group out rather than guessing,
/// so a key it cannot open must not read as "off".
#[cfg(windows)]
pub(super) fn run_value_state() -> Result<RunEntry, std::io::Error> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    use windows::core::PCWSTR;

    let subkey = crate::platform::utf16::wide_z(RUN_KEY);
    let name = crate::platform::utf16::wide_z(autostart_key_name());
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
        return Ok(RunEntry::Absent);
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
        return Ok(RunEntry::Absent);
    }
    if present.is_err() {
        return Err(win32_err(present));
    }
    Ok(if startup_approved_enabled()? {
        RunEntry::Enabled
    } else {
        RunEntry::Disabled
    })
}

/// Whether Task Manager's Startup tab still has our entry switched on.
///
/// A missing key, a missing value, or a value with no state byte all read as
/// enabled: that is the shell's own default when there is nothing to remember.
#[cfg(windows)]
fn startup_approved_enabled() -> Result<bool, std::io::Error> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    use windows::core::PCWSTR;

    let subkey = crate::platform::utf16::wide_z(RUN_APPROVED_KEY);
    let name = crate::platform::utf16::wide_z(autostart_key_name());
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
    Ok(startup_approved_state(&data))
}

/// Whether a `StartupApproved` value means "enabled".
///
/// The state is the *first* byte of the value: odd marks the entry disabled by
/// the user (`0x03`), even marks it enabled (`0x02`, `0x06`). The eight bytes
/// after it are the timestamp of that change, which the shell updates when an
/// entry is switched back on — so a rule that read the timestamp reported an
/// entry the user had just re-enabled as still disabled.
///
/// An empty or truncated value counts as enabled, which is the shell's own
/// default when there is nothing to remember.
#[must_use]
pub(super) fn startup_approved_state(data: &[u8]) -> bool {
    data.first().is_none_or(|state| (*state & 1) == 0)
}

#[cfg(not(windows))]
pub(super) fn enable_run_value() -> Result<(), AutostartError> {
    Err(AutostartError::Enable(std::io::Error::other(
        "the logon Run value is Windows-only",
    )))
}

#[cfg(not(windows))]
pub(super) fn disable_run_value() -> Result<(), AutostartError> {
    Err(AutostartError::Disable(std::io::Error::other(
        "the logon Run value is Windows-only",
    )))
}

#[cfg(not(windows))]
pub(super) fn run_value_state() -> Result<RunEntry, std::io::Error> {
    Err(std::io::Error::other("the logon Run value is Windows-only"))
}

/// Apply the current-user `Run` value, then drop the pre-scheme names.
///
/// [`enable_run_value`]/[`disable_run_value`] replace what `auto-launch` did for
/// this one call site: the `Run` command plus the `StartupApproved` marker. The
/// dependency also carried OS detection and a macOS service crate for the two
/// platforms this tool does not build for.
pub(super) fn set_run_value(enable: bool) -> Result<(), AutostartError> {
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

/// Best effort: failures are logged, never fatal.
pub(super) fn cleanup_legacy_keys() {
    #[cfg(windows)]
    {
        use windows::Win32::System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
        };
        use windows::core::PCWSTR;

        let run_w = crate::platform::utf16::wide_z(RUN_KEY);
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
            let name_w = crate::platform::utf16::wide_z(name);
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
