//! Undocumented `IPolicyConfig` default-endpoint switching.
//!
//! The interface is not in any public SDK, so the vtable offsets are probed
//! per candidate CLSID and only a primary-offset `S_OK` counts. Every call is
//! raw: this module owns the `transmute` into a function pointer, so a
//! contract change here is a memory-safety change.
//!
//! Threading: called on the message-loop thread with COM already initialized
//! (see `WasapiBackend::set_default_inner`, the only caller).

use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
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
pub(super) unsafe fn set_default_endpoint_raw(
    device_id: &str,
    role: i32,
) -> windows::core::Result<()> {
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
    let wide: Vec<u16> = crate::platform::utf16::wide_z(device_id);
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
                    // SAFETY: slot 2 of any COM vtable is IUnknown::Release; the
                    // pointer comes from the live interface `iface`.
                    let rel: ReleaseFn = unsafe { std::mem::transmute(*vtbl_iface.add(2)) };
                    // SAFETY: `rel` is that Release, called once on the interface
                    // it belongs to; this balances exactly the QI above.
                    unsafe { rel(iface) };
                    if hr.is_ok() {
                        return Ok(());
                    }
                    last_err = Some(windows::core::Error::from(hr));
                    // On Vista, a failed primary offset stops probing this CLSID:
                    // trying the other offset could fake S_OK; move to the next CLSID.
                    continue;
                }
                // No release fallback belongs here: a successful QI always
                // yields a vtable, and without one `Release` cannot be reached.
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
