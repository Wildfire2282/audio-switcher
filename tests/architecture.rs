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

/// Layering (see the crate docs): Win32 lives in `platform` and `audio::wasapi`
/// only. `ui` and `app` stay portable, and that is what keeps `cargo test`
/// runnable everywhere.
#[test]
fn ui_and_app_never_touch_win32() {
    for layer in ["ui", "app"] {
        let src = stripped_source(layer);
        for banned in ["windows::", "unsafe", "std::ffi::c_void"] {
            assert!(
                !src.contains(banned),
                "`{layer}` contains `{banned}`: Win32 belongs in `platform`, so route \
                 the call through a wrapper there instead"
            );
        }
    }
}

/// The tray icon must stay tooltip-free: the shell draws a tooltip exactly where
/// the volume overlay appears and covers it. This is the rule that keeps coming
/// back, because a tooltip is the obvious thing to add.
#[test]
fn tray_carries_no_tooltip() {
    // The whole crate, not just `ui/tray.rs`: `TrayWrapper::tray` is a public
    // field of a public type, so any layer can reach `set_tooltip` without a
    // banned word ever appearing in the wrapper that owns the icon.
    let src = stripped_source(".");
    for banned in [
        "with_tooltip",
        "update_tooltip",
        "set_tooltip",
        "tooltip",
        "NIF_TIP",
    ] {
        assert!(
            !src.contains(banned),
            "`src` uses `{banned}`: the overlay is the only read-out, \
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
        for gate in 0..text.lines().count() {
            let Some(block) = inline_test_module_span(&text, gate) else {
                continue;
            };
            assert!(
                block <= LIMIT,
                "{}: an inline test module of {block} lines (limit {LIMIT}): move it \
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
fn agent_doc_stays_within_its_budget() {
    const BUDGET: usize = 6_500;

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("AGENTS.md");
    // Tracked, so a clone always has it: an absent file is a broken checkout or
    // a deleted rule book, and passing here would disable the budget with it.
    let text = fs::read_to_string(&path).expect("AGENTS.md is tracked and readable");
    assert!(
        text.len() <= BUDGET,
        "AGENTS.md is {} bytes (budget {BUDGET}): every session pays this before \
         reading anything, so trim a rule or move it into the crate docs",
        text.len()
    );
}

/// Whether `line` is the attribute that gates the item after it on tests.
fn is_test_gate(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("#[cfg(test)]") || line.starts_with("#[cfg(all(test")
}

/// Net `{` minus `}` per line, counting only braces in real code: string, raw
/// string and char literals and comments contribute nothing.
///
/// A literal's braces would otherwise keep the depth from returning to where a
/// skipped item opened — the skip then runs past its end and every production
/// line below it goes uncounted, the same silent undercount the per-item skip
/// exists to prevent.
fn code_depth_deltas(text: &str) -> Vec<i32> {
    enum State {
        Code,
        LineComment,
        BlockComment(u32),
        Str,
        RawStr(u32),
        Char,
    }
    let chars: Vec<char> = text.chars().collect();
    let mut deltas = Vec::new();
    let mut delta = 0i32;
    let mut state = State::Code;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            deltas.push(delta);
            delta = 0;
            if matches!(state, State::LineComment) {
                state = State::Code;
            }
            i += 1;
            continue;
        }
        match state {
            State::Code => match c {
                '/' if chars.get(i + 1) == Some(&'/') => {
                    state = State::LineComment;
                    i += 1;
                }
                '/' if chars.get(i + 1) == Some(&'*') => {
                    state = State::BlockComment(1);
                    i += 1;
                }
                '"' => state = State::Str,
                // Raw strings end at `"` + as many `#` as they opened with, so
                // an inner quote cannot close them. `r#ident` (raw identifier)
                // has no quote and falls through as code.
                'r' if matches!(chars.get(i + 1), Some(&'"') | Some(&'#')) => {
                    let hashes = (1..).take_while(|k| chars.get(i + k) == Some(&'#')).count();
                    if chars.get(i + 1 + hashes) == Some(&'"') {
                        state = State::RawStr(hashes as u32);
                        i += 1 + hashes;
                    }
                }
                '\'' => match chars.get(i + 1) {
                    // `'\n'`, `'\''`, `'\u{7b}'`: braces inside are literal.
                    Some(&'\\') => state = State::Char,
                    // One plain character `'{'`; anything else is a lifetime.
                    Some(_) if chars.get(i + 2) == Some(&'\'') => i += 2,
                    _ => {}
                },
                '{' => delta += 1,
                '}' => delta -= 1,
                _ => {}
            },
            State::LineComment => {}
            State::BlockComment(nesting) => {
                if c == '*' && chars.get(i + 1) == Some(&'/') {
                    state = if nesting == 1 {
                        State::Code
                    } else {
                        State::BlockComment(nesting - 1)
                    };
                    i += 1;
                } else if c == '/' && chars.get(i + 1) == Some(&'*') {
                    state = State::BlockComment(nesting + 1);
                    i += 1;
                }
            }
            // `\` escapes the next character — but not a line break, whose
            // newline still has to be counted above.
            State::Str => {
                if c == '\\' {
                    i += usize::from(chars.get(i + 1).is_some_and(|n| *n != '\n'));
                } else if c == '"' {
                    state = State::Code;
                }
            }
            State::RawStr(hashes) => {
                if c == '"' && (0..hashes).all(|k| chars.get(i + 1 + k as usize) == Some(&'#')) {
                    state = State::Code;
                    i += hashes as usize;
                }
            }
            State::Char => {
                if c == '\\' {
                    i += usize::from(chars.get(i + 1).is_some_and(|n| *n != '\n'));
                } else if c == '\'' {
                    state = State::Code;
                }
            }
        }
        i += 1;
    }
    if !text.is_empty() && !text.ends_with('\n') {
        deltas.push(delta);
    }
    deltas
}

/// Lines a production read has to get through: everything except the items
/// gated on `#[cfg(test)]`.
///
/// Each gated item has to be skipped on its own, because a gated item is not
/// only the trailing test module: `config` and `audio` each keep a test-only
/// helper or re-export between production items (`config.rs:453`,
/// `audio/mod.rs:145`). Stopping at the first attribute — what this used to do —
/// hid every production line below it, so those two files reported roughly
/// two-thirds of their real length.
fn production_lines(text: &str) -> usize {
    let deltas = code_depth_deltas(text);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        deltas.len(),
        lines.len(),
        "the brace scan lost or invented lines"
    );
    let mut count = 0;
    let mut depth = 0i32;
    // `Some(depth)` while inside a skipped item: the depth it opened at.
    let mut skipping: Option<i32> = None;
    // Set by a `#[cfg(test)]` attribute, cleared by the item it gates.
    let mut gated = false;
    for (line, delta) in lines.iter().zip(deltas) {
        let before = depth;
        depth += delta;
        if let Some(open) = skipping {
            // A skipped item ends where its braces close; `open` is the depth it
            // opened at, so the closing brace is the first line below it.
            if depth < open {
                skipping = None;
                gated = false;
            }
            continue;
        }
        if gated {
            if line.contains('{') {
                // A block item runs to its closing brace — unless it opened and
                // closed on this same line.
                skipping = (depth > before).then_some(depth);
                gated = skipping.is_some();
            } else if line.trim_end().ends_with(';') {
                // A declaration item (`mod tests;`, `pub use mock::MockBackend;`).
                gated = false;
            }
            continue;
        }
        if is_test_gate(line) {
            gated = true;
            continue;
        }
        count += 1;
    }
    count
}

/// A file an agent cannot read in one pass is a file it will mis-edit. The limit
/// is the ceiling, not the target: split along a seam instead of raising it, and
/// lower the number once the worst file shrinks.
#[test]
fn no_source_file_outgrows_one_reading_pass() {
    const LIMIT: usize = 800;

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root, &mut files);

    for path in files {
        let text = fs::read_to_string(&path).expect("readable source");
        let production = production_lines(&text);
        assert!(
            production <= LIMIT,
            "{} is {production} production lines (limit {LIMIT}): split it along \
             a seam so the next change only has to read that seam",
            path.display()
        );
    }
}

/// The counting rule above decides whether the size ceiling is enforced at all,
/// so its own shapes are pinned: a declaration item, an inline module, a gated
/// helper between two production items, and a brace inside a gated body that
/// must not end the skip early.
#[test]
fn production_lines_skips_every_gated_item() {
    let source = "\
fn before() {}
#[cfg(test)]
mod tests;
#[cfg(test)]
pub use mock::MockBackend;
#[cfg(test)]
fn helper() {
    let json = \"{}\";
    assert_eq!(json, \"{}\");
    let open = '{';
    let close = \"}\";
    let raw = r#\"{\"#;
    let text = \"{\";
    /* a { that never closes */
}
fn after() {}
#[cfg(test)]
mod inline {
    fn inner() {}
}
";
    assert_eq!(production_lines(source), 2, "lines: {source}");
}

/// Lines of the inline `mod ... {` a `#[cfg(test)]` gate opens at `gate`
/// (the gate line included), or `None` when that line gates no inline module.
///
/// Attributes between the gate and the item belong to it, an indented gate is
/// still a gate, and the span runs to the module's own closing brace: measuring
/// to end of file would charge a mid-file module every line below it.
fn inline_test_module_span(text: &str, gate: usize) -> Option<usize> {
    let lines: Vec<&str> = text.lines().collect();
    if !is_test_gate(lines.get(gate)?) {
        return None;
    }
    let mut item = gate + 1;
    while let Some(line) = lines.get(item) {
        let line = line.trim();
        if line.is_empty() || line.starts_with("#[") {
            item += 1;
        } else {
            break;
        }
    }
    let declaration = lines.get(item)?.trim();
    // `mod tests {` is an inline module; `mod tests;` is the sibling-file form.
    if !declaration.starts_with("mod ") || !declaration.ends_with('{') {
        return None;
    }
    let deltas = code_depth_deltas(text);
    let mut depth = 0i32;
    for (at, delta) in deltas.iter().enumerate().skip(item) {
        depth += delta;
        if depth <= 0 {
            return Some(at - gate + 1);
        }
    }
    None
}

/// The gate shapes `inline_test_module_span` has to recognise: an indented
/// gate, attributes between the gate and the item, and a module that is not the
/// last item in the file.
#[test]
fn inline_test_module_span_sees_every_gate_shape() {
    let source = "\
fn before() {}
mod outer {
    #[cfg(test)]
    #[allow(dead_code)]
    mod tests {
        fn inner() {}
        fn more() {}
    }
    fn middle() {}
}
fn after() {}
";
    // Gate at line 2 through the module's own closing brace at line 7.
    assert_eq!(inline_test_module_span(source, 2), Some(6), "{source}");
    assert_eq!(inline_test_module_span(source, 0), None, "not a gate");
    assert_eq!(inline_test_module_span(source, 5), None, "not a gate");
}

/// Release notes are the last rule that lived only in prose: they must be
/// bilingual mirrors of each other, and must describe user-visible behaviour
/// only. The release job checks the file exists; this checks it is usable.
#[test]
fn release_notes_are_bilingual_and_user_visible() {
    const MIRROR: &[(&str, &str)] = &[("新增", "Added"), ("修复", "Fixed"), ("变更", "Changed")];

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/release-notes");
    // Tracked with the notes it holds: a missing directory must not quietly
    // switch the bilingual-mirror rule off.
    let entries = fs::read_dir(&dir).expect("the release-notes directory is tracked");

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

        // Every heading is one of the mirrored pairs: a fourth section type has
        // no mirror rule and could ship half-translated.
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("### ") {
                let heading = rest.trim();
                assert!(
                    MIRROR
                        .iter()
                        .any(|(zh, en)| heading == *zh || heading == *en),
                    "{name}: `### {heading}` is none of the mirrored sections"
                );
            }
        }

        // No hashes and no artefact sizes, in any spelling: the notes describe
        // behaviour, never the file that carries it. Units need word boundaries
        // (`remembered` contains `mb`) while still firing after a digit (`2MB`);
        // the CJK spellings have no word boundaries at all.
        let lowered = text.to_lowercase();
        for word in [
            "sha256", "sha-256", "hash", "byte", "bytes", "kb", "kib", "mb", "mib", "gb",
        ] {
            assert!(
                !has_token(&lowered, word),
                "{name}: release notes must not mention `{word}` - only user-visible behaviour"
            );
        }
        for phrase in ["哈希", "散列", "字节", "体积"] {
            assert!(
                !lowered.contains(phrase),
                "{name}: release notes must not mention `{phrase}` - only user-visible behaviour"
            );
        }
    }
}

/// Whether `text` contains `word` as a standalone token: only a letter before
/// it blocks the match, so `2MB` is banned while `remembered` is not.
fn has_token(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        before.is_none_or(|c| !c.is_alphabetic()) && after.is_none_or(|c| !c.is_alphanumeric())
    })
}
