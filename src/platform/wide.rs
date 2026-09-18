//! UTF-16 conversion for the Win32 string boundary.
//!
//! Win32 splits into two string conventions, and mixing them up is a
//! memory-safety bug rather than a cosmetic one:
//!
//! - `PCWSTR` parameters read until a NUL terminator, so a buffer without one
//!   lets the API run past the end of the allocation — [`wide_z`].
//! - Length-taking APIs (`DrawTextW`) read exactly the length given, so a
//!   terminator would be a stray glyph — [`wide`].
//!
//! Both are here so the choice is named at every call site instead of being
//! re-derived from the expression each time.

/// Encode `text` as a NUL-terminated UTF-16 buffer, for `PCWSTR` parameters.
#[must_use]
pub(crate) fn wide_z(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Encode `text` as UTF-16 without a terminator, for length-taking APIs.
#[must_use]
pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_z_appends_exactly_one_terminator() {
        assert_eq!(wide_z("ab"), [0x61, 0x62, 0x00]);
        assert_eq!(wide_z(""), [0x00]);
    }

    #[test]
    fn wide_appends_nothing() {
        assert_eq!(wide("ab"), [0x61, 0x62]);
        assert!(wide("").is_empty());
    }

    #[test]
    fn astral_planes_survive_as_surrogate_pairs() {
        // Emoji are two code units; the terminator still lands last.
        let encoded = wide_z("🎧");
        assert_eq!(encoded.len(), 3);
        assert_eq!(*encoded.last().unwrap(), 0);
    }
}
