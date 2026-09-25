//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

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

#[cfg(windows)]
mod centring {
    use super::*;
    use crate::platform::dialog::dialogs_on_screen;
    use std::time::{Duration, Instant};

    /// Both system tools open at a remembered or default position — the volume
    /// mixer lands at `0,0`, a corner on a large display — so the app moves the
    /// dialog it just produced to the work-area centre.
    ///
    /// Opens the real tools and closes them again; run explicitly.
    #[test]
    #[ignore = "opens the real system tools; run explicitly"]
    fn the_launched_system_tool_is_centred() {
        let _gate = crate::INTEGRATION_GATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (what, open) in [
            ("volume mixer", open_volume_mixer as fn(&str)),
            ("sound settings", open_sound_settings as fn(&str)),
        ] {
            let before = dialogs_on_screen();
            open("centring check");
            let hwnd = wait_for_new_dialog(&before)
                .unwrap_or_else(|| panic!("{what}: no dialog appeared"));
            // The centring pass runs on its own thread as soon as the dialog is
            // found; give it a moment before reading the position back.
            std::thread::sleep(Duration::from_millis(800));

            let rect = window_rect(hwnd).unwrap_or_else(|| panic!("{what}: no window rect"));
            // Size is never touched; the centre is the monitor's work area.
            let (work, screen) = monitor_work_area(hwnd);
            let expected = (
                work.0 + (work.2 - rect.2).max(0) / 2,
                work.1 + (work.3 - rect.3).max(0) / 2,
            );
            assert!(
                (rect.0 - expected.0).abs() <= 2 && (rect.1 - expected.1).abs() <= 2,
                "{what}: at {},{} instead of the work-area centre {expected:?} (work {work:?}, screen {screen:?})",
                rect.0,
                rect.1
            );
            close_window(hwnd);
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    /// Poll for a dialog that was not on screen before the launch.
    fn wait_for_new_dialog(before: &[isize]) -> Option<isize> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(hwnd) = dialogs_on_screen()
                .into_iter()
                .find(|hwnd| !before.contains(hwnd))
            {
                return Some(hwnd);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }

    /// `(x, y, width, height)` of a live window.
    fn window_rect(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
        use windows::Win32::Foundation::{HWND, RECT};
        use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

        let mut rect = RECT::default();
        // SAFETY: plain out-parameter write on a window handle from an
        // enumeration.
        unsafe { GetWindowRect(HWND(hwnd as *mut std::ffi::c_void), &mut rect) }
            .ok()
            .map(|()| {
                (
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                )
            })
    }

    /// `(work area, screen)` of the monitor the window is on.
    fn monitor_work_area(hwnd: isize) -> ((i32, i32, i32, i32), (i32, i32)) {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
        };
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

        let hwnd = HWND(hwnd as *mut std::ffi::c_void);
        // SAFETY: plain Win32 calls with live out-parameters.
        unsafe {
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let screen = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
            if GetMonitorInfoW(monitor, &mut info).as_bool() {
                let r = info.rcWork;
                ((r.left, r.top, r.right - r.left, r.bottom - r.top), screen)
            } else {
                ((0, 0, screen.0, screen.1), screen)
            }
        }
    }

    /// Ask the window to close, as the user would.
    fn close_window(hwnd: isize) {
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};

        // SAFETY: posting a close request to a live window.
        unsafe {
            let _ = PostMessageW(
                Some(HWND(hwnd as *mut std::ffi::c_void)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}
