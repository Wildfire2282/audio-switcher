//! Application configuration persistence.
//!
//! `AppConfig` is stored as JSON at `%APPDATA%\audio-switcher\config.json`
//! (kebab-case dir from `TOOL_ID`). Lookup chain: `%APPDATA%` → then
//! `%LOCALAPPDATA%` → then temp (degraded: in-memory load still works, every
//! save logs a warning). The `./config.json` fallback is banned (`Program
//! Files` is not writable). Unknown fields are rejected so config typos fail
//! loudly (backup + reset to defaults).

use serde::{Deserialize, Deserializer, Serialize};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::platform::autostart::AutostartMode;
use crate::platform::hotkey::{Hotkey, HotkeyAction};

/// Current config schema version. v2 migrates the v1 `Zh` default to `System`
/// (v1 could not distinguish an explicit `zh` choice from the old default, so
/// explicit `zh` users re-pick once). v3 adds the opt-in `hotkeys` object (absent
/// in older files → every hotkey off). v4 replaces the `autostart` boolean with
/// the three-way `autostart_mode`.
const CURRENT_VERSION: u32 = 4;

/// Legacy (v1, PascalCase) config filename for one-time import.
const LEGACY_DIR_NAME: &str = "AudioSwitcher";

/// Cached config path — computed once per process.
static CONFIG_PATH_CACHE: LazyLock<(PathBuf, bool)> = LazyLock::new(resolve_config_path);

/// UI language. `System` (the default) follows the OS locale once at startup;
/// live re-resolution is deferred (no locale listener is installed).
///
/// Variant names are the documentation; `Zh` is Simplified only.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    System,
    Zh,
    En,
}

impl Lang {
    /// Whether this is Chinese.
    #[must_use]
    pub fn is_zh(self) -> bool {
        self == Self::Zh
    }

    /// Map a locale name (`"zh-CN"`, `"en-US"`) to a language. Pure and
    /// branch-tested. Traditional-Chinese locales fall back to English
    /// (untranslated); everything else is English.
    #[must_use]
    pub fn for_locale_name(name: &str) -> Self {
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("zh") {
            let traditional = lower.contains("hant")
                || ["-hk", "-tw", "-mo", "_hk", "_tw", "_mo"]
                    .iter()
                    .any(|s| lower.contains(s));
            if traditional {
                return Self::En;
            }
            return Self::Zh;
        }
        Self::En
    }

    /// Resolve the system locale once. Read failure falls back to English
    /// with a warning (never "failure means Chinese").
    #[must_use]
    pub fn system() -> Self {
        let name = crate::platform::locale::system_locale_name();
        if name.is_empty() {
            tracing::warn!("system locale unreadable, falling back to English");
            return Self::En;
        }
        Self::for_locale_name(&name)
    }
}

fn deserialize_volume_limit<'de, D>(de: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error as DeError;
    let v = serde_json::Value::deserialize(de)?;
    match v {
        serde_json::Value::Number(n) => n
            .as_u64()
            .and_then(|x| u32::try_from(x).ok())
            .ok_or_else(|| DeError::custom("invalid volume_limit")),
        serde_json::Value::String(s) => s
            .trim()
            .parse::<u32>()
            .map_err(|_| DeError::custom("invalid volume_limit")),
        _ => Err(DeError::custom("invalid volume_limit")),
    }
}

/// Global-hotkey opt-ins, one combination string per [`HotkeyAction`].
///
/// Combinations are stored in canonical form (`"Ctrl+Alt+M"`) so `config.json`
/// stays human-editable; `null`/absent means "no hotkey". [`AppConfig::migrate`]
/// canonicalizes what it can read and drops an unparsable value with a warning
/// (a bad combo never reaches `RegisterHotKey`).
///
/// # Examples
///
/// ```
/// use audio_switcher::config::Hotkeys;
/// assert_eq!(Hotkeys::default().mute, None);
/// ```
// Field names are the documentation; `get`/`set` map them to actions.
#[allow(missing_docs)]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Hotkeys {
    pub mute: Option<String>,
    pub volume_up: Option<String>,
    pub volume_down: Option<String>,
    pub next_device: Option<String>,
    pub prev_device: Option<String>,
}

impl Hotkeys {
    /// Combination bound to `action`, if the action is switched on.
    #[must_use]
    pub fn get(&self, action: HotkeyAction) -> Option<&str> {
        match action {
            HotkeyAction::Mute => self.mute.as_deref(),
            HotkeyAction::VolumeUp => self.volume_up.as_deref(),
            HotkeyAction::VolumeDown => self.volume_down.as_deref(),
            HotkeyAction::NextDevice => self.next_device.as_deref(),
            HotkeyAction::PrevDevice => self.prev_device.as_deref(),
        }
    }

    /// Bind (`Some`) or clear (`None`) `action`'s combination.
    pub fn set(&mut self, action: HotkeyAction, combo: Option<String>) {
        let slot = match action {
            HotkeyAction::Mute => &mut self.mute,
            HotkeyAction::VolumeUp => &mut self.volume_up,
            HotkeyAction::VolumeDown => &mut self.volume_down,
            HotkeyAction::NextDevice => &mut self.next_device,
            HotkeyAction::PrevDevice => &mut self.prev_device,
        };
        *slot = combo;
    }

    /// Canonicalize every slot: trim, drop empties, reject unparsable combos.
    fn normalize(&mut self) {
        for action in HotkeyAction::ALL {
            let Some(raw) = self.get(action).map(str::to_owned) else {
                continue;
            };
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                self.set(action, None);
                continue;
            }
            match trimmed.parse::<Hotkey>() {
                Ok(hotkey) => self.set(action, Some(hotkey.to_string())),
                Err(e) => {
                    tracing::warn!(
                        "hotkey for {} ignored ({e}); value {raw:?} is not a supported combination",
                        action.config_key()
                    );
                    self.set(action, None);
                }
            }
        }
    }
}

/// Persisted application configuration.
///
/// Unknown fields are rejected (`deny_unknown_fields`): a typo must reset
/// loudly (backup + defaults), never be silently ignored.
///
/// # Examples
///
/// ```
/// use audio_switcher::config::{AppConfig, Lang};
/// let cfg = AppConfig::default();
/// assert_eq!(cfg.lang, Lang::System);
/// assert_eq!(cfg.volume_limit, 25);
/// ```
// Only fields whose meaning is not already in their name carry a doc line.
#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    /// Schema version driving `migrate`.
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default = "default_lang")]
    pub lang: Lang,
    #[serde(default = "default_volume_limit_enabled")]
    pub volume_limit_enabled: bool,
    /// `1..=100`.
    #[serde(
        default = "default_volume_limit",
        deserialize_with = "deserialize_volume_limit",
        alias = "volumeLimit",
        alias = "VolumeLimit"
    )]
    pub volume_limit: u32,
    #[serde(default = "default_autostart_mode")]
    pub autostart_mode: AutostartMode,
    /// Read only to migrate a v3 file into [`Self::autostart_mode`]; never
    /// written back.
    #[serde(default, skip_serializing, rename = "autostart")]
    pub legacy_autostart: Option<bool>,
    /// One combination per action; all off unless set.
    #[serde(default)]
    pub hotkeys: Hotkeys,
}

fn default_version() -> u32 {
    CURRENT_VERSION
}
fn default_lang() -> Lang {
    Lang::System
}
fn default_volume_limit_enabled() -> bool {
    true
}
fn default_volume_limit() -> u32 {
    25
}
fn default_autostart_mode() -> AutostartMode {
    AutostartMode::User
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            lang: default_lang(),
            volume_limit_enabled: default_volume_limit_enabled(),
            volume_limit: default_volume_limit(),
            autostart_mode: default_autostart_mode(),
            legacy_autostart: None,
            hotkeys: Hotkeys::default(),
        }
    }
}

/// Bilingual header written above the JSON body on every save.
///
/// `config.json` is JSONC: `//` line comments and `/* */` blocks are stripped
/// on load, so users can keep notes. All hotkeys are unbound by default;
/// editing is manual only (no menu toggles).
///
/// Takes the path this file is actually being written to: the lookup chain
/// falls back to `%LOCALAPPDATA%` and then to temp, and the Location line used
/// to name `%APPDATA%` unconditionally — pointing the user at a file that does
/// not exist on exactly the machines where they need to find it.
#[must_use]
pub fn config_comment_header(path: &Path) -> String {
    format!(
        "// AudioSwitcher config — edit, save, then restart the app to apply.\n\
    // 配置文件 — 改完保存后重启生效。\n\
    // Location / 位置: {}\n\
    // Language / 语言: \"system\" (follow OS / 跟随系统), \"zh\", \"en\".\n\
    // Volume limit / 音量上限: \"volume_limit_enabled\" true/false, \"volume_limit\" 1-100.\n\
    // Autostart / 开机自启: \"autostart_mode\" = \"off\" | \"user\" (Run value / 注册表启动) | \"admin\" (elevated logon task / 管理员权限登录任务).\n\
    //\n\
    // Hotkeys / 快捷键 (all unbound by default / 默认无绑定):\n\
    //   Each action takes a combination string; null disables it.\n\
    //   每个动作填组合字符串，null 表示关闭。\n\
    //   Format / 格式: modifiers + key, e.g. \"Ctrl+Alt+M\".\n\
    //   Modifiers / 修饰键 (at least one / 至少一个): Ctrl, Alt, Shift, Win (case-insensitive, any order / 不区分大小写，顺序不限).\n\
    //   Key / 按键: A-Z, 0-9, F1-F24, Up/Down/Left/Right, Space, Enter, etc.\n\
    //   Actions / 动作:\n\
    //     \"mute\"         Toggle mute / 静音切换\n\
    //     \"volume_up\"    Volume up 2% per press / 音量加（每次 2%）\n\
    //     \"volume_down\"  Volume down 2% per press / 音量减（每次 2%）\n\
    //     \"next_device\"  Next output device / 下一个输出设备\n\
    //     \"prev_device\"  Previous output device / 上一个输出设备\n\
    //   Example / 示例:\n\
    //     \"hotkeys\": {{ \"mute\": \"Ctrl+Alt+M\", \"volume_up\": \"Ctrl+Alt+Up\", \"volume_down\": \"Ctrl+Alt+Down\", \"next_device\": null, \"prev_device\": null }}\n\
    //   A combination owned by another program is disabled with a dialog; the rest keep working.\n\
    //   被其他程序占用的组合会弹窗并自动禁用，其余照常工作。\n",
        path.display()
    )
}

/// Strip `//` line comments and `/* */` blocks outside strings (JSONC).
///
/// `//` inside a string (e.g. a value) is preserved. Invalid UTF-8 is
/// returned unchanged so the caller still fails loudly with backup+reset.
fn strip_json_comments(bytes: &[u8]) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return bytes.to_vec();
    };
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == '/' && chars.peek() == Some(&'/') {
            for c2 in chars.by_ref() {
                if c2 == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev_star = false;
            for c2 in chars.by_ref() {
                if c2 == '\n' {
                    out.push('\n');
                } else if prev_star && c2 == '/' {
                    break;
                }
                prev_star = c2 == '*';
            }
        } else {
            out.push(c);
        }
    }
    out.into_bytes()
}

/// Resolve `(path, degraded)` from explicit roots. Pure for tests; the live
/// [`resolve_config_path`] reads the environment.
fn resolve_for(appdata: Option<&str>, localappdata: Option<&str>, tmp: &Path) -> (PathBuf, bool) {
    let under = |root: &str| {
        let dir = PathBuf::from(root);
        if dir.is_absolute() {
            Some(dir.join(crate::TOOL_ID).join("config.json"))
        } else {
            None
        }
    };
    if let Some(path) = appdata.and_then(under) {
        return (path, false);
    }
    if let Some(path) = localappdata.and_then(under) {
        return (path, false);
    }
    (tmp.join(crate::TOOL_ID).join("config.json"), true)
}

fn resolve_config_path() -> (PathBuf, bool) {
    resolve_for(
        std::env::var("APPDATA").ok().as_deref(),
        std::env::var("LOCALAPPDATA").ok().as_deref(),
        &std::env::temp_dir(),
    )
}

/// A per-call token for a sibling file name: process id, then nanoseconds.
///
/// Shared by the temporary file and the corrupt-config backup so both use one
/// format. Not a security boundary — the config folder is the user's own — but
/// it keeps two writers from choosing the same name.
fn unique_token() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{}-{nanos}", std::process::id())
}

/// Legacy v1 path (`%APPDATA%\AudioSwitcher\config.json`) for one-time import.
fn legacy_config_path() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    let dir = PathBuf::from(appdata);
    if !dir.is_absolute() {
        return None;
    }
    Some(dir.join(LEGACY_DIR_NAME).join("config.json"))
}

/// Copy the legacy file to `new_path` when `new_path` is missing. Pure over
/// explicit paths so tests can isolate it from the real `%APPDATA%`.
///
/// A legacy file the current schema cannot parse is reported and skipped, not
/// replaced by defaults: `deny_unknown_fields` makes one removed key fail the
/// whole parse, and silently overwriting the user's settings as "imported" is
/// the opposite of the loud reset `AppConfig::load_from_bytes` performs.
fn import_legacy_file(new_path: &Path, legacy_path: &Path) -> bool {
    if new_path.exists() || !legacy_path.exists() {
        return false;
    }
    let Ok(bytes) = std::fs::read(legacy_path) else {
        return false;
    };
    let cfg = match serde_json::from_slice::<AppConfig>(&strip_json_comments(&bytes)) {
        Ok(cfg) => AppConfig::migrate(cfg),
        Err(e) => {
            tracing::warn!(
                "legacy config {} ignored ({e}); starting from defaults",
                legacy_path.display()
            );
            return false;
        }
    };
    cfg.save_to(new_path).is_ok()
}

impl AppConfig {
    /// Returns the cached config file path.
    #[must_use]
    pub fn config_path() -> PathBuf {
        CONFIG_PATH_CACHE.0.clone()
    }

    /// Returns the cached config folder (parent of [`Self::config_path`]).
    /// Created on demand by [`Self::save_to`] and the open-folder menu action.
    #[must_use]
    pub fn config_dir() -> PathBuf {
        Self::config_path().parent().map_or_else(
            || std::env::temp_dir().join(crate::TOOL_ID),
            Path::to_path_buf,
        )
    }

    /// Whether the resolved path is the degraded temp fallback (saves warn).
    #[must_use]
    pub fn config_degraded() -> bool {
        CONFIG_PATH_CACHE.1
    }

    /// Effective UI language: `System` resolves the OS locale once.
    #[must_use]
    pub fn effective_lang(&self) -> Lang {
        match self.lang {
            Lang::System => Lang::system(),
            other => other,
        }
    }

    /// Test helper: config path inside a temp dir.
    #[cfg(test)]
    #[must_use]
    pub fn config_path_for(dir: &Path) -> PathBuf {
        dir.join("config.json")
    }

    /// Load config from the standard location, falling back to defaults.
    /// Imports the legacy PascalCase file once when the new path is missing.
    #[must_use]
    pub fn load() -> Self {
        let path = Self::config_path();
        if !path.exists() {
            if let Some(legacy) = legacy_config_path() {
                if import_legacy_file(&path, &legacy) {
                    tracing::debug!("imported legacy config");
                }
            }
        }
        Self::load_from(&path)
    }

    /// Load config from an explicit path with validation and migration.
    #[must_use]
    pub fn load_from(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => Self::load_from_bytes(&bytes, path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let def = Self::default();
                // Not fatal: the in-memory defaults run this session either way.
                if let Err(e) = def.save_to(path) {
                    tracing::warn!("first config write failed: {e}");
                }
                def
            }
            Err(e) => {
                // Transient IO error (e.g. permission) — don't clobber file, return in-memory default.
                // Named, because otherwise the user's edits appear to be applied
                // and are silently discarded at exit.
                tracing::warn!("config read failed ({e}); using in-memory defaults");
                Self::default()
            }
        }
    }

    /// Parse `bytes` read from `path`, migrating and validating. Unknown
    /// fields or corrupt JSON back the file up and reset to defaults (loud,
    /// never silent). Leading `//` / `/* */` comments are stripped first
    /// (see [`config_comment_header`]).
    fn load_from_bytes(bytes: &[u8], path: &Path) -> Self {
        let stripped = strip_json_comments(bytes);
        match serde_json::from_slice::<Self>(&stripped) {
            Ok(cfg) => Self::migrate(cfg),
            Err(e) => {
                tracing::warn!("config parse failed ({e}); backing up and resetting");
                Self::backup_and_reset(bytes, path)
            }
        }
    }

    /// Migrate older schemas: bump the version, reset an out-of-range limit,
    /// move the v1 `Zh` default to `System`, fold the v3 autostart boolean into
    /// `autostart_mode`, and canonicalize the hotkey combos.
    ///
    /// The limit is *reset* to the default, never clamped to the nearest bound:
    /// a value outside `1..=100` is only reachable by hand-editing, so it says
    /// nothing about which limit the user meant, and picking one for them would
    /// silently move their volume ceiling. `default_version` makes the same
    /// choice in the other direction: a file with no `version` field is parsed
    /// as the current schema and left unmigrated, because there is no older
    /// schema it can be shown to belong to.
    fn migrate(mut cfg: Self) -> Self {
        // Scope to v1: that schema could not tell an explicit `zh` choice
        // apart from its own default, so its `zh` re-picks once. From v2 on,
        // `zh` is an explicit choice and must survive migration.
        if cfg.version < 2 && cfg.lang == Lang::Zh {
            cfg.lang = Lang::System;
        }
        // v3 had a bare boolean whose default was `true`; it wins over the new
        // field's default for any file older than v4.
        if cfg.version < 4 {
            if let Some(enabled) = cfg.legacy_autostart {
                cfg.autostart_mode = if enabled {
                    AutostartMode::User
                } else {
                    AutostartMode::Off
                };
            }
        }
        cfg.legacy_autostart = None;
        cfg.version = CURRENT_VERSION;
        if !(1..=100).contains(&cfg.volume_limit) {
            // Reset, never clamped: the doc comment above says why.
            cfg.volume_limit = default_volume_limit();
        }
        cfg.hotkeys.normalize();
        cfg
    }

    /// Back up offending `bytes` next to `path`, overwrite with defaults, and return them.
    fn backup_and_reset(bytes: &[u8], path: &Path) -> Self {
        let file_name = path
            .file_name()
            .map_or_else(|| "config.json".into(), |n| n.to_string_lossy().to_string());
        let backup = path.with_file_name(format!("{file_name}.bak.{}", unique_token()));
        let def = Self::default();
        match std::fs::write(&backup, bytes) {
            Ok(()) => {
                // Reset only once the offending bytes are safe elsewhere: this
                // rewrite destroys them, and the warning above promised a backup.
                if let Err(e) = def.save_to(path) {
                    tracing::warn!("config reset write failed: {e}");
                }
            }
            Err(e) => tracing::warn!("config backup failed ({e}); leaving the file in place"),
        }
        def
    }

    /// Synchronous atomic save: write to a unique temporary file alongside the
    /// target then rename. The unique suffix avoids races between concurrent
    /// callers. Warns when writing to the degraded temp fallback.
    /// The file is JSONC: [`config_comment_header`] is written above the JSON
    /// body so users learn the manual hotkey format in place.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if directory creation, write, or rename fails.
    ///
    /// # Panics
    ///
    /// If the config cannot be serialized. `AppConfig` holds only strings,
    /// numbers and bools, so serializing a value built by this crate cannot
    /// fail; the panic marks that invariant rather than a user error.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if Self::config_degraded() {
            tracing::warn!("saving to degraded temp config path");
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).expect("AppConfig serialization never fails");
        let body = format!("{}\n{json}\n", config_comment_header(path));
        let tmp_path = {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let file_name = path
                .file_name()
                .map_or_else(|| "config.json".into(), |n| n.to_string_lossy().to_string());
            let suffix = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            path.with_file_name(format!("{file_name}.tmp.{}-{suffix}", unique_token()))
        };
        // Flushed before the rename: the rename is atomic for readers, but on
        // its own it can land with the new name and old or empty contents after
        // a power loss — the one way a settings file loses everything. Small
        // and rare (user actions only), so the fsync is worth it here.
        {
            use std::io::Write as _;
            let mut file = std::fs::File::create(&tmp_path)?;
            file.write_all(body.as_bytes())?;
            file.sync_all()?;
        }
        // On Windows rename uses MoveFileExW(REPLACE_EXISTING) and atomically replaces.
        if let Err(e) = std::fs::rename(&tmp_path, path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }
        Ok(())
    }
}

/// Clamp `volume` according to `cfg`.
/// Output is always capped to 100 to preserve invariant,
/// even if `volume_limit` is out of range via direct construction.
#[must_use]
pub fn clamp_volume(volume: u32, cfg: &AppConfig) -> u32 {
    if cfg.volume_limit_enabled {
        volume.min(cfg.volume_limit.min(100))
    } else {
        volume.min(100)
    }
}

#[cfg(test)]
mod tests;
