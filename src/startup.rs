//! Windows autostart via the `Run` registry key.
//!
//! On Windows, a program can register itself for automatic start at user
//! logon by writing a REG_SZ value under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.  The value name
//! is the entry's label; the data is the executable path.

use std::os::windows::ffi::OsStrExt;
use windows::core::PCWSTR;
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_SZ,
};

const REG_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "AudioSwitcher";

/// Whether the application is registered to start at boot.
pub fn is_enabled() -> bool {
    unsafe {
        let subkey: Vec<u16> = REG_SUBKEY
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let value: Vec<u16> = VALUE_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let mut hkey = HKEY(std::ptr::null_mut());
        let open = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_READ,
            &mut hkey as *mut _,
        );
        if open != WIN32_ERROR(0) {
            return false;
        }

        let mut data = [0u16; 1024];
        let mut cb = (data.len() * 2) as u32;
        let q = RegQueryValueExW(
            hkey,
            PCWSTR(value.as_ptr()),
            None,
            None,
            Some(data.as_mut_ptr() as *mut u8),
            Some(&mut cb as *mut _),
        );
        let _ = RegCloseKey(hkey);
        q == WIN32_ERROR(0)
    }
}

/// Enable or disable autostart.  On any failure the operation is silently
/// ignored (the menu item state won't change on the next rebuild).
pub fn set_enabled(enabled: bool) {
    unsafe {
        let subkey: Vec<u16> = REG_SUBKEY
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let value: Vec<u16> = VALUE_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let mut hkey = HKEY(std::ptr::null_mut());
        let open = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_SET_VALUE,
            &mut hkey as *mut _,
        );
        if open != WIN32_ERROR(0) {
            return;
        }

        if enabled {
            if let Ok(exe) = std::env::current_exe() {
                let wide: Vec<u16> = exe
                    .as_os_str()
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                let _ = RegSetValueExW(
                    hkey,
                    PCWSTR(value.as_ptr()),
                    None,
                    REG_SZ,
                    Some(std::slice::from_raw_parts(
                        wide.as_ptr() as *const u8,
                        wide.len() * 2,
                    )),
                );
            }
        } else {
            let _ = RegDeleteValueW(hkey, PCWSTR(value.as_ptr()));
        }
        let _ = RegCloseKey(hkey);
    }
}
