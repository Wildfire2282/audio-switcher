#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::ptr_as_ptr,
    clippy::borrow_as_ptr
)]
use super::{AudioBackend, AudioDevice, AudioError, AudioSnapshot};
use crate::config::{AppConfig, clamp_volume};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering as AtomicOrdering};
#[cfg(windows)]
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
#[cfg(windows)]
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
#[cfg(windows)]
use windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, EDataFlow, IMMDevice, IMMDeviceCollection, IMMDeviceEnumerator,
    IMMNotificationClient, MMDeviceEnumerator, eCapture, eMultimedia, eRender,
};
#[cfg(windows)]
use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
#[cfg(windows)]
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ};
#[cfg(windows)]
use windows::Win32::System::Variant::VT_LPWSTR;
#[cfg(windows)]
use windows::core::{GUID, HRESULT, Interface, PCWSTR};
#[cfg(windows)]
type SetDefaultEndpointFn =
    unsafe extern "system" fn(*mut std::ffi::c_void, PCWSTR, i32) -> HRESULT;
/// Set the default endpoint through undocumented `IPolicyConfig` interfaces.
///
/// # Safety
///
/// The caller must pass a NUL-free `device_id` (encoded below as UTF-16 with
/// a single terminator) and a valid `ERole` value for `role`, with COM
/// initialized on this thread. Offsets are probed per candidate: only the
/// primary offset with `S_OK` counts, so a mis-probe surfaces as an error
/// (or a no-op visibility call on the Vista path, which is rejected by the
/// primary-offset discipline), never as an out-of-bounds vtable read — every
/// slot read below is null-checked first.
#[cfg(windows)]
unsafe fn set_default_endpoint_raw(device_id: &str, role: i32) -> windows::core::Result<()> {
    use windows::core::IUnknown;
    // Multi-CLSID/vtable-offset fallback: Windows 11 builds differ, so a
    // single GUID/offset fails with 0x80040154 or mis-calls
    // SetEndpointVisibility (looks fine but never switches).
    // Per audioswitch/IPolicyConfig.h:
    //   IPolicyConfig::SetDefaultEndpoint @ vtbl[13]
    //   IPolicyConfigVista::SetDefaultEndpoint @ vtbl[12]
    // Each CLSID binds its canonical IID with its primary offset; only the
    // primary offset with S_OK counts as success, so a Vista client probed
    // at 13 cannot fake success via SetEndpointVisibility.
    const IID_IPOLICYCONFIG: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
    const IID_IPOLICYCONFIG_VISTA: GUID = GUID::from_u128(0x568b9108_44bf_40b4_9006_86afe5b5a620);
    const CANDIDATES: &[(GUID, GUID, usize)] = &[
        (
            GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9),
            IID_IPOLICYCONFIG,
            13,
        ),
        (
            GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2ea9),
            IID_IPOLICYCONFIG,
            13,
        ),
        (
            GUID::from_u128(0x294935ce_f637_4e7c_a41b_ab255460b862),
            IID_IPOLICYCONFIG_VISTA,
            12,
        ),
        (
            GUID::from_u128(0x294935ce_f588_4bd5_9f8c_bab13166b487),
            IID_IPOLICYCONFIG_VISTA,
            12,
        ),
    ];
    type QiFn = unsafe extern "system" fn(
        *mut std::ffi::c_void,
        *const GUID,
        *mut *mut std::ffi::c_void,
    ) -> HRESULT;
    type ReleaseFn = unsafe extern "system" fn(*mut std::ffi::c_void) -> u32;
    let wide: Vec<u16> = crate::platform::wide::wide_z(device_id);
    let mut last_err: Option<windows::core::Error> = None;
    for &(clsid, iid, primary_off) in CANDIDATES {
        // SAFETY: CoCreateInstance with valid CLSID, no aggregation, CLSCTX_ALL.
        let instance: windows::core::Result<IUnknown> =
            unsafe { CoCreateInstance(&clsid, None, CLSCTX_ALL) };
        let Ok(unk) = instance else {
            if let Err(e) = instance {
                last_err = Some(e);
            }
            continue;
        };
        // 1) Prefer QI to the canonical IID, then call the primary offset on that interface.
        let raw_unk = unk.as_raw();
        debug_assert!(!raw_unk.is_null(), "CoCreateInstance returned null object");
        // SAFETY: deref of a COM object pointer is sound when non-null; the
        // vtable pointer itself is checked below before any slot read.
        let vtbl_unk = unsafe { *(raw_unk as *mut *mut *mut std::ffi::c_void) };
        if !vtbl_unk.is_null() {
            // SAFETY: slot 0 of any COM vtable is IUnknown::QueryInterface.
            let qi: QiFn = unsafe { std::mem::transmute(*vtbl_unk) };
            let mut iface: *mut std::ffi::c_void = std::ptr::null_mut();
            // SAFETY: `qi` is the object's own QueryInterface; `iid` borrows a
            // live GUID and `iface` is a valid out-pointer.
            let hr_qi = unsafe { qi(raw_unk, std::ptr::from_ref(&iid), &mut iface) };
            if hr_qi.is_ok() && !iface.is_null() {
                // SAFETY: `iface` came from a successful QI; its vtable read is
                // null-checked before the primary-offset slot is touched.
                let vtbl_iface = unsafe { *(iface as *mut *mut *mut std::ffi::c_void) };
                if !vtbl_iface.is_null() {
                    // SAFETY: primary offset holds SetDefaultEndpoint for the
                    // candidate IID by the header contract above; a mismatch
                    // fails the HRESULT check and is never treated as success.
                    let func: SetDefaultEndpointFn =
                        unsafe { std::mem::transmute(*vtbl_iface.add(primary_off)) };
                    // SAFETY: `iface` is live until the Release below; `wide`
                    // outlives the call.
                    let hr = unsafe { func(iface, PCWSTR(wide.as_ptr()), role) };
                    // Release iface
                    // SAFETY: slot 2 of any COM vtable is IUnknown::Release;
                    // balances exactly this QI.
                    let rel: ReleaseFn = unsafe { std::mem::transmute(*vtbl_iface.add(2)) };
                    unsafe { rel(iface) };
                    if hr.is_ok() {
                        return Ok(());
                    }
                    last_err = Some(windows::core::Error::from(hr));
                    // On Vista, a failed primary offset stops probing this CLSID:
                    // trying the other offset could fake S_OK; move to the next CLSID.
                    continue;
                }
                // QI succeeded but the vtable is null: release as fallback.
                if !vtbl_iface.is_null() {
                    let rel: ReleaseFn = unsafe { std::mem::transmute(*vtbl_iface.add(2)) };
                    unsafe { rel(iface) };
                }
            }
        }
        // 2) QI unsupported here: call the primary offset on the raw IUnknown
        //    pointer (concrete classes usually implement the interface, so the raw call works).
        // SAFETY: re-read of the checked object pointer; null-checked below.
        let vtbl = unsafe { *(raw_unk as *mut *mut *mut std::ffi::c_void) };
        if vtbl.is_null() {
            last_err = Some(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x8000_4005_u32 as i32,
            )));
            continue;
        }
        // SAFETY: same primary-offset discipline as path 1; a mismatch fails
        // the HRESULT check below and is never treated as success.
        let func: SetDefaultEndpointFn = unsafe { std::mem::transmute(*vtbl.add(primary_off)) };
        // SAFETY: `raw_unk` is the live CoCreateInstance object; `wide` outlives the call.
        let hr = unsafe { func(raw_unk, PCWSTR(wide.as_ptr()), role) };
        if hr.is_ok() {
            return Ok(());
        }
        last_err = Some(windows::core::Error::from(hr));
    }
    Err(
        last_err.unwrap_or(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x8004_0154_u32 as i32,
        ))),
    )
}

#[cfg(windows)]
static DEVICE_CHANGED: AtomicBool = AtomicBool::new(false);

/// Window (ms) during which notifications after a self-initiated change are
/// treated as echoes of that change. Endpoint notifications typically arrive
/// within tens of milliseconds; 200ms covers the tail while minimizing the
/// window in which a genuine external change could be coalesced away.
#[cfg(windows)]
const SUPPRESS_WINDOW_MS: u64 = 200;

#[cfg(windows)]
static SUPPRESS_UNTIL_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Monotonic milliseconds since boot. Uses `GetTickCount64`, which is
/// unaffected by wall-clock adjustments (NTP, manual changes) — a wall
/// clock could otherwise stretch or collapse the suppression window.
#[cfg(windows)]
fn now_ms() -> u64 {
    // SAFETY: `GetTickCount64` takes no arguments and has no failure modes.
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}

/// Suppress self-initiated audio-change notifications for `ms` milliseconds.
///
/// Endpoint-volume/property notifications are delivered asynchronously via
/// the audio service, so they arrive *after* the setter call returns — an
/// RAII guard scoped to the call is not enough. A short window after each
/// self-initiated change is used instead; external changes inside the
/// window are rare and self-correct on the next external change.
#[cfg(windows)]
fn suppress_self_changes_for(ms: u64) {
    SUPPRESS_UNTIL_MS.store(now_ms().saturating_add(ms), AtomicOrdering::Release);
}

/// Whether self-initiated changes are currently being suppressed.
#[cfg(windows)]
fn suppress_notify() -> bool {
    now_ms() < SUPPRESS_UNTIL_MS.load(AtomicOrdering::Acquire)
}

/// Returns and clears the device-change flag set by `IMMNotificationClient`.
#[cfg(windows)]
pub fn take_device_changed() -> bool {
    DEVICE_CHANGED.swap(false, AtomicOrdering::AcqRel)
}

/// Set when the endpoint volume/mute changed externally (media keys, other
/// apps, system mixer). Self-initiated changes are suppressed via
/// [`suppress_self_changes_for`].
#[cfg(windows)]
static VOLUME_CHANGED: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
/// Returns and clears the external volume-change flag.
pub fn take_volume_changed() -> bool {
    VOLUME_CHANGED.swap(false, AtomicOrdering::AcqRel)
}

/// Manual COM callback objects (hand-written vtables on `windows::` paths
/// only, no extra codegen dependency; everything here resolves through the
/// `windows` re-exports).
///
/// Both objects share one layout: a `#[repr(C)]` header (vtable pointer
/// first, then the refcount) followed by no payload — the callbacks only
/// flip atomics. Each class publishes its own static vtable; the three
/// `IUnknown` slots funnel into the shared `com_addref`/`com_release`
/// helpers plus a per-class `QueryInterface`.
// Hungarian `lpVtbl` matches the COM ABI naming used by the vtable structs.
#[cfg(windows)]
#[repr(C)]
#[allow(non_snake_case)]
struct ComHeader {
    lpVtbl: *const std::ffi::c_void,
    refs: AtomicU32,
}

/// Bump the object refcount; returns the new count.
#[cfg(windows)]
fn com_addref(header: *mut ComHeader) -> u32 {
    // SAFETY: `header` is a live leaked callback object; `refs` is valid.
    unsafe {
        (*header)
            .refs
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
    }
}

/// Drop a reference; frees the box at zero. Returns the remaining count.
#[cfg(windows)]
fn com_release(header: *mut ComHeader) -> u32 {
    // SAFETY: `header` is a live leaked callback object. AcqRel pairs with
    // every AddRef; the Box is rebuilt exactly once at zero.
    unsafe {
        let remaining = (*header)
            .refs
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel)
            - 1;
        if remaining == 0 {
            drop(Box::from_raw(header));
        }
        remaining
    }
}

/// Notified by the audio engine whenever the endpoint volume or mute state
/// changes (`IAudioEndpointVolumeCallback`). Only touches atomics so it is
/// safe to invoke from any COM thread.
#[cfg(windows)]
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolumeCallback_Vtbl;
#[cfg(windows)]
static VOLUME_CALLBACK_VTBL: IAudioEndpointVolumeCallback_Vtbl =
    IAudioEndpointVolumeCallback_Vtbl {
        base__: windows::core::IUnknown_Vtbl {
            QueryInterface: volume_query_interface,
            AddRef: volume_add_ref,
            Release: volume_release,
        },
        OnNotify: volume_on_notify,
    };

#[cfg(windows)]
unsafe extern "system" fn volume_query_interface(
    this: *mut std::ffi::c_void,
    iid: *const windows::core::GUID,
    interface: *mut *mut std::ffi::c_void,
) -> windows::core::HRESULT {
    use windows::Win32::Foundation::{E_NOINTERFACE, S_OK};
    use windows::core::{IUnknown, Interface};
    // SAFETY: COM contract — `iid`/`interface` valid; `this` is a live object.
    unsafe {
        if iid.is_null() || interface.is_null() {
            return E_NOINTERFACE;
        }
        let iid = &*iid;
        if iid == &IUnknown::IID
            || iid
                == &<windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolumeCallback as Interface>::IID
        {
            *interface = this;
            com_addref(this as *mut ComHeader);
            S_OK
        } else {
            *interface = std::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

#[cfg(windows)]
unsafe extern "system" fn volume_add_ref(this: *mut std::ffi::c_void) -> u32 {
    com_addref(this as *mut ComHeader)
}

#[cfg(windows)]
unsafe extern "system" fn volume_release(this: *mut std::ffi::c_void) -> u32 {
    com_release(this as *mut ComHeader)
}

#[cfg(windows)]
unsafe extern "system" fn volume_on_notify(
    _this: *mut std::ffi::c_void,
    _notify: *mut windows::Win32::Media::Audio::AUDIO_VOLUME_NOTIFICATION_DATA,
) -> windows::core::HRESULT {
    use windows::Win32::Foundation::S_OK;
    if suppress_notify() {
        return S_OK;
    }
    VOLUME_CHANGED.store(true, AtomicOrdering::Release);
    S_OK
}

#[cfg(windows)]
fn create_volume_callback() -> windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolumeCallback
{
    use windows::core::Interface;
    let header = Box::new(ComHeader {
        lpVtbl: std::ptr::from_ref(&VOLUME_CALLBACK_VTBL) as *const std::ffi::c_void,
        refs: AtomicU32::new(1),
    });
    // SAFETY: leaked with refcount 1; header layout matches the COM object
    // contract (vtable pointer first), ownership moves to the interface.
    unsafe {
        windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolumeCallback::from_raw(
            Box::into_raw(header) as *mut std::ffi::c_void,
        )
    }
}

/// The volume callback object only touches atomics, so it may be invoked
/// from any thread. The handle is owned by the MTA worker for process lifetime.
#[cfg(windows)]
struct VolumeCallbackHolder(
    #[allow(dead_code)] windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolumeCallback,
);
#[cfg(windows)]
// SAFETY: see struct doc.
unsafe impl Send for VolumeCallbackHolder {}
#[cfg(windows)]
// SAFETY: see struct doc.
unsafe impl Sync for VolumeCallbackHolder {}
#[cfg(windows)]
static VOLUME_CALLBACK: std::sync::OnceLock<VolumeCallbackHolder> = std::sync::OnceLock::new();
/// Endpoint id whose volume interface currently has the callback registered.
/// `None` means (re-)registration is pending. Informational (tests/diag);
/// the worker keeps its own live-instance state.
#[cfg(windows)]
static VOLUME_NOTIFY_ID: Mutex<Option<String>> = Mutex::new(None);
/// Set by `OnDefaultDeviceChanged` — the worker drops its live endpoint
/// instance (killing the registration bound to it) and re-registers on the
/// new default endpoint.
#[cfg(windows)]
static VOLUME_REREGISTER: AtomicBool = AtomicBool::new(false);

/// Spawn the process-lifetime MTA worker that owns the volume-callback
/// registration.
///
/// Two Win32 constraints shape this design (both verified empirically):
///
/// 1. Callback delivery is bound to the owning `IAudioEndpointVolume`
///    instance's lifetime: once the activated endpoint-volume interface is
///    released, its registration dies silently. The worker therefore keeps
///    the interface alive in `current` for as long as the registration
///    should stand.
/// 2. The engine invokes callbacks from its own threads; registering on a
///    dedicated multithreaded-apartment thread lets those calls land
///    directly without cross-apartment marshaling, and the callback body
///    only touches atomics so any delivery thread is safe.
///
/// On default-device change (signaled via [`VOLUME_REREGISTER`]) the worker
/// releases the old instance and registers on the new default endpoint.
#[cfg(windows)]
fn spawn_volume_notify_worker() {
    static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    let _ = std::thread::Builder::new().name("volume-notify".into()).spawn(|| {
        use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
        // SAFETY: CoInitializeEx MTA on a dedicated worker thread; the
        // thread lives for the process lifetime, so CoUninitialize is never
        // needed.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        // Live registration: keeping the endpoint-volume interface alive is
        // what keeps the callback registration alive.
        let mut current: Option<(String, IAudioEndpointVolume)> = None;
        loop {
            std::thread::sleep(Duration::from_millis(1000));
            // Already registered and no re-register requested → nothing to do.
            // When `current` is `None` the `||` short-circuits, preserving a
            // pending re-register flag for the iteration that succeeds.
            let needs_register =
                current.is_none() || VOLUME_REREGISTER.swap(false, AtomicOrdering::AcqRel);
            if !needs_register {
                continue;
            }
            // Drop the old instance — its registration dies with it.
            current = None;
            *VOLUME_NOTIFY_ID.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
            // SAFETY: standard WASAPI calls on an initialized MTA thread.
            unsafe {
                let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(
                    &MMDeviceEnumerator,
                    None,
                    CLSCTX_ALL,
                ) else {
                    continue;
                };
                let Ok(dev) = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) else {
                    continue;
                };
                let Ok(id) = RealBackend::device_id(&dev) else { continue };
                let Ok(vol) = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) else {
                    continue;
                };
                let callback = if let Some(holder) = VOLUME_CALLBACK.get() {
                    holder.0.clone()
                } else {
                    let cb: windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolumeCallback =
                        create_volume_callback();
                    let _ = VOLUME_CALLBACK.set(VolumeCallbackHolder(cb.clone()));
                    cb
                };
                if vol.RegisterControlChangeNotify(&callback).is_ok() {
                    current = Some((id.clone(), vol));
                    *VOLUME_NOTIFY_ID.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(id);
                }
            }
        }
    });
}

/// Endpoint-notification client (`IMMNotificationClient`) as a manual COM
/// object: same `ComHeader` layout as the volume callback, its own static
/// vtable. Only flips atomics, so any COM thread may invoke it.
#[cfg(windows)]
use windows::Win32::Media::Audio::IMMNotificationClient_Vtbl;
#[cfg(windows)]
static DEVICE_NOTIFIER_VTBL: IMMNotificationClient_Vtbl = IMMNotificationClient_Vtbl {
    base__: windows::core::IUnknown_Vtbl {
        QueryInterface: device_query_interface,
        AddRef: device_add_ref,
        Release: device_release,
    },
    OnDeviceStateChanged: device_on_state_changed,
    OnDeviceAdded: device_on_added,
    OnDeviceRemoved: device_on_removed,
    OnDefaultDeviceChanged: device_on_default_changed,
    OnPropertyValueChanged: device_on_property_changed,
};

#[cfg(windows)]
unsafe extern "system" fn device_query_interface(
    this: *mut std::ffi::c_void,
    iid: *const windows::core::GUID,
    interface: *mut *mut std::ffi::c_void,
) -> windows::core::HRESULT {
    use windows::Win32::Foundation::{E_NOINTERFACE, S_OK};
    use windows::core::{IUnknown, Interface};
    // SAFETY: COM contract — `iid`/`interface` valid; `this` is a live object.
    unsafe {
        if iid.is_null() || interface.is_null() {
            return E_NOINTERFACE;
        }
        let iid = &*iid;
        if iid == &IUnknown::IID
            || iid == &<windows::Win32::Media::Audio::IMMNotificationClient as Interface>::IID
        {
            *interface = this;
            com_addref(this as *mut ComHeader);
            S_OK
        } else {
            *interface = std::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

#[cfg(windows)]
unsafe extern "system" fn device_add_ref(this: *mut std::ffi::c_void) -> u32 {
    com_addref(this as *mut ComHeader)
}

#[cfg(windows)]
unsafe extern "system" fn device_release(this: *mut std::ffi::c_void) -> u32 {
    com_release(this as *mut ComHeader)
}

#[cfg(windows)]
unsafe extern "system" fn device_on_state_changed(
    _this: *mut std::ffi::c_void,
    _device_id: windows::core::PCWSTR,
    _state: windows::Win32::Media::Audio::DEVICE_STATE,
) -> windows::core::HRESULT {
    use windows::Win32::Foundation::S_OK;
    DEVICE_CHANGED.store(true, AtomicOrdering::Release);
    S_OK
}

#[cfg(windows)]
unsafe extern "system" fn device_on_added(
    _this: *mut std::ffi::c_void,
    _device_id: windows::core::PCWSTR,
) -> windows::core::HRESULT {
    use windows::Win32::Foundation::S_OK;
    DEVICE_CHANGED.store(true, AtomicOrdering::Release);
    S_OK
}

#[cfg(windows)]
unsafe extern "system" fn device_on_removed(
    _this: *mut std::ffi::c_void,
    _device_id: windows::core::PCWSTR,
) -> windows::core::HRESULT {
    use windows::Win32::Foundation::S_OK;
    DEVICE_CHANGED.store(true, AtomicOrdering::Release);
    S_OK
}

#[cfg(windows)]
unsafe extern "system" fn device_on_default_changed(
    _this: *mut std::ffi::c_void,
    _flow: windows::Win32::Media::Audio::EDataFlow,
    _role: windows::Win32::Media::Audio::ERole,
    _device_id: windows::core::PCWSTR,
) -> windows::core::HRESULT {
    use windows::Win32::Foundation::S_OK;
    // Signal the MTA worker to drop its (dying device's) registration
    // and re-register on the new default endpoint. Never block the COM
    // callback thread: the id is informational only, so a contended
    // lock is simply skipped — the worker clears/sets it itself.
    VOLUME_REREGISTER.store(true, AtomicOrdering::Release);
    if let Ok(mut id) = VOLUME_NOTIFY_ID.try_lock() {
        *id = None;
    }
    DEVICE_CHANGED.store(true, AtomicOrdering::Release);
    S_OK
}

#[cfg(windows)]
unsafe extern "system" fn device_on_property_changed(
    _this: *mut std::ffi::c_void,
    _device_id: windows::core::PCWSTR,
    key: windows::Win32::Foundation::PROPERTYKEY,
) -> windows::core::HRESULT {
    use windows::Win32::Foundation::S_OK;
    // Only a display-name change affects the UI; any other property
    // (icon, form factor, …) must not trigger a menu rebuild.
    if key != PKEY_Device_FriendlyName {
        return S_OK;
    }
    if suppress_notify() {
        return S_OK;
    }
    DEVICE_CHANGED.store(true, AtomicOrdering::Release);
    S_OK
}

#[cfg(windows)]
fn create_device_notifier() -> windows::Win32::Media::Audio::IMMNotificationClient {
    use windows::core::Interface;
    let header = Box::new(ComHeader {
        lpVtbl: std::ptr::from_ref(&DEVICE_NOTIFIER_VTBL) as *const std::ffi::c_void,
        refs: AtomicU32::new(1),
    });
    // SAFETY: leaked with refcount 1; header layout matches the COM object
    // contract (vtable pointer first), ownership moves to the interface.
    unsafe {
        windows::Win32::Media::Audio::IMMNotificationClient::from_raw(
            Box::into_raw(header) as *mut std::ffi::c_void
        )
    }
}

#[cfg(windows)]
/// Holder for the COM notification client kept for process lifetime.
/// The manual object is stateless and only touches `DEVICE_CHANGED` atomics,
/// so it is effectively `Send`/`Sync` even though COM STA objects are
/// normally thread-affine. We only create/register on the main STA thread,
/// and `OnceLock` only extends lifetime — no cross-thread COM call is made
/// through the holder.
struct NotifierHolder(#[allow(dead_code)] IMMNotificationClient);
#[cfg(windows)]
// SAFETY: the manual object only flips atomics; its methods are stateless
// and thread-safe. Register is called once on the main STA thread; holding
// the client for lifetime is sound.
unsafe impl Send for NotifierHolder {}
#[cfg(windows)]
// SAFETY: see Send impl.
unsafe impl Sync for NotifierHolder {}
#[cfg(windows)]
static NOTIFIER_HOLDER: std::sync::OnceLock<NotifierHolder> = std::sync::OnceLock::new();

#[cfg(windows)]
fn register_notification_client() {
    if NOTIFIER_HOLDER.get().is_some() {
        return;
    }
    unsafe {
        // SAFETY: CoCreateInstance and RegisterEndpointNotificationCallback are valid on initialized STA thread; NOTIFIER_HOLDER ensures lifetime
        if let Ok(enumerator) =
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
        {
            let notifier: IMMNotificationClient = create_device_notifier();
            let _ = enumerator.RegisterEndpointNotificationCallback(&notifier);
            let _ = NOTIFIER_HOLDER.set(NotifierHolder(notifier));
        }
    }
}

/// Real Windows WASAPI backend.
#[cfg(windows)]
pub struct RealBackend {
    cached: Option<Vec<AudioDevice>>,
    cache_time: Option<Instant>,
    input_cached: Option<Vec<AudioDevice>>,
    input_cache_time: Option<Instant>,
    // Reuse the enumerator/endpoint across startup batch queries (fewer CoCreateInstance calls).
    cached_enumerator: Option<windows::Win32::Media::Audio::IMMDeviceEnumerator>,
}

#[cfg(windows)]
impl RealBackend {
    /// Create a backend and register the endpoint notification client once.
    pub fn new() -> Self {
        let s = Self {
            cached: None,
            cache_time: None,
            input_cached: None,
            input_cache_time: None,
            cached_enumerator: None,
        };
        // register once per process
        static REGISTERED: AtomicBool = AtomicBool::new(false);
        if !REGISTERED.swap(true, AtomicOrdering::SeqCst) {
            register_notification_client();
            spawn_volume_notify_worker();
        }
        s
    }

    /// Invalidate the device enumeration caches (render and capture).
    pub fn clear_cache(&mut self) {
        self.cached = None;
        self.cache_time = None;
        self.input_cached = None;
        self.input_cache_time = None;
        // The enumerator stays valid; only the device-list caches expire.
    }

    /// Get or create the cached IMMDeviceEnumerator (fewer CoCreateInstance calls).
    fn enumerator_mut(
        &mut self,
    ) -> windows::core::Result<windows::Win32::Media::Audio::IMMDeviceEnumerator> {
        if let Some(e) = &self.cached_enumerator {
            return Ok(e.clone());
        }
        let e = Self::get_enumerator()?;
        self.cached_enumerator = Some(e.clone());
        Ok(e)
    }

    #[allow(dead_code)]
    /// Batch-fetch startup state (devices + default + volume + mute) sharing one enumerator/endpoint.
    ///
    /// Same as [`fetch_snapshot_clamped`](Self::fetch_snapshot_clamped) without clamping;
    /// kept for debug/benchmark comparison.
    pub fn fetch_snapshot(&mut self) -> AudioSnapshot {
        self.fetch_snapshot_inner(None)
    }

    /// Clamped snapshot: applies the limit inline, avoiding a second get_volume_and_mute.
    pub fn fetch_snapshot_clamped(&mut self, cfg: &AppConfig) -> AudioSnapshot {
        self.fetch_snapshot_inner(Some(cfg))
    }

    fn fetch_snapshot_inner(&mut self, cfg: Option<&AppConfig>) -> AudioSnapshot {
        let mut snap = AudioSnapshot::default();
        // Prefer the cached enumerator.
        let enumerator = match self.enumerator_mut() {
            Ok(e) => e,
            Err(_) => return snap,
        };
        // Devices (3000ms cache; notifications already cleared it, so hits last longer).
        if let Ok(devs) = self.enumerate_devices_inner(&enumerator, eRender) {
            snap.devices = devs;
        }
        // Default + volume/mute share one endpoint.
        unsafe {
            if let Ok(dev) = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) {
                if let Ok(id) = Self::device_id(&dev) {
                    let name = Self::device_friendly_name(&dev);
                    snap.default_device = Some(AudioDevice { id, name });
                }
                if let Ok(vol) = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                    if let Ok(scalar) = vol.GetMasterVolumeLevelScalar() {
                        // Contractually 0.0..=1.0; clamp so a lying driver cannot
                        // break the 0..=100 invariant (NaN folds to 0 via the
                        // saturating float-to-int cast).
                        snap.volume = (scalar.clamp(0.0, 1.0) * 100.0).round() as u32;
                    }
                    if let Ok(m) = vol.GetMute() {
                        snap.mute = m.as_bool();
                    }
                    // Clamp inline so callers skip a second get_volume_and_mute.
                    if let Some(cfg) = cfg {
                        if cfg.volume_limit_enabled {
                            let clamped = clamp_volume(snap.volume, cfg);
                            if clamped != snap.volume {
                                suppress_self_changes_for(SUPPRESS_WINDOW_MS);
                                let v = clamped.min(100) as f32 / 100.0;
                                if vol.SetMasterVolumeLevelScalar(v, std::ptr::null()).is_ok() {
                                    snap.volume = clamped;
                                }
                            }
                        }
                    }
                }
            }
        }
        // capture devices share the cached enumerator; render-only volume/mute untouched.
        if let Ok(inputs) = self.enumerate_devices_inner(&enumerator, eCapture) {
            snap.input_devices = inputs;
        }
        snap.default_input_device = Self::default_device_with(&self.cached_enumerator, eCapture);
        snap
    }

    /// Fetch volume+mute with one endpoint activation (one fewer CoCreateInstance+Activate).
    /// Reuses cached_enumerator instead of creating a new one.
    pub fn get_volume_and_mute(&self) -> Result<(u32, bool), AudioError> {
        Self::volume_and_mute_with(&self.cached_enumerator)
    }

    /// Shared single-Activate core: trait and inherent methods both route here
    /// instead of triplicating the `get_volume_and_mute` body.
    fn volume_and_mute_with(
        cached: &Option<windows::Win32::Media::Audio::IMMDeviceEnumerator>,
    ) -> Result<(u32, bool), AudioError> {
        unsafe {
            let enumerator = if let Some(e) = cached {
                e.clone()
            } else {
                Self::get_enumerator().map_err(AudioError::from)?
            };
            let dev = enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .map_err(AudioError::from)?;
            let vol: IAudioEndpointVolume =
                dev.Activate(CLSCTX_ALL, None).map_err(AudioError::from)?;
            let scalar = vol.GetMasterVolumeLevelScalar().map_err(AudioError::from)?;
            let m = vol.GetMute().map_err(AudioError::from)?;
            // Same driver-scalar clamp as the snapshot path (see above).
            Ok(((scalar.clamp(0.0, 1.0) * 100.0).round() as u32, m.as_bool()))
        }
    }

    fn enumerate_devices_inner(
        &mut self,
        enumerator: &windows::Win32::Media::Audio::IMMDeviceEnumerator,
        flow: EDataFlow,
    ) -> Result<Vec<AudioDevice>, AudioError> {
        if take_device_changed() {
            self.clear_cache();
        }
        let (cache, cache_time) = if flow == eCapture {
            (&self.input_cached, &self.input_cache_time)
        } else {
            (&self.cached, &self.cache_time)
        };
        if let Some(cached) = cache {
            if let Some(t) = cache_time {
                if t.elapsed() < Duration::from_millis(3000) {
                    return Ok(cached.clone());
                }
            }
        }
        unsafe {
            let collection: IMMDeviceCollection = enumerator
                .EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)
                .map_err(AudioError::from)?;
            let count = collection.GetCount().map_err(AudioError::from)?;
            // `GetCount` just returned it, so the capacity is exact on re-walk.
            let mut devices = Vec::with_capacity(count as usize);
            for i in 0..count {
                if let Ok(dev) = collection.Item(i) {
                    if let Ok(id) = Self::device_id(&dev) {
                        let name = Self::device_friendly_name(&dev);
                        devices.push(AudioDevice { id, name });
                    }
                }
            }
            // Always update cache, even if empty — UI must see removal vs stale list.
            if flow == eCapture {
                self.input_cached = Some(devices.clone());
                self.input_cache_time = Some(Instant::now());
            } else {
                self.cached = Some(devices.clone());
                self.cache_time = Some(Instant::now());
            }
            Ok(devices)
        }
    }

    /// Polls the notification flag and clears cache if a device changed.
    /// Single helper shared by the inherent method and the trait impl.
    fn take_notification(&mut self) -> bool {
        if take_device_changed() {
            self.clear_cache();
            return true;
        }
        false
    }

    /// Inherent alias kept for callers using `RealBackend` directly.
    pub fn poll_device_changed(&mut self) -> bool {
        self.take_notification()
    }

    fn get_enumerator() -> windows::core::Result<IMMDeviceEnumerator> {
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
    }

    /// Default endpoint for `flow` using the cached enumerator when available.
    fn default_device_with(
        cached: &Option<windows::Win32::Media::Audio::IMMDeviceEnumerator>,
        flow: EDataFlow,
    ) -> Option<AudioDevice> {
        unsafe {
            let enumerator = if let Some(e) = cached {
                e.clone()
            } else {
                Self::get_enumerator().ok()?
            };
            let dev = enumerator.GetDefaultAudioEndpoint(flow, eMultimedia).ok()?;
            let id = Self::device_id(&dev).ok()?;
            let name = Self::device_friendly_name(&dev);
            Some(AudioDevice { id, name })
        }
    }

    /// Shared validation + `IPolicyConfig` switch for both flows; roles are
    /// orthogonal to direction, so capture reuses the render role sequence.
    fn set_default_inner(&mut self, id: &str) -> Result<(), AudioError> {
        if id.is_empty() || id.contains('\0') {
            return Err(AudioError::Failed("invalid device id".into()));
        }
        unsafe {
            // Primary role: eMultimedia (1), must succeed.
            set_default_endpoint_raw(id, eMultimedia.0)
                .map_err(|e| AudioError::Failed(e.to_string()))?;
            // Secondary roles: best-effort but log failures (do not hide).
            for role in [0i32, 2i32] {
                if let Err(e) = set_default_endpoint_raw(id, role) {
                    // 0x80070490 = not found, 0x80070057 = invalid arg — don't retry, just warn.
                    tracing::warn!("set_default role {role} failed: {e}");
                }
            }
            self.clear_cache();
            Ok(())
        }
    }

    fn device_id(device: &IMMDevice) -> windows::core::Result<String> {
        unsafe {
            let pw = device.GetId()?;
            let s = pw.to_string().unwrap_or_default();
            CoTaskMemFree(Some(pw.0 as *const std::ffi::c_void));
            Ok(s)
        }
    }

    fn device_friendly_name(device: &IMMDevice) -> String {
        unsafe {
            if let Ok(store) = device.OpenPropertyStore(STGM_READ) {
                if let Ok(mut pv) = store.GetValue(&PKEY_Device_FriendlyName) {
                    let vt = pv.Anonymous.Anonymous.vt;
                    let s = if vt == VT_LPWSTR {
                        let pw = pv.Anonymous.Anonymous.Anonymous.pwszVal;
                        if !pw.0.is_null() {
                            pw.to_string().unwrap_or_default()
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };
                    let _ = PropVariantClear(&mut pv as *mut _);
                    if !s.is_empty() {
                        return s.chars().take(80).collect();
                    }
                }
            }
            // Fall back to a shortened endpoint id.
            if let Ok(id) = Self::device_id(device) {
                let short = id.split('\\').next_back().unwrap_or(&id);
                let truncated: String = short.chars().take(40).collect();
                if !truncated.is_empty() {
                    return truncated;
                }
                return id.chars().take(40).collect();
            }
            "Unknown".to_string()
        }
    }
}

#[cfg(windows)]
impl AudioBackend for RealBackend {
    fn fetch_snapshot_clamped(&mut self, cfg: &AppConfig) -> AudioSnapshot {
        RealBackend::fetch_snapshot_clamped(self, cfg)
    }
    fn get_volume_and_mute(&self) -> Result<(u32, bool), AudioError> {
        Self::volume_and_mute_with(&self.cached_enumerator)
    }
    fn clear_cache(&mut self) {
        RealBackend::clear_cache(self);
    }
    fn poll_device_changed(&mut self) -> bool {
        self.take_notification()
    }
    fn take_volume_changed(&mut self) -> bool {
        take_volume_changed()
    }
    fn enumerate_devices(&mut self) -> Result<Vec<AudioDevice>, AudioError> {
        let enumerator = self.enumerator_mut().map_err(AudioError::from)?;
        self.enumerate_devices_inner(&enumerator, eRender)
    }

    fn enumerate_input_devices(&mut self) -> Result<Vec<AudioDevice>, AudioError> {
        let enumerator = self.enumerator_mut().map_err(AudioError::from)?;
        self.enumerate_devices_inner(&enumerator, eCapture)
    }

    fn get_default_device(&self) -> Option<AudioDevice> {
        Self::default_device_with(&self.cached_enumerator, eRender)
    }

    fn get_default_input_device(&self) -> Option<AudioDevice> {
        Self::default_device_with(&self.cached_enumerator, eCapture)
    }

    fn set_default_device(&mut self, id: &str) -> Result<(), AudioError> {
        self.set_default_inner(id)
    }

    fn set_default_input_device(&mut self, id: &str) -> Result<(), AudioError> {
        self.set_default_inner(id)
    }

    fn get_volume(&self) -> Result<u32, AudioError> {
        self.get_volume_and_mute().map(|(v, _)| v)
    }

    fn set_volume(&mut self, volume: u32) -> Result<(), AudioError> {
        let v = volume.min(100) as f32 / 100.0;
        unsafe {
            let enumerator = self.enumerator_mut().map_err(AudioError::from)?;
            let dev = enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .map_err(AudioError::from)?;
            let vol: IAudioEndpointVolume =
                dev.Activate(CLSCTX_ALL, None).map_err(AudioError::from)?;
            // Self-initiated change: suppress the asynchronously delivered
            // notification of our own write.
            suppress_self_changes_for(SUPPRESS_WINDOW_MS);
            match vol.SetMasterVolumeLevelScalar(v, std::ptr::null()) {
                Ok(()) => Ok(()),
                Err(e) => {
                    // Only retry on busy/timeout HRESULTs; invalid arg is permanent.
                    let hr = e.code().0 as u32;
                    if (hr == 0x8007_001E || hr == 0x8007_04D4)
                        && vol.SetMasterVolumeLevelScalar(v, std::ptr::null()).is_ok()
                    {
                        return Ok(());
                    }
                    Err(AudioError::Failed(e.to_string()))
                }
            }
        }
    }

    fn get_mute(&self) -> Result<bool, AudioError> {
        self.get_volume_and_mute().map(|(_, m)| m)
    }

    fn set_mute(&mut self, mute: bool) -> Result<(), AudioError> {
        unsafe {
            let enumerator = self.enumerator_mut().map_err(AudioError::from)?;
            let dev = enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .map_err(AudioError::from)?;
            let vol: IAudioEndpointVolume =
                dev.Activate(CLSCTX_ALL, None).map_err(AudioError::from)?;
            // Self-initiated change: suppress our own notification.
            suppress_self_changes_for(SUPPRESS_WINDOW_MS);
            vol.SetMute(mute, std::ptr::null())
                .map_err(|e| AudioError::Failed(e.to_string()))
        }
    }

    fn clamp_volume_if_needed(&mut self, cfg: &AppConfig) -> Result<(), AudioError> {
        if !cfg.volume_limit_enabled {
            return Ok(());
        }
        let (vol, _) = self.get_volume_and_mute()?;
        let clamped = clamp_volume(vol, cfg);
        if clamped != vol {
            suppress_self_changes_for(SUPPRESS_WINDOW_MS);
            return self.set_volume(clamped);
        }
        Ok(())
    }
}

/// Real backend stub for non-Windows (compilation only).
#[cfg(not(windows))]
pub struct RealBackend {
    cached: Option<Vec<AudioDevice>>,
    cache_time: Option<Instant>,
}
#[cfg(not(windows))]
impl RealBackend {
    /// Create a stub backend.
    pub fn new() -> Self {
        Self {
            cached: None,
            cache_time: None,
        }
    }
    /// Invalidate cache (no-op on non-Windows).
    pub fn clear_cache(&mut self) {
        self.cached = None;
        self.cache_time = None;
    }
    /// Poll device change — always false on non-Windows.
    pub fn poll_device_changed(&mut self) -> bool {
        false
    }
    /// Fetch snapshot — default on non-Windows.
    pub fn fetch_snapshot(&mut self) -> AudioSnapshot {
        AudioSnapshot::default()
    }
    /// Fetch clamped snapshot — default on non-Windows.
    pub fn fetch_snapshot_clamped(&mut self, _cfg: &AppConfig) -> AudioSnapshot {
        AudioSnapshot::default()
    }
    /// Get volume and mute — returns `(50, false)` on non-Windows.
    pub fn get_volume_and_mute(&self) -> Result<(u32, bool), AudioError> {
        Ok((50, false))
    }
}
#[cfg(not(windows))]
impl AudioBackend for RealBackend {
    fn enumerate_devices(&mut self) -> Result<Vec<AudioDevice>, AudioError> {
        Ok(vec![])
    }
    fn get_default_device(&self) -> Option<AudioDevice> {
        None
    }
    fn set_default_device(&mut self, _id: &str) -> Result<(), AudioError> {
        Ok(())
    }
    fn enumerate_input_devices(&mut self) -> Result<Vec<AudioDevice>, AudioError> {
        Ok(vec![])
    }
    fn get_default_input_device(&self) -> Option<AudioDevice> {
        None
    }
    fn set_default_input_device(&mut self, _id: &str) -> Result<(), AudioError> {
        Ok(())
    }
    fn get_volume(&self) -> Result<u32, AudioError> {
        Ok(50)
    }
    fn set_volume(&mut self, _volume: u32) -> Result<(), AudioError> {
        Ok(())
    }
    fn get_mute(&self) -> Result<bool, AudioError> {
        Ok(false)
    }
    fn set_mute(&mut self, _mute: bool) -> Result<(), AudioError> {
        Ok(())
    }
    fn clamp_volume_if_needed(&mut self, _cfg: &AppConfig) -> Result<(), AudioError> {
        Ok(())
    }
    fn clear_cache(&mut self) {
        RealBackend::clear_cache(self);
    }
}
/// Stub — always false on non-Windows.
#[cfg(not(windows))]
pub fn take_device_changed() -> bool {
    false
}

#[cfg(all(test, windows))]
mod volume_notify_tests {
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
        use std::time::Duration;
        let _com = crate::platform::ComGuard::init().expect("COM init");
        let mut backend = RealBackend::new();
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
        use std::time::Duration;
        let _com = crate::platform::ComGuard::init().expect("COM init");
        let mut backend = RealBackend::new();
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
                let mut backend = RealBackend::new();
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
}
