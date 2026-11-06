//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::*;

#[test]
fn key_names_locked() {
    // Renaming the value would orphan every installed Run entry.
    assert_eq!(autostart_key_name(), "AudioSwitcher");
    assert_eq!(
        autostart_key_name(),
        crate::display_name_for(crate::TOOL_ID)
    );
    // The cleanup must catch the legacy names and never the canonical one.
    assert_eq!(LEGACY_AUTOSTART_KEYS, &["audio-switcher", "Audio Switcher"]);
    assert!(!LEGACY_AUTOSTART_KEYS.contains(&autostart_key_name()));
}

#[test]
fn the_read_backs_map_to_the_installed_mode() {
    // This is the menu's only input: a state mapped to the wrong mode would
    // check the wrong entry.
    assert_eq!(
        mode_from(Ok(RunEntry::Enabled), Some(false)),
        Some(AutostartMode::User)
    );
    assert_eq!(
        mode_from(Ok(RunEntry::Absent), Some(false)),
        Some(AutostartMode::Off)
    );
    assert_eq!(
        mode_from(Ok(RunEntry::Absent), Some(true)),
        Some(AutostartMode::Admin)
    );
    // Switched off in Task Manager's Startup tab: only a task can still be live.
    assert_eq!(
        mode_from(Ok(RunEntry::Disabled), Some(true)),
        Some(AutostartMode::Admin)
    );
    assert_eq!(
        mode_from(Ok(RunEntry::Disabled), Some(false)),
        Some(AutostartMode::Off)
    );
}

#[test]
fn an_unreadable_state_is_unknown_never_off() {
    assert_eq!(
        mode_from(Err(std::io::Error::other("access denied")), Some(true)),
        None,
        "an unreadable Run entry must not render as a mode"
    );
    for run in [RunEntry::Absent, RunEntry::Disabled, RunEntry::Enabled] {
        assert_eq!(mode_from(Ok(run), None), None, "unknown task with {run:?}");
    }
}

#[test]
fn exe_path_available_in_tests() {
    assert!(get_exe_path().is_some());
}

#[test]
fn mode_defaults_to_user() {
    // v3 defaulted `autostart` to true; the mode type keeps that behaviour so
    // upgrading users see no change.
    assert_eq!(AutostartMode::default(), AutostartMode::User);
}

#[test]
fn mode_json_is_lowercase_and_round_trips() {
    for (mode, json) in [
        (AutostartMode::Off, "\"off\""),
        (AutostartMode::User, "\"user\""),
        (AutostartMode::Admin, "\"admin\""),
    ] {
        assert_eq!(serde_json::to_string(&mode).expect("serializes"), json);
        assert_eq!(
            serde_json::from_str::<AutostartMode>(json).expect("deserializes"),
            mode
        );
    }
}

#[test]
fn task_cache_starts_unknown_until_a_successful_query() {
    with_fixture(vec![], Ok(RunEntry::Absent), || {
        assert_eq!(autostart_state(), None);
    });
}

thread_local! {
    static TEST_KEY: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

pub(super) fn key_name() -> Option<&'static str> {
    TEST_KEY.with(std::cell::Cell::get)
}

#[cfg(windows)]
fn isolated_key() -> impl Drop {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_KEY.with(|key| key.set(None));
        }
    }
    static NAME: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| format!("AudioSwitcher-Autostart-Test-{}", std::process::id()));
    TEST_KEY.with(|key| key.set(Some(NAME.as_str())));
    Reset
}

const TASK_ROW: &[u8] = b"\"\\AudioSwitcher\",\"N/A\",\"Ready\"\r\n";
const TASK_MISSING: i32 = 0x8007_0002u32 as i32;
const ACCESS_DENIED: i32 = 0x8007_0005u32 as i32;

#[test]
fn a_terminated_query_does_not_confirm_absence() {
    let (_, out) = output(0, TASK_ROW, b"").expect("synthetic output");
    assert!(classify_query_result(&out, None).is_err());
}

struct Fixture {
    queries: std::collections::VecDeque<std::io::Result<(i32, std::process::Output)>>,
    run: Result<RunEntry, std::io::Error>,
    helpers: Vec<String>,
    writes: Vec<bool>,
}

thread_local! {
    static FIXTURE: std::cell::RefCell<Option<Fixture>> = const { std::cell::RefCell::new(None) };
}

fn with_fixture(
    queries: Vec<std::io::Result<(i32, std::process::Output)>>,
    run: Result<RunEntry, std::io::Error>,
    check: impl FnOnce(),
) {
    static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _gate = GATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    struct Restore(Option<bool>);
    impl Drop for Restore {
        fn drop(&mut self) {
            FIXTURE.with(|fixture| *fixture.borrow_mut() = None);
            set_admin_task_cached(self.0);
        }
    }
    let _restore = Restore(admin_task_cached());
    set_admin_task_cached(None);
    FIXTURE.with(|fixture| {
        *fixture.borrow_mut() = Some(Fixture {
            queries: queries.into(),
            run,
            helpers: Vec::new(),
            writes: Vec::new(),
        });
    });
    check();
    FIXTURE.with(|fixture| {
        assert!(
            fixture
                .borrow()
                .as_ref()
                .expect("fixture")
                .queries
                .is_empty()
        );
    });
}

pub(super) fn task_query_result() -> Option<Result<bool, AutostartError>> {
    FIXTURE.with(|fixture| {
        let mut fixture = fixture.borrow_mut();
        let fixture = fixture.as_mut()?;
        Some(
            match fixture.queries.pop_front().expect("unexpected task query") {
                Ok((code, output)) => classify_query_result(&output, Some(code)),
                Err(error) => Err(AutostartError::Task {
                    action: "query",
                    detail: error.to_string(),
                }),
            },
        )
    })
}

pub(super) fn helper_result(action: &str) -> Option<Result<(), AutostartError>> {
    FIXTURE.with(|fixture| {
        let mut fixture = fixture.borrow_mut();
        fixture.as_mut()?.helpers.push(action.to_string());
        Some(Ok(()))
    })
}

pub(super) fn run_state() -> Option<Result<RunEntry, std::io::Error>> {
    FIXTURE.with(|fixture| {
        fixture.borrow().as_ref().map(|fixture| {
            fixture
                .run
                .as_ref()
                .copied()
                .map_err(|error| std::io::Error::new(error.kind(), error.to_string()))
        })
    })
}

pub(super) fn run_write(enable: bool) -> Option<Result<(), AutostartError>> {
    FIXTURE.with(|fixture| {
        let mut fixture = fixture.borrow_mut();
        fixture.as_mut()?.writes.push(enable);
        Some(Ok(()))
    })
}

fn output(code: i32, stdout: &[u8], stderr: &[u8]) -> std::io::Result<(i32, std::process::Output)> {
    #[cfg(not(windows))]
    use std::os::unix::process::ExitStatusExt;
    #[cfg(windows)]
    use std::os::windows::process::ExitStatusExt;
    Ok((
        code,
        std::process::Output {
            #[cfg(windows)]
            status: std::process::ExitStatus::from_raw(code as u32),
            #[cfg(not(windows))]
            status: std::process::ExitStatus::from_raw(code),
            stdout: stdout.to_vec(),
            stderr: stderr.to_vec(),
        },
    ))
}

fn assert_effects(helpers: &[&str], writes: &[bool]) {
    FIXTURE.with(|fixture| {
        let fixture = fixture.borrow();
        let fixture = fixture.as_ref().expect("fixture");
        assert_eq!(fixture.helpers, helpers);
        assert_eq!(fixture.writes, writes);
    });
}

#[test]
fn confirmed_missing_task_renders_off_and_allows_user_autostart() {
    with_fixture(
        vec![
            output(TASK_MISSING, b"", b"localized missing"),
            output(TASK_MISSING, b"", b""),
            output(TASK_MISSING, b"", b""),
        ],
        Ok(RunEntry::Absent),
        || {
            refresh_admin_task_cache();
            assert_eq!(autostart_state(), Some(AutostartMode::Off));
            set_autostart_mode(AutostartMode::User).expect("enable after a live absence");
            assert_effects(&[], &[true]);
        },
    );
}

#[test]
fn query_failures_invalidate_successful_cache_and_block_every_mode() {
    for mode in [
        AutostartMode::Off,
        AutostartMode::User,
        AutostartMode::Admin,
    ] {
        for previous in [false, true] {
            for failure in [
                output(ACCESS_DENIED, b"", b"access denied"),
                output(1, b"", b"localized failure"),
                output(0, b"", b""),
                output(0, b"not CSV", b""),
                output(0, b"\"\\AudioSwitcher\",\"N/A\"", b""),
                output(0, b"\"\\OtherTask\",\"N/A\",\"Ready\"", b""),
                output(0, TASK_ROW, b"partial query failure"),
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "cannot spawn schtasks",
                )),
            ] {
                with_fixture(vec![failure], Ok(RunEntry::Enabled), || {
                    set_admin_task_cached(Some(previous));
                    assert!(set_autostart_mode(mode).is_err(), "must reject {mode:?}");
                    assert_eq!(autostart_state(), None, "must disable menu for {mode:?}");
                    assert_effects(&[], &[]);
                });
            }
        }
    }
}

#[test]
fn refresh_failure_never_keeps_a_previous_off_or_admin_display() {
    for previous in [false, true] {
        with_fixture(
            vec![output(ACCESS_DENIED, b"", b"denied")],
            Ok(RunEntry::Absent),
            || {
                set_admin_task_cached(Some(previous));
                refresh_admin_task_cache();
                assert_eq!(autostart_state(), None);
            },
        );
    }
}

#[test]
fn task_confirmation_failure_never_removes_run_or_reports_success() {
    for mode in [
        AutostartMode::Off,
        AutostartMode::User,
        AutostartMode::Admin,
    ] {
        let mut queries = vec![output(0, TASK_ROW, b"")];
        if mode != AutostartMode::Admin {
            queries.push(output(0, TASK_ROW, b""));
        }
        queries.push(output(ACCESS_DENIED, b"", b"denied"));
        with_fixture(queries, Ok(RunEntry::Enabled), || {
            assert!(set_autostart_mode(mode).is_err());
            assert_eq!(autostart_state(), None);
            assert_effects(
                &[if mode == AutostartMode::Admin {
                    HELPER_TASK_ON
                } else {
                    HELPER_TASK_OFF
                }],
                &[],
            );
        });
    }
}

#[test]
fn live_delete_query_cannot_fall_back_to_an_absent_cache() {
    with_fixture(
        vec![
            output(TASK_MISSING, b"", b""),
            output(ACCESS_DENIED, b"", b"denied"),
        ],
        Ok(RunEntry::Absent),
        || {
            assert!(set_autostart_mode(AutostartMode::Off).is_err());
            assert_eq!(autostart_state(), None);
            assert_effects(&[], &[]);
        },
    );
}

#[test]
fn confirmed_task_changes_allow_the_run_write_only_after_confirmation() {
    for mode in [
        AutostartMode::Off,
        AutostartMode::User,
        AutostartMode::Admin,
    ] {
        let mut queries = vec![output(0, TASK_ROW, b"")];
        if mode != AutostartMode::Admin {
            queries.push(output(0, TASK_ROW, b""));
        }
        queries.push(if mode == AutostartMode::Admin {
            output(0, b"\"\\audioswitcher\",\"N/A\",\"Pr\x88t\"\r\n", b"")
        } else {
            output(TASK_MISSING, b"", b"")
        });
        with_fixture(queries, Ok(RunEntry::Enabled), || {
            set_autostart_mode(mode).expect("confirmed task change");
            assert_effects(
                &[if mode == AutostartMode::Admin {
                    HELPER_TASK_ON
                } else {
                    HELPER_TASK_OFF
                }],
                &[mode == AutostartMode::User],
            );
        });
    }
}

#[test]
fn unreadable_run_blocks_task_mutation_too() {
    for mode in [
        AutostartMode::Off,
        AutostartMode::User,
        AutostartMode::Admin,
    ] {
        with_fixture(
            vec![output(0, TASK_ROW, b"")],
            Err(std::io::Error::other("denied")),
            || {
                assert!(set_autostart_mode(mode).is_err());
                assert_effects(&[], &[]);
            },
        );
    }
}

#[test]
fn helper_switch_is_not_a_normal_launch() {
    // The prefix must stay a single distinctive token: `main` exits on it before
    // the single-instance guard, so a false positive would kill startup.
    assert!(HELPER_ARG_PREFIX.starts_with("--"));
    assert!(HELPER_ARG_PREFIX.ends_with('='));
    assert_ne!(HELPER_TASK_ON, HELPER_TASK_OFF);
}

#[test]
#[cfg(windows)]
fn describe_carries_exit_code_and_tool_output() {
    // The detail is what makes a failed `schtasks` diagnosable; a bare exit
    // code was the whole story before.
    let out = std::process::Command::new("cmd")
        .args(["/c", "echo boom & exit 3"])
        .output()
        .expect("spawn cmd");
    let detail = describe(&out);
    assert!(detail.contains("exit code 3"), "{detail}");
    assert!(detail.contains("boom"), "{detail}");
}

/// Reads fixture bytes without treating unreadable state as absence.
///
/// Reads the registry directly (rather than through the module's own helpers)
/// so the assertions can pin the stored format: a `REG_SZ` command with its
/// terminator, and the twelve-byte Task Manager marker.
#[cfg(windows)]
fn read_registry_value(subkey: &str, name: &str) -> std::io::Result<Option<Vec<u8>>> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    use windows::core::PCWSTR;

    let subkey = crate::platform::utf16::wide_z(subkey);
    let name = crate::platform::utf16::wide_z(name);
    let mut hkey = HKEY(std::ptr::null_mut());
    // SAFETY: the subkey string outlives the call; `hkey` is written only on
    // success and closed on every path below.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_QUERY_VALUE,
            &raw mut hkey,
        )
    };
    if opened == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if opened.is_err() {
        return Err(std::io::Error::from_raw_os_error(opened.0 as i32));
    }
    let mut len = 0u32;
    // SAFETY: a null data pointer with a size pointer asks for the size only.
    let sized = unsafe {
        RegQueryValueExW(
            hkey,
            PCWSTR(name.as_ptr()),
            None,
            None,
            None,
            Some(&raw mut len),
        )
    };
    if sized.is_err() {
        // SAFETY: closes the handle opened above.
        unsafe {
            let _ = RegCloseKey(hkey);
        };
        return if sized == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND {
            Ok(None)
        } else {
            Err(std::io::Error::from_raw_os_error(sized.0 as i32))
        };
    }
    let mut data = vec![0u8; usize::try_from(len).unwrap_or(0)];
    // SAFETY: `data` holds exactly `len` bytes, the size just read.
    let read = unsafe {
        RegQueryValueExW(
            hkey,
            PCWSTR(name.as_ptr()),
            None,
            None,
            Some(data.as_mut_ptr()),
            Some(&raw mut len),
        )
    };
    // SAFETY: closes the handle opened above.
    unsafe {
        let _ = RegCloseKey(hkey);
    };
    if read.is_err() {
        return Err(std::io::Error::from_raw_os_error(read.0 as i32));
    }
    data.truncate(len as usize);
    Ok(Some(data))
}

#[test]
#[cfg(windows)]
#[ignore = "writes an isolated HKCU Run test value; run explicitly"]
fn run_value_round_trip() {
    let _gate = crate::INTEGRATION_GATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _key = isolated_key();
    assert_eq!(
        read_registry_value(RUN_KEY, autostart_key_name()).expect("read Run fixture"),
        None
    );
    assert_eq!(
        read_registry_value(RUN_APPROVED_KEY, autostart_key_name()).expect("read marker fixture"),
        None
    );
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            use windows::Win32::System::Registry::{
                HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
            };
            use windows::core::PCWSTR;
            for subkey in [RUN_KEY, RUN_APPROVED_KEY] {
                let subkey = crate::platform::utf16::wide_z(subkey);
                let name = crate::platform::utf16::wide_z(autostart_key_name());
                let mut key = HKEY(std::ptr::null_mut());
                // SAFETY: strings and output handle live through the call.
                let opened = unsafe {
                    RegOpenKeyExW(
                        HKEY_CURRENT_USER,
                        PCWSTR(subkey.as_ptr()),
                        None,
                        KEY_SET_VALUE,
                        &raw mut key,
                    )
                };
                if opened.is_ok() {
                    // SAFETY: key is open and name is terminated; only our isolated value is removed.
                    let removed = unsafe { RegDeleteValueW(key, PCWSTR(name.as_ptr())) };
                    // SAFETY: closes the handle opened above exactly once.
                    unsafe {
                        let _ = RegCloseKey(key);
                    }
                    if removed.is_err()
                        && removed != windows::Win32::Foundation::ERROR_FILE_NOT_FOUND
                    {
                        tracing::warn!("isolated registry fixture cleanup failed: {removed:?}");
                    }
                }
            }
        }
    }
    let _cleanup = Cleanup;
    enable_run_value().expect("enable");
    assert_eq!(
        run_value_state().expect("read back after enable"),
        RunEntry::Enabled,
        "the Run value must read as installed and enabled"
    );
    let exe = get_exe_path().expect("exe path");
    let expected: Vec<u8> = format!("\"{}\"", exe.display())
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .chain([0, 0])
        .collect();
    assert_eq!(
        read_registry_value(RUN_KEY, autostart_key_name()).expect("read Run fixture"),
        Some(expected),
        "the stored Run command must be this exe, in quotes"
    );
    assert_eq!(
        read_registry_value(RUN_APPROVED_KEY, autostart_key_name()).expect("read marker fixture"),
        Some(STARTUP_APPROVED_ENABLED.to_vec()),
        "the Task Manager marker must be rewritten as enabled"
    );

    disable_run_value().expect("disable");
    assert_eq!(
        run_value_state().expect("read back after disable"),
        RunEntry::Absent,
        "the Run value must be gone, not merely switched off"
    );
    assert_eq!(
        read_registry_value(RUN_KEY, autostart_key_name()).expect("read Run fixture"),
        None,
        "the Run value must be removed, not blanked"
    );
}

#[test]
fn startup_approved_state_follows_the_state_byte() {
    // The enabled forms the shell writes: never touched (0x06) and enabled by
    // the user (0x02).
    assert!(startup_approved_state(&[
        0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
    ]));
    assert!(startup_approved_state(&[
        0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
    ]));
    // Disabled: the same value with an odd state byte.
    assert!(!startup_approved_state(&[
        0x03, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
    ]));
    assert!(!startup_approved_state(&[
        0x07, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
    ]));
    // Switched off and on again: the state byte says enabled, and the timestamp
    // the shell stamps next to it must not be read as "disabled".
    assert!(startup_approved_state(&[
        0x02, 0, 0, 0, 0x40, 0x9A, 0x2C, 0x3D, 0x8B, 0x0E, 0xDB, 0x01,
    ]));
    // Nothing to remember reads as enabled.
    assert!(startup_approved_state(&[]));
}

#[test]
#[cfg(windows)]
#[ignore = "runs real schtasks against an isolated test task; run explicitly"]
fn the_task_round_trip_respects_the_helper_boundary() {
    let _gate = crate::INTEGRATION_GATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _key = isolated_key();
    // An unexpected collision must not be overwritten, even for a fixture name.
    if task_exists().expect("query must establish absence before any write") {
        assert!(
            task_exists().expect("query task"),
            "the task stays untouched"
        );
        return;
    }
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Err(error) = delete_task() {
                tracing::warn!("isolated task fixture cleanup failed: {error}");
            }
        }
    }
    let _cleanup = Cleanup;
    let elevated = {
        // SAFETY: `IsUserAnAdmin` takes no arguments and only reads the
        // calling token.
        unsafe { windows::Win32::UI::Shell::IsUserAnAdmin() }.as_bool()
    };
    if !elevated {
        // Medium integrity cannot write a HIGHEST task, which is exactly why
        // the menu re-launches an elevated helper (see the autostart module
        // doc). The detail is localized, so only the failure is asserted.
        match create_task() {
            Err(AutostartError::Task {
                action: "create", ..
            }) => {
                assert!(
                    !task_exists().expect("query task"),
                    "a denied create must not leave a task behind"
                );
            }
            other => panic!("a medium-integrity process must not create the task: {other:?}"),
        }
        return;
    }
    // Production runs this from the elevated helper; exercise it directly.
    create_task().expect("create task");
    assert!(task_exists().expect("query task"), "task should exist");
    delete_task().expect("delete task");
    assert!(!task_exists().expect("query task"), "task should be gone");
}
