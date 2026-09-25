//! Unit tests for the wheel event the hook hands to the app loop.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::*;

/// The gate judges the position the notch carried, so the packed word has to
/// round-trip both halves exactly — including the negative coordinates a
/// monitor left of or above the primary screen produces.
#[test]
fn packed_position_round_trips_signed_coordinates() {
    for (x, y) in [
        (0, 0),
        (3489, 2127),
        (-1920, 40),
        (40, -1080),
        (-1, -1),
        (i32::MAX, i32::MIN),
    ] {
        assert_eq!(unpack_point(pack_point(x, y)), (x, y), "packed {x},{y}");
    }
}
