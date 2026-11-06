//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along.

use super::*;
use windows::Win32::Media::Audio::AUDIO_VOLUME_NOTIFICATION_DATA;
use windows::core::{GUID, PCWSTR};

/// Our own write's echo is ignored by its event context alone, so an external
/// change arriving right beside it is never swallowed.
#[test]
fn our_own_echo_never_hides_an_external_change() {
    let mut own = AUDIO_VOLUME_NOTIFICATION_DATA {
        guidEventContext: SELF_EVENT_CONTEXT,
        ..Default::default()
    };
    let mut external = AUDIO_VOLUME_NOTIFICATION_DATA {
        guidEventContext: GUID::from_u128(0),
        ..Default::default()
    };
    let _ = take_volume_changed();
    // SAFETY: each payload stays live through its synchronous callback.
    unsafe {
        let _ = volume_on_notify(std::ptr::null_mut(), &raw mut own);
        assert!(
            !take_volume_changed(),
            "our own write must not request a resync"
        );
        let _ = volume_on_notify(std::ptr::null_mut(), &raw mut external);
        let _ = volume_on_notify(std::ptr::null_mut(), &raw mut own);
        assert!(
            take_volume_changed(),
            "a self echo must not discard the external change that preceded it"
        );
        let _ = volume_on_notify(std::ptr::null_mut(), &raw mut own);
        let _ = volume_on_notify(std::ptr::null_mut(), &raw mut external);
        assert!(
            take_volume_changed(),
            "an external change right after our write must survive"
        );
    }
}

/// A display-name change is user-visible menu content, not a volume echo.
#[test]
fn a_device_rename_is_never_treated_as_an_echo() {
    let _ = take_device_changed();
    // SAFETY: the callback only reads the supplied property key.
    unsafe {
        let _ = device_on_property_changed(
            std::ptr::null_mut(),
            PCWSTR::null(),
            PKEY_Device_FriendlyName,
        );
    }
    assert!(
        take_device_changed(),
        "a device rename must invalidate the device list"
    );
}

/// A broken COM caller is reported, never dereferenced.
#[test]
fn a_null_notification_payload_is_rejected() {
    // SAFETY: the callback checks for null before reading.
    let hr = unsafe { volume_on_notify(std::ptr::null_mut(), std::ptr::null_mut()) };
    assert!(hr.is_err(), "a null payload must not report success");
}
