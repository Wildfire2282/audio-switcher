//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::*;

#[test]
fn key_names_locked() {
    // Renaming the value would orphan every installed Run entry.
    assert_eq!(autostart_key_name(), "AudioSwitcher");
    assert_eq!(
        autostart_key_name(),
        crate::display_name_for(crate::TOOL_ID)
    );
    // The cleanup must catch the legacy names and never the canonical one.
    assert_eq!(LEGACY_AUTOSTART_KEYS, &["audio-switcher", "Audio Switcher"]);
    assert!(!LEGACY_AUTOSTART_KEYS.contains(&autostart_key_name()));
}

#[test]
fn unknown_never_implies_off() {
    let unknown = AutostartState::Unknown("test".to_string());
    assert_ne!(unknown, AutostartState::Off);
    assert_ne!(unknown, AutostartState::User);
    assert_ne!(unknown, AutostartState::Admin);
}

#[test]
fn mode_maps_every_known_state_and_none_for_unknown() {
    // This is the menu's only input: a state mapped to the wrong mode would
    // check the wrong entry, and `Unknown` must never render as a real mode.
    assert_eq!(AutostartState::Off.mode(), Some(AutostartMode::Off));
    assert_eq!(AutostartState::User.mode(), Some(AutostartMode::User));
    assert_eq!(AutostartState::Admin.mode(), Some(AutostartMode::Admin));
    assert_eq!(AutostartState::Unknown("unreadable".into()).mode(), None);
}

#[test]
fn exe_path_available_in_tests() {
    assert!(get_exe_path().is_some());
}

#[test]
fn mode_defaults_to_user() {
    // v3 defaulted `autostart` to true; the mode type keeps that behaviour so
    // upgrading users see no change.
    assert_eq!(AutostartMode::default(), AutostartMode::User);
}

#[test]
fn mode_json_is_lowercase_and_round_trips() {
    for (mode, json) in [
        (AutostartMode::Off, "\"off\""),
        (AutostartMode::User, "\"user\""),
        (AutostartMode::Admin, "\"admin\""),
    ] {
        assert_eq!(serde_json::to_string(&mode).expect("serializes"), json);
        assert_eq!(
            serde_json::from_str::<AutostartMode>(json).expect("deserializes"),
            mode
        );
    }
}

#[test]
fn task_cache_defaults_to_absent() {
    // The cache starts optimistic-absent so the menu is never stuck grayed
    // while the startup query is still in flight.
    assert!(!admin_task_cached());
}

#[test]
fn helper_switch_is_not_a_normal_launch() {
    // The prefix must stay a single distinctive token: `main` exits on it before
    // the single-instance guard, so a false positive would kill startup.
    assert!(HELPER_ARG_PREFIX.starts_with("--"));
    assert!(HELPER_ARG_PREFIX.ends_with('='));
    assert_ne!(HELPER_TASK_ON, HELPER_TASK_OFF);
}

#[test]
#[cfg(windows)]
fn describe_carries_exit_code_and_tool_output() {
    // The detail is what makes a failed `schtasks` diagnosable; a bare exit
    // code was the whole story before.
    let out = std::process::Command::new("cmd")
        .args(["/c", "echo boom & exit 3"])
        .output()
        .expect("spawn cmd");
    let detail = describe(&out);
    assert!(detail.contains("exit code 3"), "{detail}");
    assert!(detail.contains("boom"), "{detail}");
}

#[test]
#[ignore = "creates and deletes a real scheduled task; run explicitly"]
fn create_and_delete_task_round_trip() {
    // Exercises the production path end to end against the real tool.
    let _ = delete_task();
    create_task().expect("create task");
    assert!(task_exists().expect("query task"), "task should exist");
    delete_task().expect("delete task");
    assert!(!task_exists().expect("query task"), "task should be gone");
}
