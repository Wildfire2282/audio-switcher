//! Startup wiring that is not part of the `App` type: autostart and hotkey
//! registration.
//!
//! Both run once, from `assemble`, and neither touches app state - which is why
//! they are plain functions instead of methods.

use crate::config::AppConfig;
use crate::platform::hotkey::{self, Hotkey, HotkeyAction, HotkeyError};
use crate::platform::{AutostartState, autostart_state};

/// Self-heal the autostart entry when the user wants it but the registry
/// reads explicit `Disabled`. `Unknown` never writes — it only logs; the
/// menu renders the item grayed.
pub(super) fn ensure_autostart(cfg: &AppConfig) {
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
pub(super) fn apply_hotkeys(cfg: &mut AppConfig) {
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
