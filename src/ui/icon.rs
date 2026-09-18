//! Tray icon creation with caching.
//!
//! Icons are 32×32 RGBA rendered from the transparent-background black-glyph
//! SVGs (`icons/headphones.svg` unmuted, `icons/headphone-off.svg` muted).

use std::sync::{LazyLock, Mutex};

use tray_icon::Icon;

/// Cached icons — index 0 = unmuted, 1 = muted.
/// std Mutex with poison recovery (`into_inner`): a poisoned cache still
/// serves icons instead of crash-looping the resident tray on every refresh.
static ICON_CACHE: LazyLock<Mutex<[Option<Icon>; 2]>> = LazyLock::new(|| Mutex::new([None, None]));

/// Create (or fetch from cache) the tray icon for `muted`.
///
/// # Errors
///
/// Returns the underlying `BadIcon` error when the embedded RGBA bytes
/// are invalid; the caller surfaces it through [`TrayError`](super::tray::TrayError)
/// (dialog + exit at startup) or logs it (runtime icon refresh).
pub fn make_icon(muted: bool) -> Result<Icon, tray_icon::BadIcon> {
    let idx = usize::from(muted);
    if let Some(cached) = ICON_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(idx)
        .and_then(Clone::clone)
    {
        return Ok(cached);
    }
    let rgba: &[u8] = if muted {
        include_bytes!("../../icons/tray_muted.rgba")
    } else {
        include_bytes!("../../icons/tray_unmuted.rgba")
    };
    let icon = Icon::from_rgba(rgba.to_vec(), 32, 32).map_err(|e| {
        tracing::warn!(
            "tray icon rgba invalid: muted={muted} len={} err={e}",
            rgba.len()
        );
        e
    })?;
    ICON_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)[idx] = Some(icon.clone());
    Ok(icon)
}
