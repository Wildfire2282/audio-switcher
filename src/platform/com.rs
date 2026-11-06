//! COM lifecycle RAII.
//!
//! [`ComGuard::init`] performs no UI: on failure the caller (`main`) shows
//! the dialog and exits with 1.

use thiserror::Error;

#[cfg(windows)]
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};

/// COM initialization failure (STA model declared explicitly).
#[derive(Debug, Error)]
pub enum ComError {
    /// `CoInitializeEx` returned a failing HRESULT; the raw code (not a
    /// rendered string) is kept so STA failures stay triageable.
    #[error("COM initialization failed (HRESULT {0:#010X})")]
    InitFailed(i32),
}

/// RAII guard for COM — thread-affine (STA) and `!Send`/`!Sync`.
///
/// Contains a `PhantomData<*const ()>` marker so it cannot be sent across
/// threads; `CoUninitialize` must run on the same STA thread that called
/// `CoInitializeEx`.
#[derive(Debug)]
pub struct ComGuard {
    #[cfg(windows)]
    _private: (),
    /// Marker to make `ComGuard` !Send + !Sync (STA thread-affine).
    _marker: std::marker::PhantomData<*const ()>,
}

impl ComGuard {
    /// Initialize COM (STA).
    ///
    /// Shows no dialog; the caller dialogs and exits on `Err`.
    ///
    /// # Errors
    ///
    /// Returns the init-failed error when `CoInitializeEx` fails.
    #[must_use = "a failed COM init must exit the process, never be ignored"]
    pub fn init() -> Result<Self, ComError> {
        #[cfg(windows)]
        {
            // SAFETY: CoInitializeEx is safe to call once per STA thread.
            let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            if hr.is_err() {
                return Err(ComError::InitFailed(hr.0));
            }
        }
        Ok(Self {
            #[cfg(windows)]
            _private: (),
            _marker: std::marker::PhantomData,
        })
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            // SAFETY: balances successful CoInitializeEx in init().
            unsafe {
                CoUninitialize();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_failed_renders_hresult_hex() {
        // 0x800401F0 (CO_E_NOTINITIALIZED) as i32 must render two's-complement hex.
        let err = ComError::InitFailed(0x8004_01F0_u32 as i32);
        assert_eq!(
            err.to_string(),
            "COM initialization failed (HRESULT 0x800401F0)"
        );
    }
}
