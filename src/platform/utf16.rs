//! UTF-16 conversion for the Win32 string boundary.
//!
//! Mixing the two conventions is a memory-safety bug, not a cosmetic one:
//! `PCWSTR` parameters read until a NUL terminator ([`wide_z`]), so a buffer
//! without one lets the API run past the allocation; length-taking APIs
//! (`DrawTextW`) read exactly the given length ([`wide_into`]), so a terminator
//! there would be a stray glyph.

/// For `PCWSTR` parameters.
#[must_use]
pub(crate) fn wide_z(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Encode `text` for a length-taking API into `buf`, replacing its contents.
///
/// Writes into a caller-owned buffer so a repeated call (the overlay paints once
/// per wheel notch) reuses one allocation instead of making a new `Vec`.
pub(crate) fn wide_into(buf: &mut Vec<u16>, text: &str) {
    buf.clear();
    buf.extend(text.encode_utf16());
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
    fn wide_into_replaces_the_buffer_without_a_terminator() {
        let mut buf = vec![0xFFFF, 0xFFFF];
        wide_into(&mut buf, "ab");
        assert_eq!(buf, [0x61, 0x62]);
        wide_into(&mut buf, "");
        assert!(buf.is_empty());
    }

    #[test]
    fn astral_planes_survive_as_surrogate_pairs() {
        // Emoji are two code units; the terminator still lands last.
        let encoded = wide_z("🎧");
        assert_eq!(encoded.len(), 3);
        assert_eq!(*encoded.last().unwrap(), 0);
    }
}
