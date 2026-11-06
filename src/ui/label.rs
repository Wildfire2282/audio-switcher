//! Truncation and sanitization shared by the menu and the overlay card, so the
//! two budgets cannot drift apart. Device names come from WASAPI and may carry
//! control characters or be too long for either.

/// Maximum label width in characters (menu budget, shared with the overlay so
/// truncation cannot drift apart).
pub(crate) const MAX_LABEL_CHARS: usize = 64;

/// Replace every control character with a plain space for UI display.
///
/// `\r`, `\n` and `\t` are the common ones, but a device name can carry any C0
/// or C1 byte; GDI would draw those as stray glyphs or drop the rest of the
/// line.
#[must_use]
pub(crate) fn sanitize_label(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
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
mod tests;
