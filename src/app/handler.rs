//! Menu action dispatch — maps `muda` IDs to typed actions.

/// Typed menu action parsed from a `MenuEvent` ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    /// Switch to device `id`.
    Device(String),
    /// Switch to input (capture) device `id`.
    InputDevice(String),
    /// Toggle mute.
    Mute,
    /// Toggle volume-limit enabled.
    VolEnabled,
    /// Set limit to `u32` percent.
    VolLimit(u32),
    /// Re-enumerate devices (manual refresh fallback).
    Refresh,
    /// Open volume mixer.
    OpenMixer,
    /// Open sound settings.
    OpenSound,
    /// Open the config folder for manual hotkey editing.
    OpenHotkeySettings,
    /// Toggle autostart.
    Autostart,
    /// Follow the system language.
    LangSystem,
    /// Switch language to Chinese.
    LangZh,
    /// Switch language to English.
    LangEn,
    /// Open about URL.
    About,
    /// Exit process.
    Exit,
    /// Unknown ID — warned and ignored by the caller.
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
            menu_id::AUTOSTART => Self::Autostart,
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
