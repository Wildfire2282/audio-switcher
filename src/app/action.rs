//! Action half of `App`: menu ids, hotkeys and control taps turned into state
//! changes.
//!
//! Each method takes `&mut self` and owns exactly one user-visible action. The
//! per-frame pumps that call them live in `super::poll`; the loop itself lives in
//! `super::run`.

use std::time::{Duration, Instant};

use crate::app::handler::MenuAction;
use crate::audio::AudioBackend;
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
        self.refresh_osd_if_visible();
    }

    /// Re-render the card when it is already on screen.
    ///
    /// The mirror moves for reasons the card did not cause — an outside mute or
    /// volume change, a device switch from the menu or a hotkey — and a card
    /// showing the previous state for the rest of its two seconds contradicts
    /// the endpoint it is reporting on.
    fn refresh_osd_if_visible(&mut self) {
        if self.osd_deadline.is_some() {
            self.show_osd();
        }
    }

    /// Re-read volume/mute/device after an external change (media keys, other
    /// apps, the system mixer) and refresh the mirror, the menu's mute check and
    /// the tray icon.
    ///
    /// A failed read keeps the last known mirror: there is no second query to
    /// fall back to, and reporting `0%`/unmuted would both mislead the next
    /// wheel step and paint a wrong check mark.
    pub(super) fn resync_volume_state(&mut self) {
        match self.backend.get_volume_and_mute() {
            Ok((volume, mute)) => {
                self.cached_volume = volume;
                self.cached_mute = mute;
            }
            Err(e) => {
                tracing::warn!("volume/mute refresh failed; keeping the last known values: {e}");
            }
        }
        self.cached_device = self.backend.get_default_device();
        // An external mute must move the menu's check mark too, or it disagrees
        // with the tray icon until some unrelated refresh corrects it.
        self.tray.sync_mute(self.cached_mute);
        self.tray.update_icon_if_changed(self.cached_mute);
        self.refresh_osd_if_visible();
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
        let action = MenuAction::from_id(id);
        match action {
            MenuAction::Device(dev_id) => self.set_default_output(&dev_id),
            MenuAction::InputDevice(dev_id) => self.set_default_input(&dev_id),
            MenuAction::Mute => self.toggle_mute(),
            MenuAction::VolEnabled | MenuAction::VolLimit(_) => {
                // The new limit applies to the volume playing now.
                if apply_menu_config(&mut self.cfg, &mut self.ui_lang, &action) {
                    self.save_and_refresh(true);
                }
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
            MenuAction::LangSystem | MenuAction::LangZh | MenuAction::LangEn => {
                // The language does not touch the volume: no clamp.
                if apply_menu_config(&mut self.cfg, &mut self.ui_lang, &action) {
                    self.save_and_refresh(false);
                }
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
        // The mute click keeps its side effect of applying the volume limit. The
        // mirror is clamped with it, because the feedback and the next step both
        // read the mirror rather than the endpoint.
        if let Err(e) = self.backend.clamp_volume_if_needed(&self.cfg) {
            tracing::warn!("volume clamp failed: {e}");
        }
        self.cached_volume = crate::config::clamp_volume(self.cached_volume, &self.cfg);
        // Only the mute read-outs move: a full `refresh_ui` would re-read the
        // whole snapshot and walk the menu for one check mark.
        self.tray.sync_mute(target);
        self.tray.update_icon_if_changed(target);
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
        // Without this the gesture is silent feedback: the card moves and
        // nothing is audible, because the endpoint is still muted.
        if lifts_mute(delta, self.cached_mute) {
            self.lift_mute();
        }
        self.show_osd();
    }

    /// Clear mute for a volume-up gesture.
    ///
    /// A failure leaves the mirror alone: the icon and the menu check keep
    /// showing muted, which is then still true.
    fn lift_mute(&mut self) {
        if let Err(e) = self.backend.set_mute(false) {
            tracing::warn!("set_mute failed while raising the volume: {e}");
            return;
        }
        self.cached_mute = false;
        // Both mute read-outs, as in `toggle_mute`.
        self.tray.sync_mute(false);
        self.tray.update_icon_if_changed(false);
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

/// Whether a volume gesture lifts mute.
///
/// Measured against the shell's own volume keys (which the tray wheel is the
/// same gesture as): volume-up clears mute, volume-down leaves it — asking for
/// less sound is not asking for sound.
fn lifts_mute(delta: i32, muted: bool) -> bool {
    delta > 0 && muted
}

/// Apply a menu action whose whole effect is a config field.
///
/// Returns whether the config changed, so the caller knows to save and
/// re-render. Only the actions with no other effect are here — a device switch,
/// an autostart change, a shell call or the exit would each need more than the
/// caller's `save_and_refresh`.
fn apply_menu_config(cfg: &mut AppConfig, ui_lang: &mut Lang, action: &MenuAction) -> bool {
    match action {
        MenuAction::VolEnabled => {
            cfg.volume_limit_enabled = !cfg.volume_limit_enabled;
        }
        MenuAction::VolLimit(limit) => {
            cfg.volume_limit = *limit;
            // Choosing a preset is also how the cap is switched on: the menu has
            // no separate "on" entry beyond the checkbox, so a preset picked
            // while the cap is off must not look like it did nothing.
            cfg.volume_limit_enabled = true;
        }
        MenuAction::LangSystem | MenuAction::LangZh | MenuAction::LangEn => {
            cfg.lang = match action {
                MenuAction::LangZh => Lang::Zh,
                MenuAction::LangEn => Lang::En,
                _ => Lang::System,
            };
            // Resolved together with the mode so the two cannot disagree
            // (`System` follows the OS locale).
            *ui_lang = cfg.effective_lang();
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests;
