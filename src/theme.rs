//! Windows theme detection.
//!
//! The tray icon must be visible on both light and dark taskbars. We render
//! two pre-baked PNG variants at build time (white glyph for dark, black
//! glyph for light) and pick one at runtime based on the system theme.
//!
//! Detection: read the user's `AppsUseLightTheme` value from
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`.
//! This is the same registry key Windows itself uses to broadcast the
//! "apps should be dark" decision via `WM_SETTINGCHANGE` (lParam =
//! "ImmersiveColorSet").

/// Light vs dark taskbar / app background. `Unknown` means we couldn't
/// read the registry; we treat it as light (the safer fallback — black
/// glyph is visible on the default Win11 light taskbar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
    Unknown,
}

/// Read the system theme from the registry. Returns `Theme::Unknown` on
/// any error.
pub fn detect() -> Theme {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::WIN32_ERROR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, KEY_READ,
    };

    // Subkey: "Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"
    // Value: "AppsUseLightTheme" (DWORD: 1 = light, 0 = dark)
    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let value: Vec<u16> = "AppsUseLightTheme"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let mut hkey = windows::Win32::System::Registry::HKEY(std::ptr::null_mut());
        let open = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_READ,
            &mut hkey as *mut _,
        );
        if open != WIN32_ERROR(0) {
            return Theme::Unknown;
        }

        let mut data: [u8; 4] = [0; 4];
        let mut cb: u32 = data.len() as u32;
        let mut kind = windows::Win32::System::Registry::REG_VALUE_TYPE(0);
        let q = RegQueryValueExW(
            hkey,
            PCWSTR(value.as_ptr()),
            None,
            Some(&mut kind as *mut _),
            Some(data.as_mut_ptr()),
            Some(&mut cb as *mut _),
        );
        let _ = RegCloseKey(hkey);
        if q != WIN32_ERROR(0) || cb < 4 {
            return Theme::Unknown;
        }
        // Little-endian DWORD
        let v = u32::from_le_bytes(data);
        if v == 0 {
            Theme::Dark
        } else {
            Theme::Light
        }
    }
}

/// Read the system theme from the registry. Call once at startup.
pub fn init() -> Theme {
    detect()
}
