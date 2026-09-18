//! Shared label helpers — single source of truth for menu/overlay text.
//!
//! Device names come from WASAPI and may contain control characters or be
//! excessively long for menus (muda) and the overlay card. Both
//! [`crate::ui::menu`] and [`crate::ui::osd`] funnel through here so truncation
//! width and sanitization cannot drift apart.

/// Maximum label width in characters (menu budget, shared with the overlay so
/// truncation cannot drift apart).
pub(crate) const MAX_LABEL_CHARS: usize = 64;

/// Replace control whitespace with a plain space for UI display.
#[must_use]
pub(crate) fn sanitize_label(name: &str) -> String {
    name.replace(['\r', '\n', '\t'], " ")
}

/// Truncate `raw` to `max_chars` characters, sanitizing first.
///
/// Short inputs are returned unchanged (single pass, no double counting).
#[must_use]
pub(crate) fn truncate_label(raw: &str, max_chars: usize) -> String {
    debug_assert!(max_chars >= 2, "max_chars must leave room for ellipsis");
    let sanitized = sanitize_label(raw);
    if sanitized.chars().count() <= max_chars {
        return sanitized;
    }
    let kept: String = sanitized
        .chars()
        .take(max_chars.saturating_sub(2))
        .collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_passthrough() {
        assert_eq!(truncate_label("Speaker", MAX_LABEL_CHARS), "Speaker");
    }

    #[test]
    fn sanitizes_control_chars() {
        assert_eq!(truncate_label("A\nB\rC\tD", MAX_LABEL_CHARS), "A B C D");
    }

    #[test]
    fn truncates_long() {
        let long = "A".repeat(100);
        let out = truncate_label(&long, MAX_LABEL_CHARS);
        assert_eq!(out.chars().count(), MAX_LABEL_CHARS - 1);
        assert!(out.ends_with('…'));
    }
}
