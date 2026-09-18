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

use super::App;
use crate::platform::hotkey;

impl<B: AudioBackend> App<B> {
    pub(super) fn maybe_install_hook(&mut self) {
        if self.hook.is_none() && Instant::now() >= self.hook_install_at {
            self.hook = mouse_hook::WheelHook::install();
        }
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
        let (pending, delta) = mouse_hook::take_wheel_event();
        if !pending || delta == 0 {
            return;
        }
        let now = Instant::now();
        #[cfg(windows)]
        {
            // EarTrumpet-style hover gate: the cursor must be over the icon
            // at event time. Fail closed when the rect is unavailable, so
            // scrolling elsewhere never changes the volume.
            if !mouse_hook::cursor_over_tray(&self.tray).unwrap_or(false) {
                return;
            }
        }
        let step = self.wheel.push(now, delta);
        let total = WheelState::total_step(delta, step);
        self.nudge_volume(total);
    }
    pub(super) fn poll_devices(&mut self) {
        if self.backend.poll_device_changed() {
            self.devices_pending = true;
        }
        if !self.devices_pending {
            return;
        }
        // coalesce bursts: IMMNotificationClient may fire Added/Removed/DefaultChanged in quick succession.
        // The notification stays latched in `devices_pending` so the deferred
        // rebuild is not lost.
        if self.last_devices_rebuild.elapsed() < Duration::from_millis(120) {
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
        if mouse_hook::take_click() {
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
    /// The idle cap, shortened to the overlay's remaining time so a hide is
    /// never late by more than the wake granularity.
    pub(super) fn wait_timeout(&self) -> u32 {
        let Some(deadline) = self.osd_deadline else {
            return pump::PUMP_WAIT_MS;
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        u32::try_from(remaining.as_millis())
            .unwrap_or(pump::PUMP_WAIT_MS)
            .min(pump::PUMP_WAIT_MS)
    }
}
