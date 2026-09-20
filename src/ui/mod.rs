//! UI layer — tray icon, menu, overlay and wheel handling.

pub mod i18n;
pub mod icon;
/// Truncation and sanitization shared by the menu and the overlay, so their
/// budgets cannot drift apart.
pub(crate) mod label;
pub mod menu;
pub mod osd;
pub mod tray;
pub mod wheel;

/// Re-exported for `crate::app`.
pub use menu::MenuState;
pub use tray::TrayWrapper;
pub use wheel::WheelState;
