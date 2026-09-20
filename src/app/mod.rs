//! Application entry — owns all runtime state and runs the message loop.
//!
//! Loop policy lives here; every Win32 call is behind `platform`
//! (`pump` for messages/wait/quit, `mouse_hook` for the wheel hook).

pub mod handler;

use std::time::{Duration, Instant};

use crate::audio::{AudioBackend, AudioDevice, WasapiBackend};
use crate::config::{AppConfig, Lang};
use crate::platform::hotkey::{self};
use crate::platform::mouse_hook;
use crate::platform::osd::OsdOverlay;
use crate::platform::{autostart_state, pump};
use crate::ui::tray::TrayError;
use crate::ui::{MenuState, TrayWrapper, WheelState};

/// Tray-build attempts at startup: Explorer may be restarting exactly then.
const TRAY_BOOT_ATTEMPTS: u32 = 3;
/// Pause between tray-build attempts (bounded: 3 × 250ms worst case).
const TRAY_BOOT_RETRY_WAIT: Duration = Duration::from_millis(250);
/// Volume percent per global-hotkey press (EarTrumpet parity: its
/// absolute-volume shortcuts step 2).
const HOTKEY_VOLUME_STEP: i32 = 2;

mod action;
mod poll;
mod setup;
mod steps;

use setup::{apply_hotkeys, ensure_autostart};

/// App owns all runtime state. Generic over [`AudioBackend`] for test injection.
pub struct App<B: AudioBackend = WasapiBackend> {
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
    hook: Option<mouse_hook::WheelHook>,
    hook_install_at: Instant,
    should_exit: bool,
    /// Authoritative for our own writes, resynced from the backend on every
    /// external change and full refresh. The wheel path must not read the
    /// endpoint back: one notch used to cost three `Activate` round-trips plus a
    /// property-store read purely to redraw feedback.
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
    /// `assemble` before the error propagates).
    #[must_use = "a failed build must dialog and exit, never be ignored"]
    pub fn build(self) -> Result<App<WasapiBackend>, TrayError> {
        let cfg = self.cfg.unwrap_or_else(AppConfig::load);
        App::assemble(cfg, WasapiBackend::new(), self.com)
    }
}
impl App<WasapiBackend> {
    /// Create a new `App` with the real Windows audio backend.
    ///
    /// # Errors
    ///
    /// Returns [`TrayError`] when the tray icon cannot be created, including
    /// after the startup retries; see [`Self::with_backend`].
    #[must_use = "a failed construction must dialog and exit, never be ignored"]
    pub fn new(com: crate::platform::ComGuard) -> Result<Self, TrayError> {
        Self::with_backend(com, WasapiBackend::new())
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
        // `None` is "unreadable": the menu grays the group instead of guessing.
        let autostart = autostart_state().mode();
        let boot = MenuState {
            cfg: &cfg,
            devices: &[],
            default_id: None,
            inputs: &[],
            default_input_id: None,
            muted: false,
            autostart,
            ui_lang,
        };
        let mut attempt = 0;
        let mut tray = loop {
            attempt += 1;
            match TrayWrapper::new(&boot) {
                Ok(built) => break built,
                // Report the last failure rather than sleeping once more: with
                // no attempt left there is nothing to wait for.
                Err(e) if attempt >= TRAY_BOOT_ATTEMPTS => return Err(e),
                Err(_) => std::thread::sleep(TRAY_BOOT_RETRY_WAIT),
            }
        };
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
            autostart,
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

    // ---- handlers extracted to keep `run` short ----

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
    use super::steps::{cycle_index, stepped_volume};
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
