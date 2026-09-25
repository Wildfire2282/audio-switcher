//! Auto-launch (autostart) helpers.
//!
//! [`AutostartMode`] picks the mechanism: none, the current-user `Run` value, or
//! a logon scheduled task with highest privileges.
//!
//! The elevated mode exists because of UIPI: while a HIGH-integrity process owns
//! the foreground window (Tencent Androws does, permanently), Windows stops
//! delivering mouse input to a medium-integrity low-level hook, so hover-wheel
//! goes dead over that window. An equally elevated process keeps receiving it.
//!
//! Writing that task needs elevation: `schtasks /Create /RL HIGHEST` from a
//! medium-integrity process fails with "Access is denied". The menu path
//! therefore re-launches this exe through `ShellExecuteW(.., "runas", ..)` with
//! [`HELPER_ARG_PREFIX`], and that child — which starts before the
//! single-instance guard — performs the write. One consent prompt per switch; an
//! already-elevated process sees none. (An earlier revision believed no
//! elevation was needed and deleted the helper; that measurement was taken in an
//! already-elevated shell, which hid the requirement.)
//!
//! Read failures are [`AutostartState::Unknown`]: the UI grays the group out, it
//! never pretends the setting is off, and no write happens on `Unknown`.

mod run_value;
mod task;

pub(super) use run_value::*;
pub(super) use task::*;

pub use task::refresh_admin_task_cache;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Which autostart mechanism the user picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutostartMode {
    Off,
    /// Current-user `Run` value; starts at the caller's integrity level.
    #[default]
    User,
    /// Logon scheduled task with highest privileges, so input aimed at elevated
    /// foreground windows keeps reaching our low-level mouse hook.
    Admin,
}

/// Autostart failure. The `#[error]` text is the user-visible message.
#[derive(Debug, Error)]
pub enum AutostartError {
    #[error("cannot determine executable path")]
    NoExePath,
    #[error("failed to enable autostart")]
    Enable(#[source] std::io::Error),
    #[error("failed to disable autostart")]
    Disable(#[source] std::io::Error),
    /// The elevated helper never started: the user declined the consent prompt
    /// or the launch itself failed (`ShellExecuteW` returned `<= 32`).
    #[error("elevation declined or unavailable (code {code})")]
    Elevation {
        /// Raw `ShellExecuteW` return code.
        code: usize,
    },
    #[error("scheduled task {action} failed: {detail}")]
    Task {
        /// One of `create`, `delete`, `query`.
        action: &'static str,
        /// Spawn error or exit code, plus what the tool printed.
        detail: String,
    },
}

/// Read-back of the autostart setting; the menu renders it and never writes on
/// `Unknown`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutostartState {
    Off,
    User,
    Admin,
    /// Read failed (reason); UI reports, never writes.
    Unknown(String),
}

impl AutostartState {
    /// The installed mode, or `None` when the read failed.
    ///
    /// The single place the read-back becomes a mode: `ui` renders `None` as the
    /// grayed group and never guesses one.
    #[must_use]
    pub fn mode(&self) -> Option<AutostartMode> {
        match self {
            Self::Off => Some(AutostartMode::Off),
            Self::User => Some(AutostartMode::User),
            Self::Admin => Some(AutostartMode::Admin),
            Self::Unknown(_) => None,
        }
    }
}

/// `Run` value name and scheduled-task name, both derived from the PascalCase
/// display name. Locked by test: renaming orphans existing installs.
///
/// Borrowed, not owned: [`autostart_state`] runs on every menu refresh and the
/// name is a `'static` constant, so allocating here would be pure churn.
#[must_use]
pub fn autostart_key_name() -> &'static str {
    crate::TOOL_DISPLAY_NAME
}

/// Command-line switch that turns this process into the elevated helper.
pub const HELPER_ARG_PREFIX: &str = "--autostart-helper=";

#[must_use]
pub fn get_exe_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// The scheduled-task half comes from a cache (see [`refresh_admin_task_cache`])
/// because `schtasks` costs a process launch and this runs on every menu refresh.
/// A read failure is `Unknown`, never `Off`: the caller grays the UI out and must
/// not write.
#[must_use]
pub fn autostart_state() -> AutostartState {
    match run_value_state() {
        Ok(RunEntry::Enabled) => AutostartState::User,
        // A `Run` value the shell has switched off is not autostarting, so the
        // menu must not show it as if it were.
        Ok(RunEntry::Absent | RunEntry::Disabled) => {
            if admin_task_cached() {
                AutostartState::Admin
            } else {
                AutostartState::Off
            }
        }
        Err(e) => AutostartState::Unknown(e.to_string()),
    }
}

/// Whether the current-user `Run` entry is missing entirely.
///
/// The one state the startup self-heal may write over. A value the user switched
/// off in Task Manager's Startup tab is present with the shell's marker, and
/// rewriting it on every launch would keep undoing that choice; a read failure
/// is not "absent" either — a value this tool cannot read is not one it may
/// overwrite.
#[must_use]
pub fn run_entry_absent() -> bool {
    matches!(run_value_state(), Ok(RunEntry::Absent))
}

/// Apply `mode`: clear the mechanism `mode` does not use, then set the one it
/// does.
///
/// Switching into or out of [`AutostartMode::Admin`] raises one consent prompt
/// (see the module docs). The helper is launched asynchronously, so a successful
/// return means "the helper started", not "the task exists" — the cache is
/// updated optimistically and the next startup query corrects it.
///
/// # Errors
///
/// [`AutostartError`] when the exe path is unknown, a registry write fails,
/// elevation was declined, or `schtasks` fails inside the helper.
pub fn set_autostart_mode(mode: AutostartMode) -> Result<(), AutostartError> {
    match mode {
        AutostartMode::Off => {
            remove_admin_task()?;
            set_run_value(false)
        }
        AutostartMode::User => {
            remove_admin_task()?;
            set_run_value(true)
        }
        AutostartMode::Admin => {
            // Create first: a failure here must leave the existing `Run` entry
            // alone rather than dropping the user to "no autostart at all".
            run_helper(HELPER_TASK_ON)?;
            set_admin_task_cached(true);
            set_run_value(false)
        }
    }
}

/// Handle the elevated-helper invocation, when this process is one.
///
/// Returns `Some(exit_code)` when the process was started with the helper switch
/// and must exit without touching the tray, the single-instance guard or COM.
/// Returns `None` for a normal start.
#[must_use]
pub fn run_autostart_helper_if_requested() -> Option<i32> {
    let action =
        std::env::args().find_map(|arg| arg.strip_prefix(HELPER_ARG_PREFIX).map(str::to_string))?;
    let outcome = match action.as_str() {
        HELPER_TASK_ON => create_task(),
        HELPER_TASK_OFF => delete_task(),
        other => {
            tracing::warn!("unknown autostart helper action: {other}");
            return Some(2);
        }
    };
    match outcome {
        Ok(()) => Some(0),
        Err(e) => {
            // The dialog is the only channel back to the user: this child has no
            // console and the parent does not wait for it.
            tracing::warn!("autostart helper failed: {e}");
            super::dialog::show_msgbox(&format!(
                "{} could not change the startup task.\n\n{e}",
                crate::TOOL_DISPLAY_NAME
            ));
            Some(1)
        }
    }
}

#[cfg(test)]
mod tests;
