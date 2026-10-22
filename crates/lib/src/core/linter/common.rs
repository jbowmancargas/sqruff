use std::borrow::Cow;
use std::ops::Range;

use sqruff_lib_core::errors::{SQLBaseError, SQLTemplaterError};
use sqruff_lib_core::parser::segments::ErasedSegment;
use sqruff_lib_core::templaters::TemplatedFile;

use crate::core::config::FluffConfig;

/// An object to store the result of a templated file/string.
///
/// This is notable as it's the intermediate state between what happens
/// in the main process and the child processes when running in parallel mode.
#[derive(Debug, Clone)]
pub struct RenderedFile {
    pub templated_file: TemplatedFile,
    pub alternate_templated_files: Vec<TemplatedFile>,
    pub templater_violations: Vec<SQLTemplaterError>,
    pub(crate) filename: String,
    /// The newline-normalised source that was templated and parsed.
    pub source_str: String,
    /// Maps offsets in `source_str` back to the input before normalisation.
    pub newline_map: NewlineMap,
}

#[derive(Debug, Clone)]
pub struct ParsedVariant {
    pub tree: Option<ErasedSegment>,
    pub violations: Vec<SQLBaseError>,
    pub templated_file: TemplatedFile,
}

/// Result of batch rendering: either a rendered file or a skipped file.
pub enum BatchRenderedResult {
    Rendered(RenderedFile),
    Skipped { filename: String, reason: String },
}

/// An object to store the result of parsing a string.
#[derive(Debug, Clone)]
pub struct ParsedString {
    pub tree: Option<ErasedSegment>,
    pub violations: Vec<SQLBaseError>,
    pub templated_file: TemplatedFile,
    pub filename: String,
    /// The newline-normalised source that was templated and parsed.
    pub source_str: String,
    /// Maps offsets in `source_str` back to the input before normalisation.
    pub newline_map: NewlineMap,
    pub config: FluffConfig,
    pub alternate_variants: Vec<ParsedVariant>,
}

const BOM: char = '\u{feff}';

/// Maps byte offsets in the newline-normalised source back to the original input.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NewlineMap {
    /// Offsets, in the normalised text, of each `\n` that replaced a `\r\n`.
    collapsed_crlf: Vec<usize>,
    /// The input started with a UTF-8 byte order mark, which was removed.
    bom: bool,
    /// Every newline in the input was `\r\n`.
    all_crlf: bool,
}

impl NewlineMap {
    /// Normalise newlines to unix-style line endings, recording where `\r\n`
    /// sequences were collapsed so offsets can later be mapped back.
    ///
    /// A leading UTF-8 byte order mark is also removed (as SQLFluff does when
    /// reading files), since the lexer can't make sense of it.
    pub fn normalise(source: &str) -> (Cow<'_, str>, Self) {
        let (source, bom) = match source.strip_prefix(BOM) {
            Some(rest) => (rest, true),
            None => (source, false),
        };
        if !source.contains('\r') {
            return (
                Cow::Borrowed(source),
                Self {
                    bom,
                    ..Self::default()
                },
            );
        }

        let mut normalised = String::with_capacity(source.len());
        let mut collapsed_crlf = Vec::new();
        let mut rest = source;
        while let Some(idx) = rest.find('\r') {
            normalised.push_str(&rest[..idx]);
            if rest.as_bytes().get(idx + 1) == Some(&b'\n') {
                collapsed_crlf.push(normalised.len());
                rest = &rest[idx + 2..];
            } else {
                rest = &rest[idx + 1..];
            }
            normalised.push('\n');
        }
        normalised.push_str(rest);

        let all_crlf = collapsed_crlf.len() == normalised.matches('\n').count();
        (
            Cow::Owned(normalised),
            Self {
                collapsed_crlf,
                bom,
                all_crlf,
            },
        )
    }

    /// Put back what [`NewlineMap::normalise`] took out of a whole fixed file:
    /// the byte order mark, and `\r\n` endings when the input used only those.
    pub fn restore(&self, fixed: String) -> String {
        let fixed = if self.all_crlf {
            fixed.replace('\n', "\r\n")
        } else {
            fixed
        };
        if self.bom {
            let mut with_bom = String::with_capacity(fixed.len() + BOM.len_utf8());
            with_bom.push(BOM);
            with_bom.push_str(&fixed);
            with_bom
        } else {
            fixed
        }
    }

    /// Translate a byte offset in the normalised text to the original input.
    pub fn to_original_offset(&self, offset: usize) -> usize {
        let bom_len = if self.bom { BOM.len_utf8() } else { 0 };
        bom_len
            + offset
            + self
                .collapsed_crlf
                .partition_point(|&collapsed| collapsed < offset)
    }

    /// Translate a byte range in the normalised text to the original input.
    pub fn to_original_range(&self, range: Range<usize>) -> Range<usize> {
        self.to_original_offset(range.start)..self.to_original_offset(range.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newline_map_normalises_and_maps_offsets_back() {
        let source = "SELECT\r\n foo\n FROM \r \n\r bar;\r\n";
        let (normalised, map) = NewlineMap::normalise(source);
        assert_eq!(normalised, "SELECT\n foo\n FROM \n \n\n bar;\n");

        // Every byte before the first `\r\n` is unchanged.
        assert_eq!(map.to_original_offset(0), 0);
        assert_eq!(map.to_original_offset(6), 6);
        // Just past the collapsed newline skips both original bytes.
        assert_eq!(map.to_original_offset(7), 8);
        // Lone `\r` keeps its width, so the shift stays at one byte until the
        // trailing `\r\n`.
        assert_eq!(&source[map.to_original_range(18..22)], "\r \n\r");
        assert_eq!(&source[map.to_original_range(23..27)], "bar;");
        assert_eq!(map.to_original_offset(normalised.len()), source.len());

        // Ranges covering a collapsed newline cover the whole `\r\n`.
        assert_eq!(&source[map.to_original_range(6..7)], "\r\n");
        assert_eq!(&source[map.to_original_range(27..28)], "\r\n");
    }

    #[test]
    fn newline_map_borrows_when_nothing_to_normalise() {
        let (normalised, map) = NewlineMap::normalise("SELECT 1\n");
        assert!(matches!(normalised, Cow::Borrowed(_)));
        assert_eq!(map, NewlineMap::default());
        assert_eq!(map.to_original_range(3..9), 3..9);
        assert_eq!(map.restore("select 1\n".into()), "select 1\n");
    }

    #[test]
    fn newline_map_strips_and_restores_bom() {
        let source = "\u{feff}SELECT 1\n";
        let (normalised, map) = NewlineMap::normalise(source);
        assert_eq!(normalised, "SELECT 1\n");
        assert_eq!(&source[map.to_original_range(0..6)], "SELECT");
        assert_eq!(map.restore("select 1\n".into()), "\u{feff}select 1\n");
    }

    #[test]
    fn newline_map_restores_crlf_only_when_consistent() {
        let (_, map) = NewlineMap::normalise("\u{feff}SELECT\r\n1\r\n");
        assert_eq!(map.restore("select\n1\n".into()), "\u{feff}select\r\n1\r\n");

        // Mixed endings stay normalised, as before.
        let (_, map) = NewlineMap::normalise("SELECT\r\n1\n");
        assert_eq!(map.restore("select\n1\n".into()), "select\n1\n");
    }
}
