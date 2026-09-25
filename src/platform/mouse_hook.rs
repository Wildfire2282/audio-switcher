//! Windows low-level mouse hook: installed lazily via [`WheelHook::install`],
//! removed on drop, and reporting wheel and button events to the main loop
//! through global atomics.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};

/// Accumulated wheel delta (WHEEL_DELTA is signed 120-per-notch).
static WHEEL_DELTA: AtomicI32 = AtomicI32::new(0);
/// Whether a wheel event is pending consumption.
static WHEEL_PENDING: AtomicBool = AtomicBool::new(false);
/// Cursor position carried by the notches in `WHEEL_DELTA`, see [`pack_point`].
///
/// The position *of the event*, not of the poll: the loop can run a frame behind
/// the gesture, and a gate that read the cursor position at poll time would
/// judge a position the wheel never scrolled at — scrolling elsewhere and then
/// moving onto the tray used to change the volume.
static WHEEL_AT: AtomicI64 = AtomicI64::new(0);

/// One drained wheel event: the accumulated delta and where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WheelEvent {
    /// Accumulated `WHEEL_DELTA` units, sign included.
    pub delta: i32,
    /// Cursor position reported with the (last) notch.
    pub at: (i32, i32),
}

/// Pack a screen position into one atomic word (x high, y low).
///
/// Coordinates are signed: a monitor left of or above the primary screen has
/// negative ones.
#[cfg(any(windows, test))]
#[must_use]
#[allow(clippy::cast_sign_loss)]
fn pack_point(x: i32, y: i32) -> i64 {
    (i64::from(x) << 32) | i64::from(y as u32)
}

/// Inverse of [`pack_point`]; both halves sign-extend.
#[must_use]
fn unpack_point(raw: i64) -> (i32, i32) {
    ((raw >> 32) as i32, raw as i32)
}

/// Whether a mouse button went down since the last poll.
#[cfg(windows)]
static CLICKED: AtomicBool = AtomicBool::new(false);

/// Whether `msg` is a mouse button going down.
///
/// Client and non-client downs both count: a click on a title bar, border or
/// scrollbar is still a click. Button ups, moves and the wheel are not.
#[cfg(windows)]
fn is_button_down(msg: u32) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_NCLBUTTONDOWN, WM_NCMBUTTONDOWN, WM_NCRBUTTONDOWN,
        WM_NCXBUTTONDOWN, WM_RBUTTONDOWN, WM_XBUTTONDOWN,
    };
    matches!(
        msg,
        WM_LBUTTONDOWN
            | WM_RBUTTONDOWN
            | WM_MBUTTONDOWN
            | WM_XBUTTONDOWN
            | WM_NCLBUTTONDOWN
            | WM_NCRBUTTONDOWN
            | WM_NCMBUTTONDOWN
            | WM_NCXBUTTONDOWN
    )
}

/// Whether this thread is inside one of its own menu loops.
///
/// A modal menu pumps messages, so the low-level hook still runs while it is
/// open: without this check a roll meant for the menu sits in the accumulator
/// and then changes the volume, late, when the menu closes.
#[cfg(windows)]
fn in_menu_mode() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        GUI_INMENUMODE, GUITHREADINFO, GUITHREADINFO_FLAGS, GetGUIThreadInfo,
    };
    let mut info = GUITHREADINFO {
        cbSize: u32::try_from(std::mem::size_of::<GUITHREADINFO>()).unwrap_or(0),
        ..Default::default()
    };
    // SAFETY: `GetGUIThreadInfo` writes a POD out-parameter sized by `cbSize`;
    // a null thread id asks about the calling thread, which is where the hook
    // procedure runs.
    let queried = unsafe { GetGUIThreadInfo(0, &raw mut info) }.is_ok();
    queried && info.flags & GUI_INMENUMODE != GUITHREADINFO_FLAGS(0)
}

/// Low-level mouse hook procedure: harvests wheel deltas and button presses,
/// forwards everything.
///
/// # Safety
///
/// Windows invokes this on the hook thread: `n_code >= 0` guarantees `l_param`
/// points to a valid `MSLLHOOKSTRUCT` (only then is it dereferenced); other
/// codes skip straight to forwarding. `CallNextHookEx` is always safe to call.
#[cfg(windows)]
unsafe extern "system" fn hook_proc(
    n_code: i32,
    w_param: windows::Win32::Foundation::WPARAM,
    l_param: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{CallNextHookEx, MSLLHOOKSTRUCT, WM_MOUSEWHEEL};
    if n_code >= 0 {
        let msg = w_param.0 as u32;
        if msg == WM_MOUSEWHEEL {
            if !in_menu_mode() {
                // SAFETY: per Win32 contract l_param points to MSLLHOOKSTRUCT
                let info = unsafe { &*(l_param.0 as *const MSLLHOOKSTRUCT) };
                #[allow(clippy::cast_possible_wrap, clippy::cast_lossless)]
                let delta = (info.mouseData >> 16) as u16 as i16 as i32;
                // Release ordering pairs with Acquire in the consumer (take_wheel_event).
                WHEEL_AT.store(pack_point(info.pt.x, info.pt.y), Ordering::Release);
                WHEEL_DELTA.fetch_add(delta, Ordering::AcqRel);
                WHEEL_PENDING.store(true, Ordering::Release);
            }
        } else if is_button_down(msg) {
            // Only "a click happened" is needed to dismiss the overlay, so one
            // flag serves every button. Release ordering pairs with Acquire in
            // the consumer (take_click).
            CLICKED.store(true, Ordering::Release);
        }
    }
    // SAFETY: CallNextHookEx is always safe to forward.
    unsafe { CallNextHookEx(None, n_code, w_param, l_param) }
}

/// RAII hook handle: owns the installed hook and removes it on drop.
///
/// The handle is owned rather than shared behind a static refcount, so
/// "installed" and "guarded" cannot disagree: dropping the guard uninstalls
/// exactly the hook that guard installed. `App` keeps a single
/// `Option<WheelHook>` and that option is what prevents a second install.
// The `PhantomData<*const ()>` makes it `!Send` — HHOOK is thread-affine.
pub struct WheelHook {
    /// `0` means no hook (the non-Windows stub, and nothing else).
    handle: isize,
    _marker: std::marker::PhantomData<*const ()>,
}

impl WheelHook {
    /// Install the low-level mouse hook.
    ///
    /// Returns `None` on Windows if `SetWindowsHookExW` fails.
    #[must_use]
    pub fn install() -> Option<Self> {
        #[cfg(windows)]
        {
            // SAFETY: WH_MOUSE_LL is process-global, hook_proc has correct signature.
            let hook = unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{SetWindowsHookExW, WH_MOUSE_LL};
                SetWindowsHookExW(WH_MOUSE_LL, Some(hook_proc), None, 0)
            };
            match hook {
                Ok(h) if !h.0.is_null() => Some(Self {
                    handle: h.0 as isize,
                    _marker: std::marker::PhantomData,
                }),
                Err(e) => {
                    tracing::warn!("mouse hook install failed: {e:?}");
                    None
                }
                Ok(_) => None,
            }
        }
        #[cfg(not(windows))]
        {
            Some(Self {
                handle: 0,
                _marker: std::marker::PhantomData,
            })
        }
    }
}

impl Drop for WheelHook {
    fn drop(&mut self) {
        if self.handle == 0 {
            return;
        }
        #[cfg(windows)]
        {
            // SAFETY: `handle` came from SetWindowsHookExW on this thread and
            // is unhooked exactly once, here.
            unsafe {
                let hook = windows::Win32::UI::WindowsAndMessaging::HHOOK(
                    self.handle as *mut std::ffi::c_void,
                );
                let _ = windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(hook);
            }
        }
    }
}

// ---- stateless helpers for App ----

/// Atomically take the pending wheel event, `None` when nothing arrived.
///
/// Merges the old `take_pending` + `take_delta` pair so callers cannot observe
/// a torn state (pending cleared but delta left behind, or vice versa).
pub(crate) fn take_wheel_event() -> Option<WheelEvent> {
    let delta = WHEEL_DELTA.swap(0, Ordering::AcqRel);
    let pending = WHEEL_PENDING.swap(false, Ordering::AcqRel);
    // A nonzero delta implies an event even if the flag raced; treat either as one.
    if !pending && delta == 0 {
        return None;
    }
    Some(WheelEvent {
        delta,
        at: unpack_point(WHEEL_AT.load(Ordering::Acquire)),
    })
}

/// Atomically take the pending click flag: did a mouse button go down?
///
/// One flag covers every button — the overlay only needs "a click happened".
#[cfg(windows)]
pub(crate) fn take_click() -> bool {
    CLICKED.swap(false, Ordering::AcqRel)
}

#[cfg(not(windows))]
pub(crate) fn take_click() -> bool {
    false
}

/// Whether the UTF-16 window class `class_name` matches a known taskbar or tray window.
#[cfg(any(windows, test))]
fn is_tray_class_name(class_name: &[u16]) -> bool {
    const TRAY_CLASSES: &[&str] = &[
        "Shell_TrayWnd",
        "Shell_SecondaryTrayWnd",
        "NotifyIconOverflowWindow",
        "TopLevelWindowForOverflowXamlIsland",
        "TrayNotifyWnd",
        "AudioSwitcherVolumeOsd",
    ];

    TRAY_CLASSES
        .iter()
        .any(|target| target.encode_utf16().eq(class_name.iter().copied()))
}

#[cfg(windows)]
fn is_tray_class(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;

    let mut buf = [0u16; 64];
    // SAFETY: `buf` is a live stack buffer and its capacity is passed correctly;
    // `hwnd` is only queried by `GetClassNameW`.
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len == 0 {
        return false;
    }
    let len = usize::try_from(len).unwrap_or(0).min(buf.len());
    is_tray_class_name(&buf[..len])
}

#[cfg(windows)]
fn is_tray_or_taskbar_at(pt: windows::Win32::Foundation::POINT) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, WindowFromPoint};

    // SAFETY: `WindowFromPoint` takes `POINT` by value and returns the topmost
    // window containing the point (or null).
    let hwnd = unsafe { WindowFromPoint(pt) };
    if hwnd.0.is_null() {
        return false;
    }
    // SAFETY: `GetAncestor` safely traverses the window parent hierarchy
    // starting from `hwnd` and returns the root window.
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    is_tray_class(hwnd) || (!root.0.is_null() && is_tray_class(root))
}

/// Whether `at` is over the tray icon's rect (with padding) and the window
/// there belongs to the taskbar/tray area.
///
/// `at` is the position the wheel event carried ([`WheelEvent::at`]), not the
/// current cursor position: the gate answers "was the gesture over the icon",
/// and the cursor can move between the notch and the poll.
///
/// Returns `None` when the tray rect is unavailable.
#[cfg(windows)]
pub(crate) fn cursor_over_tray(
    wrapper: &crate::ui::tray::TrayWrapper,
    at: (i32, i32),
) -> Option<bool> {
    let (rx, ry, rw, rh) = wrapper.icon_rect()?;
    // Keep tolerance minimal (DPI rounding only). A large pad plus the
    // old grace window caused volume changes when the cursor had already
    // left the icon / was over a neighboring tray icon.
    const PAD: i32 = 2;
    if !(at.0 >= rx - PAD && at.0 < rx + rw + PAD && at.1 >= ry - PAD && at.1 < ry + rh + PAD) {
        return Some(false);
    }
    Some(is_tray_or_taskbar_at(windows::Win32::Foundation::POINT {
        x: at.0,
        y: at.1,
    }))
}

#[cfg(not(windows))]
pub(crate) fn cursor_over_tray(
    _wrapper: &crate::ui::tray::TrayWrapper,
    _at: (i32, i32),
) -> Option<bool> {
    Some(false)
}

#[cfg(all(test, windows))]
mod click_tests;

#[cfg(all(test, windows))]
mod wake_latency_tests;

#[cfg(test)]
mod tray_window_tests;

#[cfg(test)]
mod wheel_tests;
