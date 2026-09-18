//! Audio Switcher — Windows tray audio switcher.
//!
//! Library crate holds all reusable logic; `main.rs` installs crash reporting
//! (file sink + panic hook) then runs guards → `App::run`.
//!
//! # Architecture
//! - `config` — strongly typed configuration and persistence
//! - `audio` — `AudioBackend` abstraction
//! - `platform` — Windows platform wrappers
//! - `ui` — tray UI
//! - `app` — runtime
//!
//! Layering: `platform` owns every Win32 call, `ui` turns state into text and
//! colour, `app` owns loop policy. Neither `ui` nor `app` calls Win32 directly,
//! and `platform` never invents UI text a caller did not hand it.
//! `tests/architecture.rs` enforces this instead of trusting a reader.
//!
//! # Where to look
//!
//! Task → the files that decide it; everything else is context you can skip.
//! Unit tests live in a sibling `tests.rs` next to the module they test, so a
//! production read never drags them along.
//! - wheel volume, hover gate, click dismissal → `platform::mouse_hook`,
//!   `app::poll::{poll_wheel, poll_click}`, `app::action::show_osd`
//! - the runtime: `App` state and the loop → `app` (`mod.rs`), per-frame pumps
//!   → `app::poll`, user actions → `app::action`, startup wiring → `app::setup`
//! - overlay: does it appear, where → `platform::osd`; what it looks like →
//!   `platform::osd::win::{draw, layout}`, `ui::osd`, `platform::theme`
//! - tray icon, context menu, menu ids → `ui::tray`, `ui::menu`, `app::handler`
//! - audio IO: devices, volume, COM callbacks → `audio::wasapi::{notify, policy}`
//! - config fields, migration, paths → `config`
//! - global hotkeys → `platform::hotkey`
//! - autostart, dialogs, single instance, logging → `platform`
//!
//! # Invariants
//!
//! Cheap to break and expensive to notice; each of these has already cost a bug:
//! - A window procedure handling `WM_PAINT` must pair `BeginPaint`/`EndPaint` on
//!   every path. Skipping them leaves the update region unvalidated, so Windows
//!   re-posts `WM_PAINT` forever and starves the entire message loop.
//! - The volume overlay must never activate and must stay click-through
//!   (`WS_EX_NOACTIVATE` / `WS_EX_TRANSPARENT`): activation steals the tray
//!   icon's hover state, and that hover is what gates wheel volume.
//! - The overlay mirrors the shell's own menu styling rather than the Windows 11
//!   XAML flyouts, and carries no tooltip — the shell would draw that tooltip
//!   exactly where the overlay appears. See `platform::osd`.
//! - The tray icon is never given a tooltip, and `TOOL_ID` is the single source
//!   for the mutex name, the config/log directories, and the autostart name.
//! - `app::App`'s volume/mute/device mirror is authoritative for our own writes
//!   and is resynced from the backend on every external change; a write that is
//!   read back instead would put an endpoint round-trip on the wheel's hot path.
//! - Encode Win32 strings through `platform::utf16`: the NUL terminator is a
//!   memory-safety detail, not a formatting one.
//!
//! # Verification
//!
//! A green `cargo test` does not clear a UI change. The overlay's worst failures
//! are invisible to unit tests: they pass while the app is unusable. Confirm the
//! message loop still runs (idle CPU near zero, the tray menu opening) before
//! believing an overlay change, and prefer runtime evidence — the render tests
//! read the painted pixels back precisely because "it compiles and the tests
//! pass" was once true of a build that pegged a core and ignored the wheel.
//!
//! The gate is `scripts/smoke.ps1` (build, test, fmt, `clippy -D warnings`).
#![warn(missing_docs)]
#![warn(unsafe_op_in_unsafe_fn)]
// Baseline-inherent duplicates (single-instance 0.3.3 pulls thiserror 1/syn 1;
// tray-icon's tree pulls old unix-gated nix/memoffset/bitflags/miniz_oxide):
// locked; upgrades need a separate decision. Re-check on every baseline
// bump with `cargo tree -i <crate>`; new direct-dep duplicates stay denied.
#![allow(clippy::multiple_crate_versions)]
// `pub` below is the minimum the `main` binary and doctests need; everything
// else defaults to `pub(crate)` (no `prelude` module: only two import sites
// ever used it, a glob re-export is not worth the indirection).
pub mod app;
pub(crate) mod audio;
pub mod config;
pub(crate) mod platform;
pub(crate) mod ui;
// Curated re-exports for the binary entry point (`main` is a separate crate,
// so anything it touches is `pub` with this justification).
pub use config::{AppConfig, Lang};
pub use platform::dialog::show_critical;
pub use platform::logging::init as init_crash_reporting;
pub use platform::{ComError, ComGuard, InstanceError, SingleInstanceGuard};

/// Canonical tool id (kebab-case). Single source for the mutex name, the
/// config/log directory names, and the autostart display-name derivation.
pub const TOOL_ID: &str = "audio-switcher";

/// PascalCase display name, derived from [`TOOL_ID`]. Used for the autostart
/// registry value, dialog titles, and the menu title only.
pub const TOOL_DISPLAY_NAME: &str = "AudioSwitcher";

/// Tool release homepage. Single definition per crate; the About menu item
/// opens exactly this URL (validated through [`platform::shell::Url`]).
pub const ABOUT_URL: &str = "https://github.com/Wildfire2282/audio-switcher";

/// Canonical single-instance mutex id: `{kebab}-single-instance-v1`.
#[must_use]
pub fn single_instance_id() -> String {
    format!("{TOOL_ID}-single-instance-v1")
}

/// Derive the PascalCase display name from a kebab-case tool id
/// (`"audio-switcher"` → `"AudioSwitcher"`).
#[must_use]
pub fn display_name_for(kebab: &str) -> String {
    kebab
        .split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                // Uppercase may expand (e.g. ligatures); lowercase the rest.
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_id_mapping() {
        // Mutex, autostart display name, and config dir all derive from TOOL_ID.
        assert_eq!(TOOL_ID, "audio-switcher");
        assert_eq!(single_instance_id(), "audio-switcher-single-instance-v1");
        assert_eq!(display_name_for(TOOL_ID), TOOL_DISPLAY_NAME);
        assert_eq!(TOOL_DISPLAY_NAME, "AudioSwitcher");
    }

    #[test]
    fn about_url_single_const() {
        // Prefix locked: About always lands on the tool release homepage.
        assert!(ABOUT_URL.starts_with("https://github.com/Wildfire2282/"));
        // Scheme validated through the same gate the menu handler uses.
        let url: crate::platform::shell::Url = ABOUT_URL.parse().expect("ABOUT_URL valid");
        assert_eq!(url.as_str(), ABOUT_URL);
    }
}
