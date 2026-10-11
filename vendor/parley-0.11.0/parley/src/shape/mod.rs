// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Text shaping implementation using `harfrust`for shaping
//! and `icu` for text analysis.

use alloc::vec::Vec;
use parley_engine::shape::{CharCluster, Coverage};
use parley_engine::{
    Analysis, AnalysisDataSources, FontInstance, FontInstanceRef, ShapeOptions, Shaper,
};
use smallvec::SmallVec;

use super::layout::Layout;
use super::resolve::{ResolveContext, ResolvedStyle};
use super::style::{Brush, FontFeature};
use crate::inline_box::LayoutInlineBox;
use crate::util::{nearly_eq, nearly_zero};
use crate::{FontContext, FontData, Spacing};

use fontique::{self, Query, QueryFamily, QueryFont};
use parlance::{GenericFamily, Tag};

/// If these font features are passed to the shaper, optional ligatures are not applied.
///
/// Required ligation such as is common in cursive scripts' fonts is still applied.
///
/// Following [CSS Text 3 § 7.2][css-letter-spacing], pass these features if you are going to apply
/// something like `letter-spacing`. Unfortunately, that specification doesn't say which font
/// features specifically should be disabled. Here, we disable the OpenType features for "common",
/// "discretionary" and "historic" ligatures as given by [CSS Fonts 4 § 6.4][css-fonts-ligatures].
///
/// As of writing, this list matches Gecko. Blink additionally disables contextual alternates
/// (`calt`), but that's perhaps not desirable: disabling contextual alternates potentially makes
/// text render wrong.
///
/// [css-letter-spacing]: https://www.w3.org/TR/css-text-3/#letter-spacing-property
/// [css-fonts-ligatures]: https://www.w3.org/TR/css-fonts-4/#font-variant-ligatures-prop
const OPTIONAL_LIGATURES_OFF: [FontFeature; 4] = [
    FontFeature::new(Tag::new(b"liga"), 0),
    FontFeature::new(Tag::new(b"clig"), 0),
    FontFeature::new(Tag::new(b"dlig"), 0),
    FontFeature::new(Tag::new(b"hlig"), 0),
];

pub(crate) fn shape_text<'a, B: Brush>(
    rcx: &'a ResolveContext,
    fcx: &'a mut FontContext,
    styles: &'a [ResolvedStyle<B>],
    inline_boxes: &[LayoutInlineBox],
    analysis: &Analysis,
    char_style_indices: &[u16],
    scx: &mut Shaper,
    mut text: &str,
    layout: &mut Layout<B>,
    analysis_data_sources: &AnalysisDataSources,
) {
    // If we have both empty text and no inline boxes, shape with a fake space
    // to generate metrics that can be used to size a cursor.
    if text.is_empty() && inline_boxes.is_empty() {
        text = " ";
    }

    // Do nothing if there is no text or styles (there should always be a default style)
    if text.is_empty() || styles.is_empty() {
        for (box_idx, input) in inline_boxes.iter().enumerate() {
            // Push the box to the list of items
            layout.data.push_inline_box(box_idx, input.bidi_level);
        }
        return;
    }

    let mut fq = fcx.collection.query(&mut fcx.source_cache);

    let mut inline_box_iter = inline_boxes.iter().map(|b| &b.inline_box).peekable();

    // Merge font features with letter-spacing ligature suppression.
    //
    // TODO: This allocation is slightly unfortunate (though solvable). It's required currently,
    // because the iterator providing `ShapeOptions` has to provide a borrowed `&'a [FontFeature]`,
    // which cannot be tied to the lifetime of the call to `Iterator::next`. What we'd need is a
    // lending iterator (i.e., we probably just need to let `parley_engine` take some trait
    // providing the items).
    let style_features: &'_ Vec<SmallVec<[FontFeature; 8]>> = &styles
        .iter()
        .map(|style| {
            let style_features = rcx.features(style.font_features).unwrap_or(&[]);
            if !nearly_zero(style.letter_spacing) {
                // Later values override earlier values.
                OPTIONAL_LIGATURES_OFF
                    .into_iter()
                    .chain(style_features.iter().copied())
                    .collect()
            } else {
                style_features.iter().copied().collect()
            }
        })
        .collect();

    // Split when shaping-relevant style properties change and at inline boxes.
    let items = {
        // TODO: we currently walk characters here, but we could instead just walk boundaries of
        // styles and inline boxes.
        let char_count = char_style_indices.len();

        // The first character of the item currently being built.
        let mut item_start_char = 0_usize;

        // Positioned at `item_start_char`, i.e., the first character not yet covered by an item.
        let mut chars = text.char_indices().enumerate().peekable();
        core::iter::from_fn(move || {
            if item_start_char == char_count {
                return None;
            }

            let item_style_index = char_style_indices[item_start_char];
            let item_style = &styles[usize::from(item_style_index)];

            // Items are at least one character long, therefore the item's own first character is
            // never a split point.
            chars.next();

            let char_end = loop {
                let Some(&(char_index, (byte_index, _))) = chars.peek() else {
                    // End of text.
                    break char_count;
                };

                // Split at inlines boxes, so each box falls on a shaping boundary.
                //
                // We loop because there may be multiple boxes at this index.
                let mut split = false;
                while let Some(inline_box) = inline_box_iter.peek() {
                    if inline_box.index < byte_index {
                        // Inline boxes *before* this index are popped (this occurs if the itemizer
                        // split a run and we were not called, such as at a bidi boundary).
                        inline_box_iter.next();
                    } else if inline_box.index == byte_index {
                        inline_box_iter.next();
                        split = true;
                    } else {
                        break;
                    }
                }

                if split {
                    break char_index;
                }

                let style_index = char_style_indices[char_index];
                if style_index != item_style_index {
                    let style = &styles[usize::from(style_index)];
                    split = !nearly_eq(style.font_size, item_style.font_size)
                        || style.locale != item_style.locale
                        || style.font_variations != item_style.font_variations
                        || style.font_features != item_style.font_features
                        || !nearly_eq(style.letter_spacing, item_style.letter_spacing)
                        || !nearly_eq(style.word_spacing, item_style.word_spacing);
                }

                if split {
                    break char_index;
                }

                chars.next();
            };

            item_start_char = char_end;
            Some(parley_engine::itemize::Item {
                char_end: char_end as u32,
                options: ShapeOptions {
                    language: item_style.locale,
                    font_size: item_style.font_size,
                    features: &style_features[item_style_index as usize],
                    variations: rcx.variations(item_style.font_variations).unwrap_or(&[]),
                },
            })
        })
    };

    let font_selector = FontSelector::new(&mut fq, rcx, styles, analysis_data_sources);

    scx.shape_text(
        text,
        analysis,
        char_style_indices,
        items,
        font_selector,
        &mut layout.data.shaped_text,
    );

    let mut inline_box_iter = inline_boxes.iter().enumerate().peekable();
    for shaped_run_idx in 0..layout.data.shaped_text.runs().len() {
        let shaped_run = &layout.data.shaped_text.runs()[shaped_run_idx];
        let run_text_byte_start = shaped_run.range.byte_range.start;
        let run_style_index = char_style_indices[shaped_run.range.char_range.start];
        let run_style = &styles[usize::from(run_style_index)];

        // Push inline boxes positioned before the start of this item.
        while let Some((box_idx, inline_box)) = inline_box_iter.peek() {
            if inline_box.inline_box.index <= run_text_byte_start {
                layout.data.push_inline_box(*box_idx, inline_box.bidi_level);
                inline_box_iter.next();
            } else {
                break;
            }
        }

        layout.data.process_shaped_run(
            shaped_run_idx,
            // TODO: should we get the *item's* style here?
            //
            // Using the run's style for word and letter spacing is probably correct (and in fact,
            // we probably shouldn't itemize on them; we should just ensure we don't ligate if
            // they're non-zero).
            run_style,
            Spacing::new(run_style.word_spacing, run_style.letter_spacing),
            char_style_indices,
        );
    }

    // Process boxes at the end of the source text, including after unshaped controls.
    for (box_idx, input) in inline_box_iter {
        layout.data.push_inline_box(box_idx, input.bidi_level);
    }
}

/// The font to use if a font query doesn't return any font candidates at all.
#[derive(Debug)]
enum LastResortFont {
    Unresolved,
    Resolved(FontInstance),
    Unavailable,
}

struct FontSelector<'a, 'b, B: Brush> {
    query: &'b mut Query<'a>,
    fonts_id: Option<usize>,
    rcx: &'a ResolveContext,
    styles: &'a [ResolvedStyle<B>],
    style_index: u16,
    attrs: fontique::Attributes,

    /// Whether [`Self::query`] yields any font, cached.
    query_yields_font: Option<bool>,

    /// The font to use if [`Self::query`] doesn't return any font.
    last_resort_font: LastResortFont,

    analysis_data_sources: &'a AnalysisDataSources,
}

impl<'a, 'b, B: Brush> FontSelector<'a, 'b, B> {
    /// Construct a new `FontSelector`.
    ///
    /// If `query` ends up not returning a font for a query, the `last_resort_font` is returned
    /// instead.
    fn new(
        query: &'b mut Query<'a>,
        rcx: &'a ResolveContext,
        styles: &'a [ResolvedStyle<B>],
        analysis_data_sources: &'a AnalysisDataSources,
    ) -> Self {
        let attrs = fontique::Attributes::default();

        Self {
            query,
            fonts_id: None,
            rcx,
            styles,
            style_index: 0,
            attrs,
            query_yields_font: None,
            last_resort_font: LastResortFont::Unresolved,

            analysis_data_sources,
        }
    }
}

impl<'a, 'b, B: Brush> parley_engine::FontSelector for FontSelector<'a, 'b, B> {
    fn begin_segment(
        &mut self,
        segment: &parley_engine::itemize::Segment,
        options: &ShapeOptions<'_>,
    ) {
        self.query.set_fallbacks(fontique::FallbackKey::new(
            segment.script,
            options.language.as_ref(),
        ));
        self.query_yields_font = None;
    }

    fn select_font(
        &mut self,
        _segment: &parley_engine::itemize::Segment,
        _options: &ShapeOptions<'_>,
        cluster: &mut CharCluster,
    ) -> Option<FontInstanceRef<'_>> {
        let style_index = cluster.style_index();
        let is_emoji = cluster.is_emoji();

        if style_index != self.style_index || is_emoji || self.fonts_id.is_none() {
            self.style_index = style_index;
            let style = &self.styles[style_index as usize];

            let fonts_id = style.font_family.id();
            let fonts = self.rcx.stack(style.font_family).unwrap_or(&[]);
            let fonts = fonts.iter().copied().map(QueryFamily::Id);
            if is_emoji {
                use core::iter::once;
                let emoji_family = QueryFamily::Generic(GenericFamily::Emoji);
                self.query.set_families(fonts.chain(once(emoji_family)));
                self.fonts_id = None;
                self.query_yields_font = None;
            } else if self.fonts_id != Some(fonts_id) {
                self.query.set_families(fonts);
                self.fonts_id = Some(fonts_id);
                self.query_yields_font = None;
            }

            let attrs = fontique::Attributes {
                width: style.font_width,
                weight: style.font_weight,
                style: style.font_style,
            };
            if self.attrs != attrs {
                self.query.set_attributes(attrs);
                self.attrs = attrs;
                self.query_yields_font = None;
            }
        }

        // Fall back to the last-resort font if the query doesn't yield a font (with a charmap) at
        // all. This is checked before the actual query below due to lifetime issues. While this is
        // not *too* expensive, once the new borrow checker (Polonius) is stable, we could remove
        // this check and do the last-resort scan after the failed query:
        // <https://blog.rust-lang.org/2026/08/04/enabling-polonius-alpha-on-nightly/>.
        let query = &mut *self.query;
        let query_yields_font = *self.query_yields_font.get_or_insert_with(|| {
            let mut yields_font = false;
            query.matches_with(|font| {
                if font.charmap().is_some() {
                    yields_font = true;
                    fontique::QueryStatus::Stop
                } else {
                    fontique::QueryStatus::Continue
                }
            });
            yields_font
        });
        if !query_yields_font {
            if matches!(self.last_resort_font, LastResortFont::Unresolved) {
                if let Some(font) = any_font(self.query) {
                    self.last_resort_font = LastResortFont::Resolved(FontInstance {
                        font: FontData {
                            data: font.blob,
                            index: font.index,
                        },
                        synthesis: font.synthesis,
                    });
                } else {
                    self.last_resort_font = LastResortFont::Unavailable;
                }

                self.fonts_id = None;
            }

            return if let LastResortFont::Resolved(ref font) = self.last_resort_font {
                Some(font.as_ref())
            } else {
                None
            };
        }

        let mut selected_font = None;
        let mut best_coverage = Coverage::NONE;

        self.query.matches_with(|font| {
            let Some(charmap) = font.charmap() else {
                return fontique::QueryStatus::Continue;
            };

            let coverage = cluster.calculate_coverage(
                |ch| {
                    charmap
                        .map(ch)
                        .map(|g| {
                            // Any non-zero value indicates the existence of a glyph.
                            g != 0
                        })
                        .unwrap_or_default()
                },
                self.analysis_data_sources,
            );
            if coverage > best_coverage {
                selected_font = Some(font);
                best_coverage = coverage;

                if coverage.is_complete() {
                    fontique::QueryStatus::Stop
                } else {
                    fontique::QueryStatus::Continue
                }
            } else {
                if selected_font.is_none() {
                    selected_font = Some(font);
                }
                fontique::QueryStatus::Continue
            }
        });

        selected_font.map(|font| FontInstanceRef {
            data: &font.blob,
            index: font.index,
            synthesis: &font.synthesis,
        })
    }
}

/// Just query for any generic family's font from the font collection.
///
/// This sets generic families on `query`, so callers need to make sure to set families back again.
/// This returns `None` if it doesn't find any fonts, but note this only checks generic families.
///
/// This can be used to have a font at hand for shaping, in case more sophisticated font querying
/// returns no fonts.
fn any_font(query: &mut Query<'_>) -> Option<QueryFont> {
    let mut found = None;

    // Try the system's generic text families.
    //
    // At this point, the font itself doesn't really matter: we mostly just need any font to work
    // with. This has a cost, as it extends our query's candidate font families with all the generic
    // families registered on the system. A better future may be to allow users to register some
    // last resort in `fontique`, or pass it directly to `parley`.
    query.set_families([
        QueryFamily::Generic(GenericFamily::SansSerif),
        QueryFamily::Generic(GenericFamily::Serif),
        QueryFamily::Generic(GenericFamily::Monospace),
    ]);
    query.matches_with(
        #[inline]
        |font: &QueryFont| {
            found = Some(font.clone());
            fontique::QueryStatus::Stop
        },
    );

    found
}
