//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::*;

#[test]
fn short_passthrough() {
    assert_eq!(truncate_label("Speaker", MAX_LABEL_CHARS), "Speaker");
}

#[test]
fn sanitizes_control_chars() {
    assert_eq!(truncate_label("A\nB\rC\tD", MAX_LABEL_CHARS), "A B C D");
}

#[test]
fn truncates_long() {
    let long = "A".repeat(100);
    let out = truncate_label(&long, MAX_LABEL_CHARS);
    assert_eq!(out.chars().count(), MAX_LABEL_CHARS - 1);
    assert!(out.ends_with('…'));
}
