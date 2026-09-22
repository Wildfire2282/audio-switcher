//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::wait_ms;
use crate::platform::pump::PUMP_IDLE_MS;
use std::time::{Duration, Instant};

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
