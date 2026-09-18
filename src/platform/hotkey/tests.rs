//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::*;

#[test]
fn parse_canonical_and_round_trip() {
    for combo in [
        "Ctrl+Alt+M",
        "Ctrl+Alt+Up",
        "Ctrl+Alt+Down",
        "Ctrl+Alt+Right",
        "Ctrl+Alt+Left",
    ] {
        let hotkey: Hotkey = combo.parse().expect("example parses");
        assert_eq!(hotkey.to_string(), combo);
        assert_ne!(hotkey.modifiers(), 0);
    }
}

#[test]
fn parse_is_case_and_order_insensitive() {
    let a: Hotkey = "ctrl+alt+m".parse().unwrap();
    let b: Hotkey = "Alt+Control+M".parse().unwrap();
    assert_eq!(a, b);
    assert_eq!(a.to_string(), "Ctrl+Alt+M");
}

#[test]
fn parse_named_keys() {
    assert_eq!(
        "Ctrl+Shift+F5".parse::<Hotkey>().unwrap().to_string(),
        "Ctrl+Shift+F5"
    );
    assert_eq!(
        "Win+Space".parse::<Hotkey>().unwrap().to_string(),
        "Win+Space"
    );
    assert_eq!(
        "Ctrl+Alt+Down".parse::<Hotkey>().unwrap().to_string(),
        "Ctrl+Alt+Down"
    );
    assert_eq!(
        "Ctrl+Alt+9".parse::<Hotkey>().unwrap().to_string(),
        "Ctrl+Alt+9"
    );
}

#[test]
fn parse_rejects_bad_shapes() {
    for raw in [
        "",
        "M",
        "Ctrl",
        "Ctrl+M+Alt",
        "Ctrl+Alt+Nope",
        "Ctrl+Ctrl+M",
        "Foo+M",
        "Ctrl++M",
    ] {
        assert!(raw.parse::<Hotkey>().is_err(), "{raw} must be rejected");
    }
    assert_eq!(
        "".parse::<Hotkey>().unwrap_err(),
        HotkeyParseError::Malformed
    );
    assert_eq!(
        "M".parse::<Hotkey>().unwrap_err(),
        HotkeyParseError::NoModifier
    );
    assert_eq!(
        "Foo+M".parse::<Hotkey>().unwrap_err(),
        HotkeyParseError::UnknownModifier
    );
    assert_eq!(
        "Ctrl+Ctrl+M".parse::<Hotkey>().unwrap_err(),
        HotkeyParseError::DuplicateModifier
    );
    assert_eq!(
        "Ctrl+Nope".parse::<Hotkey>().unwrap_err(),
        HotkeyParseError::UnknownKey
    );
}

#[test]
fn action_ids_are_stable_and_reversible() {
    for action in HotkeyAction::ALL {
        assert_eq!(HotkeyAction::from_id(action.id()), Some(action));
    }
    assert_eq!(HotkeyAction::from_id(0), None);
    assert_eq!(HotkeyAction::from_id(6), None);
    // Config keys must stay unique (they are the `hotkeys` field names).
    let mut keys: Vec<&str> = HotkeyAction::ALL.iter().map(|a| a.config_key()).collect();
    keys.sort_unstable();
    let count = keys.len();
    keys.dedup();
    assert_eq!(keys.len(), count);
}

#[test]
fn summarize_lists_every_occupied_combo() {
    let occupied = vec![
        (HotkeyAction::Mute, "Ctrl+Alt+M".parse::<Hotkey>().unwrap()),
        (
            HotkeyAction::VolumeUp,
            "Ctrl+Alt+Up".parse::<Hotkey>().unwrap(),
        ),
    ];
    let text = summarize(&occupied);
    assert!(text.contains("Ctrl+Alt+M (Toggle Mute)"), "{text}");
    assert!(text.contains("Ctrl+Alt+Up (Volume Up)"), "{text}");
    // The error renders the same list (single formatting path).
    assert!(HotkeyError(occupied).to_string().contains(&text));
}

#[test]
fn pending_queue_drains_lowest_id_first() {
    // `note_pending` is the Win32 path; the drain order is exercised here
    // through the same mask the pump feeds.
    PENDING.store(
        bit(HotkeyAction::NextDevice) | bit(HotkeyAction::Mute),
        std::sync::atomic::Ordering::Release,
    );
    assert_eq!(take_pending(), Some(HotkeyAction::Mute));
    assert_eq!(take_pending(), Some(HotkeyAction::NextDevice));
    assert_eq!(take_pending(), None);
}
