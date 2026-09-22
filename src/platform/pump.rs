//! Message-pump primitives. `app` owns the loop policy; this module owns the
//! Win32 calls (`ui` and `app` must not call Win32 directly).

/// Idle wait cap: periodic work (hook install, polling) still runs while idle
/// CPU stays negligible.
///
/// This is an upper bound, not a fixed cadence: `app` passes a shorter timeout
/// when it holds a deadline (the overlay must hide on time).
pub const PUMP_WAIT_MS: u32 = 200;

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
            // SAFETY: `msg` was just written by `PeekMessageW` above.
            let _ = unsafe { TranslateMessage(&raw const msg) };
            // SAFETY: same freshly-drained `msg`; standard dispatch pair.
            unsafe { DispatchMessageW(&raw const msg) };
        }
    }
}

/// Block until input arrives or `timeout_ms` elapses.
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
