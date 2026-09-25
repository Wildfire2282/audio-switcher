//! Unit tests for tray window detection in `mouse_hook`.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::is_tray_class_name;

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[test]
fn matches_known_taskbar_and_tray_classes() {
    for class in [
        "Shell_TrayWnd",
        "Shell_SecondaryTrayWnd",
        "NotifyIconOverflowWindow",
        "TopLevelWindowForOverflowXamlIsland",
        "TrayNotifyWnd",
        "AudioSwitcherVolumeOsd",
    ] {
        let wide = to_wide(class);
        assert!(
            is_tray_class_name(&wide),
            "expected '{class}' to be recognized as a tray or taskbar window"
        );
    }
}

#[test]
fn rejects_non_tray_and_game_classes() {
    for class in [
        "CASCADIA_HOSTING_WINDOW_CLASS",
        "UnityWndClass",
        "UnrealWindow",
        "Valve001",
        "Chrome_WidgetWin_1",
        "Notepad",
        "",
        "Shell_Tray",
        "Shell_TrayWnd_Extra",
    ] {
        let wide = to_wide(class);
        assert!(
            !is_tray_class_name(&wide),
            "expected '{class}' NOT to be recognized as a tray or taskbar window"
        );
    }
}
