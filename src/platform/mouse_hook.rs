//! Windows low-level mouse hook: installed lazily via [`WheelHook::install`],
//! removed on drop, and reporting wheel and button events to the main loop
//! through global atomics.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// Accumulated wheel delta (WHEEL_DELTA is signed 120-per-notch).
static WHEEL_DELTA: AtomicI32 = AtomicI32::new(0);
/// Whether a wheel event is pending consumption.
static WHEEL_PENDING: AtomicBool = AtomicBool::new(false);
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
            // SAFETY: per Win32 contract l_param points to MSLLHOOKSTRUCT
            let info = unsafe { &*(l_param.0 as *const MSLLHOOKSTRUCT) };
            #[allow(clippy::cast_possible_wrap, clippy::cast_lossless)]
            let delta = (info.mouseData >> 16) as u16 as i16 as i32;
            // Release ordering pairs with Acquire in the consumer (take_wheel_event).
            WHEEL_DELTA.fetch_add(delta, Ordering::AcqRel);
            WHEEL_PENDING.store(true, Ordering::Release);
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

/// Atomically take the pending wheel event: `(had_event, accumulated_delta)`.
///
/// Merges the old `take_pending` + `take_delta` pair so callers cannot observe
/// a torn state (pending cleared but delta left behind, or vice versa).
pub(crate) fn take_wheel_event() -> (bool, i32) {
    let delta = WHEEL_DELTA.swap(0, Ordering::AcqRel);
    let pending = WHEEL_PENDING.swap(false, Ordering::AcqRel);
    // A nonzero delta implies an event even if the flag raced; treat either as pending.
    (pending || delta != 0, delta)
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

/// Whether the cursor is over the tray icon's rect (with padding).
///
/// Returns `None` when the tray rect is unavailable.
#[cfg(windows)]
pub(crate) fn cursor_over_tray(wrapper: &crate::ui::tray::TrayWrapper) -> Option<bool> {
    // SAFETY: GetCursorPos writes to POINT out-param; rect() is tray-icon API.
    unsafe {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut pt = POINT { x: 0, y: 0 };
        if let Err(e) = GetCursorPos(&mut pt) {
            // Fail closed: without a position the gate cannot say "over the
            // icon", and a wrong "yes" would let any scroll change the volume.
            tracing::debug!("cursor_over_tray: GetCursorPos failed: {e:?}");
            return Some(false);
        }
        let rect = wrapper.tray.rect()?;
        let x = f64::from(pt.x);
        let y = f64::from(pt.y);
        // Keep tolerance minimal (DPI rounding only). A large pad plus the
        // old grace window caused volume changes when the cursor had already
        // left the icon / was over a neighboring tray icon.
        let pad = 2.0;
        Some(
            x >= rect.position.x - pad
                && x < rect.position.x + f64::from(rect.size.width) + pad
                && y >= rect.position.y - pad
                && y < rect.position.y + f64::from(rect.size.height) + pad,
        )
    }
}

#[cfg(not(windows))]
pub(crate) fn cursor_over_tray(_wrapper: &crate::ui::tray::TrayWrapper) -> Option<bool> {
    Some(false)
}

#[cfg(all(test, windows))]
mod click_tests;

#[cfg(all(test, windows))]
mod wake_latency_tests;
