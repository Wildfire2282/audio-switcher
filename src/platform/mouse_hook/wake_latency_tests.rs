//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{MOUSEEVENTF_MOVE, mouse_event};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW,
    QS_ALLINPUT, SetWindowsHookExW, UnhookWindowsHookEx, WH_MOUSE_LL,
};

static PROBE_HITS: AtomicUsize = AtomicUsize::new(0);

/// Counts hook deliveries on the installing thread.
///
/// # Safety
///
/// Called by Windows on the hook thread with `n_code`/`w_param`/`l_param`
/// from the hook contract. Only the counter is touched, and the call is
/// always forwarded with `CallNextHookEx`.
unsafe extern "system" fn probe_proc(n_code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if n_code >= 0 {
        PROBE_HITS.fetch_add(1, Ordering::AcqRel);
    }
    // SAFETY: always safe to forward down the hook chain.
    unsafe { CallNextHookEx(None, n_code, w, l) }
}

/// Answers the one question the wheel latency hinges on: does a low-level
/// hook event wake `MsgWaitForMultipleObjectsEx(QS_ALLINPUT)` early, or does
/// the loop sit out its full idle timeout before the hook can even run?
///
/// The injector moves the cursor one pixel and immediately moves it back, so
/// the pointer ends where it started while the input still traverses the
/// hook chain. (A zero-delta move is dropped by the system and never reaches
/// the hook at all.)
#[test]
#[ignore = "injects synthetic mouse input; run explicitly"]
fn low_level_hook_wakes_the_message_wait() {
    let _gate = crate::INTEGRATION_GATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    const IDLE_MS: u32 = 200;
    const INJECT_AFTER_MS: u64 = 50;
    // SAFETY: installing a WH_MOUSE_LL hook on this thread with a valid
    // callback; unhooked at the end of the test.
    unsafe {
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(probe_proc), None, 0)
            .expect("SetWindowsHookExW failed");
        PROBE_HITS.store(0, Ordering::Release);

        // Drain anything already queued so the measurement starts clean.
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {}

        let injector = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(INJECT_AFTER_MS));
            mouse_event(MOUSEEVENTF_MOVE, 1, 0, 0, 0);
            mouse_event(MOUSEEVENTF_MOVE, -1, 0, 0, 0);
        });

        let t0 = Instant::now();
        let _ = MsgWaitForMultipleObjectsEx(Some(&[]), IDLE_MS, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        let woke_after = t0.elapsed();

        // The hook callback runs during message retrieval, not inside the
        // wait, so it only counts after this drain.
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {}

        injector.join().expect("injector thread panicked");
        let _ = UnhookWindowsHookEx(hook);

        let hits = PROBE_HITS.load(Ordering::Acquire);
        println!("wait returned after {woke_after:?}; hook hits: {hits}");
        assert!(
            hits > 0,
            "synthetic input never reached the low-level hook; measurement invalid"
        );
        assert!(
            woke_after < Duration::from_millis(150),
            "low-level hook did NOT wake the wait (took {woke_after:?}); \
                 the loop idles for its full {IDLE_MS}ms timeout before wheel events surface"
        );
    }
}
