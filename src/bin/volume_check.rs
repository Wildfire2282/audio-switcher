//! Smoke test for `audio::adjust_volume`: call up once, then down once, then
//! restore. Verifies both directions complete without error.
//!
//! Invoke: `cargo run --release --bin volume_check`.

use audio_switcher::audio;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

fn main() -> windows::core::Result<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };

    let result = run();

    unsafe { CoUninitialize() };
    result
}

fn run() -> windows::core::Result<()> {
    // Up 1 notch — should increase volume by ~1%.
    audio::adjust_volume(1)?;
    println!("[volume_check] adjust_volume(+1) ok");

    // Down 1 notch — should bring volume back within ~1% of original.
    audio::adjust_volume(-1)?;
    println!("[volume_check] adjust_volume(-1) ok");

    println!("[volume_check] PASS");
    Ok(())
}
