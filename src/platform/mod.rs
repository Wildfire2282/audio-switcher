//! Windows platform abstractions: guards, shell, dialogs, pump, autostart.
//!
//! `platform/*` modules are mutually unreferenced except for foundational
//! use: `dialog` (message boxes, including the autostart error box) and
//! `shell` may be called from `app`/`ui`, `shell` borrows `autostart`'s exe
//! path, and `utf16` is the shared UTF-16 boundary conversion. Those edges are
//! noted on the callee side per the module contract.

pub(crate) mod autostart;
pub(crate) mod com;
pub(crate) mod dialog;
pub(crate) mod hotkey;
pub(crate) mod locale;
pub(crate) mod logging;
pub(crate) mod mouse_hook;
pub(crate) mod osd;
pub(crate) mod pump;
pub(crate) mod shell;
pub(crate) mod single_instance;
pub(crate) mod theme;
#[cfg(windows)]
pub(crate) mod utf16;

pub(crate) use autostart::{
    AutostartMode, autostart_state, refresh_admin_task_cache, run_entry_absent, set_autostart_mode,
};
pub use com::ComGuard;
pub use single_instance::SingleInstanceGuard;
