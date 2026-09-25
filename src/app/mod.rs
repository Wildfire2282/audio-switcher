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
/// Pause before retrying the wheel hook after a failed install. A failure is
/// stable (another process holds the slot, or policy blocks it), so retrying
/// every frame only means the warning drowns in its own repeats.
const HOOK_RETRY_WAIT: Duration = Duration::from_secs(2);
/// A frame that took this long blocked this thread, which is how the wheel hook
/// gets silently dropped: Windows removes a low-level hook whose callback the
/// installing thread stops servicing (`LowLevelHooksTimeout`), and an unhooked
/// mouse means no hover volume for the rest of the session. A UAC helper or a
/// cold `schtasks` spawn is enough to lose it, so the hook is re-armed whenever
/// a frame ran this long — an unhook/re-hook pair per slow frame, nothing in a
/// normal one.
const HOOK_STALL_REARM: Duration = Duration::from_millis(500);
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

    /// Single assembly path for [`with_backend`](Self::with_backend): tray,
    /// snapshot, and state init.
    ///
    /// The tray build retries briefly: Explorer may be restarting exactly as
    /// we start. Other failures (bad icon bytes) are deterministic, so the
    /// bound keeps a broken install from hanging startup — the surviving
    /// error propagates for a visible dialog + exit, never a panic.
    fn assemble(
        cfg: AppConfig,
        mut backend: B,
        com: crate::platform::ComGuard,
    ) -> Result<Self, TrayError> {
        ensure_autostart(&cfg);
        // Register before the first menu build so its checks match reality.
        apply_hotkeys(&cfg);
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

    // ---- handlers extracted to keep `run` short ----

    /// Run the message loop until `Exit` is requested.
    pub fn run(mut self) {
        // Before the loop: the audio callbacks started at assembly already post
        // to this channel, and every wake before it exists is just lost until
        // the idle timeout.
        pump::init_wake_channel();
        loop {
            let frame_start = Instant::now();
            // `WM_QUIT` is the message loop's own exit signal: honoring it here
            // keeps a quit posted by anything else from being swallowed.
            if !pump::pump_messages() {
                break;
            }
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
            if frame_start.elapsed() >= HOOK_STALL_REARM {
                self.rearm_hook();
            }
            let timeout = self.wait_timeout();
            pump::wait_for_input(timeout);
        }
        // Registration is thread-affine: release the combos on this thread.
        hotkey::unregister_all();
    }
}
