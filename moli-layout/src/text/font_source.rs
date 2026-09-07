//! FontFace sources are actual font data, not successful CSS fallback queries.

use std::collections::BTreeMap;

use parley::fontique::{FontInfo, SourceCache};
use read_fonts::{FontRef, TableProvider, types::NameId};

use super::*;

/// One decoded, validated SFNT face, ready for document registration.
#[derive(Clone, Debug)]
pub struct FontFaceData {
    pub(super) bytes: Arc<[u8]>,
}

impl FontFaceData {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, WebFontRegistrationError> {
        let bytes = decode_web_font_bytes(bytes)?;
        Self::from_index(bytes, 0)
    }

    fn from_index(bytes: Arc<[u8]>, index: u32) -> Result<Self, WebFontRegistrationError> {
        let font = FontRef::from_index(&bytes, index)
            .map_err(|_| WebFontRegistrationError::UnsupportedPayload)?;
        let bytes = if font.ttc_index().is_some() {
            standalone_face(&font).ok_or(WebFontRegistrationError::UnsupportedPayload)?
        } else {
            bytes
        };
        validate_registered_font(&WebFontFace::new("FontFace validation"), bytes.clone())?;
        Ok(Self { bytes })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

// Fontique registers every member of a collection. Extract the selected local
// face first so a CSS alias cannot accidentally refer to another TTC member.
fn standalone_face(font: &FontRef<'_>) -> Option<Arc<[u8]>> {
    let directory = font.table_directory();
    let mut records = directory.table_records().to_vec();
    records.retain(|record| record.tag() != read_fonts::types::Tag::new(b"DSIG"));
    records.sort_by_key(|record| record.tag());
    let count = u16::try_from(records.len()).ok()?;
    if count == 0 || count > 4095 {
        return None;
    }
    let selector = count.ilog2() as u16;
    let search_range = (1_u16 << selector) * 16;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&directory.sfnt_version().to_be_bytes());
    for value in [count, search_range, selector, count * 16 - search_range] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.resize(12 + records.len() * 16, 0);
    let mut head_offset = None;
    for (index, record) in records.iter().enumerate() {
        let data = font.table_data(record.tag())?;
        let data = data.as_bytes();
        let offset = bytes.len();
        bytes.extend_from_slice(data);
        bytes.resize(bytes.len().next_multiple_of(4), 0);
        if record.tag() == read_fonts::types::Tag::new(b"head") {
            if data.len() < 12 {
                return None;
            }
            bytes[offset + 8..offset + 12].fill(0);
            head_offset = Some(offset);
        }
        let checksum = sfnt_checksum(&bytes[offset..]);
        let entry = 12 + index * 16;
        bytes[entry..entry + 4].copy_from_slice(&record.tag().to_be_bytes());
        bytes[entry + 4..entry + 8].copy_from_slice(&checksum.to_be_bytes());
        bytes[entry + 8..entry + 12].copy_from_slice(&u32::try_from(offset).ok()?.to_be_bytes());
        bytes[entry + 12..entry + 16].copy_from_slice(&record.length().to_be_bytes());
    }
    let head = head_offset?;
    let adjustment = 0xb1b0_afba_u32.wrapping_sub(sfnt_checksum(&bytes));
    bytes[head + 8..head + 12].copy_from_slice(&adjustment.to_be_bytes());
    Some(Arc::from(bytes))
}

fn sfnt_checksum(bytes: &[u8]) -> u32 {
    bytes.chunks_exact(4).fold(0_u32, |sum, word| {
        sum.wrapping_add(u32::from_be_bytes(
            word.try_into().expect("four-byte chunk"),
        ))
    })
}

/// The platform-only collection is separate from document web fonts. A CSS
/// alias, generic fallback, or previously downloaded font is not a local font.
pub(super) struct LocalFontSources {
    collection: Collection,
    cache: SourceCache,
    names: Option<BTreeMap<String, FontInfo>>,
}

impl LocalFontSources {
    pub(super) fn new(system_fonts: bool) -> Self {
        Self {
            collection: Collection::new(CollectionOptions {
                shared: false,
                system_fonts,
            }),
            cache: SourceCache::default(),
            names: None,
        }
    }

    pub(super) fn find(&mut self, name: &str) -> Option<FontFaceData> {
        if self.names.is_none() {
            let mut names = BTreeMap::new();
            let families = self
                .collection
                .family_names()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            for family in families {
                let Some(family) = self.collection.family_by_name(&family) else {
                    continue;
                };
                for info in family.fonts() {
                    let Some(data) = info.load(Some(&mut self.cache)) else {
                        continue;
                    };
                    let Ok(font) = FontRef::from_index(data.as_ref(), info.index()) else {
                        continue;
                    };
                    let Ok(table) = font.name() else {
                        continue;
                    };
                    for record in table.name_record() {
                        if !matches!(
                            record.name_id(),
                            NameId::FULL_NAME | NameId::POSTSCRIPT_NAME
                        ) {
                            continue;
                        }
                        let Ok(name) = record.string(table.string_data()) else {
                            continue;
                        };
                        let name = name.to_string().to_lowercase();
                        if !name.is_empty() {
                            names.entry(name).or_insert_with(|| info.clone());
                        }
                    }
                }
            }
            self.names = Some(names);
        }
        let info = self.names.as_ref()?.get(&name.to_lowercase())?;
        let data = info.load(Some(&mut self.cache))?;
        FontFaceData::from_index(Arc::from(data.as_ref()), info.index()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: &[u8] = include_bytes!("../../tests/fixtures/moli-ahem.ttf");

    #[test]
    fn font_face_data_rejects_headers_without_font_tables() {
        for bytes in [
            &b"\x00\x01\x00\x00"[..],
            &b"OTTO"[..],
            &b"wOFF"[..],
            &b"wOF2"[..],
            &[0; 32][..],
        ] {
            assert!(FontFaceData::from_bytes(bytes).is_err());
        }
        assert!(FontFaceData::from_bytes(FONT).is_ok());
        assert!(
            FontFaceData::from_bytes(include_bytes!("../../tests/fixtures/moli-ahem.woff")).is_ok()
        );
        assert!(
            FontFaceData::from_bytes(include_bytes!("../../tests/fixtures/moli-ahem.woff2"))
                .is_ok()
        );
    }

    #[test]
    fn local_font_lookup_uses_full_and_postscript_names_not_css_aliases() {
        let mut sources = LocalFontSources::new(false);
        sources.collection.register_fonts(
            Blob::new(Arc::new(FONT.to_vec())),
            Some(FontInfoOverride {
                family_name: Some("NotTheFontName"),
                ..Default::default()
            }),
        );
        let font = FontRef::new(FONT).unwrap();
        let table = font.name().unwrap();
        let mut checked = 0;
        for record in table.name_record() {
            if matches!(
                record.name_id(),
                NameId::FULL_NAME | NameId::POSTSCRIPT_NAME
            ) {
                let name = record.string(table.string_data()).unwrap().to_string();
                assert!(sources.find(&name).is_some(), "{name}");
                assert!(sources.find(&name.to_lowercase()).is_some(), "{name}");
                checked += 1;
            }
        }
        assert!(checked > 0);
        assert!(sources.find("NotTheFontName").is_none());
        assert!(sources.find("serif").is_none());
        assert!(sources.find("MoliDefinitelyNotAFont").is_none());
    }

    #[test]
    fn local_collection_selection_extracts_only_the_requested_face() {
        let fonts = [
            FONT,
            &include_bytes!("../../tests/fixtures/moli-cjk.ttf")[..],
        ];
        let mut collection = b"ttcf\x00\x01\x00\x00\x00\x00\x00\x02".to_vec();
        collection.resize(20, 0);
        for (index, bytes) in fonts.iter().enumerate() {
            let offset = collection.len();
            collection[12 + index * 4..16 + index * 4]
                .copy_from_slice(&(offset as u32).to_be_bytes());
            collection.extend_from_slice(bytes);
            let font = FontRef::new(bytes).unwrap();
            for (table, record) in font.table_directory().table_records().iter().enumerate() {
                let position = offset + 12 + table * 16 + 8;
                collection[position..position + 4]
                    .copy_from_slice(&(record.offset() + offset as u32).to_be_bytes());
            }
            collection.resize(collection.len().next_multiple_of(4), 0);
        }
        for (index, original) in fonts.iter().enumerate() {
            let selected =
                FontFaceData::from_index(Arc::from(collection.as_slice()), index as u32).unwrap();
            let font = FontRef::new(selected.bytes()).unwrap();
            assert!(font.ttc_index().is_none());
            assert_eq!(
                font.maxp().unwrap().num_glyphs(),
                FontRef::new(original).unwrap().maxp().unwrap().num_glyphs()
            );
            assert_eq!(sfnt_checksum(selected.bytes()), 0xb1b0_afba);
        }
    }
}
