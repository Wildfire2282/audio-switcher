//! Windows dialogs: error boxes, centered without hooks.
//!
//! Centering uses a transient `FindWindowW` + `SetWindowPos` pass from a
//! short-lived worker thread: no hook is ever installed just to center a
//! dialog. Contract: `shell`, `autostart` and `app` may call `show_msgbox`
//! with text they own; dialog composes none of it.

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
        let title = crate::TOOL_DISPLAY_NAME.to_string();
        center_soon(title.clone());
        let title = wide_z(&title);
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
        let title = crate::TOOL_DISPLAY_NAME.to_string();
        center_soon(title.clone());
        let title = wide_z(&title);
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

/// Move the first window `matches` accepts to its monitor's work-area centre.
///
/// One shot on a short-lived worker thread: polls for up to `wait`, centres once
/// via `SetWindowPos` (size/z-order untouched), then the thread exits. Later
/// user drags are never corrected. Nothing matching means nothing is moved.
#[cfg(windows)]
fn center_when_found(wait: std::time::Duration, matches: impl Fn(isize) -> bool + Send + 'static) {
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + wait;
        while std::time::Instant::now() < deadline {
            if let Some(hwnd) = first_window_matching(&matches) {
                center(hwnd);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    });
}

/// The first top-level window `matches` accepts, in Z-order.
#[cfg(windows)]
fn first_window_matching(matches: &impl Fn(isize) -> bool) -> Option<isize> {
    use windows::Win32::Foundation::{FALSE, LPARAM, TRUE};
    use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
    use windows::core::BOOL;

    /// Everything `enum_proc` needs, passed through `EnumWindows`' `l_param`.
    struct Ctx<'a> {
        matches: &'a dyn Fn(isize) -> bool,
        found: Option<isize>,
    }

    /// # Safety
    ///
    /// `l_param` must be a live pointer to a `Ctx` for the whole call, which is
    /// what the caller below passes: `Ctx` outlives `EnumWindows`, so the
    /// callback may read it and record the match in it.
    unsafe extern "system" fn enum_proc(
        hwnd: windows::Win32::Foundation::HWND,
        l_param: LPARAM,
    ) -> BOOL {
        // SAFETY: the contract above.
        let ctx = unsafe { &mut *(l_param.0 as *mut Ctx) };
        if (ctx.matches)(hwnd.0 as isize) {
            ctx.found = Some(hwnd.0 as isize);
            return FALSE;
        }
        TRUE
    }

    let mut ctx = Ctx {
        matches,
        found: None,
    };
    // SAFETY: `ctx` is a live local for the whole call, and the callback's
    // contract (above) is satisfied for it; `EnumWindows` is otherwise safe.
    unsafe {
        let _ = EnumWindows(
            Some(enum_proc),
            LPARAM(std::ptr::from_mut(&mut ctx) as isize),
        );
    }
    ctx.found
}

/// The window's title, or an empty string when it has none.
#[cfg(windows)]
fn window_title(hwnd: isize) -> String {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowTextW;

    let mut buf = [0u16; 512];
    // SAFETY: `hwnd` comes from a window enumeration and is only read; `buf` is
    // a live buffer of the length passed.
    let len = unsafe { GetWindowTextW(HWND(hwnd as *mut std::ffi::c_void), &mut buf) };
    let len = usize::try_from(len).unwrap_or(0).min(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Whether `hwnd`'s window class is `name`.
#[cfg(windows)]
fn class_is(hwnd: isize, name: &str) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;

    let mut buf = [0u16; 256];
    // SAFETY: as in `window_title`; `buf` is a live buffer of the length passed.
    let len = unsafe { GetClassNameW(HWND(hwnd as *mut std::ffi::c_void), &mut buf) };
    let len = usize::try_from(len).unwrap_or(0).min(buf.len());
    String::from_utf16_lossy(&buf[..len]) == name
}

/// Whether `hwnd` is visible.
#[cfg(windows)]
fn is_visible(hwnd: isize) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;

    // SAFETY: as in `window_title`: the handle is only read.
    unsafe { IsWindowVisible(HWND(hwnd as *mut std::ffi::c_void)) }.as_bool()
}

/// Handles of the dialogs on screen right now.
///
/// Lets a caller tell the dialog its own launch produces from any dialog that
/// was already there.
#[cfg(windows)]
#[must_use]
pub(crate) fn dialogs_on_screen() -> Vec<isize> {
    use windows::Win32::Foundation::{LPARAM, TRUE};
    use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
    use windows::core::BOOL;

    const DIALOG_CLASS: &str = "#32770";

    /// # Safety
    ///
    /// `l_param` must be a live pointer to a `Vec<isize>`; the callback only
    /// appends to it.
    unsafe extern "system" fn collect(
        hwnd: windows::Win32::Foundation::HWND,
        l_param: LPARAM,
    ) -> BOOL {
        // SAFETY: the contract above.
        let list = unsafe { &mut *(l_param.0 as *mut Vec<isize>) };
        if class_is(hwnd.0 as isize, DIALOG_CLASS) {
            list.push(hwnd.0 as isize);
        }
        TRUE
    }

    let mut list = Vec::new();
    // SAFETY: `list` outlives the call; the callback only writes into it.
    unsafe {
        let _ = EnumWindows(
            Some(collect),
            LPARAM(std::ptr::from_mut(&mut list) as isize),
        );
    }
    list
}

/// Centre the dialog that appears after [`dialogs_on_screen`] was taken.
///
/// The system tools this centres (`SndVol.exe`, `control mmsys.cpl`) open at a
/// remembered or default position — the volume mixer lands at `0,0`, a corner on
/// a large display. Neither can be found by title: both titles are localized,
/// and the mixer's even carries the device name. A window that was already on
/// screen is never moved, so a foreign dialog cannot be caught by accident.
#[cfg(windows)]
pub(crate) fn center_new_dialog(known: Vec<isize>) {
    center_when_found(std::time::Duration::from_secs(3), move |hwnd| {
        is_visible(hwnd) && class_is(hwnd, "#32770") && !known.contains(&hwnd)
    });
}

/// Move `hwnd` to the centre of its monitor's work area.
///
/// Size and z-order are untouched and the window is not activated: the caller is
/// centring a window it did not create.
#[cfg(windows)]
fn center(hwnd: isize) {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, GetWindowRect, SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOSIZE,
        SWP_NOZORDER, SetWindowPos,
    };

    let hwnd = HWND(hwnd as *mut std::ffi::c_void);
    // SAFETY: plain Win32 calls on a top-level window handle; every
    // out-parameter is a live local.
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return;
        }
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        if w <= 0 || h <= 0 {
            return;
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
}

/// Centre the app's own dialog with this title once it appears.
#[cfg(windows)]
fn center_soon(title: String) {
    center_when_found(std::time::Duration::from_secs(2), move |hwnd| {
        window_title(hwnd) == title
    });
}
