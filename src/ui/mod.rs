//! UI layer — tray icon, menu, overlay and wheel handling.

/// Internationalisation helper.
pub mod i18n;
/// Icon rendering.
pub mod icon;
/// Shared label sanitization/truncation for menu + overlay text.
pub(crate) mod label;
/// Tray menu builder.
pub mod menu;
/// Overlay (OSD) content formatting.
pub mod osd;
/// Tray wrapper.
pub mod tray;
/// Wheel acceleration state.
pub mod wheel;

/// Re-exported for `crate::app`.
pub use menu::MenuState;
pub use tray::TrayWrapper;
pub use wheel::WheelState;
