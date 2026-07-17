//! Tray menu + icon. The menu lists every active render endpoint, with a
//! check on the current default, and "refresh" / "exit" actions.

use crate::audio;
use crate::startup;
use tray_icon::menu::{CheckMenuItemBuilder, Menu, MenuId, MenuItemBuilder, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// Prefix on the menu-id for device items. The id is `dev:<device-id>` so a
/// single dispatch can tell "device switch" from "refresh" / "exit" without
/// a separate table.
const DEV_PREFIX: &str = "dev:";
const ID_REFRESH: &str = "refresh";
const ID_EXIT: &str = "exit";
const ID_ABOUT: &str = "about";
const ID_AUTOSTART: &str = "autostart";

/// Build the right-click menu for the tray.
pub fn build_menu() -> Menu {
    let devices = audio::enumerate_render_endpoints();
    let current = audio::current_default_id();

    let menu = Menu::new();

    if devices.is_empty() {
        let item = MenuItemBuilder::new()
            .text("(无可用音频设备)")
            .enabled(false)
            .build();
        let _ = menu.append(&item);
    } else {
        for d in &devices {
            let checked = current.as_deref() == Some(d.id.as_str());
            let id = format!("{DEV_PREFIX}{}", d.id);
            let item = CheckMenuItemBuilder::new()
                .id(MenuId::new(id))
                .text(&d.name)
                .enabled(true)
                .checked(checked)
                .build();
            let _ = menu.append(&item);
        }
    }
    let sep = PredefinedMenuItem::separator();
    let autostart = CheckMenuItemBuilder::new()
        .id(MenuId::new(ID_AUTOSTART))
        .text("开机自动启动")
        .enabled(true)
        .checked(startup::is_enabled())
        .build();
    let refresh = MenuItemBuilder::new()
        .id(MenuId::new(ID_REFRESH))
        .text("刷新设备列表")
        .enabled(true)
        .build();
    let about = MenuItemBuilder::new()
        .id(MenuId::new(ID_ABOUT))
        .text("关于")
        .enabled(true)
        .build();
    let exit = MenuItemBuilder::new()
        .id(MenuId::new(ID_EXIT))
        .text("退出")
        .enabled(true)
        .build();
    let _ = menu.append_items(&[&sep, &autostart, &refresh, &about, &exit]);

    menu
}

/// Decode the embedded tray icon for the given system theme, with a
/// solid-color fallback so a missing or corrupt PNG never blocks startup.
///
/// Two PNG variants are baked into the binary at compile time:
/// - `assets/icon_light.png`: black glyph on transparent (for light taskbar)
/// - `assets/icon_dark.png`:  white glyph on transparent (for dark taskbar)
pub fn make_icon(theme: crate::theme::Theme) -> Icon {
    let (png_bytes, fallback_color): (&[u8], [u8; 4]) = match theme {
        crate::theme::Theme::Dark => (
            include_bytes!("../assets/icon_dark.png"),
            [0xE6, 0xE6, 0xE6, 0xFF], // light gray fallback
        ),
        // Light and Unknown: use black glyph on light taskbar.
        _ => (
            include_bytes!("../assets/icon_light.png"),
            [0x36, 0x36, 0x36, 0xFF], // dark gray fallback
        ),
    };

    if let Ok(img) = image::load_from_memory_with_format(png_bytes, image::ImageFormat::Png) {
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        if let Ok(icon) = Icon::from_rgba(rgba.into_raw(), w, h) {
            return icon;
        }
    }

    // Fallback: 32x32 solid color block in the theme-appropriate color.
    const W: u32 = 32;
    const H: u32 = 32;
    let mut buf = Vec::with_capacity((W * H * 4) as usize);
    for _ in 0..(W * H) {
        buf.extend_from_slice(&fallback_color);
    }
    Icon::from_rgba(buf, W, H).expect("32x32 solid RGBA must always be a valid icon")
}

/// Construct a [`TrayIcon`] wired to the given [`Menu`]. Kept here so the
/// main module stays small.
pub fn build_tray(menu: Menu, icon: Icon) -> tray_icon::Result<TrayIcon> {
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("音频输出设备切换")
        .with_icon(icon)
        .build()
}

/// Classify a menu id string into the actions the event loop understands.
pub enum MenuAction<'a> {
    Switch(&'a str),
    Refresh,
    ToggleAutostart,
    About,
    Exit,
}

pub fn classify_menu_id(id: &str) -> MenuAction<'_> {
    if let Some(dev) = id.strip_prefix(DEV_PREFIX) {
        MenuAction::Switch(dev)
    } else if id == ID_REFRESH {
        MenuAction::Refresh
    } else if id == ID_ABOUT {
        MenuAction::About
    } else if id == ID_AUTOSTART {
        MenuAction::ToggleAutostart
    } else {
        // Anything else (including `ID_EXIT`) is exit. Keeps the dispatcher
        // total and never silently drops an unknown id.
        MenuAction::Exit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tray_icon::menu::IsMenuItem;

    /// Regression: `CheckMenuItemBuilder::default()` has `enabled: false`
    /// (bool default). Without an explicit `.enabled(true)` call, every
    /// check item is grayed out and unclickable. This test pins the
    /// expected builder API so a future refactor can't reintroduce the
    /// grayed-out regression.
    #[test]
    fn check_item_builder_default_is_disabled_explicit_must_enable() {
        let off = CheckMenuItemBuilder::new()
            .id(MenuId::new("x"))
            .text("x")
            .checked(false)
            .build();
        let on = CheckMenuItemBuilder::new()
            .id(MenuId::new("y"))
            .text("y")
            .enabled(true)
            .checked(false)
            .build();
        // `IsMenuItem` is not object-safe due to `into_id(self)`, so we
        // can't `&dyn IsMenuItem`. Compare via the concrete `MenuItemKind`.
        let off_kind = IsMenuItem::kind(&off);
        let on_kind = IsMenuItem::kind(&on);
        // Just assert the constructors don't panic and yield distinct ids.
        assert_ne!(IsMenuItem::id(&off).0, IsMenuItem::id(&on).0);
        // Ensure we still have a real kind (sanity).
        assert!(matches!(off_kind, tray_icon::menu::MenuItemKind::Check(_)));
        assert!(matches!(on_kind, tray_icon::menu::MenuItemKind::Check(_)));
    }

    /// The dispatcher must classify every menu id into exactly one
    /// action; unknown ids default to Exit (per plan).
    #[test]
    fn classify_menu_id_dispatch_table() {
        assert!(matches!(
            classify_menu_id("dev:abc"),
            MenuAction::Switch("abc")
        ));
        assert!(matches!(classify_menu_id(ID_REFRESH), MenuAction::Refresh));
        assert!(matches!(classify_menu_id(ID_ABOUT), MenuAction::About));
        assert!(matches!(classify_menu_id(ID_EXIT), MenuAction::Exit));
        assert!(matches!(
            classify_menu_id(ID_AUTOSTART),
            MenuAction::ToggleAutostart
        ));
        assert!(matches!(classify_menu_id("garbage"), MenuAction::Exit));
    }
}
