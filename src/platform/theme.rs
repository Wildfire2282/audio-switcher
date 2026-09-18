//! Windows shell appearance read-back (theme and accent colour).
//!
//! The overlay mirrors Windows 11's flyout design language, so it follows the
//! same two signals the shell itself uses: the **system** theme (the taskbar,
//! the tray and the taskbar's own context menus follow `SystemUsesLightTheme`,
//! not the per-app setting) and the user's accent colour.
//!
//! Both live in the registry, which the crate already opens for autostart, so
//! this costs no new dependency and no new `windows` feature.

#[cfg(windows)]
use super::utf16::wide_z;

/// Shell appearance the overlay mirrors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Appearance {
    /// Whether the shell (taskbar, tray, flyouts) is in its light theme.
    pub(crate) light: bool,
    /// Accent colour as `(r, g, b)`; `None` when it cannot be read.
    pub(crate) accent: Option<(u8, u8, u8)>,
}

/// `HKCU` subkey holding the shell theme switches.
#[cfg(windows)]
const KEY_PERSONALIZE: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Themes\Personalize";
/// `HKCU` subkey holding the accent colour (an `ABGR` DWORD).
#[cfg(windows)]
const KEY_DWM: &str = r"SOFTWARE\Microsoft\Windows\DWM";

/// Read the current shell appearance.
///
/// Values that cannot be read fall back to the dark theme with no accent — the
/// appearance the overlay had before it became theme-aware.
pub(crate) fn appearance() -> Appearance {
    #[cfg(windows)]
    {
        Appearance {
            // Absent value ⇒ dark, matching the fallback above.
            light: read_dword(KEY_PERSONALIZE, "SystemUsesLightTheme").is_some_and(|v| v != 0),
            accent: read_dword(KEY_DWM, "AccentColor").map(accent_from_abgr),
        }
    }
    #[cfg(not(windows))]
    {
        Appearance {
            light: false,
            accent: None,
        }
    }
}

/// Unpack the registry's `0xAABBGGRR` accent into `(r, g, b)`.
#[cfg(windows)]
fn accent_from_abgr(raw: u32) -> (u8, u8, u8) {
    // Masked to a byte before narrowing, so the fallback never fires.
    let byte = |shift: u32| u8::try_from((raw >> shift) & 0xFF).unwrap_or(0);
    (byte(0), byte(8), byte(16))
}

/// Read a `REG_DWORD` from `HKCU\<subkey>`.
///
/// `None` covers every "not configured" case: missing key, missing value, a
/// value of another type, or a failed read.
#[cfg(windows)]
fn read_dword(subkey: &str, value: &str) -> Option<u32> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD, REG_VALUE_TYPE, RegCloseKey, RegOpenKeyExW,
        RegQueryValueExW,
    };
    use windows::core::PCWSTR;

    let sub_w = wide_z(subkey);
    let name_w = wide_z(value);
    let mut hkey = HKEY(std::ptr::null_mut());
    // SAFETY: the subkey string outlives the call; `hkey` is written only on
    // success.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(sub_w.as_ptr()),
            None,
            KEY_READ,
            &raw mut hkey,
        )
    };
    if opened.is_err() {
        tracing::debug!("appearance: cannot open HKCU\\{subkey}");
        return None;
    }
    let mut data = 0u32;
    let mut kind = REG_VALUE_TYPE(0);
    let mut len = u32::try_from(std::mem::size_of::<u32>()).unwrap_or(4);
    // SAFETY: `hkey` is open, the value name outlives the call, and the three
    // out-parameters are sized for exactly one DWORD.
    let read = unsafe {
        RegQueryValueExW(
            hkey,
            PCWSTR(name_w.as_ptr()),
            None,
            Some(&raw mut kind),
            Some((&raw mut data).cast::<u8>()),
            Some(&raw mut len),
        )
    };
    // SAFETY: balances the successful `RegOpenKeyExW` above, exactly once.
    unsafe {
        let _ = RegCloseKey(hkey);
    }
    (read.is_ok() && kind == REG_DWORD).then_some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registry values are little-endian `0xAABBGGRR`: the low byte is red.
    #[test]
    #[cfg(windows)]
    fn accent_unpacks_low_byte_first() {
        // The Windows default accent `#0078D4`, as the shell stores it.
        assert_eq!(accent_from_abgr(0xFFD4_7800), (0x00, 0x78, 0xD4));
        assert_eq!(accent_from_abgr(0x0000_0000), (0, 0, 0));
        assert_eq!(accent_from_abgr(0xFFFF_FFFF), (0xFF, 0xFF, 0xFF));
    }

    /// The reading itself must never panic, whatever this machine is set to.
    #[test]
    fn appearance_is_readable() {
        let appearance = appearance();
        // No assertion on the value: themes differ per machine. The point is
        // that the read path returns instead of panicking.
        let _ = appearance.light;
        let _ = appearance.accent;
    }
}
