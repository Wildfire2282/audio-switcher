//! Windows platform abstractions: guards, shell, dialogs, pump, autostart.
//!
//! `platform/*` modules are mutually unreferenced except for foundational
//! use: `dialog` (message boxes, including the autostart error box) and
//! `shell` may be called from `app`/`ui`, `shell` borrows `autostart`'s exe
//! path, and `utf16` is the shared UTF-16 boundary conversion. Those edges are
//! noted on the callee side per the module contract.

pub mod autostart;
pub mod com;
pub mod dialog;
pub mod hotkey;
pub mod locale;
pub mod logging;
pub mod mouse_hook;
pub mod osd;
pub mod pump;
pub mod shell;
pub mod single_instance;
pub mod theme;
#[cfg(windows)]
pub mod utf16;

pub use autostart::{
    AutostartMode, autostart_state, refresh_admin_task_cache, run_entry_absent, set_autostart_mode,
};
pub use com::ComGuard;
pub use single_instance::SingleInstanceGuard;
