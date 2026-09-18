//! COM notifications: endpoint volume/mute callbacks and device changes.
//!
//! The callback objects are hand-rolled COM (a manual vtable plus a refcount)
//! because the notification interfaces must stay alive for the process
//! lifetime. They only touch atomics, so any delivery thread is safe.
//!
//! Threading: `spawn_volume_notify_worker` runs its own MTA thread; the
//! callbacks arrive on audio-service threads. Everything the message loop
//! reads goes through [`take_device_changed`] / [`take_volume_changed`], which
//! swap the flag back to false so a burst collapses into one wake.
//!
//! The suppression window exists because the snapshot path writes the volume
//! itself: its own notification would otherwise arrive as an "external"
//! change and re-trigger a resync.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering as AtomicOrdering};
use std::time::Duration;

use super::WasapiBackend;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    IMMDeviceEnumerator, IMMNotificationClient, MMDeviceEnumerator, eMultimedia, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};

#[cfg(windows)]
static DEVICE_CHANGED: AtomicBool = AtomicBool::new(false);

/// Window (ms) during which notifications after a self-initiated change are
/// treated as echoes of that change. Endpoint notifications typically arrive
/// within tens of milliseconds; 200ms covers the tail while minimizing the
/// window in which a genuine external change could be coalesced away.
#[cfg(windows)]
pub(super) const SUPPRESS_WINDOW_MS: u64 = 200;

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
pub(super) fn suppress_self_changes_for(ms: u64) {
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
pub(super) static VOLUME_NOTIFY_ID: Mutex<Option<String>> = Mutex::new(None);
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
pub(super) fn spawn_volume_notify_worker() {
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
                let Ok(id) = WasapiBackend::device_id(&dev) else { continue };
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
pub(super) fn register_notification_client() {
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
