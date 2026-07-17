//! Verify menu items are enabled (not grayed out). Catches the bug
//! where `MenuItemBuilder::default()` produces `enabled = false`.

use tray_icon::menu::ContextMenu;

use audio_switcher::tray;
use windows::Win32::UI::WindowsAndMessaging::{
    GetMenuItemCount, GetMenuItemInfoW, MENUITEMINFOW, MIIM_FTYPE, MIIM_STATE, MFT_SEPARATOR,
    MFS_CHECKED, MFS_DISABLED, HMENU,
};

fn main() {
    let _ = unsafe {
        windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        )
    };

    let menu = tray::build_menu();
    let hmenu = HMENU(menu.hpopupmenu() as *mut _);
    let count = unsafe { GetMenuItemCount(Some(hmenu)) };
    println!("[menu-check] item count = {count}");

    let mut any_disabled = false;
    for i in 0..count {
        let mut info = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE | MIIM_STATE,
            ..unsafe { std::mem::zeroed() }
        };
        let _ = unsafe { GetMenuItemInfoW(hmenu, i.try_into().unwrap(), true, &mut info as *mut _) };
        let sep = info.fType == MFT_SEPARATOR;
        let dis = (info.fState.0 & MFS_DISABLED.0) != 0;
        let chk = (info.fState.0 & MFS_CHECKED.0) != 0;
        let label = if sep {
            "sep"
        } else if dis {
            "DISABLED!"
        } else {
            "ok"
        };
        let chk_lbl = if chk { "checked" } else { "unchecked" };
        println!("  [{i}] {label:>10} {chk_lbl:>10}");
        if dis && !sep {
            any_disabled = true;
        }
    }
    if any_disabled {
        eprintln!("[menu-check] FAIL: at least one clickable item is disabled");
        std::process::exit(1);
    }
    println!("[menu-check] PASS: all clickable items are enabled");
}
