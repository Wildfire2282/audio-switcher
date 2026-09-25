//! Startup wiring that is not part of the `App` type: autostart and hotkey
//! registration.
//!
//! Both run once, from `assemble`, and neither touches app state - which is why
//! they are plain functions instead of methods.

use crate::config::AppConfig;
use crate::platform::AutostartMode;
use crate::platform::hotkey::{self, Hotkey, HotkeyAction, HotkeyError};

/// Self-heal the current-user `Run` value when the user asked for it but no
/// entry is there at all. Recreating the elevated task would need a UAC prompt,
/// and raising one on every logon is worse than the missing entry: the menu
/// shows it as not set and the user re-enables it deliberately.
///
/// A read failure never writes, and neither does an entry the shell reports as
/// switched off: that one is not missing, and rewriting it would undo a choice
/// the user made in Task Manager's Startup tab. The write is synchronous — a
/// `schtasks` presence check plus one registry value, only on the pass that
/// finds the entry missing — because the caller builds the menu from that same
/// read-back immediately afterwards: from a worker thread the menu would show
/// "Off" until some unrelated refresh happened to correct it.
pub(super) fn ensure_autostart(cfg: &AppConfig) {
    crate::platform::refresh_admin_task_cache();
    match cfg.autostart_mode {
        AutostartMode::Off | AutostartMode::Admin => {}
        AutostartMode::User => {
            if crate::platform::run_entry_absent() {
                if let Err(e) = crate::platform::set_autostart_mode(AutostartMode::User) {
                    crate::platform::dialog::show_msgbox(&format!(
                        "{}: {e}",
                        crate::ui::i18n::tr("autostart_error", cfg.effective_lang())
                    ));
                }
            }
        }
    }
}

/// Bind the configured hotkeys, reporting the combos another program owns.
///
/// A conflict only skips registration for this session: the combos stay in the
/// config, so they work again once the other program exits. Erasing them instead
/// would let any program that happens to be running at launch delete the user's
/// setting permanently.
pub(super) fn apply_hotkeys(cfg: &AppConfig) {
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
    let lang = cfg.effective_lang();
    crate::platform::dialog::show_msgbox(&format!(
        "{}: {}\n\n{}\n\n{} {}",
        crate::TOOL_DISPLAY_NAME,
        crate::ui::i18n::tr("hotkey_conflict", lang),
        hotkey::summarize(&occupied),
        crate::ui::i18n::tr("hotkey_conflict_hint", lang),
        AppConfig::config_path().display(),
    ));
}
