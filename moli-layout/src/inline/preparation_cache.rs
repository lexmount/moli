use std::ops::Range;

use parley::{FontFamily, FontFamilyName, FontFeatures, FontVariations, TextStyle};

use crate::{LayoutBoxId, stylo_to_parley::TextBrush};

type PreparedStyle = TextStyle<'static, 'static, TextBrush>;

const MAX_ENTRIES: usize = 64;
const MAX_TEXT_BYTES: usize = 64;

/// A bounded index into pristine Parley results already owned by this pass.
/// It owns no layouts, fonts, DOM nodes, or previous-pass geometry. Numeric
/// layout runs after the index has been dropped and can mutate each clone.
///
/// Blink's InlineNode::IsNGShapeCacheAllowed also restricts reuse to simple
/// inputs: inline structure can affect shaping even when text matches. Our
/// first admission rule requires one complete style run and no inline boxes.
#[derive(Default)]
pub(super) struct InlinePreparationCache {
    entries: Vec<Entry>,
    next_slot: usize,
    hits: usize,
}

struct Entry {
    text: String,
    style: PreparedStyle,
    quantize: bool,
    owner: LayoutBoxId,
}

impl InlinePreparationCache {
    pub(super) fn find(
        &mut self,
        text: &str,
        style: &PreparedStyle,
        quantize: bool,
    ) -> Option<LayoutBoxId> {
        let entry = self.entries.iter().rev().find(|entry| {
            entry.text == text && entry.quantize == quantize && entry.style == *style
        })?;
        self.hits += 1;
        Some(entry.owner)
    }

    pub(super) fn insert(
        &mut self,
        text: &str,
        style: &PreparedStyle,
        quantize: bool,
        owner: LayoutBoxId,
    ) {
        let entry = Entry {
            text: text.to_owned(),
            style: style.clone(),
            quantize,
            owner,
        };
        if self.entries.len() < MAX_ENTRIES {
            self.entries.push(entry);
        } else {
            self.entries[self.next_slot] = entry;
            self.next_slot = (self.next_slot + 1) % MAX_ENTRIES;
        }
    }

    pub(super) fn hits(&self) -> usize {
        self.hits
    }
}

pub(super) fn candidate_style<'a>(
    text: &str,
    styles: &'a [PreparedStyle],
    runs: &[(Range<usize>, usize)],
    no_objects: bool,
) -> Option<&'a PreparedStyle> {
    let [style] = styles else { return None };
    let [(range, 0)] = runs else { return None };
    if !no_objects
        || text.is_empty()
        || text.len() > MAX_TEXT_BYTES
        || *range != (0..text.len())
        || !bounded_style(style)
    {
        return None;
    }
    Some(style)
}

// Bound the variable-sized key too. A short string alone does not bound CSS
// family lists, feature settings or paint brushes. These checks affect reuse,
// never which fonts/styles the shared uncached builder accepts.
fn bounded_style(style: &PreparedStyle) -> bool {
    let family_name_fits = |name: &FontFamilyName<'_>| match name {
        FontFamilyName::Named(name) => name.len() <= 256,
        FontFamilyName::Generic(_) => true,
    };
    let family_fits = match &style.font_family {
        FontFamily::Single(name) => family_name_fits(name),
        FontFamily::List(names) => names.len() <= 8 && names.iter().all(family_name_fits),
        FontFamily::Source(source) => source.len() <= 256,
    };
    let features_fit = match &style.font_features {
        FontFeatures::Source(source) => source.len() <= 256,
        FontFeatures::List(features) => features.len() <= 16,
    };
    let variations_fit = match &style.font_variations {
        FontVariations::Source(source) => source.len() <= 256,
        FontVariations::List(variations) => variations.len() <= 16,
    };
    family_fits
        && features_fit
        && variations_fit
        && std::iter::once(&style.brush)
            .chain(style.underline_brush.as_ref())
            .chain(style.strikethrough_brush.as_ref())
            .all(|brush| brush.shadows.len() <= 8)
}

#[cfg(test)]
mod tests;
