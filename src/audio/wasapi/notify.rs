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

/// Whether a device change is still waiting for the message loop to consume.
///
/// Peek, not take: enumeration can run before the loop's device pump does (a
/// hotkey or menu action enumerates first), and consuming the flag there would
/// leave the loop never latching the change — the menu then stays stale until
/// an unrelated notification happens to arrive.
#[cfg(windows)]
pub fn device_changed_pending() -> bool {
    DEVICE_CHANGED.load(AtomicOrdering::Acquire)
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
    crate::platform::pump::wake();
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

/// Wakes the re-registration worker instead of it polling once a second.
///
/// The signal is raised while holding this mutex, and the worker's wait
/// re-reads [`VOLUME_REREGISTER`] under the same mutex: a signal sent outside
/// the lock can land between the worker's flag check and its wait, which would
/// leave the stale registration in place until the wait timed out.
#[cfg(windows)]
static REREG_CV: std::sync::LazyLock<(Mutex<()>, std::sync::Condvar)> =
    std::sync::LazyLock::new(|| (Mutex::new(()), std::sync::Condvar::new()));

/// Signal the worker to drop its live endpoint instance and re-register the
/// volume callback on the current default endpoint.
///
/// The registration dies with the endpoint instance that owns it, and that
/// instance's proxy dies with the `audiosrv` instance behind it — after a
/// service restart (sleep/resume, a crash, a driver update) the callback
/// simply stops arriving, with no error anywhere. This signal is the recovery:
/// `WasapiBackend::clear_cache`, the menu's "Refresh", raises it too.
///
/// Raised under the mutex for the reason in [`REREG_CV`]'s doc.
#[cfg(windows)]
pub(super) fn request_volume_reregister() {
    let _guard = REREG_CV
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    VOLUME_REREGISTER.store(true, AtomicOrdering::Release);
    REREG_CV.1.notify_one();
}

/// Activate the default render endpoint and register the volume callback on it.
///
/// Returns the live `(endpoint id, interface)` pair: keeping the interface alive
/// is what keeps the registration alive. `None` on any failure, which leaves the
/// caller's `current` empty so the next pass retries.
///
/// # Safety
///
/// Must run on a thread with COM initialized (the MTA worker).
#[cfg(windows)]
unsafe fn activate_volume_callback() -> Option<(String, IAudioEndpointVolume)> {
    // SAFETY: the caller guarantees an initialized apartment on this thread.
    unsafe {
        let enumerator =
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .ok()?;
        let dev = enumerator
            .GetDefaultAudioEndpoint(eRender, eMultimedia)
            .ok()?;
        let id = WasapiBackend::device_id(&dev).ok()?;
        let vol = dev
            .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
            .ok()?;
        let callback = if let Some(holder) = VOLUME_CALLBACK.get() {
            holder.0.clone()
        } else {
            let cb = create_volume_callback();
            let _ = VOLUME_CALLBACK.set(VolumeCallbackHolder(cb.clone()));
            cb
        };
        vol.RegisterControlChangeNotify(&callback).ok()?;
        Some((id, vol))
    }
}

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
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.load(AtomicOrdering::Acquire) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("volume-notify".into())
        .spawn(|| {
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
                // Register when nothing is live, or when the default endpoint
                // changed. The request is consumed unconditionally: it is the
                // only signal while a registration is live, and while one is
                // missing the pass retries anyway — but a flag left set would
                // make the wait below return at once and turn that retry into a
                // spin.
                let requested = VOLUME_REREGISTER.swap(false, AtomicOrdering::AcqRel);
                let live = if current.is_none() || requested {
                    // Drop the old instance first — its registration dies with it,
                    // and the new registration must not be made while it is live.
                    drop(current.take());
                    *VOLUME_NOTIFY_ID
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
                    // SAFETY: this thread initialized COM above.
                    current = unsafe { activate_volume_callback() };
                    if let Some((id, _)) = &current {
                        *VOLUME_NOTIFY_ID
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(id.clone());
                    }
                    current.is_some()
                } else {
                    true
                };
                // A failing registration retried once a second (the pace this loop
                // always had); a live one waits for the next request, with a minute
                // cap so a lost signal still self-heals.
                let wait = if live {
                    Duration::from_secs(60)
                } else {
                    Duration::from_secs(1)
                };
                let (lock, cv) = &*REREG_CV;
                let guard = lock
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                drop(
                    cv.wait_timeout_while(guard, wait, |()| {
                        !VOLUME_REREGISTER.load(AtomicOrdering::Acquire)
                    })
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                );
            }
        });
    // Latched only once the thread exists: a failed spawn must stay retryable
    // (the next `WasapiBackend::new` asks again), or external volume changes
    // go dark for the process lifetime.
    match spawned {
        Ok(_thread) => {
            STARTED.store(true, AtomicOrdering::Release);
        }
        Err(e) => tracing::warn!("volume-notify worker spawn failed: {e}"),
    }
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
    crate::platform::pump::wake();
    S_OK
}

#[cfg(windows)]
unsafe extern "system" fn device_on_added(
    _this: *mut std::ffi::c_void,
    _device_id: windows::core::PCWSTR,
) -> windows::core::HRESULT {
    use windows::Win32::Foundation::S_OK;
    DEVICE_CHANGED.store(true, AtomicOrdering::Release);
    crate::platform::pump::wake();
    S_OK
}

#[cfg(windows)]
unsafe extern "system" fn device_on_removed(
    _this: *mut std::ffi::c_void,
    _device_id: windows::core::PCWSTR,
) -> windows::core::HRESULT {
    use windows::Win32::Foundation::S_OK;
    DEVICE_CHANGED.store(true, AtomicOrdering::Release);
    crate::platform::pump::wake();
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
    // Re-register on the new default endpoint (the old device's registration
    // is dying with it). Never block the COM callback thread on the id lock
    // below: the id is informational only, so a contended lock is simply
    // skipped — the worker clears/sets it itself.
    request_volume_reregister();
    if let Ok(mut id) = VOLUME_NOTIFY_ID.try_lock() {
        *id = None;
    }
    DEVICE_CHANGED.store(true, AtomicOrdering::Release);
    crate::platform::pump::wake();
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
    crate::platform::pump::wake();
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
/// The objects a device-change registration needs for the process lifetime: the
/// enumerator the callback was registered through, and the client it arrives at.
///
/// The enumerator is held rather than dropped after registering: the contract is
/// that the client calls `UnregisterEndpointNotificationCallback` before
/// releasing it, and a registration whose enumerator is gone is not something
/// the shell promises to keep delivering — a lost registration means the menu
/// stops noticing device changes.
///
/// The manual client object is stateless and only touches `DEVICE_CHANGED`
/// atomics, so it is effectively `Send`/`Sync` even though COM STA objects are
/// normally thread-affine. Creation and registration happen on the main STA
/// thread, and `OnceLock` only extends lifetime — no cross-thread COM call is
/// made through the holder.
struct NotifierHolder {
    #[allow(dead_code)]
    enumerator: IMMDeviceEnumerator,
    #[allow(dead_code)]
    notifier: IMMNotificationClient,
}
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
pub(super) fn register_notification_client() -> bool {
    if NOTIFIER_HOLDER.get().is_some() {
        return true;
    }
    unsafe {
        // SAFETY: CoCreateInstance and RegisterEndpointNotificationCallback are
        // valid on an initialized STA thread.
        let Ok(enumerator) =
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            tracing::warn!("device notifier: CoCreateInstance failed; device changes unseen");
            return false;
        };
        let notifier: IMMNotificationClient = create_device_notifier();
        // A registration that never took is otherwise silent: the menu stops
        // noticing device changes with nothing left to read.
        if let Err(e) = enumerator.RegisterEndpointNotificationCallback(&notifier) {
            tracing::warn!("device notifier: registration failed: {e}");
            return false;
        }
        let _ = NOTIFIER_HOLDER.set(NotifierHolder {
            enumerator,
            notifier,
        });
    }
    true
}
