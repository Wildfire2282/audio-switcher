//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::*;

/// End-to-end check for the external volume-change notification: an
/// independent COM thread changes the master volume (simulating another
/// app / media keys); the registered `IAudioEndpointVolumeCallback` must
/// raise the flag consumed by [`take_volume_changed`].
///
/// Volume is restored afterwards.
#[test]
#[ignore = "requires WASAPI hardware, run with --ignored"]
fn integration_external_volume_change_notifies() {
    let _gate = crate::INTEGRATION_GATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    use super::notify::VOLUME_NOTIFY_ID;
    use std::time::Duration;
    let _com = crate::platform::ComGuard::init().expect("COM init");
    let mut backend = WasapiBackend::new();
    // The MTA worker registers the callback within ~1s; wait for it.
    let mut registered = false;
    for _ in 0..40 {
        if VOLUME_NOTIFY_ID
            .lock()
            .expect("volume notify id poisoned")
            .is_some()
        {
            registered = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        registered,
        "MTA worker did not register the volume callback"
    );
    let (vol0, _) = backend.get_volume_and_mute().expect("volume read");
    let new_vol = if vol0 >= 50 { vol0 - 10 } else { vol0 + 10 };

    let set_ext = |vol: u32| {
        std::thread::spawn(move || {
            let _com = crate::platform::ComGuard::init();
            // SAFETY: standard WASAPI calls on an initialized COM thread.
            unsafe {
                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
                let dev = enumerator
                    .GetDefaultAudioEndpoint(eRender, eMultimedia)
                    .unwrap();
                let vol_iface: IAudioEndpointVolume = dev.Activate(CLSCTX_ALL, None).unwrap();
                vol_iface
                    .SetMasterVolumeLevelScalar(vol as f32 / 100.0, std::ptr::null())
                    .unwrap();
            }
        })
        .join()
        .unwrap();
    };

    set_ext(new_vol);
    let (vol_now, _) = backend.get_volume_and_mute().expect("post-change read");
    assert_eq!(vol_now, new_vol, "external volume change did not apply");
    // The engine invokes MTA callbacks on its own threads — poll for the
    // flag without a message pump.
    let mut fired = false;
    for _ in 0..40 {
        if take_volume_changed() {
            fired = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // Restore via the backend (suppressed — must not re-raise the flag).
    // Self-notifications arrive asynchronously, so wait past the
    // suppression window and assert none arrived.
    backend.set_volume(vol0).expect("restore volume");
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !take_volume_changed(),
            "suppressed self-change raised the flag"
        );
    }
    assert!(
        fired,
        "external volume change did not raise the notify flag"
    );
}

/// End-to-end check for the capture path: enumerate `eCapture`
/// endpoints, read the default capture device, switch to another
/// capture device (or re-set the current one on single-mic machines),
/// and restore the original default on scope exit.
///
/// Alters the system default capture device while running; the
/// original is always restored via RAII.
#[test]
#[ignore = "requires WASAPI hardware, run with --ignored"]
fn integration_capture_enumerate_switch_restores() {
    let _gate = crate::INTEGRATION_GATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    use std::time::Duration;
    let _com = crate::platform::ComGuard::init().expect("COM init");
    let mut backend = WasapiBackend::new();
    backend.clear_cache();
    let inputs = backend
        .enumerate_input_devices()
        .expect("capture enumerate");
    assert!(!inputs.is_empty(), "expected at least one capture device");
    let current = backend
        .get_default_input_device()
        .expect("default capture device");
    assert!(
        inputs.iter().any(|d| d.id == current.id),
        "default capture device missing from enumeration"
    );
    // RAII: restore the original default even if an assert below panics.
    struct RestoreCaptureDefault {
        id: String,
    }
    impl Drop for RestoreCaptureDefault {
        fn drop(&mut self) {
            let mut backend = WasapiBackend::new();
            let _ = backend.set_default_input_device(&self.id);
        }
    }
    let _restore = RestoreCaptureDefault {
        id: current.id.clone(),
    };
    // Prefer a different device to prove the switch; fall back to the
    // current one (idempotent path) on single-mic machines.
    let target = inputs
        .iter()
        .find(|d| d.id != current.id)
        .unwrap_or(&current);
    backend
        .set_default_input_device(&target.id)
        .expect("switch capture device");
    // Endpoint switch propagates asynchronously — poll for it.
    let mut after = None;
    for _ in 0..40 {
        backend.clear_cache();
        after = backend.get_default_input_device();
        if after.as_ref().is_some_and(|d| d.id == target.id) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        after.map(|d| d.id),
        Some(target.id.clone()),
        "capture switch did not apply"
    );
}
