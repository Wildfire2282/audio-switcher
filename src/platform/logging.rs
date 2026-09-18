//! Crash reporting: file sink + panic hook, installed once from `main`.
//!
//! Libraries never install a subscriber; this helper only exists so the
//! binary entry stays at guards → `App::run`. The install site is `main`.
//!
//! Level: `WARN` and above by default, overridden by the `AUDIO_SWITCHER_LOG`
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

/// Install the `%LOCALAPPDATA%\<tool>\logs\` file sink and the
/// show-dialog-and-exit panic hook. Idempotent best effort: when the log
/// file cannot be opened, diagnostics still reach the dialog on panic.
pub fn init() {
    let log_path = log_file_path();

    let file = log_path
        .parent()
        .map(std::fs::create_dir_all)
        .and(std::fs::File::create(&log_path).ok());
    if let Some(file) = file {
        // File sink only: `windows_subsystem = "windows"` detaches stdio, so
        // a console layer would be invisible; the file is the record.
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(max_level())
            .with_writer(std::sync::Mutex::new(file))
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
    }

    std::panic::set_hook(Box::new(|info| {
        let msg = format!(
            "{} hit an unexpected error and must exit.\n\n{info}",
            crate::TOOL_DISPLAY_NAME
        );
        tracing::error!("panic: {info}");
        crate::platform::dialog::show_critical(&msg);
        std::process::exit(1);
    }));

    tracing::debug!("logging to {}", log_file_path().display());
}

/// Daily log file: day-indexed name gives rotation without a date library
/// (SystemTime days since epoch; a wall-clock jump only renames the file).
fn log_file_path() -> std::path::PathBuf {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0);
    log_dir().join(format!("{}-{days}.log", crate::TOOL_ID))
}

fn log_dir() -> std::path::PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join(crate::TOOL_ID).join("logs")
}
