//! One-shot system locale read for `Lang::System`.
//!
//! The value is resolved once at startup; live `intl` re-resolution is
//! deferred, so this module owns no listener and no cache.

/// Read the user locale name (e.g. `"zh-CN"`, `"en-US"`).
///
/// Returns an empty string when the read fails; the caller falls back to
/// English and logs. Never panics.
#[must_use]
pub(crate) fn system_locale_name() -> String {
    #[cfg(windows)]
    {
        use windows::Win32::Globalization::GetUserDefaultLocaleName;
        // LOCALE_NAME_MAX_LENGTH (85) including the NUL.
        let mut buf = [0u16; 85];
        // SAFETY: buffer sized for the API contract, alive through the call.
        let written = unsafe { GetUserDefaultLocaleName(&mut buf) };
        if written <= 0 {
            return String::new();
        }
        let len = (written as usize).min(buf.len());
        let slice = &buf[..len];
        let end = slice.iter().position(|&c| c == 0).unwrap_or(len);
        String::from_utf16_lossy(&slice[..end])
    }
    #[cfg(not(windows))]
    {
        String::new()
    }
}
