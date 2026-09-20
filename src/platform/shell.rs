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
pub(crate) fn open_url(url: &Url) {
    #[cfg(windows)]
    {
        let target = match wide_nul(url.as_str()) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("open_url validation failed: {e}");
                crate::platform::dialog::show_msgbox("Failed to open the link (invalid address).");
                return;
            }
        };
        if let Err(e) = shell_execute(&target, None) {
            tracing::warn!("open_url failed: {e}");
            crate::platform::dialog::show_msgbox("Failed to open the link in the browser.");
        }
    }
    #[cfg(not(windows))]
    {
        let _ = url;
    }
}

/// Open the system volume mixer. `err_msg` is shown when launching fails.
pub(crate) fn open_volume_mixer(err_msg: &str) {
    #[cfg(windows)]
    {
        match wide_nul("SndVol.exe") {
            Ok(target) => {
                if let Err(e) = shell_execute(&target, None) {
                    tracing::warn!("open_volume_mixer failed: {e}");
                    crate::platform::dialog::show_msgbox(err_msg);
                }
            }
            Err(e) => {
                tracing::warn!("open_volume_mixer validation failed: {e}");
                crate::platform::dialog::show_msgbox(err_msg);
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = err_msg;
    }
}

/// Open the system sound settings. `err_msg` is shown when launching fails.
pub(crate) fn open_sound_settings(err_msg: &str) {
    #[cfg(windows)]
    {
        if let (Ok(target), Ok(params)) = (wide_nul("control"), wide_nul("mmsys.cpl")) {
            if let Err(e) = shell_execute(&target, Some(&params)) {
                tracing::warn!("open_sound_settings failed: {e}");
                crate::platform::dialog::show_msgbox(err_msg);
            }
        } else {
            tracing::warn!("open_sound_settings validation failed");
            crate::platform::dialog::show_msgbox(err_msg);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = err_msg;
    }
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
mod tests {
    use super::*;

    #[test]
    fn url_scheme_gate() {
        // https + allowlisted host passes.
        assert!(
            "https://github.com/Wildfire2282/audio-switcher"
                .parse::<Url>()
                .is_ok()
        );
        // Illegal schemes never reach ShellExecuteW.
        assert_eq!("http://github.com/x".parse::<Url>(), Err(UrlError::Scheme));
        assert_eq!("file:///C:/x".parse::<Url>(), Err(UrlError::Scheme));
        assert_eq!("javascript:alert(1)".parse::<Url>(), Err(UrlError::Scheme));
        assert_eq!("".parse::<Url>(), Err(UrlError::Empty));
        // Non-allowlisted hosts are rejected even over https.
        assert_eq!("https://example.com/x".parse::<Url>(), Err(UrlError::Host));
        assert_eq!(
            "https://github.com.evil.com/x".parse::<Url>(),
            Err(UrlError::Host)
        );
    }
}
