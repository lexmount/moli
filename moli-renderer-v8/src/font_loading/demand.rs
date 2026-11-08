//! Font resource selection from style/text demand, independent of layout.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use moli_css_parse::{FontSelectionCapabilities, font_family_name_key};
use moli_layout::WebFontFace;
use read_fonts::{FontRef, TableProvider};
use style::{properties::ComputedValues, values::computed::font::SingleFontFamily};

use super::{FontFaceLoad, FontFaceStatus};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct FontUse {
    pub(crate) families: Vec<String>,
    weight: u32,
    width: u32,
    slope: u32,
}

impl FontUse {
    pub(crate) fn from_style(style: &ComputedValues) -> Self {
        let font = style.get_font();
        let families = font
            .font_family
            .families
            .list
            .iter()
            .filter_map(|family| match family {
                SingleFontFamily::FamilyName(name) => {
                    Some(font_family_name_key(name.name.as_ref()))
                }
                SingleFontFamily::Generic(_) => None,
            })
            .collect();
        let slope = match font.font_style {
            style::values::computed::font::FontStyle::NORMAL => 0.0_f32,
            style::values::computed::font::FontStyle::ITALIC => 14.0,
            value => value.oblique_degrees(),
        };
        Self {
            families,
            weight: font.font_weight.value().to_bits(),
            width: font.font_width.0.to_float().to_bits(),
            slope: slope.to_bits(),
        }
    }

    fn capabilities(&self) -> FontSelectionCapabilities {
        FontSelectionCapabilities::single(
            f32::from_bits(self.weight),
            f32::from_bits(self.width),
            f32::from_bits(self.slope),
        )
    }
}

pub(crate) type FontUses = HashMap<FontUse, BTreeSet<char>>;

#[derive(Clone, PartialEq)]
pub(crate) struct FontSourceSignature {
    identity: usize,
    descriptor: Arc<WebFontFace>,
    status: FontFaceStatus,
}

impl FontSourceSignature {
    pub(crate) fn new(load: &FontFaceLoad) -> Self {
        let face = load.borrow();
        Self {
            identity: std::rc::Rc::as_ptr(load) as usize,
            descriptor: face.descriptor().clone(),
            status: face.status(),
        }
    }
}

pub(crate) fn pending_families(sources: &[FontFaceLoad], manual_start: usize) -> HashSet<String> {
    sources[manual_start..]
        .iter()
        .filter(|source| source.borrow().status() == FontFaceStatus::Unloaded)
        .map(|source| source.borrow().descriptor().family_name_key().to_owned())
        .collect()
}

/// Fonts at the end of the vector win ties: CSS cascade order, then manual
/// insertion order. Capability matching happens before unicode-range.
///
/// CSS permits a referenced family to be fetched in anticipation of character
/// fallback. We need no platform font queries, shaping, boxes or coordinates to
/// identify the authored faces that can serve the text.
pub(crate) fn requested_sources(
    sources: &[FontFaceLoad],
    manual_start: usize,
    uses: &FontUses,
) -> BTreeSet<String> {
    let mut families = HashMap::<&str, Vec<(usize, &FontFaceLoad)>>::new();
    let descriptors = sources
        .iter()
        .map(|source| source.borrow().descriptor().clone())
        .collect::<Vec<_>>();
    for (index, descriptor) in descriptors.iter().enumerate() {
        if sources[index].borrow().status() == FontFaceStatus::Error {
            continue;
        }
        families
            .entry(descriptor.family_name_key())
            .or_default()
            .push((index, &sources[index]));
    }
    let mut requested = BTreeSet::new();
    let mut coverage = HashMap::<(usize, char), bool>::new();
    for (usage, characters) in uses {
        let capabilities = usage.capabilities();
        for family in &usage.families {
            let Some(candidates) = families.get(family.as_str()) else {
                continue;
            };
            let best = candidates
                .iter()
                .map(|(index, _)| {
                    descriptors[*index]
                        .css_capabilities()
                        .expect("validated face")
                })
                .min_by(|first, second| first.compare(*second, capabilities))
                .expect("nonempty family");
            let selected = candidates
                .iter()
                .filter(|(index, _)| {
                    descriptors[*index]
                        .css_capabilities()
                        .expect("validated face")
                        .compare(best, capabilities)
                        .is_eq()
                })
                .collect::<Vec<_>>();
            for &character in characters {
                // Reject range misses before querying any loaded font. In
                // particular, Latin text never probes a pending CJK subset.
                if !selected.iter().any(|&&(index, load)| {
                    index >= manual_start
                        && matches!(
                            load.borrow().status(),
                            FontFaceStatus::Unloaded | FontFaceStatus::Loading
                        )
                        && covers_range(&descriptors[index], character)
                }) {
                    continue;
                }
                for &&(index, load) in selected.iter().rev() {
                    if !covers_range(&descriptors[index], character) {
                        continue;
                    }
                    let face = load.borrow();
                    match face.status() {
                        FontFaceStatus::Error => continue,
                        FontFaceStatus::Loaded => {
                            let covers = *coverage.entry((index, character)).or_insert_with(|| {
                                face.font_data().is_some_and(|data| {
                                    FontRef::new(data.bytes())
                                        .ok()
                                        .and_then(|font| font.cmap().ok())
                                        .and_then(|cmap| cmap.map_codepoint(character))
                                        .is_some_and(|glyph| glyph.to_u32() != 0)
                                })
                            });
                            if !covers {
                                continue;
                            }
                        }
                        FontFaceStatus::Unloaded => {
                            if index >= manual_start {
                                requested.insert(face.slot().to_owned());
                            }
                        }
                        FontFaceStatus::Loading => {}
                    }
                    break;
                }
            }
        }
    }
    requested
}

fn covers_range(face: &WebFontFace, character: char) -> bool {
    face.unicode_ranges().is_empty()
        || face
            .unicode_ranges()
            .iter()
            .any(|range| range.contains(character))
}
