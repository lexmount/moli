// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Mapping codepoints to nominal glyph identifiers.

// TODO(dfrg): move this code to read-fonts so it can be shared among other
// crates.

use read_fonts::{
    FontData, FontRead, FontRef, TableProvider, TopLevelTable,
    tables::cmap::{Cmap, CmapSubtable},
    types::GlyphId,
};

/// Metadata for constructing a character map from font data.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct CharmapIndex {
    subtable_offset: u32,
    is_symbol: bool,
    is_mac_roman: bool,
}

impl CharmapIndex {
    pub(crate) fn new(font: &FontRef<'_>) -> Option<Self> {
        let cmap = font.cmap().ok()?;
        let cmap_offset = font
            .table_directory()
            .table_records()
            .iter()
            .find(|rec| rec.tag() == Cmap::TAG)
            .map(|rec| rec.offset())?;
        let (_, rec, _) = cmap.best_subtable()?;
        let subtable_offset = cmap_offset.checked_add(rec.subtable_offset().to_u32())?;
        Some(Self {
            subtable_offset,
            is_symbol: rec.is_symbol(),
            is_mac_roman: rec.is_mac_roman(),
        })
    }

    /// Creates a character map from the given font data.
    pub fn charmap<'a>(&self, font_data: &'a [u8]) -> Option<Charmap<'a>> {
        let subtable_data = font_data.get(self.subtable_offset as usize..)?;
        let subtable = CmapSubtable::read(FontData::new(subtable_data)).ok()?;
        Some(Charmap {
            subtable,
            is_symbol: self.is_symbol,
            is_mac_roman: self.is_mac_roman,
        })
    }
}

/// Mapping from Unicode codepoints to nominal glyph identifiers.
#[derive(Clone)]
pub struct Charmap<'a> {
    subtable: CmapSubtable<'a>,
    is_symbol: bool,
    is_mac_roman: bool,
}

impl Charmap<'_> {
    /// Returns the glyph identifier for the given codepoint.
    pub fn map(&self, codepoint: impl Into<u32>) -> Option<u32> {
        const ASCII_MAX: u32 = 0x7F;
        let mut c = codepoint.into();
        // The Mac Roman encoding requires special processing for codepoints
        // above the ASCII range.
        if self.is_mac_roman && c > ASCII_MAX {
            c = unicode_to_mac_roman(c);
        }
        let result = match &self.subtable {
            CmapSubtable::Format0(table) => table.map_codepoint(c),
            CmapSubtable::Format6(table) => table.map_codepoint(c),
            CmapSubtable::Format10(table) => {
                if let Some(index) = c.checked_sub(table.start_char_code()) {
                    table
                        .glyph_id_array()
                        .get(index as usize)
                        .map(|gid| GlyphId::from(gid.get()))
                } else {
                    None
                }
            }
            CmapSubtable::Format4(table) => table.map_codepoint(c),
            CmapSubtable::Format12(table) => table.map_codepoint(c),
            CmapSubtable::Format13(table) => table.map_codepoint(c),
            _ => None,
        };
        if result.is_none() && self.is_symbol && c <= 0x00FF {
            // For symbol-encoded OpenType fonts, we duplicate the
            // U+F000..F0FF range at U+0000..U+00FF.  That's what
            // Windows seems to do, and that's hinted about at:
            // https://docs.microsoft.com/en-us/typography/opentype/spec/recom
            // under "Non-Standard (Symbol) Fonts".
            return self.map(0xF000 + c);
        }
        result.map(|gid| gid.to_u32())
    }
}

#[rustfmt::skip]
static UNICODE_TO_MAC_ROMAN: &[u16] = &[
    0x00C4, 0x00C5, 0x00C7, 0x00C9, 0x00D1, 0x00D6, 0x00DC, 0x00E1,
    0x00E0, 0x00E2, 0x00E4, 0x00E3, 0x00E5, 0x00E7, 0x00E9, 0x00E8,
    0x00EA, 0x00EB, 0x00ED, 0x00EC, 0x00EE, 0x00EF, 0x00F1, 0x00F3,
    0x00F2, 0x00F4, 0x00F6, 0x00F5, 0x00FA, 0x00F9, 0x00FB, 0x00FC,
    0x2020, 0x00B0, 0x00A2, 0x00A3, 0x00A7, 0x2022, 0x00B6, 0x00DF,
    0x00AE, 0x00A9, 0x2122, 0x00B4, 0x00A8, 0x2260, 0x00C6, 0x00D8,
    0x221E, 0x00B1, 0x2264, 0x2265, 0x00A5, 0x00B5, 0x2202, 0x2211,
    0x220F, 0x03C0, 0x222B, 0x00AA, 0x00BA, 0x03A9, 0x00E6, 0x00F8,
    0x00BF, 0x00A1, 0x00AC, 0x221A, 0x0192, 0x2248, 0x2206, 0x00AB,
    0x00BB, 0x2026, 0x00A0, 0x00C0, 0x00C3, 0x00D5, 0x0152, 0x0153,
    0x2013, 0x2014, 0x201C, 0x201D, 0x2018, 0x2019, 0x00F7, 0x25CA,
    0x00FF, 0x0178, 0x2044, 0x20AC, 0x2039, 0x203A, 0xFB01, 0xFB02,
    0x2021, 0x00B7, 0x201A, 0x201E, 0x2030, 0x00C2, 0x00CA, 0x00C1,
    0x00CB, 0x00C8, 0x00CD, 0x00CE, 0x00CF, 0x00CC, 0x00D3, 0x00D4,
    0xF8FF, 0x00D2, 0x00DA, 0x00DB, 0x00D9, 0x0131, 0x02C6, 0x02DC,
    0x00AF, 0x02D8, 0x02D9, 0x02DA, 0x00B8, 0x02DD, 0x02DB, 0x02C7,
];

fn unicode_to_mac_roman(c: u32) -> u32 {
    let u = c as u16;
    let Some(index) = UNICODE_TO_MAC_ROMAN.iter().position(|m| *m == u) else {
        return 0;
    };
    (0x80 + index) as u32
}
