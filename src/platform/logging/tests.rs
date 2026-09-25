//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::{LOG_RETENTION, failure_note, open_log_file, prune_old_logs};
use std::io::Write;
use std::time::{Duration, SystemTime};

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

/// Age a file by back-dating its modification time, the only clock the prune
/// reads.
fn age_by(path: &std::path::Path, age: Duration) {
    let file = std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    file.set_modified(SystemTime::now() - age)
        .unwrap_or_else(|e| panic!("set mtime on {}: {e}", path.display()));
}

/// The day-indexed name means one file per day forever, so a stale log is
/// deleted at startup — and only a stale one.
#[test]
fn a_log_past_the_retention_window_is_pruned_but_a_fresh_one_is_kept() {
    let dir = tempfile::tempdir().expect("tempdir");
    let old = dir.path().join(format!("{}-1.log", crate::TOOL_ID));
    let fresh = dir.path().join(format!("{}-2.log", crate::TOOL_ID));
    std::fs::write(&old, b"old").expect("write old log");
    std::fs::write(&fresh, b"fresh").expect("write fresh log");
    age_by(&old, LOG_RETENTION + Duration::from_secs(86_400));

    prune_old_logs(dir.path(), SystemTime::now());

    assert!(!old.exists(), "a log past the window survived");
    assert!(fresh.exists(), "today's log was pruned");
}

/// `LOCALAPPDATA` is an environment variable, so the directory it names can
/// hold files that are not ours — another tool's log, or a non-log file that
/// merely shares our prefix.
#[test]
fn a_prune_leaves_other_files_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let foreign = dir.path().join("other-tool-1.log");
    let not_a_log = dir.path().join(format!("{}-1.txt", crate::TOOL_ID));
    for path in [&foreign, &not_a_log] {
        std::fs::write(path, b"x").expect("write");
        age_by(path, LOG_RETENTION + Duration::from_secs(86_400));
    }

    prune_old_logs(dir.path(), SystemTime::now());

    assert!(foreign.exists(), "another tool's log was deleted");
    assert!(not_a_log.exists(), "a file that is not a log was deleted");
}

#[test]
fn log_file_opens_in_append_mode_preserving_prior_content() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test.log");
    {
        let mut first = open_log_file(&path).expect("open first");
        first.write_all(b"run 1\n").expect("write first");
    }
    {
        let mut second = open_log_file(&path).expect("open second");
        second.write_all(b"run 2\n").expect("write second");
    }
    let contents = std::fs::read_to_string(&path).expect("read log");
    assert_eq!(contents, "run 1\nrun 2\n");
}
