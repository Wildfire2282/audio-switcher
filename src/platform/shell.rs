//! External navigation and system-tool launching.
//!
//! [`open_url`] is the only external-link path: it takes a validated [`Url`]
//! (`https` + publishing-domain allowlist), runs `ShellExecuteW` with
//! verb `open` and an explicit working directory, and dialogs on failure.
//! Local system tools (`open_volume_mixer`/`open_sound_settings`) share the
//! same `ShellExecuteW` core with typed errors. Contract: `ui` and `app` may
//! call these; shell owns no menu or config state.

use std::str::FromStr;

use thiserror::Error;

/// Hosts allowed for external navigation (publishing domains only).
pub const URL_ALLOWLIST: &[&str] = &["github.com"];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum UrlError {
    /// Scheme is not `https`.
    #[error("URL scheme must be https")]
    Scheme,
    /// Host is missing or not on the allowlist.
    #[error("URL host is not allowlisted")]
    Host,
    /// The URL has no usable content.
    #[error("URL is empty")]
    Empty,
}

/// Validated external URL: `https` scheme plus allowlisted host.
///
/// Parse once via [`FromStr`]; illegal schemes never reach `ShellExecuteW`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url(String);

impl Url {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Url {
    type Err = UrlError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(UrlError::Empty);
        }
        let rest = raw.strip_prefix("https://").ok_or(UrlError::Scheme)?;
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let host = host
            .strip_prefix("www.")
            .unwrap_or(host)
            .to_ascii_lowercase();
        if URL_ALLOWLIST.contains(&host.as_str()) {
            Ok(Self(raw.to_string()))
        } else {
            Err(UrlError::Host)
        }
    }
}

/// Shell execution failure with the verb/target preserved for diagnostics.
#[derive(Debug, Error)]
pub enum ShellError {
    /// Value contains an interior NUL and cannot become a wide string.
    #[error("shell target contains interior NUL")]
    InteriorNul,
    /// `ShellExecuteW` returned a value `<= 32`.
    #[error("ShellExecuteW failed for {target} (code {code})")]
    Execute {
        target: String,
        /// Raw `ShellExecuteW` return code.
        code: usize,
    },
}

#[cfg(windows)]
fn shell_execute_with_verb(
    verb: &str,
    target_w: &[u16],
    params_w: Option<&[u16]>,
) -> Result<(), ShellError> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::PCWSTR;
    let op: Vec<u16> = verb.encode_utf16().chain(std::iter::once(0)).collect();
    let dir_w = working_dir_wide();
    // SAFETY: ShellExecuteW with null-terminated buffers alive through the call.
    unsafe {
        let res = ShellExecuteW(
            None,
            PCWSTR(op.as_ptr()),
            PCWSTR(target_w.as_ptr()),
            params_w.map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr())),
            PCWSTR(dir_w.as_ptr()),
            SW_SHOWNORMAL,
        );
        let code = res.0 as usize;
        if code <= 32 {
            Err(ShellError::Execute {
                target: String::from_utf16_lossy(target_w),
                code,
            })
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
fn shell_execute(target_w: &[u16], params_w: Option<&[u16]>) -> Result<(), ShellError> {
    shell_execute_with_verb("open", target_w, params_w)
}

/// Launch `target` through the `runas` verb to obtain an elevated child.
///
/// Windows raises one consent prompt unless this process already runs elevated.
/// The call blocks until that prompt is answered, so callers stay off the
/// message loop; the child itself is *not* waited for. A return code `<= 32` is
/// the failure channel (a declined prompt included).
///
/// # Errors
///
/// [`ShellError::Execute`] with the raw `ShellExecuteW` code.
#[cfg(windows)]
pub(crate) fn execute_runas(target_w: &[u16], params_w: &[u16]) -> Result<(), ShellError> {
    shell_execute_with_verb("runas", target_w, Some(params_w))
}

/// Non-Windows stub: elevation has no equivalent here.
#[cfg(not(windows))]
pub(crate) fn execute_runas(target_w: &[u16], params_w: &[u16]) -> Result<(), ShellError> {
    let _ = params_w;
    Err(ShellError::Execute {
        target: String::from_utf16_lossy(target_w),
        code: 0,
    })
}

/// Explicit working directory for `ShellExecuteW`: the exe parent, falling
/// back to the system root when the exe path is unknown.
#[cfg(windows)]
fn working_dir_wide() -> Vec<u16> {
    let dir = crate::platform::autostart::get_exe_path()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        .map_or_else(|| r"C:\".to_string(), |p| p.to_string_lossy().to_string());
    super::utf16::wide_z(&dir)
}

#[cfg(windows)]
fn wide_nul(s: &str) -> Result<Vec<u16>, ShellError> {
    if s.contains('\0') {
        return Err(ShellError::InteriorNul);
    }
    Ok(super::utf16::wide_z(s))
}

/// Open a validated external URL. Failures surface a dialog (never swallowed).
///
/// `err_msg` is the caller's, like the other openers here: the wording follows
/// the UI language, which this layer does not own.
pub(crate) fn open_url(url: &Url, err_msg: &str) {
    #[cfg(windows)]
    {
        let target = match wide_nul(url.as_str()) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("open_url validation failed: {e}");
                crate::platform::dialog::show_msgbox(err_msg);
                return;
            }
        };
        if let Err(e) = shell_execute(&target, None) {
            tracing::warn!("open_url failed: {e}");
            crate::platform::dialog::show_msgbox(err_msg);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (url, err_msg);
    }
}

/// Launch a system tool through the shell and centre the dialog it opens.
///
/// Both tools open at a remembered or default position (the volume mixer lands
/// at `0,0`), which is a corner on a large display; centring them matches the
/// app's own dialogs. Failures surface `err_msg`.
#[cfg(windows)]
fn open_system_tool(target: &str, params: Option<&str>, err_msg: &str, what: &str) {
    let (Ok(target_w), Ok(params_w)) = (wide_nul(target), params.map(wide_nul).transpose()) else {
        tracing::warn!("{what} validation failed");
        crate::platform::dialog::show_msgbox(err_msg);
        return;
    };
    // Taken before the launch, so the dialog the launch produces is the one that
    // was not there yet.
    let before = crate::platform::dialog::dialogs_on_screen();
    if let Err(e) = shell_execute(&target_w, params_w.as_deref()) {
        tracing::warn!("{what} failed: {e}");
        crate::platform::dialog::show_msgbox(err_msg);
        return;
    }
    crate::platform::dialog::center_new_dialog(before);
}

/// Open the system volume mixer. `err_msg` is shown when launching fails.
pub(crate) fn open_volume_mixer(err_msg: &str) {
    #[cfg(windows)]
    open_system_tool("SndVol.exe", None, err_msg, "open_volume_mixer");
    #[cfg(not(windows))]
    let _ = err_msg;
}

/// Open the system sound settings. `err_msg` is shown when launching fails.
pub(crate) fn open_sound_settings(err_msg: &str) {
    #[cfg(windows)]
    open_system_tool("control", Some("mmsys.cpl"), err_msg, "open_sound_settings");
    #[cfg(not(windows))]
    let _ = err_msg;
}

/// Open a folder in Explorer. `err_msg` is shown when launching fails.
///
/// The folder is created first so opening the config folder never fails on a
/// fresh install. Contract: callers pass the folder; shell owns no config state.
pub(crate) fn open_folder(dir: &std::path::Path, err_msg: &str) {
    #[cfg(windows)]
    {
        if let Err(e) = std::fs::create_dir_all(dir) {
            tracing::warn!("open_folder create_dir_all failed: {e}");
            crate::platform::dialog::show_msgbox(err_msg);
            return;
        }
        match wide_nul(&dir.to_string_lossy()) {
            Ok(target) => {
                if let Err(e) = shell_execute(&target, None) {
                    tracing::warn!("open_folder failed: {e}");
                    crate::platform::dialog::show_msgbox(err_msg);
                }
            }
            Err(e) => {
                tracing::warn!("open_folder validation failed: {e}");
                crate::platform::dialog::show_msgbox(err_msg);
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = dir;
        let _ = err_msg;
    }
}

#[cfg(test)]
mod tests;
