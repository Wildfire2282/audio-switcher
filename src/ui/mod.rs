//! UI layer — tray icon, menu, overlay and wheel handling.

/// Internationalisation helper.
pub mod i18n;
/// Icon rendering.
pub mod icon;
/// Tray menu builder.
pub mod menu;
/// Overlay (OSD) content formatting.
pub mod osd;
/// Shared label sanitization/truncation for menu + overlay text.
pub(crate) mod text;
/// Tray wrapper.
pub mod tray;
/// Wheel acceleration state.
pub mod wheel;

/// Re-exported for `crate::app`.
pub use menu::MenuState;
pub use tray::TrayWrapper;
pub use wheel::WheelState;
