//! Per-frame half of `App`: one pump per event source, plus the overlay hide
//! deadline.
//!
//! `run` (in `app`) calls these in a fixed order, and that order is policy: the
//! click pump must run before the tray pump so a middle-click clears the stale
//! overlay before its own feedback appears.

use std::time::{Duration, Instant};

use crate::audio::AudioBackend;
use crate::platform::mouse_hook;
use crate::platform::pump;
use crate::ui::WheelState;

use super::{App, HOOK_RETRY_WAIT};
use crate::platform::hotkey;

/// Device notifications are coalesced over this window before the menu is
/// rebuilt: `IMMNotificationClient` reports Added/Removed/DefaultChanged for a
/// single change in quick succession.
const DEVICE_COALESCE_WINDOW: Duration = Duration::from_millis(120);

impl<B: AudioBackend> App<B> {
    pub(super) fn maybe_install_hook(&mut self) {
        if self.hook.is_some() || Instant::now() < self.hook_install_at {
            return;
        }
        match mouse_hook::WheelHook::install() {
            Some(hook) => self.hook = Some(hook),
            // Without a new deadline the install would be retried on every
            // frame — a hundred log lines a second while the failure lasts.
            None => self.hook_install_at = Instant::now() + HOOK_RETRY_WAIT,
        }
    }
    /// Drop the wheel hook so the next frame installs a fresh one.
    ///
    /// `App` cannot tell a live hook from one Windows dropped, and it never
    /// says: the only signal is that the frame that blocked this thread ran
    /// long. The install deadline is cleared rather than set to a retry pause —
    /// the settle delay at startup is for Explorer, not for a re-arm.
    pub(super) fn rearm_hook(&mut self) {
        if self.hook.take().is_none() {
            return;
        }
        tracing::debug!("wheel hook re-armed: the previous frame blocked this thread");
        self.hook_install_at = Instant::now();
    }
    /// Reset wheel acceleration so a stale burst cannot jump the volume
    /// (fresh hover, menu takeover, or cursor leave).
    pub(super) fn reset_wheel(&mut self) {
        self.wheel.clear();
    }
    pub(super) fn poll_tray(&mut self) {
        use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
        // Drain: a burst of tray events must all be consumed each frame.
        let tray_rx = TrayIconEvent::receiver();
        while let Ok(event) = tray_rx.try_recv() {
            match event {
                TrayIconEvent::Click {
                    button: MouseButton::Middle,
                    button_state: MouseButtonState::Up,
                    ..
                } => {
                    // Same path as the menu item and the mute hotkey.
                    self.toggle_mute();
                    self.reset_wheel();
                }
                // EarTrumpet-style hover volume: no click required. A fresh
                // hover resets acceleration so a stale burst cannot jump;
                // Move must not reset or continuous rolling would never
                // accelerate. Right-click hands the gesture to the context
                // menu: a wheel roll over the open menu must scroll the
                // menu, not the volume.
                TrayIconEvent::Enter { .. }
                | TrayIconEvent::Click {
                    button: MouseButton::Right,
                    ..
                }
                | TrayIconEvent::DoubleClick {
                    button: MouseButton::Right,
                    ..
                }
                | TrayIconEvent::Leave { .. } => self.reset_wheel(),
                // Left-click is a no-op for volume (hover alone governs);
                // other buttons and Move have no gesture.
                TrayIconEvent::Move { .. }
                | TrayIconEvent::Click { .. }
                | TrayIconEvent::DoubleClick { .. } => {}
                // Required: `TrayIconEvent` is `#[non_exhaustive]`, so future
                // shell variants must ignore-and-continue rather than break the
                // build (same policy as the menu unknown-id path, minus the
                // warn: unknown hover/move-class events are noise).
                _ => {}
            }
        }
    }
    pub(super) fn poll_menu(&mut self) {
        // Drain: rapid clicks must all dispatch each frame.
        let menu_rx = muda::MenuEvent::receiver();
        while let Ok(event) = menu_rx.try_recv() {
            self.handle_menu(&event.id.0);
        }
    }
    pub(super) fn poll_wheel(&mut self) {
        let Some(event) = mouse_hook::take_wheel_event() else {
            return;
        };
        if event.delta == 0 {
            return;
        }
        #[cfg(windows)]
        {
            // EarTrumpet-style hover gate: the cursor must have been over the
            // icon when the notch was made, which is why the gate judges the
            // position the event carried rather than the one the poll sees.
            // Fail closed when the rect is unavailable, so scrolling elsewhere
            // never changes the volume.
            if !mouse_hook::cursor_over_tray(&self.tray, event.at).unwrap_or(false) {
                return;
            }
        }
        let step = self.wheel.push(Instant::now(), event.delta);
        let total = WheelState::total_step(event.delta, step);
        self.nudge_volume(total);
    }
    pub(super) fn poll_devices(&mut self) {
        if self.backend.poll_device_changed() {
            self.devices_pending = true;
        }
        // coalesce bursts: IMMNotificationClient may fire Added/Removed/DefaultChanged in quick succession.
        // The notification stays latched in `devices_pending` so the deferred
        // rebuild is not lost.
        if !devices_refresh_due(
            self.devices_pending,
            self.last_devices_rebuild,
            Instant::now(),
        ) {
            return;
        }
        self.devices_pending = false;
        self.last_devices_rebuild = Instant::now();
        if let Err(e) = self.backend.clamp_volume_if_needed(&self.cfg) {
            tracing::warn!("volume clamp failed: {e}");
        }
        self.refresh_ui();
    }
    /// External volume/mute change (media keys, other apps) — refresh the
    /// cached state and the tray icon without touching the menu.
    pub(super) fn poll_volume_state(&mut self) {
        if self.backend.take_volume_changed() {
            self.resync_volume_state();
        }
    }
    /// Drain global hotkeys pressed since the last frame.
    pub(super) fn poll_hotkeys(&mut self) {
        while let Some(action) = hotkey::take_pending() {
            self.handle_hotkey(action);
        }
    }
    /// Dismiss the overlay on any mouse button press.
    ///
    /// Runs before the tray and menu handlers, so the middle-click that toggles
    /// mute clears the stale overlay first and the mute feedback that follows
    /// still shows.
    pub(super) fn poll_click(&mut self) {
        // `osd_deadline` tracks visibility: `show_osd` sets it, `hide_osd` and
        // the deadline clear it. Nothing on screen means no call to make.
        if mouse_hook::take_click() && self.osd_deadline.is_some() {
            self.hide_osd();
        }
    }
    /// Hide the overlay now and stop its deadline.
    pub(super) fn hide_osd(&mut self) {
        self.osd_deadline = None;
        if let Some(osd) = &self.osd {
            osd.hide();
        }
    }
    /// Hide the overlay once its deadline passes.
    ///
    /// The deadline is loop policy rather than a `SetTimer`: `wait_timeout`
    /// already shortens the wait to the remaining span, so the loop wakes on
    /// time without a second timing mechanism.
    pub(super) fn poll_osd(&mut self) {
        let Some(deadline) = self.osd_deadline else {
            return;
        };
        if Instant::now() >= deadline {
            self.hide_osd();
        }
    }
    /// Wait timeout for this iteration.
    ///
    /// The earliest instant the loop must not sleep past — the overlay's hide,
    /// a pending hook install, a coalesced device rebuild — capped by the idle
    /// timeout so a lost wake still self-heals.
    pub(super) fn wait_timeout(&self) -> u32 {
        let mut next = self.osd_deadline;
        let mut add = |at: Instant| next = Some(next.map_or(at, |cur| cur.min(at)));
        if self.hook.is_none() {
            add(self.hook_install_at);
        }
        if self.devices_pending {
            add(self.last_devices_rebuild + DEVICE_COALESCE_WINDOW);
        }
        wait_ms(Instant::now(), next)
    }
}

/// Whether a latched device change is old enough to rebuild the menu for.
///
/// Pure so the window can be tested without a backend. The latch is what keeps
/// the last notification of a burst from being dropped, and the window is what
/// turns the burst into one rebuild.
fn devices_refresh_due(pending: bool, last: Instant, now: Instant) -> bool {
    pending && now.saturating_duration_since(last) >= DEVICE_COALESCE_WINDOW
}

/// Milliseconds to wait for `next`, rounded up and capped at
/// [`pump::PUMP_IDLE_MS`].
///
/// Rounding up is not cosmetic: truncating a sub-millisecond remainder to `0`
/// hands `MsgWaitForMultipleObjectsEx` a zero timeout, which turns the sleep
/// into a spin.
fn wait_ms(now: Instant, next: Option<Instant>) -> u32 {
    let Some(next) = next else {
        return pump::PUMP_IDLE_MS;
    };
    let remaining = next.saturating_duration_since(now);
    let millis = remaining.as_micros().div_ceil(1_000);
    u32::try_from(millis.min(u128::from(pump::PUMP_IDLE_MS))).unwrap_or(pump::PUMP_IDLE_MS)
}

#[cfg(test)]
mod tests;
