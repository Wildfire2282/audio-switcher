//! Global hotkeys (`RegisterHotKey` on the message-pumping thread).
//!
//! The parsing/describing half is platform-independent and unit-tested; only
//! registration touches Win32. Module contract: `pump` routes `WM_HOTKEY`
//! into [`note_pending`] (foundational edge, noted here), `app` owns policy
//! (which combos are bound, when to re-register). `config` stores combos as
//! canonical strings, so users edit `config.json` manually (all unbound by
//! default; no menu toggles).
//!
//! Registration is thread-affine: `RegisterHotKey(None, ...)` binds the combo
//! to the calling thread, and `WM_HOTKEY` is posted to that thread's queue.
//! `App` therefore registers and unregisters from the thread that pumps
//! messages.

use std::fmt;
use std::str::FromStr;

use thiserror::Error;

/// Win32 `MOD_ALT`. Plain integers keep the pure half of this module free of
/// the Windows bindings; the values are the documented `MOD_*` bits.
const MOD_ALT: u32 = 0x0001;
/// Win32 `MOD_CONTROL`.
const MOD_CONTROL: u32 = 0x0002;
/// Win32 `MOD_SHIFT`.
const MOD_SHIFT: u32 = 0x0004;
/// Win32 `MOD_WIN`.
const MOD_WIN: u32 = 0x0008;

/// Bit `action.id() - 1`, the per-action slot in the registration/pending masks.
#[must_use]
const fn bit(action: HotkeyAction) -> u32 {
    1 << (action.id() - 1)
}

/// Action ids currently registered on the pumping thread (survives until
/// [`unregister_all`], which is what actually releases them).
static REGISTERED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Action ids whose `WM_HOTKEY` is waiting for the app loop (bitmask, so two
/// hotkeys arriving in one frame are both dispatched, in id order).
static PENDING: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// A hotkey-bindable action.
///
/// The discriminant doubles as the `RegisterHotKey` id and as the bit position
/// in [`PENDING`], so an id is never reused for a different action — a stale id
/// would otherwise alias whichever action took the slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    /// Toggle the default output device's mute.
    Mute,
    /// Raise the master volume one step.
    VolumeUp,
    /// Lower the master volume one step.
    VolumeDown,
    /// Switch to the next output device (wraps at the end).
    NextDevice,
    /// Switch to the previous output device (wraps at the start).
    PrevDevice,
}

impl HotkeyAction {
    /// Every action, in menu order.
    pub const ALL: [Self; 5] = [
        Self::Mute,
        Self::VolumeUp,
        Self::VolumeDown,
        Self::NextDevice,
        Self::PrevDevice,
    ];

    /// Stable registration id (`1..=5`) used for `RegisterHotKey`/`WM_HOTKEY`.
    #[must_use]
    pub const fn id(self) -> i32 {
        match self {
            Self::Mute => 1,
            Self::VolumeUp => 2,
            Self::VolumeDown => 3,
            Self::NextDevice => 4,
            Self::PrevDevice => 5,
        }
    }

    /// Map a `WM_HOTKEY` id back to its action.
    #[must_use]
    pub const fn from_id(id: i32) -> Option<Self> {
        match id {
            1 => Some(Self::Mute),
            2 => Some(Self::VolumeUp),
            3 => Some(Self::VolumeDown),
            4 => Some(Self::NextDevice),
            5 => Some(Self::PrevDevice),
            _ => None,
        }
    }

    /// `AppConfig::hotkeys` field name.
    #[must_use]
    pub const fn config_key(self) -> &'static str {
        match self {
            Self::Mute => "mute",
            Self::VolumeUp => "volume_up",
            Self::VolumeDown => "volume_down",
            Self::NextDevice => "next_device",
            Self::PrevDevice => "prev_device",
        }
    }

    /// English action name for logs and dialogs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mute => "Toggle Mute",
            Self::VolumeUp => "Volume Up",
            Self::VolumeDown => "Volume Down",
            Self::NextDevice => "Next Output Device",
            Self::PrevDevice => "Previous Output Device",
        }
    }
}

/// Display form of a key: either an ASCII letter/digit (rendered from the
/// virtual-key code) or a table name (`Up`, `F5`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyName {
    /// `A`–`Z` / `0`–`9`, canonicalized to uppercase.
    Ascii(char),
    /// Named key from [`NAMED_KEYS`].
    Named(&'static str),
}

/// Named non-character keys: `(canonical name, Win32 virtual-key code)`.
const NAMED_KEYS: &[(&str, u32)] = &[
    ("Backspace", 0x08),
    ("Tab", 0x09),
    ("Enter", 0x0D),
    ("Escape", 0x1B),
    ("Space", 0x20),
    ("PageUp", 0x21),
    ("PageDown", 0x22),
    ("End", 0x23),
    ("Home", 0x24),
    ("Left", 0x25),
    ("Up", 0x26),
    ("Right", 0x27),
    ("Down", 0x28),
    ("Insert", 0x2D),
    ("Delete", 0x2E),
    ("Numpad0", 0x60),
    ("Numpad1", 0x61),
    ("Numpad2", 0x62),
    ("Numpad3", 0x63),
    ("Numpad4", 0x64),
    ("Numpad5", 0x65),
    ("Numpad6", 0x66),
    ("Numpad7", 0x67),
    ("Numpad8", 0x68),
    ("Numpad9", 0x69),
    ("Multiply", 0x6A),
    ("Add", 0x6B),
    ("Subtract", 0x6D),
    ("Decimal", 0x6E),
    ("Divide", 0x6F),
    ("F1", 0x70),
    ("F2", 0x71),
    ("F3", 0x72),
    ("F4", 0x73),
    ("F5", 0x74),
    ("F6", 0x75),
    ("F7", 0x76),
    ("F8", 0x77),
    ("F9", 0x78),
    ("F10", 0x79),
    ("F11", 0x7A),
    ("F12", 0x7B),
    ("F13", 0x7C),
    ("F14", 0x7D),
    ("F15", 0x7E),
    ("F16", 0x7F),
    ("F17", 0x80),
    ("F18", 0x81),
    ("F19", 0x82),
    ("F20", 0x83),
    ("F21", 0x84),
    ("F22", 0x85),
    ("F23", 0x86),
    ("F24", 0x87),
];

/// A global hotkey: at least one modifier plus exactly one key.
///
/// Parsed from a canonical string (`"Ctrl+Alt+M"`, case-insensitive) and
/// rendered back in canonical form, so a config round-trip is lossless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    /// Win32 `MOD_*` bitmask.
    mods: u32,
    /// Key display name (letters/digits render from `vk`).
    key: KeyName,
    /// Win32 virtual-key code.
    vk: u32,
}

impl Hotkey {
    /// Win32 `MOD_*` bitmask, used directly as `HOT_KEY_MODIFIERS`.
    #[must_use]
    pub(crate) const fn modifiers(self) -> u32 {
        self.mods
    }

    /// Win32 virtual-key code.
    #[must_use]
    pub(crate) const fn vk(self) -> u32 {
        self.vk
    }
}

/// Why a combination string was rejected.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum HotkeyParseError {
    /// Empty input or an empty `+`-separated token (e.g. `"Ctrl++"`).
    #[error("empty or malformed combination")]
    Malformed,
    /// A leading token was not `Ctrl`/`Alt`/`Shift`/`Win`.
    #[error("unknown modifier (use Ctrl, Alt, Shift, Win)")]
    UnknownModifier,
    /// The same modifier appeared twice.
    #[error("duplicate modifier")]
    DuplicateModifier,
    /// No modifier at all — a bare key would swallow normal typing.
    #[error("at least one modifier is required")]
    NoModifier,
    /// The trailing token is not a supported key name.
    #[error("unknown key name")]
    UnknownKey,
}

/// Resolve one key token to its display name and virtual-key code.
fn lookup_key(token: &str) -> Option<(KeyName, u32)> {
    let upper = token.to_ascii_uppercase();
    let mut chars = upper.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_uppercase() {
            return Some((KeyName::Ascii(c), 0x41 + (c as u32 - 'A' as u32)));
        }
        if c.is_ascii_digit() {
            return Some((KeyName::Ascii(c), 0x30 + (c as u32 - '0' as u32)));
        }
        return None;
    }
    let upper = upper.as_str();
    NAMED_KEYS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(upper))
        .map(|(name, vk)| (KeyName::Named(name), *vk))
}

impl FromStr for Hotkey {
    type Err = HotkeyParseError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let tokens: Vec<&str> = raw.split('+').map(str::trim).collect();
        if tokens.iter().any(|t| t.is_empty()) {
            return Err(HotkeyParseError::Malformed);
        }
        let Some((key_token, mod_tokens)) = tokens.split_last() else {
            return Err(HotkeyParseError::Malformed);
        };
        let mut mods = 0u32;
        for token in mod_tokens {
            let bit = match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => MOD_CONTROL,
                "alt" => MOD_ALT,
                "shift" => MOD_SHIFT,
                "win" | "meta" | "super" => MOD_WIN,
                _ => return Err(HotkeyParseError::UnknownModifier),
            };
            if mods & bit != 0 {
                return Err(HotkeyParseError::DuplicateModifier);
            }
            mods |= bit;
        }
        if mods == 0 {
            return Err(HotkeyParseError::NoModifier);
        }
        let (key, vk) = lookup_key(key_token).ok_or(HotkeyParseError::UnknownKey)?;
        Ok(Self { mods, key, vk })
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Canonical modifier order; the parsed set is order-insensitive.
        if self.mods & MOD_CONTROL != 0 {
            f.write_str("Ctrl+")?;
        }
        if self.mods & MOD_ALT != 0 {
            f.write_str("Alt+")?;
        }
        if self.mods & MOD_SHIFT != 0 {
            f.write_str("Shift+")?;
        }
        if self.mods & MOD_WIN != 0 {
            f.write_str("Win+")?;
        }
        match self.key {
            KeyName::Ascii(c) => write!(f, "{c}"),
            KeyName::Named(name) => f.write_str(name),
        }
    }
}

/// Human-readable combination (`"Ctrl+Alt+M"`) for logs and dialogs.
#[must_use]
pub fn describe_hotkey(hotkey: &Hotkey) -> String {
    hotkey.to_string()
}

/// Registration failure: combinations another program already owns. Every
/// occupied combo is collected in one report (`describe_hotkey` guidance in
/// the message) so the user fixes them in one pass.
#[derive(Debug, Clone, Error)]
#[error("hotkeys already in use by another program: {}", summarize(.0))]
pub struct HotkeyError(pub Vec<(HotkeyAction, Hotkey)>);

/// `"Ctrl+Alt+M (Toggle Mute), …"` — shared by [`HotkeyError`] and the app dialog.
#[must_use]
pub fn summarize(occupied: &[(HotkeyAction, Hotkey)]) -> String {
    occupied
        .iter()
        .map(|(action, hotkey)| format!("{} ({})", describe_hotkey(hotkey), action.name()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Register `bindings` on the calling thread, replacing any previous set.
///
/// Occupied combinations are collected and returned together; the remaining
/// ones stay registered so the app keeps working with the available subset.
/// The caller must run this on the thread that pumps messages (see module doc).
///
/// # Errors
///
/// Returns [`HotkeyError`] listing every combination another program owns.
#[cfg(windows)]
pub fn register_all(bindings: &[(HotkeyAction, Hotkey)]) -> Result<(), HotkeyError> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, RegisterHotKey};
    unregister_all();
    let mut occupied = Vec::new();
    let mut mask = 0u32;
    for &(action, hotkey) in bindings {
        // SAFETY: a null window registers on this thread; the combo was
        // validated by `Hotkey::from_str` (modifier bits + virtual key).
        let registered = unsafe {
            RegisterHotKey(
                None,
                action.id(),
                HOT_KEY_MODIFIERS(hotkey.modifiers()),
                hotkey.vk(),
            )
        };
        match registered {
            Ok(()) => mask |= bit(action),
            Err(e) => {
                // Usually ERROR_HOTKEY_ALREADY_REGISTERED; the raw error is
                // logged so a different cause is distinguishable in the log.
                tracing::warn!(
                    "hotkey {} ({}) rejected: {e}",
                    describe_hotkey(&hotkey),
                    action.name()
                );
                occupied.push((action, hotkey));
            }
        }
    }
    REGISTERED.store(mask, std::sync::atomic::Ordering::Release);
    if occupied.is_empty() {
        Ok(())
    } else {
        Err(HotkeyError(occupied))
    }
}

/// Non-Windows stub (registration is a Win32 facility).
#[cfg(not(windows))]
pub fn register_all(_bindings: &[(HotkeyAction, Hotkey)]) -> Result<(), HotkeyError> {
    Ok(())
}

/// Release every combination [`register_all`] bound. Idempotent; must run on
/// the registering thread.
#[cfg(windows)]
pub fn unregister_all() {
    use windows::Win32::UI::Input::KeyboardAndMouse::UnregisterHotKey;
    let mask = REGISTERED.swap(0, std::sync::atomic::Ordering::AcqRel);
    for action in HotkeyAction::ALL {
        if mask & bit(action) != 0 {
            // SAFETY: this thread registered the id in `register_all`.
            let _ = unsafe { UnregisterHotKey(None, action.id()) };
        }
    }
}

/// Non-Windows stub.
#[cfg(not(windows))]
pub fn unregister_all() {}

/// Record a `WM_HOTKEY` id for the app loop. Called by `pump`; never blocks.
#[cfg(windows)]
pub(crate) fn note_pending(id: i32) {
    if let Some(action) = HotkeyAction::from_id(id) {
        PENDING.fetch_or(bit(action), std::sync::atomic::Ordering::AcqRel);
    } else {
        tracing::warn!("unknown WM_HOTKEY id ignored: {id}");
    }
}

/// Take one pending hotkey action, lowest id first (`None` when drained).
pub(crate) fn take_pending() -> Option<HotkeyAction> {
    use std::sync::atomic::Ordering;
    loop {
        let mask = PENDING.load(Ordering::Acquire);
        if mask == 0 {
            return None;
        }
        let lowest = mask & mask.wrapping_neg();
        if PENDING
            .compare_exchange(mask, mask & !lowest, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return HotkeyAction::from_id((lowest.trailing_zeros() + 1) as i32);
        }
    }
}

#[cfg(test)]
mod tests;
