//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::*;

#[test]
fn cycle_index_wraps_both_ways() {
    assert_eq!(cycle_index(3, Some(0), 1), Some(1));
    // Forward past the end wraps to the first device, backward to the last.
    assert_eq!(cycle_index(3, Some(2), 1), Some(0));
    assert_eq!(cycle_index(3, Some(0), -1), Some(2));
    // Unknown/absent current starts at the first device.
    assert_eq!(cycle_index(3, None, 1), Some(1));
    assert_eq!(cycle_index(3, Some(9), -1), Some(2));
    // Single device and empty list.
    assert_eq!(cycle_index(1, Some(0), 1), Some(0));
    assert_eq!(cycle_index(0, None, 1), None);
}

#[test]
fn stepped_volume_stays_in_range() {
    assert_eq!(stepped_volume(50, 2), 52);
    assert_eq!(stepped_volume(50, -2), 48);
    assert_eq!(stepped_volume(0, -2), 0);
    assert_eq!(stepped_volume(1, -5), 0);
    assert_eq!(stepped_volume(99, 5), 100);
    // Overflow-safe at both extremes.
    assert_eq!(stepped_volume(100, i32::MAX), 100);
    assert_eq!(stepped_volume(0, i32::MIN), 0);
}
