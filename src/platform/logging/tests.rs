//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::failure_note;

/// A crash next to a log file that was never written is the one report a user
/// cannot act on: name the reason in the report itself.
#[test]
fn an_unwritable_log_file_is_named_in_the_crash_report() {
    let note = failure_note(Some("Access is denied. (os error 5)"));
    assert!(note.contains("log file unavailable"), "{note}");
    assert!(note.contains("denied"), "{note}");
}

#[test]
fn a_writable_log_file_adds_nothing_to_the_crash_report() {
    assert_eq!(failure_note(None), "");
}
