//! Windows dialogs: error boxes, centered without hooks.
//!
//! Centering uses a transient `FindWindowW` + `SetWindowPos` pass from a
//! short-lived worker thread: no hook is ever installed just to center a
//! dialog. Contract: `shell`, `autostart` and `app` may call `show_msgbox`
//! and `show_autostart_error`; dialog owns no other platform state.

use super::autostart::AutostartError;
#[cfg(windows)]
use super::utf16::wide_z;

/// Show a warning box titled with the tool display name.
///
/// Silent on success paths by construction: callers only invoke this on
/// failure. Critical startup failures use `MB_TOPMOST` via
/// [`show_critical`]; regular errors must not steal focus.
pub(crate) fn show_msgbox(msg: &str) {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MB_OK, MessageBoxW};
        use windows::core::PCWSTR;
        let wide = wide_z(msg);
        let title = wide_z(crate::TOOL_DISPLAY_NAME);
        center_soon(title.clone());
        // SAFETY: MessageBoxW with null-terminated buffers alive through the call.
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(wide.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_OK | MB_ICONWARNING,
            );
        }
    }
    #[cfg(not(windows))]
    {
        let _ = msg;
    }
}

/// Show a critical startup box that stays on top, then return.
///
/// `pub` (not `pub(crate)`) because the binary entry dialogs init failures.
pub fn show_critical(msg: &str) {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            MB_ICONERROR, MB_OK, MB_TOPMOST, MessageBoxW,
        };
        use windows::core::PCWSTR;
        let wide = wide_z(msg);
        let title = wide_z(crate::TOOL_DISPLAY_NAME);
        center_soon(title.clone());
        // SAFETY: MessageBoxW with null-terminated buffers alive through the call.
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(wide.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_OK | MB_ICONERROR | MB_TOPMOST,
            );
        }
    }
    #[cfg(not(windows))]
    {
        let _ = msg;
    }
}

/// Show the autostart failure with its cause. Takes the typed error so the
/// detail (exit code plus whatever `schtasks` said) reaches the user instead of
/// a bare string.
pub(crate) fn show_autostart_error(err: &AutostartError) {
    show_msgbox(&format!(
        "Failed to change the autostart setting.\n\nDetails: {err}"
    ));
}

/// Move the tool-titled top-level window to the work-area center.
///
/// One shot: polls `FindWindowW` for up to 2s, centers once via
/// `SetWindowPos` (size/z-order untouched), then the thread exits. Later user
/// drags are never corrected.
#[cfg(windows)]
fn center_soon(title: Vec<u16>) {
    std::thread::spawn(move || {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            FindWindowW, GetSystemMetrics, GetWindowRect, SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE,
            SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
        };
        use windows::core::PCWSTR;

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            // SAFETY: FindWindowW with a null class and a live title pointer.
            let Ok(hwnd) = (unsafe { FindWindowW(None, PCWSTR(title.as_ptr())) }) else {
                std::thread::sleep(std::time::Duration::from_millis(30));
                continue;
            };
            // SAFETY: plain Win32 FFI with a valid top-level HWND.
            unsafe {
                let mut rect = RECT::default();
                if GetWindowRect(hwnd, &mut rect).is_err() {
                    break;
                }
                let w = rect.right - rect.left;
                let h = rect.bottom - rect.top;
                if w <= 0 || h <= 0 {
                    break;
                }
                let (area_x, area_y, area_w, area_h) = {
                    let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                    let mut info = MONITORINFO {
                        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                        ..Default::default()
                    };
                    if GetMonitorInfoW(monitor, &mut info).as_bool() {
                        let r = info.rcWork;
                        (r.left, r.top, r.right - r.left, r.bottom - r.top)
                    } else {
                        (
                            0,
                            0,
                            GetSystemMetrics(SM_CXSCREEN),
                            GetSystemMetrics(SM_CYSCREEN),
                        )
                    }
                };
                let x = area_x + (area_w - w).max(0) / 2;
                let y = area_y + (area_h - h).max(0) / 2;
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            break;
        }
    });
}
