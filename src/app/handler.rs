//! Menu action dispatch — maps `muda` IDs to typed actions.

use crate::platform::AutostartMode;

/// Typed menu action parsed from a `MenuEvent` ID. Variant names are the
/// documentation; a payload is the id the menu emitted.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    Device(String),
    InputDevice(String),
    Mute,
    VolEnabled,
    VolLimit(u32),
    /// Manual fallback after sleep/resume or a lost callback.
    Refresh,
    OpenMixer,
    OpenSound,
    /// Opens the config folder for manual hotkey editing.
    OpenHotkeySettings,
    /// Off / current-user `Run` value / elevated logon task.
    Autostart(AutostartMode),
    LangSystem,
    LangZh,
    LangEn,
    About,
    Exit,
    /// Warned and ignored by the caller.
    Unknown(String),
}

impl MenuAction {
    /// Parse a menu ID into a typed action.
    ///
    /// IDs are produced by [`crate::ui::menu`]; device IDs keep the
    /// [`crate::ui::menu::DEVICE_PREFIX`] prefix and `vol_N` presets parse
    /// through [`crate::ui::menu::id::parse_vol_preset`].
    #[must_use]
    pub fn from_id(id: &str) -> Self {
        use crate::ui::menu::{DEVICE_PREFIX, INPUT_PREFIX, id as menu_id};
        if let Some(dev) = id.strip_prefix(DEVICE_PREFIX) {
            if dev.is_empty() || dev.contains('\0') {
                return Self::Unknown(id.to_string());
            }
            return Self::Device(dev.to_string());
        }
        if let Some(dev) = id.strip_prefix(INPUT_PREFIX) {
            if dev.is_empty() || dev.contains('\0') {
                return Self::Unknown(id.to_string());
            }
            return Self::InputDevice(dev.to_string());
        }
        if let Some(preset) = menu_id::parse_vol_preset(id) {
            return Self::VolLimit(preset);
        }
        match id {
            menu_id::REFRESH => Self::Refresh,
            menu_id::MUTE => Self::Mute,
            menu_id::VOL_ENABLED => Self::VolEnabled,
            menu_id::OPEN_MIXER => Self::OpenMixer,
            menu_id::OPEN_SOUND => Self::OpenSound,
            menu_id::OPEN_HOTKEY_SETTINGS => Self::OpenHotkeySettings,
            menu_id::AUTOSTART_OFF => Self::Autostart(AutostartMode::Off),
            menu_id::AUTOSTART_USER => Self::Autostart(AutostartMode::User),
            menu_id::AUTOSTART_ADMIN => Self::Autostart(AutostartMode::Admin),
            menu_id::LANG_SYSTEM => Self::LangSystem,
            menu_id::LANG_ZH => Self::LangZh,
            menu_id::LANG_EN => Self::LangEn,
            menu_id::ABOUT => Self::About,
            menu_id::EXIT => Self::Exit,
            other => Self::Unknown(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests;
