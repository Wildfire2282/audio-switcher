//! Tray icon wrapper.
//!
//! Construction failures return [`TrayError`]; runtime updates log through
//! `tracing` and the loop continues on a failed icon push.
//!
//! Carries **no tooltip** on purpose: the shell draws it on hover exactly where
//! the volume overlay appears, covering the feedback it was meant to complement.

use thiserror::Error;
use tray_icon::{TrayIcon, TrayIconBuilder};

use crate::ui::icon::make_icon;
use crate::ui::menu::{MenuHandles, MenuState, build_menu};

/// How long a tray-icon rectangle may be reused.
///
/// `Shell_NotifyIconGetRect` crosses into Explorer, and the hover gate asks for
/// the rect twice per wheel notch. The icon *can* move with the taskbar, so the
/// cache expires rather than holding the last answer for the session.
const ICON_RECT_TTL: std::time::Duration = std::time::Duration::from_millis(250);

/// A tray-icon rectangle `(x, y, width, height)`, with the instant it was read.
type CachedIconRect = ((i32, i32, i32, i32), std::time::Instant);

/// Tray construction failure.
#[derive(Debug, Error)]
pub enum TrayError {
    /// The underlying `tray-icon` build failed (e.g. Explorer not running).
    /// The source keeps the OS error kind so triage can tell Explorer-down
    /// apart from a bad icon (`{:#}` renders the full chain).
    #[error("tray icon build failed")]
    Build(#[from] tray_icon::Error),
    /// The embedded tray RGBA bytes are invalid (deterministic packaging
    /// error, not retried beyond the boot bound).
    #[error("tray icon invalid")]
    Icon(#[from] tray_icon::BadIcon),
    /// A menu entry or submenu could not be created. Win32 menu creation can
    /// fail under resource pressure, so this is reported rather than panicked:
    /// startup dialogs and exits, a runtime rebuild keeps the current menu.
    #[error("tray menu build failed")]
    Menu(#[from] muda::Error),
}

/// Wrapper around `tray-icon`'s `TrayIcon` holding the menu handles.
pub struct TrayWrapper {
    pub tray: TrayIcon,
    /// Kept alive for the tray icon's lifetime.
    pub handles: MenuHandles,
    /// Last mute state pushed to the icon.
    ///
    /// The volume wheel repaints feedback once per notch; an unconditional
    /// `set_icon` would spend a `Shell_NotifyIcon` on every one of them to
    /// redraw the exact same bitmap.
    pushed_mute: Option<bool>,
    /// Last `Shell_NotifyIconGetRect` answer and when it was taken.
    ///
    /// `Cell` (not `RefCell`) because the payload is `Copy` and the tray icon is
    /// a UI-thread singleton like the overlay's paint cache.
    rect_cache: std::cell::Cell<Option<CachedIconRect>>,
}

impl TrayWrapper {
    /// Build a new tray icon and menu for `state`.
    ///
    /// No tooltip is registered (`NIF_TIP` stays clear), so hovering the icon
    /// shows nothing and cannot cover the volume overlay.
    ///
    /// # Errors
    ///
    /// Returns [`TrayError::Build`] when the tray icon cannot be created, and
    /// [`TrayError::Menu`] when the menu itself cannot be built.
    pub fn new(state: &MenuState<'_>) -> Result<Self, TrayError> {
        let handles = build_menu(state)?;
        let icon = make_icon(state.muted)?;
        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            // Attach the boot menu here, not only from the post-snapshot rebuild
            // in `assemble`: if that rebuild fails, a menu-less tray icon would
            // have no way to switch devices or exit at all.
            .with_menu(Box::new(handles.menu.clone()))
            .with_menu_on_left_click(false)
            .build()?;
        Ok(Self {
            tray,
            handles,
            pushed_mute: Some(state.muted),
            rect_cache: std::cell::Cell::new(None),
        })
    }

    /// Move only the mute check for an external mute change.
    ///
    /// A full [`Self::sync_menu`] would walk the device list for nothing; the
    /// icon is updated separately so the two read-outs stay in step.
    pub fn sync_mute(&mut self, muted: bool) {
        self.handles.set_muted(muted);
    }

    /// Update the tray icon for `muted`, skipping an unchanged state.
    ///
    /// A failed push leaves the recorded state alone so the next call retries
    /// instead of silently believing the icon is current.
    pub fn update_icon_if_changed(&mut self, muted: bool) {
        if self.pushed_mute == Some(muted) {
            return;
        }
        let icon = match make_icon(muted) {
            Ok(icon) => icon,
            Err(e) => {
                tracing::warn!("tray make_icon failed: {e:?}");
                return;
            }
        };
        if let Err(e) = self.tray.set_icon(Some(icon)) {
            tracing::warn!("tray set_icon failed: {e:?}");
            return;
        }
        self.pushed_mute = Some(muted);
    }

    /// The tray icon's screen rectangle as `(x, y, width, height)`.
    ///
    /// Anchors the volume overlay to the icon and answers the hover gate, so it
    /// is asked once per wheel notch — the answer is cached for
    /// [`ICON_RECT_TTL`] instead of taking the Explorer round-trip every time.
    /// `None` while the shell has not reported a rectangle yet (the caller then
    /// falls back to the screen).
    #[must_use]
    pub fn icon_rect(&self) -> Option<(i32, i32, i32, i32)> {
        if let Some((rect, taken)) = self.rect_cache.get() {
            if taken.elapsed() < ICON_RECT_TTL {
                return Some(rect);
            }
        }
        let rect = self.tray.rect()?;
        let rect = (
            rect.position.x as i32,
            rect.position.y as i32,
            i32::try_from(rect.size.width).unwrap_or(0),
            i32::try_from(rect.size.height).unwrap_or(0),
        );
        self.rect_cache.set(Some((rect, std::time::Instant::now())));
        Some(rect)
    }

    /// Rebuild the context menu from `state`.
    ///
    /// A failure logs and keeps the menu that is already on screen. Dropping it
    /// would leave the tray icon with no way to switch devices, and panicking
    /// would kill a running app over a transient Win32 resource problem.
    pub fn rebuild_menu(&mut self, state: &MenuState<'_>) {
        let new_handles = match build_menu(state) {
            Ok(handles) => handles,
            Err(e) => {
                tracing::warn!("menu rebuild failed, keeping the current menu: {e}");
                return;
            }
        };
        self.tray.set_menu(Some(Box::new(new_handles.menu.clone())));
        self.handles = new_handles;
    }

    /// Update the context menu from current state.
    ///
    /// Applies checks/enabled states in place when the device list is
    /// unchanged; falls back to a full rebuild only when devices were
    /// added/removed/reordered.
    pub fn sync_menu(&mut self, state: &MenuState<'_>) {
        if !self.handles.sync_state(state) {
            self.rebuild_menu(state);
        }
    }
}
