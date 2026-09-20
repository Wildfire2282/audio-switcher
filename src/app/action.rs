//! Action half of `App`: menu ids, hotkeys and control taps turned into state
//! changes.
//!
//! Each method takes `&mut self` and owns exactly one user-visible action. The
//! per-frame pumps that call them live in `super::poll`; the loop itself lives in
//! `super::run`.

use std::time::{Duration, Instant};

use crate::app::handler::MenuAction;
use crate::audio::{AudioBackend, AudioDevice};
use crate::config::{AppConfig, Lang};
use crate::platform::hotkey::HotkeyAction;
use crate::platform::osd::VISIBLE_MS;
use crate::platform::{AutostartMode, autostart_state, pump};
use crate::ui::MenuState;
use crate::ui::i18n::tr;
use crate::ui::osd::format as format_osd;

use super::steps::{cycle_index, stepped_volume};
use super::{App, HOTKEY_VOLUME_STEP};

impl<B: AudioBackend> App<B> {
    pub(super) fn refresh_ui(&mut self) {
        // Use batch snapshot to avoid 3 separate COM round-trips.
        let snap = self.backend.fetch_snapshot_clamped(&self.cfg);
        let def_id = snap.default_device.as_ref().map(|d| d.id.as_str());
        let def_input_id = snap.default_input_device.as_ref().map(|d| d.id.as_str());
        let autostart = autostart_state().mode();
        // In-place menu update; rebuilds only when the device list changed.
        self.tray.sync_menu(&MenuState {
            cfg: &self.cfg,
            devices: &snap.devices,
            default_id: def_id,
            inputs: &snap.input_devices,
            default_input_id: def_input_id,
            muted: snap.mute,
            autostart,
            ui_lang: self.ui_lang,
        });
        // The snapshot is authoritative: resyncing here is also what keeps an
        // external change from leaving the mirror stale.
        self.cached_volume = snap.volume;
        self.cached_mute = snap.mute;
        self.cached_device.clone_from(&snap.default_device);
        self.tray.update_icon_if_changed(snap.mute);
    }

    /// Read the default endpoint's device, volume and mute.
    ///
    /// Prefers the single-COM-round-trip batch; the returned flag is `false` when
    /// the individual queries were needed instead, so the caller can say so once.
    pub(super) fn read_volume_state(&self) -> (Option<AudioDevice>, u32, bool, bool) {
        #[cfg(windows)]
        if let Ok((volume, mute)) = self.backend.get_volume_and_mute() {
            return (self.backend.get_default_device(), volume, mute, true);
        }
        (
            self.backend.get_default_device(),
            self.backend.get_volume().unwrap_or(0),
            self.backend.get_mute().unwrap_or(false),
            false,
        )
    }

    /// Re-read volume/mute/device after an external change (media keys, other
    /// apps, the system mixer) and refresh the mirror plus the tray icon.
    pub(super) fn resync_volume_state(&mut self) {
        let (device, volume, mute, batched) = self.read_volume_state();
        if !batched {
            tracing::warn!("batch volume read failed; falling back to individual queries");
        }
        self.cached_volume = volume;
        self.cached_mute = mute;
        self.cached_device = device;
        self.tray.update_icon_if_changed(mute);
    }

    /// Show the volume overlay for the mirrored state and restart its deadline.
    ///
    /// The overlay is the feedback channel: the tray tooltip only paints after
    /// the system hover delay and does not repaint against wheel input, so the
    /// feedback has to be painted by this process.
    pub(super) fn show_osd(&mut self) {
        let content = format_osd(
            self.cached_device.as_ref(),
            self.cached_volume,
            self.cached_mute,
            self.ui_lang,
        );
        let anchor = self.tray.icon_rect();
        if let Some(osd) = &mut self.osd {
            osd.show(content, anchor);
        }
        self.osd_deadline = Some(Instant::now() + Duration::from_millis(VISIBLE_MS));
    }

    pub(super) fn save_and_refresh(&mut self, clamp: bool) {
        // Synchronous save for critical config — avoids loss on fast exit.
        if let Err(e) = self.cfg.save_to(&AppConfig::config_path()) {
            tracing::warn!("config save failed: {e}");
        }
        if clamp && self.cfg.volume_limit_enabled {
            if let Err(e) = self.backend.clamp_volume_if_needed(&self.cfg) {
                tracing::warn!("volume clamp failed: {e}");
            }
        }
        self.refresh_ui();
    }

    pub(super) fn lang(&self) -> Lang {
        self.ui_lang
    }

    pub(super) fn handle_menu(&mut self, id: &str) {
        match MenuAction::from_id(id) {
            MenuAction::Device(dev_id) => self.set_default_output(&dev_id),
            MenuAction::InputDevice(dev_id) => self.set_default_input(&dev_id),
            MenuAction::Mute => self.toggle_mute(),
            MenuAction::VolEnabled => {
                self.cfg.volume_limit_enabled = !self.cfg.volume_limit_enabled;
                self.save_and_refresh(true);
            }
            MenuAction::VolLimit(v) => {
                self.cfg.volume_limit = v;
                self.cfg.volume_limit_enabled = true;
                self.save_and_refresh(true);
            }
            MenuAction::Refresh => {
                // Manual fallback for sleep-resume/callback loss: drop caches,
                // re-enumerate, and rebuild the UI from fresh state.
                self.backend.clear_cache();
                self.refresh_ui();
            }
            MenuAction::OpenMixer => {
                crate::platform::shell::open_volume_mixer(&tr("mixer_error", self.lang()));
            }
            MenuAction::OpenSound => {
                crate::platform::shell::open_sound_settings(&tr("sound_error", self.lang()));
            }
            MenuAction::OpenHotkeySettings => {
                // Manual-only hotkeys: ensure the commented config exists, then
                // open its folder so the user can edit `hotkeys` and restart.
                if let Err(e) = self.cfg.save_to(&AppConfig::config_path()) {
                    tracing::warn!("config save failed before opening folder: {e}");
                }
                crate::platform::shell::open_folder(
                    &AppConfig::config_dir(),
                    &tr("config_error", self.lang()),
                );
            }
            MenuAction::Autostart(mode) => self.apply_autostart_mode(mode),
            MenuAction::LangSystem => {
                self.cfg.lang = Lang::System;
                self.ui_lang = self.cfg.effective_lang();
                self.save_and_refresh(false);
            }
            MenuAction::LangZh => {
                self.cfg.lang = Lang::Zh;
                self.ui_lang = Lang::Zh;
                self.save_and_refresh(false);
            }
            MenuAction::LangEn => {
                self.cfg.lang = Lang::En;
                self.ui_lang = Lang::En;
                self.save_and_refresh(false);
            }
            MenuAction::About => {
                if let Ok(url) = crate::ABOUT_URL.parse::<crate::platform::shell::Url>() {
                    crate::platform::shell::open_url(&url);
                }
            }
            MenuAction::Exit => {
                self.should_exit = true;
                pump::quit();
            }
            MenuAction::Unknown(s) => {
                // Unknown ids are a menu/handler contract breach: loud in
                // debug, logged and ignored in release (never silent).
                debug_assert!(false, "unknown menu id: {s}");
                tracing::warn!("unknown menu id ignored: {s}");
            }
        }
    }

    // ---- shared actions: menu dispatch and global hotkeys both land here ----
    /// Switch the default output device, then re-apply the volume limit.
    pub(super) fn set_default_output(&mut self, id: &str) {
        match self.backend.set_default_device(id) {
            Ok(()) => {
                if let Err(e) = self.backend.clamp_volume_if_needed(&self.cfg) {
                    tracing::warn!("volume clamp failed: {e}");
                }
                self.refresh_ui();
            }
            Err(e) => {
                tracing::warn!("set_default_device failed: {e}");
                crate::platform::dialog::show_msgbox(&format!(
                    "{}: {e}",
                    crate::ui::i18n::tr("device_error", self.lang())
                ));
            }
        }
    }

    /// Switch the default input (capture) device.
    pub(super) fn set_default_input(&mut self, id: &str) {
        match self.backend.set_default_input_device(id) {
            Ok(()) => self.refresh_ui(),
            Err(e) => {
                tracing::warn!("set_default_input_device failed: {e}");
                crate::platform::dialog::show_msgbox(&format!(
                    "{}: {e}",
                    crate::ui::i18n::tr("input_error", self.lang())
                ));
            }
        }
    }

    /// Switch the autostart mechanism and persist the choice.
    ///
    /// Runs on the UI thread: `schtasks` costs ~100 ms, and a short stall right
    /// after the click beats a state update the user cannot correlate with it.
    fn apply_autostart_mode(&mut self, mode: AutostartMode) {
        match crate::platform::set_autostart_mode(mode) {
            Ok(()) => {
                self.cfg.autostart_mode = mode;
                self.save_and_refresh(false);
            }
            Err(e) => {
                tracing::warn!("set_autostart_mode failed: {e}");
                crate::platform::dialog::show_autostart_error(&e);
            }
        }
    }

    /// Toggle the default output device's mute.
    pub(super) fn toggle_mute(&mut self) {
        let target = !self.cached_mute;
        if let Err(e) = self.backend.set_mute(target) {
            tracing::warn!("set_mute failed: {e}");
            return;
        }
        self.cached_mute = target;
        // The menu check mark tracks the backend; `refresh_ui` re-reads it and
        // also repaints the tray icon, which genuinely changed here.
        self.refresh_ui();
        self.show_osd();
    }

    /// Nudge the master volume by `delta` percent and show feedback.
    ///
    /// Shared by the wheel (accelerated step) and the volume hotkeys (fixed
    /// [`HOTKEY_VOLUME_STEP`]); the configured limit clamps the result.
    ///
    /// Stepping from the mirror keeps one notch down to a single endpoint
    /// write: no read-back, no property-store lookup, no Shell round-trip.
    pub(super) fn nudge_volume(&mut self, delta: i32) {
        let target =
            crate::config::clamp_volume(stepped_volume(self.cached_volume, delta), &self.cfg);
        match self.backend.set_volume(target) {
            // Only a confirmed write updates the mirror, so a failed write
            // cannot leave the step origin ahead of the endpoint.
            Ok(()) => self.cached_volume = target,
            Err(e) => {
                tracing::warn!(error = %e, "set_volume failed");
                return;
            }
        }
        self.show_osd();
    }

    /// Switch to the default output `step` positions away, wrapping at both
    /// ends (the device list order is the Windows enumeration order).
    pub(super) fn cycle_device(&mut self, step: i32) {
        let devices = match self.backend.enumerate_devices() {
            Ok(devices) => devices,
            Err(e) => {
                tracing::warn!("enumerate_devices failed: {e}");
                return;
            }
        };
        let current = self
            .backend
            .get_default_device()
            .and_then(|default| devices.iter().position(|dev| dev.id == default.id));
        let Some(index) = cycle_index(devices.len(), current, step) else {
            tracing::warn!("no output device to cycle through");
            return;
        };
        let id = devices[index].id.clone();
        self.set_default_output(&id);
    }

    /// Dispatch one global hotkey through the same paths as the menu items.
    pub(super) fn handle_hotkey(&mut self, action: HotkeyAction) {
        tracing::debug!("hotkey pressed: {}", action.config_key());
        match action {
            HotkeyAction::Mute => self.toggle_mute(),
            HotkeyAction::VolumeUp => self.nudge_volume(HOTKEY_VOLUME_STEP),
            HotkeyAction::VolumeDown => self.nudge_volume(-HOTKEY_VOLUME_STEP),
            HotkeyAction::NextDevice => self.cycle_device(1),
            HotkeyAction::PrevDevice => self.cycle_device(-1),
        }
    }
}
