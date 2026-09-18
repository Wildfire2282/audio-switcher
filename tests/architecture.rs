//! Architecture rules that used to live only in prose.
//!
//! Each of these is cheap to violate and expensive to notice: a stray Win32 call
//! in `ui` compiles, ships, and only a later refactor pays for the broken
//! layering. Reading the source to check a rule is the cost these tests remove —
//! the gate answers instead.

use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Concatenated source of `src/<relative>` (a file or a directory of files),
/// with line comments stripped so a doc mention of a banned word is not a
/// violation.
fn stripped_source(relative: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join(relative);
    let label = root.display().to_string();
    let mut files = Vec::new();
    if root.is_file() {
        files.push(root);
    } else {
        rust_files(&root, &mut files);
    }
    assert!(!files.is_empty(), "no Rust sources at {label}");

    files
        .iter()
        .map(|path| fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
        .flat_map(|text| {
            text.lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Layering (see the crate docs): Win32 lives in `platform` only. `ui` and `app`
/// stay portable, and that is what keeps `cargo test` runnable everywhere.
#[test]
fn ui_and_app_never_touch_win32() {
    for layer in ["ui", "app"] {
        let src = stripped_source(layer);
        for banned in ["windows::", "unsafe", "std::ffi::c_void"] {
            assert!(
                !src.contains(banned),
                "`{layer}` contains `{banned}`: Win32 belongs in `platform`, \
                 so route the call through a wrapper there instead"
            );
        }
    }
}

/// The tray icon must stay tooltip-free: the shell draws a tooltip exactly where
/// the volume overlay appears and covers it. This is the rule that keeps coming
/// back, because a tooltip is the obvious thing to add.
#[test]
fn tray_carries_no_tooltip() {
    let src = stripped_source("ui/tray.rs");
    for banned in ["with_tooltip", "update_tooltip", "set_tooltip", "NIF_TIP"] {
        assert!(
            !src.contains(banned),
            "`ui/tray.rs` uses `{banned}`: the overlay is the only read-out, \
             and a tooltip would cover it"
        );
    }
}

/// The `windows` crate feature list is the release size budget's main lever, so
/// it may only change deliberately — and the change has to be recorded here.
#[test]
fn windows_features_are_the_approved_set() {
    const APPROVED: &[&str] = &[
        "Win32_Foundation",
        "Win32_UI_WindowsAndMessaging",
        "Win32_UI_Shell",
        "Win32_Media_Audio_Endpoints",
        "Win32_UI_Shell_PropertiesSystem",
        "Win32_System_Com_StructuredStorage",
        "Win32_System_Variant",
        "Win32_Media_Audio",
        "Win32_Devices_FunctionDiscovery",
        "Win32_System_Registry",
        "Win32_Graphics_Gdi",
        "Win32_System_SystemInformation",
        "Win32_Globalization",
        "Win32_UI_Input_KeyboardAndMouse",
        "Win32_System_LibraryLoader",
    ];

    let manifest = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("Cargo.toml is readable");
    let block = manifest
        .split_once("[dependencies.windows]")
        .expect("Cargo.toml has a [dependencies.windows] table")
        .1;
    let list = block
        .split_once("features = [")
        .expect("the windows table lists features")
        .1
        .split_once(']')
        .expect("the feature list is closed")
        .0;
    let features: Vec<&str> = list
        .lines()
        .filter_map(|line| line.trim().split('"').nth(1))
        .collect();

    assert_eq!(
        features, APPROVED,
        "the windows feature set changed: justify it, re-measure the release \
         size, then update this list in the same commit"
    );
}

/// Unit tests for a module live beside it, not inside it. A production read must
/// not drag a test corpus along, and the sibling file still reaches every
/// private item through `use super::*`.
#[test]
fn unit_tests_live_beside_the_module_not_inside_it() {
    const LIMIT: usize = 50;

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root, &mut files);

    for path in files {
        let text = fs::read_to_string(&path).expect("readable source");
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if !line.starts_with("#[cfg(") || !line.contains("test") {
                continue;
            }
            let Some(next) = lines.get(index + 1) else {
                continue;
            };
            let next = next.trim();
            // `mod tests {` is an inline module; `mod tests;` is the sibling-file form.
            if !next.starts_with("mod ") || !next.ends_with('{') {
                continue;
            }
            let block = lines.len() - index;
            assert!(
                block <= LIMIT,
                "{}: an inline test module of ~{block} lines (limit {LIMIT}): move it \
                 to `<dir>/tests.rs` and declare `mod tests;` so a production read stays short",
                path.display()
            );
        }
    }
}

/// `AGENTS.md` is injected into every session, so its size is a *fixed* tax paid
/// before any work starts — the one document whose cost is not opt-in. Keep it a
/// budget: raise the number deliberately or trim the text, never let it creep.
#[test]
fn agent_doc_stays_within_its_token_budget() {
    const BUDGET: usize = 6_500;

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("AGENTS.md");
    let Ok(text) = fs::read_to_string(&path) else {
        // Local-only file: a fresh clone has none, and that is not a failure.
        return;
    };
    assert!(
        text.len() <= BUDGET,
        "AGENTS.md is {} bytes (budget {BUDGET}): every session pays this before \
         reading anything, so trim a rule or move it into the crate docs",
        text.len()
    );
}

/// A file an agent cannot read in one pass is a file it will mis-edit. The limit
/// is the ceiling, not the target: today's worst file is `app/mod.rs` at ~750
/// production lines (it is cohesive — its methods share `App`'s state, so the
/// split cost would be wider field visibility). Split along a seam instead of
/// raising this, and lower the number once the worst file shrinks.
#[test]
fn no_source_file_outgrows_one_reading_pass() {
    const LIMIT: usize = 800;

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root, &mut files);

    for path in files {
        let text = fs::read_to_string(&path).expect("readable source");
        let production = text
            .lines()
            .take_while(|line| {
                !line.trim_start().starts_with("#[cfg(test)]")
                    && !line.trim_start().starts_with("#[cfg(all(test")
            })
            .count();
        assert!(
            production <= LIMIT,
            "{} is {production} production lines (limit {LIMIT}): split it along \
             a seam so the next change only has to read that seam",
            path.display()
        );
    }
}

/// Release notes are the last rule that lived only in prose: they must be
/// bilingual mirrors of each other, and must describe user-visible behaviour
/// only. The release job checks the file exists; this checks it is usable.
#[test]
fn release_notes_are_bilingual_and_user_visible() {
    const MIRROR: &[(&str, &str)] = &[("新增", "Added"), ("修复", "Fixed"), ("变更", "Changed")];

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/release-notes");
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "md") {
            continue;
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let text = fs::read_to_string(&path).expect("readable release notes");

        let title = text.lines().next().unwrap_or_default();
        assert!(
            title.starts_with("## [") && title.contains("] - "),
            "{name}: first line must be `## [<version>] - YYYY-MM-DD`, found `{title}`"
        );

        // Bullets under one heading, or None when the heading is absent.
        let bullets = |heading: &str| -> Option<usize> {
            let mut count = None;
            let mut inside = false;
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("### ") {
                    inside = rest.trim() == heading;
                    if inside {
                        count = Some(0);
                    }
                } else if inside && line.starts_with("- ") {
                    count = Some(count.unwrap_or(0) + 1);
                }
            }
            count
        };

        for (zh, en) in MIRROR {
            match (bullets(zh), bullets(en)) {
                (None, None) => {}
                (Some(n), Some(m)) => assert_eq!(
                    n, m,
                    "{name}: `### {zh}` has {n} entries but `### {en}` has {m}"
                ),
                (Some(_), None) => panic!("{name}: `### {zh}` has no `### {en}` mirror"),
                (None, Some(_)) => panic!("{name}: `### {en}` has no `### {zh}` mirror"),
            }
        }

        for banned in ["sha256", "SHA256", "bytes", "KB"] {
            assert!(
                !text.contains(banned),
                "{name}: release notes must not mention `{banned}` - only user-visible behaviour"
            );
        }
    }
}
