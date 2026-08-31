use std::collections::HashMap;

use moli_core::page::RendererPdfTextLayer;
use read_fonts::FontRef;
use read_fonts::TableProvider;
use read_fonts::types::Tag;

/// The font-program format embedded into the PDF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PdfFontFileType {
    /// TrueType outlines (`glyf`), embedded as `FontFile2`.
    TrueType,
    /// CFF outlines, embedded as `FontFile3 /Subtype /Type1C`.
    Type1C,
    /// CFF2 outlines, embedded as `FontFile3 /Subtype /CIDFontType0C`.
    CidType0C,
}

/// One pre-parsed font needed by the print text layer.
///
/// All values are derived once from the raw font program; the PDF page loop
/// only serializes these into font dictionaries and a `ToUnicode` CMap.
pub(super) struct PdfFont {
    /// BaseFont / FontName, sanitized for the PDF name object.
    pub(super) name: String,
    pub(super) file_type: PdfFontFileType,
    /// Full embedded font program.
    pub(super) file_bytes: Vec<u8>,
    /// Glyph-to-Unicode `bfchar` entries as `(glyph id, UTF-16BE hex)`.
    pub(super) to_unicode: Vec<(u16, String)>,
    /// Font bounding box in font units.
    pub(super) bbox: [i32; 4],
    pub(super) ascent: i32,
    pub(super) descent: i32,
    pub(super) cap_height: i32,
}

/// Parses every font referenced by a print text layer into a PDF-ready plan.
pub(super) fn build_font_plans(text_layer: &RendererPdfTextLayer) -> Vec<PdfFont> {
    let mut plans = Vec::with_capacity(text_layer.fonts.len());
    for (font_index, resource) in text_layer.fonts.iter().enumerate() {
        let plan = FontRef::from_index(resource.data.as_ref(), resource.collection_index)
            .ok()
            .and_then(|font| plan_font(&font, font_index, text_layer));
        plans.push(plan.unwrap_or_else(|| default_plan(font_index)));
    }
    plans
}

fn plan_font(
    font: &FontRef<'_>,
    font_index: usize,
    text_layer: &RendererPdfTextLayer,
) -> Option<PdfFont> {
    let used_glyphs: std::collections::BTreeSet<u16> = text_layer
        .runs
        .iter()
        .filter(|run| run.font == font_index)
        .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.id as u16))
        .filter(|glyph_id| *glyph_id != 0)
        .collect();

    let reverse = reverse_cmap(font).unwrap_or_default();
    let to_unicode = used_glyphs
        .iter()
        .filter_map(|glyph_id| {
            reverse
                .get(&u32::from(*glyph_id))
                .copied()
                .map(|codepoint| (*glyph_id, utf16be_hex(codepoint)))
        })
        .collect::<Vec<_>>();

    let name = font_postscript_name(font).unwrap_or_else(|| format!("MoliPdfFont{font_index}"));

    let file_type = if font.table_data(Tag::new(b"glyf")).is_some() {
        PdfFontFileType::TrueType
    } else if font.table_data(Tag::new(b"CFF ")).is_some() {
        PdfFontFileType::Type1C
    } else if font.table_data(Tag::new(b"CFF2")).is_some() {
        PdfFontFileType::CidType0C
    } else {
        PdfFontFileType::TrueType
    };

    let head = font.head().ok();
    let hhea = font.hhea().ok();
    let os2 = font.os2().ok();
    let units_per_em = head.as_ref().map(|h| h.units_per_em()).unwrap_or(1000);
    let bbox = [
        head.as_ref().map(|h| i32::from(h.x_min())).unwrap_or(0),
        head.as_ref().map(|h| i32::from(h.y_min())).unwrap_or(0),
        head.as_ref()
            .map(|h| i32::from(h.x_max()))
            .unwrap_or(i32::from(units_per_em)),
        head.as_ref()
            .map(|h| i32::from(h.y_max()))
            .unwrap_or(i32::from(units_per_em)),
    ];
    let ascent = hhea
        .as_ref()
        .map(|h| i32::from(h.ascender().to_i16()))
        .unwrap_or(i32::from(units_per_em));
    let descent = hhea
        .as_ref()
        .map(|h| i32::from(h.descender().to_i16()))
        .unwrap_or(-(i32::from(units_per_em) / 5));
    let cap_height = os2
        .and_then(|o| o.s_cap_height())
        .map(i32::from)
        .unwrap_or(i32::from(units_per_em) * 7 / 10);

    Some(PdfFont {
        name,
        file_type,
        file_bytes: font.data().as_ref().to_vec(),
        to_unicode,
        bbox,
        ascent,
        descent,
        cap_height,
    })
}

fn default_plan(font_index: usize) -> PdfFont {
    PdfFont {
        name: format!("MoliPdfFont{font_index}"),
        file_type: PdfFontFileType::TrueType,
        file_bytes: Vec::new(),
        to_unicode: Vec::new(),
        bbox: [0, 0, 0, 0],
        ascent: 0,
        descent: 0,
        cap_height: 0,
    }
}

/// Builds a glyph-to-Unicode reverse map from the best `cmap` subtable.
///
/// The first codepoint that maps to each glyph wins, which is the convention
/// Chromium uses for its `ToUnicode` CMaps.
fn reverse_cmap(font: &FontRef<'_>) -> Option<HashMap<u32, u32>> {
    let cmap = font.cmap().ok()?;
    let (_, _, subtable) = cmap.best_subtable()?;
    let mut map = HashMap::new();
    match subtable {
        read_fonts::tables::cmap::CmapSubtable::Format0(table) => {
            for (codepoint, glyph_id) in table.glyph_id_array().iter().enumerate() {
                if *glyph_id != 0 {
                    map.entry(u32::from(*glyph_id)).or_insert(codepoint as u32);
                }
            }
        }
        read_fonts::tables::cmap::CmapSubtable::Format4(table) => {
            reverse_cmap_format4(&table, &mut map);
        }
        read_fonts::tables::cmap::CmapSubtable::Format6(table) => {
            let first = u32::from(table.first_code());
            for (offset, glyph_id) in table.glyph_id_array().iter().enumerate() {
                let glyph_id = u32::from(glyph_id.get());
                if glyph_id != 0 {
                    map.entry(glyph_id).or_insert(first + offset as u32);
                }
            }
        }
        read_fonts::tables::cmap::CmapSubtable::Format12(table) => {
            reverse_cmap_format12(&table, &mut map);
        }
        _ => {}
    }
    Some(map)
}

fn reverse_cmap_format4(table: &read_fonts::tables::cmap::Cmap4<'_>, map: &mut HashMap<u32, u32>) {
    let segment_count = usize::from(table.seg_count_x2()) / 2;
    let start_code = table.start_code();
    let end_code = table.end_code();
    let id_delta = table.id_delta();
    let id_range_offsets = table.id_range_offsets();
    let glyph_id_array = table.glyph_id_array();
    for index in 0..segment_count {
        let start = u32::from(start_code[index].get());
        let end = u32::from(end_code[index].get());
        if start == 0xFFFF && end == 0xFFFF || start > end {
            continue;
        }
        let delta = i64::from(id_delta[index].get());
        let range_offset = usize::from(id_range_offsets[index].get());
        for codepoint in start..=end {
            let glyph_id = if range_offset == 0 {
                (i64::from(codepoint) + delta) & 0xFFFF
            } else {
                // The glyph id array is located relative to the address of the
                // `idRangeOffset` word itself (index `index` within the array).
                let word_index = range_offset / 2 + (codepoint - start) as usize + index;
                let Some(array_index) = word_index.checked_sub(segment_count) else {
                    continue;
                };
                if array_index >= glyph_id_array.len() {
                    continue;
                }
                let glyph_id = i64::from(glyph_id_array[array_index].get());
                if glyph_id == 0 {
                    continue;
                }
                (glyph_id + delta) & 0xFFFF
            };
            if glyph_id != 0 {
                map.entry(glyph_id as u32).or_insert(codepoint);
            }
        }
    }
}

fn reverse_cmap_format12(
    table: &read_fonts::tables::cmap::Cmap12<'_>,
    map: &mut HashMap<u32, u32>,
) {
    let mut remaining = 200_000usize;
    for group in table.groups() {
        let start = group.start_char_code();
        let end = group.end_char_code();
        let start_glyph_id = group.start_glyph_id();
        let count = usize::try_from(end.saturating_sub(start) + 1).unwrap_or(usize::MAX);
        let count = count.min(remaining);
        for offset in 0..count {
            let codepoint = start + offset as u32;
            let glyph_id = start_glyph_id + offset as u32;
            map.entry(glyph_id).or_insert(codepoint);
        }
        remaining = remaining.saturating_sub(count);
        if remaining == 0 {
            break;
        }
    }
}

fn utf16be_hex(codepoint: u32) -> String {
    if codepoint > 0xFFFF {
        let codepoint = codepoint - 0x10000;
        let high = 0xD800 + (codepoint >> 10);
        let low = 0xDC00 + (codepoint & 0x3FF);
        format!("{high:04X}{low:04X}")
    } else {
        format!("{codepoint:04X}")
    }
}

/// Reads the PostScript name (name ID 6) from the `name` table.
fn font_postscript_name(font: &FontRef<'_>) -> Option<String> {
    let name = font.name().ok()?;
    let table_bytes = font.table_data(Tag::new(b"name"))?;
    let table_bytes = table_bytes.as_ref();
    if table_bytes.len() < 6 {
        return None;
    }
    let storage_offset = u16::from_be_bytes([table_bytes[4], table_bytes[5]]) as usize;
    let mut best: Option<(i32, String)> = None;
    for record in name.name_record() {
        if record.name_id() != read_fonts::types::NameId::POSTSCRIPT_NAME {
            continue;
        }
        let platform = record.platform_id();
        let encoding = record.encoding_id();
        let score = match (platform, encoding) {
            (0, _) => 3,
            (3, 1) => 2,
            (3, 0) => 1,
            (1, 0) => 1,
            _ => 0,
        };
        let string_start = storage_offset.checked_add(record.string_offset().to_u32() as usize)?;
        let string_end = string_start.checked_add(record.length() as usize)?;
        if string_end > table_bytes.len() {
            continue;
        }
        let bytes = &table_bytes[string_start..string_end];
        let decoded = if platform == 1 {
            bytes.iter().map(|&byte| byte as char).collect::<String>()
        } else {
            decode_utf16be(bytes)?
        };
        if best
            .as_ref()
            .is_none_or(|(best_score, _)| score > *best_score)
        {
            best = Some((score, decoded));
        }
    }
    best.map(|(_, name)| sanitize_font_name(&name))
}

fn decode_utf16be(bytes: &[u8]) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let units = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    String::from_utf16(&units).ok()
}

fn sanitize_font_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    if sanitized.is_empty() {
        "MoliPdfFont".to_owned()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ahem_font_bytes() -> Vec<u8> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.ttf"
        );
        std::fs::read(path).expect("ahem fixture exists")
    }

    fn font_ref(bytes: &[u8]) -> FontRef<'_> {
        FontRef::from_index(bytes, 0).expect("parses as a font")
    }

    #[test]
    fn reverse_cmap_maps_ahem_glyphs() {
        let bytes = ahem_font_bytes();
        let font = font_ref(&bytes);
        let map = reverse_cmap(&font).expect("cmap parses");
        // The Ahem font maps every codepoint to a distinct glyph.
        assert_eq!(map.get(&1), Some(&0x20));
        assert!(map.contains_key(&u32::from(b'A')));
    }

    #[test]
    fn ps_name_is_sanitized_ascii() {
        let bytes = ahem_font_bytes();
        let font = font_ref(&bytes);
        let name = font_postscript_name(&font).expect("name table parses");
        assert!(name.chars().all(|ch| ch.is_ascii_alphanumeric()));
        assert!(!name.is_empty());
    }

    #[test]
    fn utf16be_encodes_surrogate_pairs() {
        assert_eq!(utf16be_hex(0x20), "0020");
        assert_eq!(utf16be_hex(0x1F600), "D83DDE00");
    }

    #[test]
    fn plans_fonts_from_text_layer() {
        let bytes = ahem_font_bytes();
        let layer = moli_core::page::RendererPdfTextLayer {
            css_width: 100.0,
            css_height: 100.0,
            fonts: vec![moli_core::page::RendererPdfFont {
                data: std::sync::Arc::from(bytes.as_slice()),
                collection_index: 0,
            }],
            runs: vec![moli_core::page::RendererPdfTextRun {
                font: 0,
                font_size: 16.0,
                glyphs: vec![moli_core::page::RendererPdfGlyph {
                    id: 65,
                    x: 1.0,
                    y: 2.0,
                }],
            }],
        };
        let plans = build_font_plans(&layer);
        assert_eq!(plans.len(), 1);
        assert!(!plans[0].file_bytes.is_empty());
        assert_eq!(plans[0].file_type, PdfFontFileType::TrueType);
        assert_eq!(plans[0].to_unicode.first().map(|(gid, _)| *gid), Some(65));
    }
}
