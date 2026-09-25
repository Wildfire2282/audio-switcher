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

/// The raw bytes of `name` under `HKCU\<subkey>`, or `None` when the key or the
/// value is absent.
///
/// Reads the registry directly (rather than through the module's own helpers)
/// so the assertions can pin the stored format: a `REG_SZ` command with its
/// terminator, and the twelve-byte Task Manager marker.
#[cfg(windows)]
fn read_registry_value(subkey: &str, name: &str) -> Option<Vec<u8>> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    use windows::core::PCWSTR;

    let subkey = crate::platform::utf16::wide_z(subkey);
    let name = crate::platform::utf16::wide_z(name);
    let mut hkey = HKEY(std::ptr::null_mut());
    // SAFETY: the subkey string outlives the call; `hkey` is written only on
    // success and closed on every path below.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_QUERY_VALUE,
            &raw mut hkey,
        )
    };
    if opened.is_err() {
        return None;
    }
    let mut len = 0u32;
    // SAFETY: a null data pointer with a size pointer asks for the size only.
    let sized = unsafe {
        RegQueryValueExW(
            hkey,
            PCWSTR(name.as_ptr()),
            None,
            None,
            None,
            Some(&raw mut len),
        )
    };
    if sized.is_err() {
        // SAFETY: closes the handle opened above.
        unsafe {
            let _ = RegCloseKey(hkey);
        };
        return None;
    }
    let mut data = vec![0u8; usize::try_from(len).unwrap_or(0)];
    // SAFETY: `data` holds exactly `len` bytes, the size just read.
    let read = unsafe {
        RegQueryValueExW(
            hkey,
            PCWSTR(name.as_ptr()),
            None,
            None,
            Some(data.as_mut_ptr()),
            Some(&raw mut len),
        )
    };
    // SAFETY: closes the handle opened above.
    unsafe {
        let _ = RegCloseKey(hkey);
    };
    read.is_ok().then_some(data)
}

#[test]
#[cfg(windows)]
#[ignore = "writes and removes the real HKCU Run value; run explicitly"]
fn run_value_round_trip() {
    let _gate = crate::INTEGRATION_GATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // The `auto-launch` behaviour this crate now implements itself, against the
    // real registry: write, read back as installed, then remove. The stored
    // bytes are the contract an installed copy of another version still has to
    // recognise, so they are asserted, not just the round trip.
    let _ = set_run_value(false);

    set_run_value(true).expect("enable");
    assert_eq!(
        run_value_state().expect("read back after enable"),
        RunEntry::Enabled,
        "the Run value must read as installed and enabled"
    );
    let exe = get_exe_path().expect("exe path");
    let expected: Vec<u8> = format!("\"{}\"", exe.display())
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .chain([0, 0])
        .collect();
    assert_eq!(
        read_registry_value(RUN_KEY, autostart_key_name()),
        Some(expected),
        "the stored Run command must be this exe, in quotes"
    );
    assert_eq!(
        read_registry_value(RUN_APPROVED_KEY, autostart_key_name()),
        Some(STARTUP_APPROVED_ENABLED.to_vec()),
        "the Task Manager marker must be rewritten as enabled"
    );

    set_run_value(false).expect("disable");
    assert_eq!(
        run_value_state().expect("read back after disable"),
        RunEntry::Absent,
        "the Run value must be gone, not merely switched off"
    );
    assert_eq!(
        read_registry_value(RUN_KEY, autostart_key_name()),
        None,
        "the Run value must be removed, not blanked"
    );
}

#[test]
fn startup_approved_state_follows_the_state_byte() {
    // The enabled forms the shell writes: never touched (0x06) and enabled by
    // the user (0x02).
    assert!(startup_approved_state(&[
        0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
    ]));
    assert!(startup_approved_state(&[
        0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
    ]));
    // Disabled: the same value with an odd state byte.
    assert!(!startup_approved_state(&[
        0x03, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
    ]));
    assert!(!startup_approved_state(&[
        0x07, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
    ]));
    // Switched off and on again: the state byte says enabled, and the timestamp
    // the shell stamps next to it must not be read as "disabled".
    assert!(startup_approved_state(&[
        0x02, 0, 0, 0, 0x40, 0x9A, 0x2C, 0x3D, 0x8B, 0x0E, 0xDB, 0x01,
    ]));
    // Nothing to remember reads as enabled.
    assert!(startup_approved_state(&[]));
}

#[test]
#[cfg(windows)]
#[ignore = "creates and deletes a real scheduled task; run explicitly"]
fn create_and_delete_task_round_trip() {
    let _gate = crate::INTEGRATION_GATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Exercises the production path end to end against the real tool.
    let _ = delete_task();
    create_task().expect("create task");
    assert!(task_exists().expect("query task"), "task should exist");
    delete_task().expect("delete task");
    assert!(!task_exists().expect("query task"), "task should be gone");
}
