//! End-to-end smoke test for the audio module. This binary has its own
//! `main` (no `EventLoop`, no `TrayIcon`) so it can run in a console, print
//! verbose output, and exit. It exercises:
//!
//!   1. `CoInitializeEx` (MTA)
//!   2. `enumerate_render_endpoints` — must return >= 1 device
//!   3. `current_default_id` — must be `Some(_)` and present in the enum
//!   4. `set_default` for every non-current device, then re-query to verify
//!      the round-trip
//!   5. Restore the original default at the end
//!
//! Invoke: `cargo run --release --bin smoke_audio`.

use audio_switcher::audio;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

fn main() -> windows::core::Result<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };

    let result = run();

    unsafe { CoUninitialize() };
    result
}

fn run() -> windows::core::Result<()> {
    let devices = audio::enumerate_render_endpoints();
    println!("[smoke] enumerated {} render device(s):", devices.len());
    for d in &devices {
        println!("  - {}  ({})", d.name, d.id);
    }
    if devices.is_empty() {
        eprintln!("[smoke] FAIL: no render endpoints");
        std::process::exit(1);
    }

    let original_default = match audio::current_default_id() {
        Some(id) => id,
        None => {
            eprintln!("[smoke] FAIL: current_default_id returned None");
            std::process::exit(1);
        }
    };
    println!("[smoke] current default id = {original_default}");

    if !devices.iter().any(|d| d.id == original_default) {
        eprintln!("[smoke] FAIL: current default id is not in the enumerated set");
        std::process::exit(1);
    }

    // Try switching to every other device, then back to the original. We
    // accept that the very first device may equal `original_default`, in
    // which case we skip the switch.
    let mut switched = 0usize;
    for d in &devices {
        if d.id == original_default {
            continue;
        }
        audio::set_default(&d.id)?;
        let now = audio::current_default_id();
        if now.as_deref() != Some(d.id.as_str()) {
            eprintln!("[smoke] FAIL: switch to '{}' did not stick (got '{:?}')", d.id, now);
            std::process::exit(1);
        }
        switched += 1;
        println!("[smoke] switch ok -> {}", d.name);
    }
    if switched == 0 {
        eprintln!("[smoke] WARN: only one device, nothing to switch to");
    } else {
        println!("[smoke] all {switched} non-default switches round-tripped");
    }

    // Restore the original default.
    audio::set_default(&original_default)?;
    let now = audio::current_default_id();
    if now.as_deref() != Some(original_default.as_str()) {
        eprintln!("[smoke] FAIL: could not restore original default");
        std::process::exit(1);
    }
    println!("[smoke] restored original default ok");

    println!("[smoke] PASS");
    Ok(())
}
