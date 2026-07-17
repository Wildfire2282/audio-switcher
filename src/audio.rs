//! Audio render (output) device enumeration + default switch.
//!
//! Built directly on the pattern from
//! `com-policy-config 0.6.0/examples/change_default_device.rs`:
//! `MMDeviceEnumerator` to enumerate active render endpoints and to query the
//! current default, `IPolicyConfig` (the undocumented `PolicyConfigClient`
//! COM class) to actually switch the default for all three `ERole`s.

use com_policy_config::{IPolicyConfig, PolicyConfigClient};
use windows::core::Result;
use windows::core::PCWSTR;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    eCommunications, eConsole, eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
    DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CLSCTX_ALL, STGM_READ,
};
use widestring::U16CStr;

/// One render endpoint.
#[derive(Debug, Clone)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
}

/// Enumerate active render (output) endpoints. Returns an empty `Vec` on any
/// failure so the menu can still render (with a "no devices" placeholder).
pub fn enumerate_render_endpoints() -> Vec<AudioDevice> {
    let enumerator: IMMDeviceEnumerator = match unsafe {
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
    } {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let collection = match unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) } {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let count = match unsafe { collection.GetCount() } {
        Ok(n) => n,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let device = match unsafe { collection.Item(i) } {
            Ok(d) => d,
            Err(_) => continue,
        };

        let store = match unsafe { device.OpenPropertyStore(STGM_READ) } {
            Ok(s) => s,
            Err(_) => continue,
        };

        let prop = match unsafe { store.GetValue(&PKEY_Device_FriendlyName) } {
            Ok(p) => p,
            Err(_) => continue,
        };

        // PROPVARIANT.Anonymous (PROPVARIANT_0) -> .Anonymous (PROPVARIANT_0_0) ->
        // .Anonymous (PROPVARIANT_0_0_0) -> .pwszVal : PWSTR. The union
        // access requires `unsafe`; the property is documented to be a
        // VT_LPWSTR for PKEY_Device_FriendlyName.
        let pwsz = unsafe { prop.Anonymous.Anonymous.Anonymous.pwszVal };
        let name = unsafe { pwsz.display() }.to_string();

        let id_pwstr = match unsafe { device.GetId() } {
            Ok(p) => p,
            Err(_) => continue,
        };
        let id = unsafe { U16CStr::from_ptr_str(id_pwstr.0) }
            .to_string_lossy();

        out.push(AudioDevice { id, name });
    }
    out
}

/// Returns the device id of the current default render endpoint, or `None`
/// on failure.
pub fn current_default_id() -> Option<String> {
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()?;

    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }.ok()?;

    let id_pwstr = unsafe { device.GetId() }.ok()?;
    let id = unsafe { U16CStr::from_ptr_str(id_pwstr.0) }.to_string_lossy();
    Some(id)
}

/// Set the default render endpoint for `device_id` across all three roles
/// (console, multimedia, communications) — mirrors the behavior of Windows'
/// "Set Default" in the sound panel.
pub fn set_default(device_id: &str) -> Result<()> {
    // Re-enumerate to look up the matching device's id-pwstr (we need a
    // `PCWSTR` to the COM-owned string).
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }?;

    let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }?;
    let count = unsafe { collection.GetCount() }?;

    // Look up the matching device. The COM string returned by GetId is
    // owned by the IMMDevice, so we must perform the COM call while the
    // device is still alive (i.e. inside the loop iteration).
    let policy: IPolicyConfig =
        unsafe { CoCreateInstance(&PolicyConfigClient, None, CLSCTX_ALL) }?;

    let mut found = false;
    for i in 0..count {
        let device = unsafe { collection.Item(i) }?;
        let id_pwstr = unsafe { device.GetId() }?;
        let candidate = unsafe { U16CStr::from_ptr_str(id_pwstr.0) }.to_string_lossy();
        if candidate != device_id {
            continue;
        }
        let pcw = PCWSTR(id_pwstr.0);
        // Per the upstream example: best-effort across all three roles;
        // errors on a single role are not fatal.
        unsafe {
            let _ = policy.SetDefaultEndpoint(pcw, eConsole);
            let _ = policy.SetDefaultEndpoint(pcw, eMultimedia);
            let _ = policy.SetDefaultEndpoint(pcw, eCommunications);
        }
        found = true;
        break;
    }

    if !found {
        return Err(windows::core::Error::new(
            windows::core::HRESULT(0x80070490u32 as i32), // ERROR_NOT_FOUND
            "device id not found in enumerated endpoints",
        ));
    }
    Ok(())
}
