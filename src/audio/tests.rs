//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::*;

#[test]
fn mock_enumerate_cache() {
    let devs = vec![AudioDevice {
        id: "a".into(),
        name: "Speaker".into(),
    }];
    let mut m = MockBackend::new(devs.clone(), Some("a".into()));
    let first = m.enumerate_devices().unwrap();
    assert_eq!(first.len(), 1);
    let count_before = m.enumerate_count;
    let second = m.enumerate_devices().unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(
        m.enumerate_count, count_before,
        "should hit cache within 800ms"
    );
    std::thread::sleep(std::time::Duration::from_millis(850));
    let _third = m.enumerate_devices().unwrap();
    assert_eq!(m.enumerate_count, count_before + 1);
}

#[test]
fn mock_empty_shows_empty() {
    let devs = vec![AudioDevice {
        id: "a".into(),
        name: "Sp".into(),
    }];
    let mut m = MockBackend::new(devs.clone(), Some("a".into()));
    let _ = m.enumerate_devices().unwrap();
    m.devices = vec![];
    std::thread::sleep(std::time::Duration::from_millis(850));
    let after = m.enumerate_devices().unwrap();
    // After removal, should show empty — not stale cached list.
    assert_eq!(after.len(), 0);
}

#[test]
fn clamp_via_backend() {
    let cfg = AppConfig {
        volume_limit: 25,
        volume_limit_enabled: true,
        ..Default::default()
    };
    let mut m = MockBackend::new(vec![], None);
    m.volume = 80;
    m.clamp_volume_if_needed(&cfg).unwrap();
    assert_eq!(m.volume, 25);
}

#[test]
fn set_default_input_device_mock() {
    let outs = vec![AudioDevice {
        id: "a".into(),
        name: "A".into(),
    }];
    let ins = vec![
        AudioDevice {
            id: "m1".into(),
            name: "Mic".into(),
        },
        AudioDevice {
            id: "m2".into(),
            name: "Headset Mic".into(),
        },
    ];
    let mut m = MockBackend::new(outs, Some("a".into())).with_inputs(ins, Some("m1".into()));
    assert_eq!(
        m.get_default_input_device().map(|d| d.id),
        Some("m1".into())
    );
    m.set_default_input_device("m2").unwrap();
    assert_eq!(
        m.get_default_input_device().map(|d| d.id),
        Some("m2".into())
    );
    assert!(m.set_default_input_device("nope").is_err());
    // Output default is untouched by input switching.
    assert_eq!(m.get_default_device().map(|d| d.id), Some("a".into()));
    // Snapshot carries both flows.
    let snap = m.fetch_snapshot_clamped(&AppConfig::default());
    assert_eq!(snap.input_devices.len(), 2);
    assert_eq!(snap.default_input_device.map(|d| d.id), Some("m2".into()));
}

#[test]
fn set_default_device_mock() {
    let devs = vec![
        AudioDevice {
            id: "a".into(),
            name: "A".into(),
        },
        AudioDevice {
            id: "b".into(),
            name: "B".into(),
        },
    ];
    let mut m = MockBackend::new(devs, Some("a".into()));
    m.set_default_device("b").unwrap();
    assert_eq!(m.default_id.as_deref(), Some("b"));
    assert!(m.set_default_device("c").is_err());
}
