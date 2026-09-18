//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::*;

fn test_devices() -> Vec<AudioDevice> {
    vec![
        AudioDevice {
            id: "a".into(),
            name: "Speaker".into(),
        },
        AudioDevice {
            id: "b".into(),
            name: "Headset".into(),
        },
    ]
}

fn test_cfg() -> AppConfig {
    AppConfig::default()
}

/// Top-level action ids in menu order (separators carry none).
fn menu_ids(menu: &Menu) -> Vec<String> {
    menu.items()
        .into_iter()
        .filter_map(|kind| match kind {
            muda::MenuItemKind::MenuItem(item) => Some(item.id().0.clone()),
            muda::MenuItemKind::Check(item) => Some(item.id().0.clone()),
            muda::MenuItemKind::Submenu(sub) => Some(sub.id().0.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn title_carries_display_name_and_version() {
    let title = title_text();
    assert!(title.starts_with(crate::TOOL_DISPLAY_NAME));
    assert!(title.contains(env!("CARGO_PKG_VERSION")));
    assert_eq!(
        title,
        format!(
            "{} v{}",
            crate::TOOL_DISPLAY_NAME,
            env!("CARGO_PKG_VERSION")
        )
    );
    // The menu head item renders exactly this string (same builder call).
    let head = MenuItem::with_id(id::TITLE, title_text(), false, None);
    assert_eq!(head.text(), title);
}

#[test]
fn vol_preset_round_trip() {
    for preset in VOLUME_PRESETS {
        let menu_id = id::vol_preset(*preset);
        assert_eq!(id::parse_vol_preset(&menu_id), Some(*preset));
    }
    assert_eq!(id::parse_vol_preset("vol_30"), None);
    assert_eq!(id::parse_vol_preset("vol_x"), None);
    assert_eq!(id::parse_vol_preset("mute"), None);
}

#[test]
fn three_way_lang_group_checks_mode() {
    for mode in [Lang::System, Lang::Zh, Lang::En] {
        let cfg = AppConfig {
            lang: mode,
            ..test_cfg()
        };
        let base = MenuState {
            cfg: &cfg,
            devices: &[],
            default_id: None,
            inputs: &[],
            default_input_id: None,
            muted: false,
            autostart: &AutostartState::Disabled,
            ui_lang: Lang::En,
        };
        let handles = build_menu(&base).expect("menu builds");
        assert_eq!(handles.lang_system.is_checked(), mode == Lang::System);
        assert_eq!(handles.lang_zh.is_checked(), mode == Lang::Zh);
        assert_eq!(handles.lang_en.is_checked(), mode == Lang::En);
    }
}

#[test]
fn tail_group_order_is_fixed() {
    let cfg = test_cfg();
    let base = MenuState {
        cfg: &cfg,
        devices: &[],
        default_id: None,
        inputs: &[],
        default_input_id: None,
        muted: false,
        autostart: &AutostartState::Disabled,
        ui_lang: Lang::En,
    };
    let handles = build_menu(&base).expect("menu builds");
    // Separators carry no stable id; the last five actionable items are
    // the frozen tail: refresh → autostart → language → about → exit.
    let actionable = menu_ids(&handles.menu);
    assert!(actionable.len() >= 5, "{actionable:?}");
    assert_eq!(
        actionable[actionable.len() - 5..],
        [
            id::REFRESH,
            id::AUTOSTART,
            id::LANGUAGE,
            id::ABOUT,
            id::EXIT,
        ]
        .map(str::to_string),
    );
}

#[test]
fn autostart_unknown_grayed_never_off() {
    let cfg = test_cfg();
    let base = MenuState {
        cfg: &cfg,
        devices: &[],
        default_id: None,
        inputs: &[],
        default_input_id: None,
        muted: false,
        autostart: &AutostartState::Disabled,
        ui_lang: Lang::En,
    };
    let unknown = build_menu(&MenuState {
        autostart: &AutostartState::Unknown("no read".into()),
        ..base
    })
    .expect("menu builds");
    assert!(!unknown.autostart.is_enabled());
    assert!(!unknown.autostart.is_checked());
    assert!(unknown.autostart.text().contains("unknown"));

    let enabled = build_menu(&MenuState {
        autostart: &AutostartState::Enabled,
        ..base
    })
    .expect("menu builds");
    assert!(enabled.autostart.is_enabled());
    assert!(enabled.autostart.is_checked());
}

#[test]
fn sync_state_updates_checks_in_place() {
    let cfg = test_cfg();
    let ui = cfg.effective_lang();
    let devices = test_devices();
    let base = MenuState {
        cfg: &cfg,
        devices: &devices,
        default_id: Some("a"),
        inputs: &[],
        default_input_id: None,
        muted: false,
        autostart: &AutostartState::Disabled,
        ui_lang: ui,
    };
    let mut handles = build_menu(&base).expect("menu builds");
    assert!(handles.sync_state(&MenuState {
        default_id: Some("b"),
        muted: true,
        ..base
    }));
}

#[test]
fn sync_state_rebuilds_on_rename_reorder_and_lang() {
    let cfg = test_cfg();
    let ui = cfg.effective_lang();
    let devices = test_devices();
    let base = MenuState {
        cfg: &cfg,
        devices: &devices,
        default_id: Some("a"),
        inputs: &[],
        default_input_id: None,
        muted: false,
        autostart: &AutostartState::Disabled,
        ui_lang: ui,
    };
    let mut handles = build_menu(&base).expect("menu builds");
    // Rename requires rebuild — labels are baked at build time.
    let mut renamed = devices.clone();
    renamed[0].name = "Renamed".into();
    assert!(!handles.sync_state(&MenuState {
        devices: &renamed,
        ..base
    }));
    // Reorder requires rebuild.
    let mut reordered = devices.clone();
    reordered.reverse();
    assert!(!handles.sync_state(&MenuState {
        devices: &reordered,
        ..base
    }));
    // Language change requires rebuild — all labels change.
    let other_ui = if ui == Lang::Zh { Lang::En } else { Lang::Zh };
    assert!(!handles.sync_state(&MenuState {
        ui_lang: other_ui,
        ..base
    }));
}

#[test]
fn sync_state_tracks_input_devices() {
    let cfg = test_cfg();
    let ui = cfg.effective_lang();
    let devices = test_devices();
    let inputs = vec![
        AudioDevice {
            id: "m1".into(),
            name: "Mic".into(),
        },
        AudioDevice {
            id: "m2".into(),
            name: "Headset Mic".into(),
        },
    ];
    let base = MenuState {
        cfg: &cfg,
        devices: &devices,
        default_id: Some("a"),
        inputs: &inputs,
        default_input_id: Some("m1"),
        muted: false,
        autostart: &AutostartState::Disabled,
        ui_lang: ui,
    };
    let mut handles = build_menu(&base).expect("menu builds");
    // Default switch applies in place.
    assert!(handles.sync_state(&MenuState {
        default_input_id: Some("m2"),
        ..base
    }));
    // Input added requires rebuild.
    let mut grown = inputs.clone();
    grown.push(AudioDevice {
        id: "m3".into(),
        name: "Cam Mic".into(),
    });
    assert!(!handles.sync_state(&MenuState {
        inputs: &grown,
        default_input_id: Some("m2"),
        ..base
    }));
}

#[test]
fn hotkey_settings_entry_follows_sound_settings() {
    let cfg = test_cfg();
    let base = MenuState {
        cfg: &cfg,
        devices: &[],
        default_id: None,
        inputs: &[],
        default_input_id: None,
        muted: false,
        autostart: &AutostartState::Disabled,
        ui_lang: Lang::En,
    };
    for ui_lang in [Lang::En, Lang::Zh] {
        let handles = build_menu(&MenuState { ui_lang, ..base }).expect("menu builds");
        let ids = menu_ids(&handles.menu);
        let mixer = ids
            .iter()
            .position(|id| id == id::OPEN_MIXER)
            .expect("mixer entry");
        let sound = ids
            .iter()
            .position(|id| id == id::OPEN_SOUND)
            .expect("sound entry");
        let hotkey = ids
            .iter()
            .position(|id| id == id::OPEN_HOTKEY_SETTINGS)
            .expect("hotkey settings entry");
        assert_eq!(mixer + 1, sound, "{ids:?}");
        assert_eq!(sound + 1, hotkey, "{ids:?}");
        // Retired submenu ids never appear.
        assert!(!ids.iter().any(|id| id == "hotkeys"), "{ids:?}");
        assert!(!ids.iter().any(|id| id.starts_with("hotkey_")), "{ids:?}");
    }
}

#[test]
fn hotkey_config_change_keeps_menu_in_place() {
    use crate::config::Hotkeys;
    let cfg = test_cfg();
    let ui = cfg.effective_lang();
    let base = MenuState {
        cfg: &cfg,
        devices: &[],
        default_id: None,
        inputs: &[],
        default_input_id: None,
        muted: false,
        autostart: &AutostartState::Disabled,
        ui_lang: ui,
    };
    let mut handles = build_menu(&base).expect("menu builds");
    assert!(handles.sync_state(&base));
    // Hotkeys are manual-only now: editing them must not force a rebuild.
    let bound = AppConfig {
        hotkeys: Hotkeys {
            volume_up: Some("Ctrl+Alt+Up".into()),
            ..Hotkeys::default()
        },
        ..test_cfg()
    };
    assert!(handles.sync_state(&MenuState {
        cfg: &bound,
        ..base
    }));
}

#[test]
fn empty_enumeration_shows_placeholder() {
    let cfg = test_cfg();
    let base = MenuState {
        cfg: &cfg,
        devices: &[],
        default_id: None,
        inputs: &[],
        default_input_id: None,
        muted: false,
        autostart: &AutostartState::Disabled,
        ui_lang: Lang::En,
    };
    let empty = build_menu(&base).expect("menu builds");
    assert!(menu_ids(&empty.menu).contains(&id::NO_DEVICES.to_string()));

    let devices = test_devices();
    let populated = build_menu(&MenuState {
        devices: &devices,
        default_id: Some("a"),
        ..base
    })
    .expect("menu builds");
    assert!(!menu_ids(&populated.menu).contains(&id::NO_DEVICES.to_string()));
}
