//! Step math for the volume and device actions: pure, `#[must_use]`, and
//! unit-tested from the sibling `tests.rs`.

/// Index of the device `step` positions from `current`, wrapping at both ends.
///
/// `None` only for an empty list; an unknown/absent `current` starts at the
/// first device, so cycling always produces a usable index.
#[must_use]
pub(super) fn cycle_index(len: usize, current: Option<usize>, step: i32) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let start = i32::try_from(current.unwrap_or(0)).unwrap_or(0);
    let len = i32::try_from(len).unwrap_or(i32::MAX);
    Some((start + step).rem_euclid(len) as usize)
}

/// Apply one `delta` percent to `volume`, clamped to the `0..=100` invariant.
///
/// `i64` math: neither a wheel burst nor `i32::MIN` can wrap, and the clamp
/// keeps a lying backend from pushing the tray past 100.
#[must_use]
pub(super) fn stepped_volume(volume: u32, delta: i32) -> u32 {
    u32::try_from((i64::from(volume) + i64::from(delta)).clamp(0, 100)).unwrap_or(0)
}

#[cfg(test)]
mod tests;
