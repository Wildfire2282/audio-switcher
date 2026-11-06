//! UI layer — tray icon, menu, overlay and wheel handling.

pub(crate) mod i18n;
pub(crate) mod icon;
/// Truncation and sanitization shared by the menu and the overlay, so their
/// budgets cannot drift apart.
pub(crate) mod label;
pub(crate) mod menu;
pub(crate) mod osd;
pub(crate) mod tray;
pub(crate) mod wheel;

/// Re-exported for `crate::app`.
pub(crate) use menu::MenuState;
pub(crate) use tray::TrayWrapper;
pub(crate) use wheel::WheelState;
