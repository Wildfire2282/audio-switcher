//! UTF-16 conversion for the Win32 string boundary.
//!
//! Mixing the two conventions is a memory-safety bug, not a cosmetic one:
//! `PCWSTR` parameters read until a NUL terminator ([`wide_z`]), so a buffer
//! without one lets the API run past the allocation; length-taking APIs
//! (`DrawTextW`) read exactly the given length ([`wide`]), so a terminator there
//! would be a stray glyph.

/// For `PCWSTR` parameters.
#[must_use]
pub(crate) fn wide_z(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// For length-taking APIs.
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
