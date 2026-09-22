//! Crash reporting: file sink + panic hook, installed once from `main`.
//!
//! Libraries never install a subscriber; this helper only exists so the
//! binary entry stays at guards → `App::run`. The install site is `main`.
//!
//! Level: `INFO` and above by default, overridden by the `AUDIO_SWITCHER_LOG`
//! environment variable (`error` / `warn` / `info` / `debug` / `trace`). The
//! variable is parsed here rather than through `tracing-subscriber`'s
//! `env-filter` feature, which would pull `regex` and `matchers` into a
//! single-file binary that has a hard size budget.

/// Parse `AUDIO_SWITCHER_LOG` into a verbosity level.
///
/// Unknown and absent values keep the default (`INFO`, i.e. the shipped
/// behaviour).
fn max_level() -> tracing::Level {
    match std::env::var("AUDIO_SWITCHER_LOG")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "error" => tracing::Level::ERROR,
        "warn" => tracing::Level::WARN,
        "debug" => tracing::Level::DEBUG,
        "trace" => tracing::Level::TRACE,
        _ => tracing::Level::INFO,
    }
}

/// Why the log file could not be opened, when it could not.
///
/// Kept for the panic hook: at that point there is no subscriber to log
/// through, and the crash report is the last channel that still works.
static INIT_FAILURE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Tail appended to the crash report when the log file was never opened.
///
/// Without it the dialog reads like any other crash, next to a log directory
/// the user will search and find nothing in.
fn failure_note(failure: Option<&str>) -> String {
    match failure {
        Some(why) => format!("\n\n(log file unavailable: {why})"),
        None => String::new(),
    }
}

/// How long a daily log file is kept.
///
/// The name is day-indexed, so an install that lives for months would
/// otherwise accumulate a file per day, none of them read again.
const LOG_RETENTION: std::time::Duration = std::time::Duration::from_secs(14 * 24 * 60 * 60);

/// Delete this tool's own log files older than [`LOG_RETENTION`].
///
/// Best effort: a file that will not delete (still open, permission) is
/// skipped, and a name that is not one of ours is left alone — the directory
/// is ours by name but `LOCALAPPDATA` can point somewhere shared.
fn prune_old_logs(dir: &std::path::Path, now: std::time::SystemTime) {
    let prefix = format!("{}-", crate::TOOL_ID);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(&prefix) || !name.ends_with(".log") {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) else {
            continue;
        };
        // A future stamp (wall-clock jump) is not "old", so it is kept.
        if now
            .duration_since(modified)
            .is_ok_and(|age| age > LOG_RETENTION)
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Install the `%LOCALAPPDATA%\<tool>\logs\` file sink and the
/// show-dialog-and-exit panic hook. Idempotent best effort: when the log
/// file cannot be opened, diagnostics still reach the dialog on panic.
pub fn init() {
    let log_path = log_file_path();
    if let Some(parent) = log_path.parent() {
        // Best effort: the `File::create` below is what reports a real failure.
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::File::create(&log_path) {
        Ok(file) => {
            // File sink only: `windows_subsystem = "windows"` detaches stdio, so
            // a console layer would be invisible; the file is the record.
            let subscriber = tracing_subscriber::fmt()
                .with_max_level(max_level())
                .with_writer(std::sync::Mutex::new(file))
                .finish();
            let _ = tracing::subscriber::set_global_default(subscriber);
        }
        Err(e) => {
            let _ = INIT_FAILURE.set(e.to_string());
        }
    }

    std::panic::set_hook(Box::new(|info| {
        let msg = format!(
            "{} hit an unexpected error and must exit.\n\n{info}{}",
            crate::TOOL_DISPLAY_NAME,
            failure_note(INIT_FAILURE.get().map(String::as_str))
        );
        tracing::error!("panic: {info}");
        crate::platform::dialog::show_critical(&msg);
        std::process::exit(1);
    }));

    tracing::debug!("logging to {}", log_file_path().display());

    prune_old_logs(&log_dir(), std::time::SystemTime::now());
}

/// Daily log file: day-indexed name gives rotation without a date library
/// (SystemTime days since epoch; a wall-clock jump only renames the file).
fn log_file_path() -> std::path::PathBuf {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400);
    log_dir().join(format!("{}-{days}.log", crate::TOOL_ID))
}

fn log_dir() -> std::path::PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .map_or_else(|_| std::env::temp_dir(), std::path::PathBuf::from);
    base.join(crate::TOOL_ID).join("logs")
}

#[cfg(test)]
mod tests;
