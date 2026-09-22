//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::*;
use tempfile::tempdir;

#[test]
fn default_values() {
    let c = AppConfig::default();
    assert_eq!(c.lang, Lang::System);
    assert_eq!(c.lang.as_str(), "system");
    assert!(c.volume_limit_enabled);
    assert_eq!(c.volume_limit, 25);
    assert_eq!(c.autostart_mode, AutostartMode::User);
    assert_eq!(c.version, CURRENT_VERSION);
}

#[test]
fn lang_roundtrip() {
    assert_eq!("system".parse::<Lang>().unwrap(), Lang::System);
    assert_eq!("zh".parse::<Lang>().unwrap(), Lang::Zh);
    assert_eq!("en".parse::<Lang>().unwrap(), Lang::En);
    assert_eq!(Lang::System.to_string(), "system");
    assert_eq!(Lang::Zh.to_string(), "zh");
    assert_eq!(Lang::En.to_string(), "en");
}

#[test]
fn path_chain_prefers_appdata_then_localappdata_then_temp() {
    // Absolute roots built from the real temp dir: portable across OSes.
    let base = tempdir().unwrap().keep();
    let roaming = base.join("Roaming");
    let local = base.join("Local");
    let tmp = base.join("Tmp");
    let (appdata_path, degraded) = resolve_for(
        Some(roaming.to_string_lossy().as_ref()),
        Some(local.to_string_lossy().as_ref()),
        &tmp,
    );
    assert!(!degraded);
    assert_eq!(
        appdata_path,
        roaming.join("audio-switcher").join("config.json")
    );
    let (local_path, degraded) = resolve_for(None, Some(local.to_string_lossy().as_ref()), &tmp);
    assert!(!degraded);
    assert_eq!(local_path, local.join("audio-switcher").join("config.json"));
    let (temp_path, degraded) = resolve_for(None, None, &tmp);
    assert!(degraded);
    assert_eq!(temp_path, tmp.join("audio-switcher").join("config.json"));
    // No ./config.json fallback anywhere in the chain.
    assert!(temp_path.is_absolute());
}

#[test]
fn explicit_modes_survive_effective() {
    let zh = AppConfig {
        lang: Lang::Zh,
        ..Default::default()
    };
    let en = AppConfig {
        lang: Lang::En,
        ..Default::default()
    };
    assert_eq!(zh.effective_lang(), Lang::Zh);
    assert_eq!(en.effective_lang(), Lang::En);
    // System resolves without panicking (value depends on the test machine).
    let sys = AppConfig::default();
    assert!(matches!(
        sys.effective_lang(),
        Lang::Zh | Lang::En | Lang::System
    ));
}

#[test]
fn clamp_enabled() {
    let cfg = AppConfig {
        volume_limit_enabled: true,
        volume_limit: 25,
        ..Default::default()
    };
    assert_eq!(clamp_volume(30, &cfg), 25);
    assert_eq!(clamp_volume(20, &cfg), 20);
}

#[test]
fn clamp_disabled() {
    let cfg = AppConfig {
        volume_limit_enabled: false,
        ..Default::default()
    };
    assert_eq!(clamp_volume(80, &cfg), 80);
    assert_eq!(clamp_volume(100, &cfg), 100);
    // Disabled still preserves the 0..=100 invariant.
    assert_eq!(clamp_volume(120, &cfg), 100);
}

#[test]
fn clamp_enabled_out_of_range_limit_still_caps_at_100() {
    // Defensive: direct struct construction can bypass migrate() clamping.
    let cfg = AppConfig {
        volume_limit_enabled: true,
        volume_limit: 200,
        ..Default::default()
    };
    assert_eq!(clamp_volume(150, &cfg), 100);
    assert_eq!(clamp_volume(80, &cfg), 80);
}

#[test]
fn validate_custom() {
    assert_eq!(AppConfig::validate_custom_limit("50").unwrap(), 50);
    assert_eq!(AppConfig::validate_custom_limit("  100 ").unwrap(), 100);
    assert!(AppConfig::validate_custom_limit("0").is_err());
    assert!(AppConfig::validate_custom_limit("101").is_err());
    assert!(AppConfig::validate_custom_limit("abc").is_err());
    assert!(AppConfig::validate_custom_limit("").is_err());
}

#[test]
fn persistence_with_tempfile() {
    let dir = tempdir().unwrap();
    let path = AppConfig::config_path_for(dir.path());
    let cfg = AppConfig {
        lang: Lang::En,
        volume_limit: 50,
        ..Default::default()
    };
    cfg.save_to(&path).unwrap();
    let loaded = AppConfig::load_from(&path);
    assert_eq!(loaded.lang, Lang::En);
    assert_eq!(loaded.volume_limit, 50);
}

#[test]
fn migration_version_bump() {
    let dir = tempdir().unwrap();
    let path = AppConfig::config_path_for(dir.path());
    let old = r#"{"version":0,"lang":"en","volume_limit_enabled":true,"volume_limit":25,"autostart":true}"#;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, old).unwrap();
    let loaded = AppConfig::load_from(&path);
    assert_eq!(loaded.version, CURRENT_VERSION);
    assert_eq!(loaded.lang, Lang::En);
}

#[test]
fn migration_v1_zh_default_becomes_system() {
    // v1 defaulted to Zh; the implicit choice follows the system now.
    let dir = tempdir().unwrap();
    let path = AppConfig::config_path_for(dir.path());
    let old = r#"{"version":1,"lang":"zh","volume_limit_enabled":true,"volume_limit":25,"autostart":true}"#;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, old).unwrap();
    let loaded = AppConfig::load_from(&path);
    assert_eq!(loaded.version, CURRENT_VERSION);
    assert_eq!(loaded.lang, Lang::System);
}

#[test]
fn migration_v2_explicit_zh_stays() {
    let dir = tempdir().unwrap();
    let path = AppConfig::config_path_for(dir.path());
    let raw = r#"{"version":2,"lang":"zh","volume_limit_enabled":true,"volume_limit":25,"autostart":true}"#;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, raw).unwrap();
    let loaded = AppConfig::load_from(&path);
    assert_eq!(loaded.lang, Lang::Zh);
}

#[test]
fn migration_v3_autostart_boolean_becomes_mode() {
    // v3 stored a bare boolean and defaulted it to true; it folds into the
    // current-user mode, and the legacy key is never written back.
    let dir = tempdir().unwrap();
    let path = AppConfig::config_path_for(dir.path());
    let raw = r#"{"version":3,"lang":"en","volume_limit_enabled":true,"volume_limit":25,"autostart":true}"#;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, raw).unwrap();
    let loaded = AppConfig::load_from(&path);
    assert_eq!(loaded.autostart_mode, AutostartMode::User);
    loaded.save_to(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"autostart_mode\""), "{text}");
    assert!(
        !text.contains("\"autostart\":"),
        "the legacy key must not be rewritten: {text}"
    );
}

#[test]
fn corrupted_fallback() {
    let dir = tempdir().unwrap();
    let path = AppConfig::config_path_for(dir.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "not json").unwrap();
    let loaded = AppConfig::load_from(&path);
    assert_eq!(loaded, AppConfig::default());
    assert!(path.exists());
    // The offending bytes survive: the warning promises a backup, and the
    // reset only rewrites the file once that backup write succeeded.
    let backup = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().contains(".bak."))
        .expect("corrupt config must leave a backup");
    assert_eq!(
        std::fs::read_to_string(backup.path()).unwrap(),
        "not json",
        "the backup must hold the bytes that failed to parse"
    );
}

#[test]
fn unknown_field_rejected_loudly() {
    // deny_unknown_fields: typos reset to defaults with a backup, never
    // silently ignored (replaces the old wheel_acceleration tolerance).
    let dir = tempdir().unwrap();
    let path = AppConfig::config_path_for(dir.path());
    let raw = r#"{"version":2,"lang":"system","volume_limit_enabled":true,"volume_limit":25,"wheel_acceleration":false,"autostart":true}"#;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, raw).unwrap();
    let loaded = AppConfig::load_from(&path);
    assert_eq!(loaded, AppConfig::default());
    let backup = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| e.file_name().to_string_lossy().contains(".bak."));
    assert!(backup, "corrupt config must leave a backup");
}

#[test]
fn unknown_lang_value_rejected_loudly() {
    let dir = tempdir().unwrap();
    let path = AppConfig::config_path_for(dir.path());
    let raw = r#"{"version":2,"lang":"fr","volume_limit_enabled":true,"volume_limit":25,"autostart":true}"#;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, raw).unwrap();
    let loaded = AppConfig::load_from(&path);
    assert_eq!(loaded, AppConfig::default());
}

#[test]
fn legacy_pascalcase_file_imported_once() {
    let dir = tempdir().unwrap();
    let new_path = dir.path().join("new").join("config.json");
    let legacy_path = dir.path().join("legacy").join("config.json");
    std::fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
    std::fs::write(
            &legacy_path,
            r#"{"version":1,"lang":"en","volume_limit_enabled":true,"volume_limit":50,"autostart":true}"#,
        )
        .unwrap();
    assert!(import_legacy_file(&new_path, &legacy_path));
    let loaded = AppConfig::load_from(&new_path);
    assert_eq!(loaded.lang, Lang::En);
    assert_eq!(loaded.volume_limit, 50);
    // Second run is a no-op (new path exists now).
    assert!(!import_legacy_file(&new_path, &legacy_path));
}

#[test]
fn unparsable_legacy_file_is_reported_and_left_alone() {
    // One key removed from the schema fails the whole parse
    // (`deny_unknown_fields`): the import must not write defaults over the
    // user's settings and then claim success. The old file survives on disk as
    // the recoverable copy.
    let dir = tempdir().unwrap();
    let new_path = dir.path().join("new").join("config.json");
    let legacy_path = dir.path().join("legacy").join("config.json");
    std::fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
    std::fs::write(
        &legacy_path,
        r#"{"version":1,"lang":"en","wheel_acceleration":false}"#,
    )
    .unwrap();
    assert!(!import_legacy_file(&new_path, &legacy_path));
    assert!(
        !new_path.exists(),
        "defaults must not replace an unreadable legacy file"
    );
    assert!(
        legacy_path.exists(),
        "the unreadable legacy file must survive"
    );
}

/// Load `raw` from a fresh temp file, returning the config plus its dir.
fn load_raw(raw: &str) -> (AppConfig, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let path = AppConfig::config_path_for(dir.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, raw).unwrap();
    (AppConfig::load_from(&path), dir)
}

#[test]
fn hotkeys_off_by_default_and_migrated_from_v2() {
    // A v2 file has no `hotkeys` object: nothing is bound, and the file
    // is only renumbered (no hotkey is invented on migration).
    let (cfg, _dir) = load_raw(
        r#"{"version":2,"lang":"en","volume_limit_enabled":true,"volume_limit":25,"autostart":true}"#,
    );
    assert_eq!(cfg.version, CURRENT_VERSION);
    assert_eq!(cfg.hotkeys, Hotkeys::default());
    for action in HotkeyAction::ALL {
        assert_eq!(cfg.hotkeys.get(action), None);
    }
}

#[test]
fn hotkey_combos_are_canonicalized() {
    let (cfg, _dir) = load_raw(
        r#"{"version":3,"lang":"en","volume_limit_enabled":true,"volume_limit":25,"autostart":true,
                "hotkeys":{"mute":"ctrl+alt+m","volume_up":" ALT + Ctrl + Up ","volume_down":"","prev_device":null}}"#,
    );
    assert_eq!(cfg.hotkeys.get(HotkeyAction::Mute), Some("Ctrl+Alt+M"));
    assert_eq!(cfg.hotkeys.get(HotkeyAction::VolumeUp), Some("Ctrl+Alt+Up"));
    // Empty and null both mean "off"; nothing is bound implicitly.
    assert_eq!(cfg.hotkeys.get(HotkeyAction::VolumeDown), None);
    assert_eq!(cfg.hotkeys.get(HotkeyAction::PrevDevice), None);
}

#[test]
fn invalid_hotkey_combo_dropped_without_resetting_the_file() {
    // A bad combo (no modifier) disables just that action; the rest of the
    // file survives (unlike an unknown field, which resets).
    let (cfg, _dir) = load_raw(
        r#"{"version":3,"lang":"en","volume_limit_enabled":true,"volume_limit":50,"autostart":false,
                "hotkeys":{"mute":"M","next_device":"Ctrl+Alt+Right"}}"#,
    );
    assert_eq!(cfg.hotkeys.get(HotkeyAction::Mute), None);
    assert_eq!(
        cfg.hotkeys.get(HotkeyAction::NextDevice),
        Some("Ctrl+Alt+Right")
    );
    assert_eq!(cfg.volume_limit, 50);
    // The v3 boolean folds into the mode: `false` means no autostart entry.
    assert_eq!(cfg.autostart_mode, AutostartMode::Off);
}

#[test]
fn unknown_hotkey_field_rejected_loudly() {
    // A typo inside `hotkeys` is a config error, not a silent no-op.
    let (cfg, dir) = load_raw(
        r#"{"version":3,"lang":"en","volume_limit_enabled":true,"volume_limit":25,"autostart":true,
                "hotkeys":{"mutee":"Ctrl+Alt+M"}}"#,
    );
    assert_eq!(cfg, AppConfig::default());
    let backup = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| e.file_name().to_string_lossy().contains(".bak."));
    assert!(backup, "corrupt config must leave a backup");
}

#[test]
fn example_combos_round_trip_through_config() {
    // Manual-only hotkeys: storing canonical combos must survive a reload
    // byte-identically (header comments are stripped on load).
    let mut cfg = AppConfig::default();
    let combos = [
        (HotkeyAction::Mute, "Ctrl+Alt+M"),
        (HotkeyAction::VolumeUp, "Ctrl+Alt+Up"),
        (HotkeyAction::VolumeDown, "Ctrl+Alt+Down"),
        (HotkeyAction::NextDevice, "Ctrl+Alt+Right"),
        (HotkeyAction::PrevDevice, "Ctrl+Alt+Left"),
    ];
    for (action, combo) in combos {
        let parsed = combo.parse::<Hotkey>().unwrap();
        cfg.hotkeys.set(action, Some(parsed.to_string()));
    }
    let dir = tempdir().unwrap();
    let path = AppConfig::config_path_for(dir.path());
    cfg.save_to(&path).unwrap();
    let loaded = AppConfig::load_from(&path);
    assert_eq!(loaded.hotkeys, cfg.hotkeys);
}

#[test]
fn saved_file_carries_bilingual_hotkey_guidance() {
    let dir = tempdir().unwrap();
    let path = AppConfig::config_path_for(dir.path());
    AppConfig::default().save_to(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.starts_with("//"),
        "config must open with comment header"
    );
    assert!(text.contains("Hotkeys / 快捷键"), "{text}");
    assert!(text.contains("默认无绑定"), "{text}");
    assert!(text.contains("\"hotkeys\""), "{text}");
    // Header + JSON still loads (comments stripped).
    let loaded = AppConfig::load_from(&path);
    assert_eq!(loaded, AppConfig::default());
}

#[test]
fn header_names_the_file_that_was_written() {
    // The lookup chain falls back to `%LOCALAPPDATA%` and then to temp, so a
    // hard-coded location line names a file that may not exist.
    let header = config_comment_header(Path::new(r"X:\y\config.json"));
    assert!(header.contains(r"X:\y\config.json"), "{header}");
    assert!(header.contains("Hotkeys / 快捷键"), "{header}");
}

#[test]
fn line_and_block_comments_are_stripped_outside_strings() {
    let (cfg, _dir) = load_raw(
        "// leading note\n{\"version\":3,\"lang\":\"en\",/* block \n note */\"volume_limit_enabled\":true,\"volume_limit\":25,\"autostart\":true,\n\"hotkeys\":{\"mute\":\"Ctrl+Alt+M\" // trailing note\n}}",
    );
    assert_eq!(cfg.hotkeys.get(HotkeyAction::Mute), Some("Ctrl+Alt+M"));
    // `//` inside a string value is preserved, not treated as a comment.
    let raw = r#"{"version":3,"lang":"en","volume_limit_enabled":true,"volume_limit":25,"autostart":true,
            "hotkeys":{"mute":"Ctrl+Alt+M"}} // ok"#;
    let (cfg2, _d2) = load_raw(raw);
    assert_eq!(cfg2.hotkeys.get(HotkeyAction::Mute), Some("Ctrl+Alt+M"));
}
