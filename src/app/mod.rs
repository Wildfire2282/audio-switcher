//! Application entry — owns all runtime state and runs the message loop.
//!
//! Loop policy lives here; every Win32 call is behind `platform`
//! (`pump` for messages/wait/quit, `hook` for the wheel hook).

pub mod handler;

use std::time::{Duration, Instant};

use crate::audio::{AudioBackend, AudioDevice, RealBackend};
use crate::config::{AppConfig, Lang};
use crate::platform::hook;
use crate::platform::hotkey::{self, Hotkey, HotkeyAction, HotkeyError};
use crate::platform::osd::{OsdOverlay, VISIBLE_MS};
use crate::platform::{AutostartState, autostart_state, pump};
use crate::ui::i18n::tr;
use crate::ui::osd::format as format_osd;
use crate::ui::tray::TrayError;
use crate::ui::{MenuState, TrayWrapper, WheelState};
use handler::MenuAction;

/// Tray-build attempts at startup: Explorer may be restarting exactly then.
const TRAY_BOOT_ATTEMPTS: u32 = 3;
/// Pause between tray-build attempts (bounded: 3 × 250ms worst case).
const TRAY_BOOT_RETRY_WAIT: Duration = Duration::from_millis(250);
/// Volume percent per global-hotkey press (EarTrumpet parity: its
/// absolute-volume shortcuts step 2).
const HOTKEY_VOLUME_STEP: i32 = 2;

/// Index of the device `step` positions from `current`, wrapping at both ends.
///
/// `None` only for an empty list; an unknown/absent `current` starts at the
/// first device, so cycling always produces a usable index.
#[must_use]
fn cycle_index(len: usize, current: Option<usize>, step: i32) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let start = i32::try_from(current.unwrap_or(0)).unwrap_or(0);
    let len = i32::try_from(len).unwrap_or(i32::MAX);
    Some((start + step).rem_euclid(len) as usize)
}

/// Apply one `delta` percent to `volume`, clamped to the `0..=100` invariant.
///
/// `i64` math: neither a wheel burst nor `i32::MIN` can wrap, and the clamp
/// keeps a lying backend from pushing the tray past 100.
#[must_use]
fn stepped_volume(volume: u32, delta: i32) -> u32 {
    u32::try_from((i64::from(volume) + i64::from(delta)).clamp(0, 100)).unwrap_or(0)
}

/// Self-heal the autostart entry when the user wants it but the registry
/// reads explicit `Disabled`. `Unknown` never writes — it only logs; the
/// menu renders the item grayed.
fn ensure_autostart(cfg: &AppConfig) {
    if !cfg.autostart {
        return;
    }
    match autostart_state() {
        AutostartState::Enabled => {}
        AutostartState::Disabled => {
            std::thread::spawn(|| {
                if let Err(e) = crate::platform::set_autostart(true) {
                    crate::platform::dialog::show_autostart_error(&e);
                }
            });
        }
        AutostartState::Unknown(reason) => {
            tracing::warn!(
                "autostart state unknown at startup ({reason}); leaving registry untouched"
            );
        }
    }
}

/// Bind the configured hotkeys, reporting (and disabling) occupied combos.
///
/// An occupied combination is never silently dropped: the affected actions are
/// cleared in `cfg` — so the menu reflects what is actually bound — persisted,
/// and surfaced in one dialog listing every conflict.
fn apply_hotkeys(cfg: &mut AppConfig) {
    let mut bindings: Vec<(HotkeyAction, Hotkey)> = Vec::new();
    for action in HotkeyAction::ALL {
        let Some(raw) = cfg.hotkeys.get(action) else {
            continue;
        };
        match raw.parse::<Hotkey>() {
            Ok(hotkey) => bindings.push((action, hotkey)),
            // `migrate` already drops unparsable combos; a value that reaches
            // this point (hand-edited file without a reload) stays off.
            Err(e) => tracing::warn!("hotkey for {} skipped: {e}", action.config_key()),
        }
    }
    let Err(HotkeyError(occupied)) = hotkey::register_all(&bindings) else {
        return;
    };
    for (action, _) in &occupied {
        cfg.hotkeys.set(*action, None);
    }
    if let Err(e) = cfg.save_to(&AppConfig::config_path()) {
        tracing::warn!("config save failed after hotkey conflict: {e}");
    }
    crate::platform::dialog::show_msgbox(&format!(
        "{}: some hotkeys are already in use by another program and were disabled:\n\n{}\n\nEdit {} to pick another combination.",
        crate::TOOL_DISPLAY_NAME,
        hotkey::summarize(&occupied),
        AppConfig::config_path().display(),
    ));
}

/// App owns all runtime state. Generic over [`AudioBackend`] for test injection.
pub struct App<B: AudioBackend = RealBackend> {
    cfg: AppConfig,
    /// Effective UI language, resolved once at startup (`System` → locale).
    ui_lang: Lang,
    backend: B,
    tray: TrayWrapper,
    wheel: WheelState,
    last_devices_rebuild: Instant,
    /// Latched device-change notification. `poll_device_changed()` consumes
    /// the backend flag, so a coalesced burst must stay latched here instead
    /// of being dropped — otherwise the menu stays stale until the next
    /// unrelated notification.
    devices_pending: bool,
    hook: Option<hook::WheelHook>,
    hook_install_at: Instant,
    should_exit: bool,
    /// In-process mirror of the default endpoint's volume/mute/device.
    ///
    /// The wheel path must not read the endpoint back: one notch used to cost
    /// three `Activate` round-trips plus a property-store read purely to redraw
    /// feedback. The mirror is authoritative for our own writes and is resynced
    /// from the backend on every external change and every full refresh.
    cached_volume: u32,
    cached_mute: bool,
    cached_device: Option<AudioDevice>,
    /// Hide deadline for the volume overlay; `None` while nothing is shown.
    osd_deadline: Option<Instant>,
    /// Lazily-created volume overlay; `None` means "no overlay" (the wheel and
    /// the volume path keep working, exactly like a failed tray update).
    osd: Option<OsdOverlay>,
    _com: crate::platform::ComGuard,
}

/// Builder for [`App`] — allows injecting a custom backend or config for tests.
///
/// # Examples
///
/// ```
/// use audio_switcher::app::AppBuilder;
/// use audio_switcher::ComGuard;
/// // let com = ComGuard::init().expect("COM");
/// // let app = AppBuilder::new(com).build().expect("tray");
/// ```
pub struct AppBuilder {
    com: crate::platform::ComGuard,
    cfg: Option<AppConfig>,
}

impl AppBuilder {
    /// Create a builder with the given COM guard.
    #[must_use]
    pub fn new(com: crate::platform::ComGuard) -> Self {
        Self { com, cfg: None }
    }

    /// Override the config (otherwise loaded from disk).
    #[must_use]
    pub fn config(mut self, cfg: AppConfig) -> Self {
        self.cfg = Some(cfg);
        self
    }

    /// Build the [`App`] with the real backend.
    ///
    /// # Errors
    ///
    /// Returns [`TrayError`] when the tray icon cannot be created; the caller
    /// dialogs and exits (a transient Explorer absence is retried inside
    #[must_use = "a failed build must dialog and exit, never be ignored"]
    pub fn build(self) -> Result<App<RealBackend>, TrayError> {
        let cfg = self.cfg.unwrap_or_else(AppConfig::load);
        App::assemble(cfg, RealBackend::new(), self.com)
    }
}
impl App<RealBackend> {
    /// Create a new `App` with the real Windows audio backend.
    #[must_use = "a failed construction must dialog and exit, never be ignored"]
    pub fn new(com: crate::platform::ComGuard) -> Result<Self, TrayError> {
        Self::with_backend(com, RealBackend::new())
    }
}

impl<B: AudioBackend> App<B> {
    /// Create an `App` with an injected backend.
    ///
    /// # Errors
    ///
    /// Returns [`TrayError`] when the tray icon cannot be created.
    pub fn with_backend(com: crate::platform::ComGuard, backend: B) -> Result<Self, TrayError> {
        let cfg = AppConfig::load();
        Self::assemble(cfg, backend, com)
    }

    /// Single assembly path shared by [`AppBuilder::build`] and
    /// [`with_backend`](Self::with_backend): tray, snapshot, and state init.
    ///
    /// The tray build retries briefly: Explorer may be restarting exactly as
    /// we start. Other failures (bad icon bytes) are deterministic, so the
    /// bound keeps a broken install from hanging startup — the surviving
    /// error propagates for a visible dialog + exit, never a panic.
    fn assemble(
        mut cfg: AppConfig,
        mut backend: B,
        com: crate::platform::ComGuard,
    ) -> Result<Self, TrayError> {
        ensure_autostart(&cfg);
        // Register before the first menu build so its checks match reality.
        apply_hotkeys(&mut cfg);
        let ui_lang = cfg.effective_lang();
        let autostart = autostart_state();
        let boot = MenuState {
            cfg: &cfg,
            devices: &[],
            default_id: None,
            inputs: &[],
            default_input_id: None,
            muted: false,
            autostart: &autostart,
            ui_lang,
        };
        let mut last_err = None;
        let mut tray = None;
        for _ in 0..TRAY_BOOT_ATTEMPTS {
            match TrayWrapper::new(&boot) {
                Ok(built) => {
                    tray = Some(built);
                    break;
                }
                Err(e) => {
                    last_err = Some(e);
                    std::thread::sleep(TRAY_BOOT_RETRY_WAIT);
                }
            }
        }
        let mut tray =
            tray.ok_or_else(|| last_err.expect("retry loop always runs at least once"))?;
        let snap = backend.fetch_snapshot_clamped(&cfg);
        let default_id = snap.default_device.as_ref().map(|d| d.id.clone());
        let default_input_id = snap.default_input_device.as_ref().map(|d| d.id.clone());
        tray.rebuild_menu(&MenuState {
            cfg: &cfg,
            devices: &snap.devices,
            default_id: default_id.as_deref(),
            inputs: &snap.input_devices,
            default_input_id: default_input_id.as_deref(),
            muted: snap.mute,
            autostart: &autostart,
            ui_lang,
        });
        tray.update_icon_if_changed(snap.mute);
        let osd = OsdOverlay::new();
        Ok(Self {
            cfg,
            ui_lang,
            backend,
            tray,
            wheel: WheelState::new(),
            last_devices_rebuild: Instant::now(),
            devices_pending: false,
            hook: None,
            hook_install_at: Instant::now() + Duration::from_millis(180),
            should_exit: false,
            cached_volume: snap.volume,
            cached_mute: snap.mute,
            cached_device: snap.default_device.clone(),
            osd_deadline: None,
            osd,
            _com: com,
        })
    }

    /// Returns true when an exit has been requested via the tray menu.
    #[must_use]
    pub fn should_exit(&self) -> bool {
        self.should_exit
    }

    fn refresh_ui(&mut self) {
        // Use batch snapshot to avoid 3 separate COM round-trips.
        let snap = self.backend.fetch_snapshot_clamped(&self.cfg);
        let def_id = snap.default_device.as_ref().map(|d| d.id.as_str());
        let def_input_id = snap.default_input_device.as_ref().map(|d| d.id.as_str());
        let autostart = autostart_state();
        // In-place menu update; rebuilds only when the device list changed.
        self.tray.sync_menu(&MenuState {
            cfg: &self.cfg,
            devices: &snap.devices,
            default_id: def_id,
            inputs: &snap.input_devices,
            default_input_id: def_input_id,
            muted: snap.mute,
            autostart: &autostart,
            ui_lang: self.ui_lang,
        });
        // The snapshot is authoritative: resyncing here is also what keeps an
        // external change from leaving the mirror stale.
        self.cached_volume = snap.volume;
        self.cached_mute = snap.mute;
        self.cached_device = snap.default_device.clone();
        self.tray.update_icon_if_changed(snap.mute);
    }

    /// Read the default endpoint's device, volume and mute.
    ///
    /// Prefers the single-COM-round-trip batch; the returned flag is `false` when
    /// the individual queries were needed instead, so the caller can say so once.
    fn read_volume_state(&self) -> (Option<AudioDevice>, u32, bool, bool) {
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
    fn resync_volume_state(&mut self) {
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
    fn show_osd(&mut self) {
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

    fn save_and_refresh(&mut self, clamp: bool) {
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

    fn lang(&self) -> Lang {
        self.ui_lang
    }

    fn handle_menu(&mut self, id: &str) {
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
            MenuAction::Autostart => {
                let new_val = !self.cfg.autostart;
                match crate::platform::set_autostart(new_val) {
                    Ok(()) => {
                        self.cfg.autostart = new_val;
                        self.save_and_refresh(false);
                    }
                    Err(e) => {
                        tracing::warn!("set_autostart failed: {e}");
                        crate::platform::dialog::show_autostart_error(&e);
                    }
                }
            }
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
    fn set_default_output(&mut self, id: &str) {
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
    fn set_default_input(&mut self, id: &str) {
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

    /// Toggle the default output device's mute.
    fn toggle_mute(&mut self) {
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
    fn nudge_volume(&mut self, delta: i32) {
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
    fn cycle_device(&mut self, step: i32) {
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
    fn handle_hotkey(&mut self, action: HotkeyAction) {
        tracing::debug!("hotkey pressed: {}", action.config_key());
        match action {
            HotkeyAction::Mute => self.toggle_mute(),
            HotkeyAction::VolumeUp => self.nudge_volume(HOTKEY_VOLUME_STEP),
            HotkeyAction::VolumeDown => self.nudge_volume(-HOTKEY_VOLUME_STEP),
            HotkeyAction::NextDevice => self.cycle_device(1),
            HotkeyAction::PrevDevice => self.cycle_device(-1),
        }
    }

    // ---- handlers extracted to keep `run` short ----
    fn maybe_install_hook(&mut self) {
        if self.hook.is_none() && Instant::now() >= self.hook_install_at {
            self.hook = hook::WheelHook::install();
        }
    }
    /// Reset wheel acceleration so a stale burst cannot jump the volume
    /// (fresh hover, menu takeover, or cursor leave).
    fn reset_wheel(&mut self) {
        self.wheel.clear();
    }
    fn poll_tray(&mut self) {
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
    fn poll_menu(&mut self) {
        // Drain: rapid clicks must all dispatch each frame.
        let menu_rx = muda::MenuEvent::receiver();
        while let Ok(event) = menu_rx.try_recv() {
            self.handle_menu(&event.id.0);
        }
    }
    fn poll_wheel(&mut self) {
        let (pending, delta) = hook::take_wheel_event();
        if !pending || delta == 0 {
            return;
        }
        let now = Instant::now();
        #[cfg(windows)]
        {
            // EarTrumpet-style hover gate: the cursor must be over the icon
            // at event time. Fail closed when the rect is unavailable, so
            // scrolling elsewhere never changes the volume.
            if !hook::cursor_over_tray(&self.tray).unwrap_or(false) {
                return;
            }
        }
        let step = self.wheel.push(now, delta);
        let total = WheelState::total_step(delta, step);
        self.nudge_volume(total);
    }
    fn poll_devices(&mut self) {
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
    fn poll_volume_state(&mut self) {
        if self.backend.take_volume_changed() {
            self.resync_volume_state();
        }
    }
    /// Drain global hotkeys pressed since the last frame.
    fn poll_hotkeys(&mut self) {
        while let Some(action) = hotkey::take_pending() {
            self.handle_hotkey(action);
        }
    }
    /// Dismiss the overlay on any mouse button press.
    ///
    /// Runs before the tray and menu handlers, so the middle-click that toggles
    /// mute clears the stale overlay first and the mute feedback that follows
    /// still shows.
    fn poll_click(&mut self) {
        if hook::take_click() {
            self.hide_osd();
        }
    }
    /// Hide the overlay now and stop its deadline.
    fn hide_osd(&mut self) {
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
    fn poll_osd(&mut self) {
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
    fn wait_timeout(&self) -> u32 {
        let Some(deadline) = self.osd_deadline else {
            return pump::PUMP_WAIT_MS;
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        u32::try_from(remaining.as_millis())
            .unwrap_or(pump::PUMP_WAIT_MS)
            .min(pump::PUMP_WAIT_MS)
    }

    /// Run the message loop until `Exit` is requested.
    pub fn run(mut self) {
        loop {
            pump::pump_messages();
            if self.should_exit {
                break;
            }
            self.maybe_install_hook();
            self.poll_click();
            self.poll_tray();
            self.poll_menu();
            if self.should_exit {
                break;
            }
            self.poll_hotkeys();
            self.poll_wheel();
            self.poll_devices();
            self.poll_volume_state();
            self.poll_osd();
            let timeout = self.wait_timeout();
            pump::wait_for_input(timeout);
        }
        // Registration is thread-affine: release the combos on this thread.
        hotkey::unregister_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_leave_resets_wheel_acceleration() {
        // EarTrumpet-style hover: Leave clears the burst history so a stale
        // burst cannot jump the volume on the next hover.
        let mut wheel = WheelState::new();
        let base = Instant::now();
        assert_eq!(wheel.push(base, 120), 1);
        assert_eq!(wheel.push(base + Duration::from_millis(50), 120), 5);
        wheel.clear();
        let later = base + Duration::from_millis(300);
        assert_eq!(wheel.push(later, 120), 1);
    }

    #[test]
    fn cycle_index_wraps_both_ways() {
        assert_eq!(cycle_index(3, Some(0), 1), Some(1));
        // Forward past the end wraps to the first device, backward to the last.
        assert_eq!(cycle_index(3, Some(2), 1), Some(0));
        assert_eq!(cycle_index(3, Some(0), -1), Some(2));
        // Unknown/absent current starts at the first device.
        assert_eq!(cycle_index(3, None, 1), Some(1));
        assert_eq!(cycle_index(3, Some(9), -1), Some(2));
        // Single device and empty list.
        assert_eq!(cycle_index(1, Some(0), 1), Some(0));
        assert_eq!(cycle_index(0, None, 1), None);
    }

    #[test]
    fn stepped_volume_stays_in_range() {
        assert_eq!(stepped_volume(50, 2), 52);
        assert_eq!(stepped_volume(50, -2), 48);
        assert_eq!(stepped_volume(0, -2), 0);
        assert_eq!(stepped_volume(1, -5), 0);
        assert_eq!(stepped_volume(99, 5), 100);
        // Overflow-safe at both extremes.
        assert_eq!(stepped_volume(100, i32::MAX), 100);
        assert_eq!(stepped_volume(0, i32::MIN), 0);
    }
}
