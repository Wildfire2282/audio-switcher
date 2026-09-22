//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::{DEVICE_COALESCE_WINDOW, devices_refresh_due, wait_ms};
use crate::platform::pump::PUMP_IDLE_MS;
use std::time::{Duration, Instant};

/// A device change is reported as several notifications in quick succession, so
/// the rebuild waits for the window — and the latch is what keeps the last one
/// of the burst from being dropped while it waits.
#[test]
fn a_device_change_is_coalesced_and_never_dropped() {
    let last = Instant::now();
    // Nothing latched: there is nothing to rebuild for, however long ago the
    // last rebuild was.
    assert!(!devices_refresh_due(
        false,
        last,
        last + Duration::from_secs(5)
    ));
    assert!(!devices_refresh_due(
        true,
        last,
        last + Duration::from_millis(50)
    ));
    assert!(devices_refresh_due(
        true,
        last,
        last + Duration::from_millis(130)
    ));
    // The window boundary itself counts as due: `wait_timeout` wakes exactly
    // there, and a strict comparison would cost another pass.
    assert!(devices_refresh_due(
        true,
        last,
        last + DEVICE_COALESCE_WINDOW
    ));
}

/// No deadline is the normal idle case: sleep the cap and let the next wake or
/// timeout start the next pass.
#[test]
fn no_deadline_waits_the_idle_cap() {
    assert_eq!(wait_ms(Instant::now(), None), PUMP_IDLE_MS);
}

/// Truncating the remainder to whole milliseconds made a deadline half a
/// millisecond away a zero timeout — a spin loop with the overlay on screen.
#[test]
fn a_sub_millisecond_remainder_still_waits() {
    let now = Instant::now();
    assert_eq!(wait_ms(now, Some(now + Duration::from_micros(500))), 1);
}

#[test]
fn a_deadline_inside_the_cap_is_waited_for_exactly() {
    let now = Instant::now();
    assert_eq!(wait_ms(now, Some(now + Duration::from_secs(5))), 5_000);
}

/// A deadline past the cap still sleeps only the cap, so the loop can do its
/// idle work even while a far-off deadline is pending.
#[test]
fn a_deadline_beyond_the_cap_is_clamped() {
    let now = Instant::now();
    assert_eq!(
        wait_ms(now, Some(now + Duration::from_secs(30))),
        PUMP_IDLE_MS
    );
}
