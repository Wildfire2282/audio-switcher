//! Message-pump primitives. `app` owns the loop policy; this module owns the
//! Win32 calls (`ui` and `app` must not call Win32 directly).

/// Idle wait cap: bounds how long a *lost* wake can delay feedback, and how
/// often the loop re-checks state no callback reports.
///
/// Not a cadence. A notification posts [`wake`] and the loop ends the wait at
/// once, so while something is happening the loop runs per event; while nothing
/// is, it sleeps here.
pub const PUMP_IDLE_MS: u32 = 15_000;

/// Posted to the wake window to end a message wait early.
#[cfg(windows)]
const WAKE_MESSAGE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;

#[cfg(windows)]
mod wake {
    use std::sync::Once;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, HWND_MESSAGE, PostMessageW, RegisterClassW,
        WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
    };
    use windows::core::w;

    use super::WAKE_MESSAGE;

    /// The message-only wake window, or `0` before [`init`] (and after a failed
    /// creation).
    static WAKE_HWND: AtomicIsize = AtomicIsize::new(0);

    /// A window procedure is required to own a window; every message is
    /// answered by the default handling, since the window exists only so
    /// notifications have a thread-safe handle to post to.
    ///
    /// # Safety
    ///
    /// Called by Windows with handles and message values from the message
    /// contract; nothing is dereferenced.
    unsafe extern "system" fn wake_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // SAFETY: default handling for every message; always safe.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    /// Create the wake window. Called once, from the UI thread, before the
    /// loop runs.
    ///
    /// Failure is not retried (same policy as the overlay's window class): it
    /// costs latency, not correctness, because [`super::PUMP_IDLE_MS`] still
    /// bounds the wait.
    pub(super) fn init() {
        static CREATE: Once = Once::new();
        CREATE.call_once(|| {
            // SAFETY: `GetModuleHandleW(None)` requests the current module; it
            // takes no pointer arguments and writes nothing.
            let Ok(hinstance) = (unsafe { GetModuleHandleW(None) }) else {
                tracing::warn!("pump: GetModuleHandleW failed; callback wakes disabled");
                return;
            };
            let class = WNDCLASSW {
                lpfnWndProc: Some(wake_proc),
                hInstance: HINSTANCE(hinstance.0),
                lpszClassName: w!("AudioSwitcherPumpWake"),
                ..Default::default()
            };
            // SAFETY: `class` is fully initialized and lives for the call; the
            // class name is a static NUL-terminated literal.
            if unsafe { RegisterClassW(&raw const class) } == 0 {
                tracing::warn!("pump: RegisterClassW failed; callback wakes disabled");
                return;
            }
            // SAFETY: the class is registered above; a message-only window
            // takes no parent, menu or parameter.
            let hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("AudioSwitcherPumpWake"),
                    w!(""),
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    0,
                    0,
                    Some(HWND_MESSAGE),
                    None,
                    Some(HINSTANCE(hinstance.0)),
                    None,
                )
            };
            match hwnd {
                Ok(h) if !h.0.is_null() => WAKE_HWND.store(h.0 as isize, Ordering::Release),
                other => tracing::warn!("pump: wake window creation failed: {other:?}"),
            }
        });
    }

    /// End a message wait from any thread. No-op before [`init`] succeeded and
    /// after a failed post: a missed wake costs latency only.
    pub(super) fn wake() {
        let raw = WAKE_HWND.load(Ordering::Acquire);
        if raw == 0 {
            return;
        }
        // SAFETY: the handle came from `CreateWindowExW` in `init` and the
        // window lives for the process lifetime; posting to a live window is
        // safe from any thread.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(raw as *mut core::ffi::c_void)),
                WAKE_MESSAGE,
                WPARAM(0),
                LPARAM(0),
            )
        };
    }
}

/// Create the wake window. Call once, from the UI thread, before the loop.
#[cfg(windows)]
pub(crate) fn init_wake_channel() {
    wake::init();
}

/// Create the wake window. Call once, from the UI thread, before the loop.
#[cfg(not(windows))]
pub(crate) fn init_wake_channel() {}

/// End a message wait from any thread: the audio callbacks run off the loop
/// thread and this is how they reach it.
#[cfg(windows)]
pub(crate) fn wake() {
    wake::wake();
}

/// End a message wait from any thread: the audio callbacks run off the loop
/// thread and this is how they reach it.
#[cfg(not(windows))]
pub(crate) fn wake() {}

/// Drain pending Win32 messages without blocking.
pub(crate) fn pump_messages() {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage, WM_HOTKEY,
        };
        loop {
            let mut msg = MSG::default();
            // SAFETY: `PeekMessageW` writes a plain POD `MSG` through the
            // raw out-pointer; no buffers escape.
            let pending = unsafe { PeekMessageW(&raw mut msg, None, 0, 0, PM_REMOVE).as_bool() };
            if !pending {
                break;
            }
            if msg.message == WM_HOTKEY {
                // `RegisterHotKey(None, ..)` binds to this thread and posts
                // `WM_HOTKEY` with a null window: `DispatchMessageW` would
                // drop it, so route the action to the hotkey module instead.
                crate::platform::hotkey::note_pending(msg.wParam.0 as i32);
                continue;
            }
            if msg.message == WAKE_MESSAGE {
                // Our own wake: ending the wait was the entire message.
                continue;
            }
            // SAFETY: `msg` was just written by `PeekMessageW` above.
            let _ = unsafe { TranslateMessage(&raw const msg) };
            // SAFETY: same freshly-drained `msg`; standard dispatch pair.
            unsafe { DispatchMessageW(&raw const msg) };
        }
    }
}

/// Block until input or a wake arrives, or `timeout_ms` elapses.
///
/// `timeout_ms` is policy and belongs to `app`, which shortens it while the
/// overlay holds a hide deadline.
///
/// No extra wake signal is needed for the wheel: the low-level hook is
/// dispatched during message retrieval, and the system wakes this same wait so
/// the retrieval can happen. A signal raised from inside the hook procedure
/// could not help — it runs only after the wake it would be trying to cause.
pub(crate) fn wait_for_input(timeout_ms: u32) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::WAIT_FAILED;
        use windows::Win32::UI::WindowsAndMessaging::{
            MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, QS_ALLINPUT,
        };
        // SAFETY: MsgWaitForMultipleObjectsEx with an empty handle slice and
        // QS_ALLINPUT is safe to call on the UI thread.
        let waited = unsafe {
            MsgWaitForMultipleObjectsEx(Some(&[]), timeout_ms, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
        };
        if waited == WAIT_FAILED {
            // A failed wait returns at once, so the loop spins a core at 100%
            // while everything still appears to work — the one failure here
            // that must leave a trace.
            tracing::warn!("pump: MsgWaitForMultipleObjectsEx failed; message wait skipped");
        }
    }
    #[cfg(not(windows))]
    std::thread::sleep(std::time::Duration::from_millis(u64::from(timeout_ms)));
}

/// Post `Quit`, ending the message loop.
pub(crate) fn quit() {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::PostQuitMessage;
        // SAFETY: Posts quit to the calling thread's queue; always safe.
        unsafe { PostQuitMessage(0) };
    }
}
