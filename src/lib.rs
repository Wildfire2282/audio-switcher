//! Library surface — re-exports the audio and tray modules so the binary
//! `main.rs` and integration tests share the same code path.
pub mod audio;
pub mod startup;
pub mod theme;
pub mod tray;
