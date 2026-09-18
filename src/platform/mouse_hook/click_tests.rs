//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::is_button_down;
use windows::Win32::UI::WindowsAndMessaging::{
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCLBUTTONDOWN,
    WM_NCMBUTTONDOWN, WM_NCRBUTTONDOWN, WM_NCXBUTTONDOWN, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_XBUTTONDOWN,
};

#[test]
fn every_button_down_counts_as_a_click() {
    for msg in [
        WM_LBUTTONDOWN,
        WM_RBUTTONDOWN,
        WM_MBUTTONDOWN,
        WM_XBUTTONDOWN,
        WM_NCLBUTTONDOWN,
        WM_NCRBUTTONDOWN,
        WM_NCMBUTTONDOWN,
        WM_NCXBUTTONDOWN,
    ] {
        assert!(is_button_down(msg), "{msg:#06x} must dismiss the overlay");
    }
}

#[test]
fn ups_moves_and_wheel_are_not_clicks() {
    for msg in [WM_LBUTTONUP, WM_RBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL] {
        assert!(
            !is_button_down(msg),
            "{msg:#06x} must not dismiss the overlay"
        );
    }
}
