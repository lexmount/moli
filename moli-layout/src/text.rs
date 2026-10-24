// SPDX-License-Identifier: MIT OR Apache-2.0
//
// The Parley shaping/projection path is narrowly adapted from DioxusLabs/blitz
// commit d788124ab881f9bb537cb452ec1d837604a374a8:
// - packages/blitz-dom/src/node/text.rs
// - packages/blitz-paint/src/text.rs

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use moli_css_parse::{
    FontSelectionCapabilities as WebFontCapabilities, FontSelectionRange, font_family_name_key,
};
use moli_system_fonts::SystemFontFamilyResolver;
use parley::{
    FontContext, FontFamily, FontFamilyName, LayoutContext, TextStyle,
    fontique::{
        Attributes, Blob, Collection, CollectionOptions, FontInfoOverride, FontStyle, FontWeight,
        FontWidth, QueryFamily, QueryStatus,
    },
};
use thiserror::Error;

use crate::stylo_to_parley::TextBrush;

mod font_source;
pub use font_source::FontFaceData;

pub(crate) struct ParleyDocumentServices {
    pub(crate) font_context: FontContext,
    pub(crate) layout_context: LayoutContext<TextBrush>,
    system_font_family_resolver: Option<SystemFontFamilyResolver>,
    web_font_families: BTreeMap<String, SegmentedWebFontFamily>,
    font_family_resolution_plans: Vec<FontFamilyResolutionPlan>,
    inline_font_metrics_cache: Vec<(
        TextStyle<'static, 'static, TextBrush>,
        Option<InlineFontMetrics>,
    )>,
    #[cfg(test)]
    font_family_resolution_miss_count: usize,
}

#[derive(Clone, Debug)]
struct FontFamilyResolutionPlan {
    requested: FontFamily<'static>,
    attributes: Attributes,
    requires_character_resolution: bool,
    default: Option<FontFamily<'static>>,
    characters: HashMap<char, FontFamily<'static>>,
}

const MAX_FONT_FAMILY_RESOLUTION_PLANS: usize = 1024;
const MAX_CHARACTER_RESOLUTIONS_PER_PLAN: usize = 4096;
const MAX_INLINE_FONT_METRICS_CACHE_ENTRIES: usize = 1024;

fn font_selection_capabilities(attributes: Attributes) -> WebFontCapabilities {
    let slope = match attributes.style {
        FontStyle::Normal => 0.0,
        FontStyle::Italic => 14.0,
        FontStyle::Oblique(angle) => angle.unwrap_or(14.0),
    };
    WebFontCapabilities::single(
        attributes.weight.value(),
        attributes.width.percentage(),
        slope,
    )
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum WebFontPriorityGroup {
    Stylesheet,
    Manual,
}

/// Manual faces outrank CSS-connected faces; later positions win in each group.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct WebFontPriority {
    group: WebFontPriorityGroup,
    position: usize,
}

impl WebFontPriority {
    const fn stylesheet(position: usize) -> Self {
        Self {
            group: WebFontPriorityGroup::Stylesheet,
            position,
        }
    }

    const fn manual(position: usize) -> Self {
        Self {
            group: WebFontPriorityGroup::Manual,
            position,
        }
    }
}

type WebFontOrder = BTreeMap<String, WebFontPriority>;

#[derive(Clone, Debug)]
struct SegmentedWebFontFace {
    source_slot: String,
    internal_family_name: String,
    source_priority: WebFontPriority,
    unicode_ranges: Vec<WebFontUnicodeRange>,
}

impl SegmentedWebFontFace {
    fn priority_key(&self) -> (WebFontPriority, &str) {
        (self.source_priority, &self.source_slot)
    }

    fn contains(&self, character: char) -> bool {
        self.unicode_ranges.is_empty()
            || self
                .unicode_ranges
                .iter()
                .any(|range| range.contains(character))
    }
}

#[derive(Clone, Debug)]
struct SegmentedWebFontCapabilityGroup {
    capabilities: WebFontCapabilities,
    faces: Vec<SegmentedWebFontFace>,
}

#[derive(Clone, Debug, Default)]
struct SegmentedWebFontFamily {
    groups: Vec<SegmentedWebFontCapabilityGroup>,
}

/// Primary-font metrics attached to one resolved Parley text style.
///
/// A shaped run may use a fallback font for its glyphs. CSSOM text geometry,
/// like Blink's `FragmentItem`, instead uses the primary font metrics of the
/// style that owns the run.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InlineFontMetrics {
    pub(crate) ascent: f32,
    pub(crate) descent: f32,
    pub(crate) line_height: f32,
    pub(crate) x_height: f32,
}

fn resolved_inline_x_height(ascent: f32, x_height: Option<f32>) -> f32 {
    x_height.unwrap_or(ascent * 0.56).max(0.0)
}

impl ParleyDocumentServices {
    /// Resolves CSS downloadable-font families into the selected segmented
    /// face, then applies explicit platform-family substitutions.
    ///
    /// Blink first chooses one font-capability group (width/style/weight),
    /// then considers only that group's faces whose `unicode-range` contains
    /// the character. Fontique has no segmented-face abstraction, so each
    /// physical face is registered under an internal family and this method
    /// preserves that capability-before-range behavior before shaping.
    pub(crate) fn resolve_font_families(
        &mut self,
        style: &mut TextStyle<'static, 'static, TextBrush>,
        character: Option<char>,
    ) {
        let plan_index = self.font_family_resolution_plan_index(style);
        let cached = match character {
            Some(character) => self.font_family_resolution_plans[plan_index]
                .characters
                .get(&character),
            None => self.font_family_resolution_plans[plan_index]
                .default
                .as_ref(),
        }
        .cloned();
        if let Some(cached) = cached {
            style.font_family = cached;
            return;
        }

        self.resolve_segmented_web_font_families(style, character);
        if let Some(resolver) = self.system_font_family_resolver.as_mut() {
            let mut resolve_family = |family: &mut FontFamilyName<'static>| {
                if let FontFamilyName::Named(name) = family
                    && let Some(substitute) =
                        resolver.substitute_family(&mut self.font_context.collection, name)
                {
                    *name = Cow::Owned(substitute);
                }
            };
            match &mut style.font_family {
                FontFamily::Single(family) => resolve_family(family),
                FontFamily::List(families) => {
                    for family in Cow::to_mut(families) {
                        resolve_family(family);
                    }
                }
                FontFamily::Source(_) => {}
            }
        }
        let resolved = style.font_family.clone().into_owned();
        match character {
            Some(character) => {
                if self.font_family_resolution_plans[plan_index]
                    .characters
                    .len()
                    >= MAX_CHARACTER_RESOLUTIONS_PER_PLAN
                {
                    self.font_family_resolution_plans[plan_index]
                        .characters
                        .clear();
                }
                self.font_family_resolution_plans[plan_index]
                    .characters
                    .insert(character, resolved);
            }
            None => self.font_family_resolution_plans[plan_index].default = Some(resolved),
        }
        #[cfg(test)]
        {
            self.font_family_resolution_miss_count =
                self.font_family_resolution_miss_count.saturating_add(1);
        }
    }

    pub(crate) fn requires_character_font_resolution(
        &mut self,
        style: &TextStyle<'static, 'static, TextBrush>,
    ) -> bool {
        let plan_index = self.font_family_resolution_plan_index(style);
        self.font_family_resolution_plans[plan_index].requires_character_resolution
    }

    fn font_family_resolution_plan_index(
        &mut self,
        style: &TextStyle<'static, 'static, TextBrush>,
    ) -> usize {
        let attributes = Attributes::new(style.font_width, style.font_style, style.font_weight);
        if let Some(index) = self
            .font_family_resolution_plans
            .iter()
            .position(|plan| plan.requested == style.font_family && plan.attributes == attributes)
        {
            return index;
        }

        if self.font_family_resolution_plans.len() >= MAX_FONT_FAMILY_RESOLUTION_PLANS {
            self.font_family_resolution_plans.clear();
        }
        let requested = style.font_family.clone().into_owned();

        let family_uses_ranges = |family: &FontFamilyName<'_>| {
            let FontFamilyName::Named(name) = family else {
                return false;
            };
            self.web_font_families
                .get(&font_family_name_key(name))
                .is_some_and(|family| {
                    family.groups.iter().any(|group| {
                        group
                            .faces
                            .iter()
                            .any(|face| !face.unicode_ranges.is_empty())
                    })
                })
        };
        let requires_character_resolution = match &requested {
            FontFamily::Single(family) => family_uses_ranges(family),
            FontFamily::List(families) => families.iter().any(family_uses_ranges),
            FontFamily::Source(source) => FontFamilyName::parse_css_list(source)
                .filter_map(Result::ok)
                .any(|family| family_uses_ranges(&family)),
        };
        let index = self.font_family_resolution_plans.len();
        self.font_family_resolution_plans
            .push(FontFamilyResolutionPlan {
                requested,
                attributes,
                requires_character_resolution,
                default: None,
                characters: HashMap::new(),
            });
        index
    }

    fn resolve_segmented_web_font_families(
        &mut self,
        style: &mut TextStyle<'static, 'static, TextBrush>,
        character: Option<char>,
    ) {
        let parsed_source;
        let families = match &style.font_family {
            FontFamily::Single(family) => vec![family.clone()],
            FontFamily::List(families) => families.iter().cloned().collect(),
            FontFamily::Source(source) => {
                parsed_source = FontFamilyName::parse_css_list(source)
                    .filter_map(Result::ok)
                    .map(FontFamilyName::into_owned)
                    .collect::<Vec<_>>();
                parsed_source
            }
        };
        let attributes = Attributes::new(style.font_width, style.font_style, style.font_weight);
        let mut resolved = Vec::with_capacity(families.len());
        for family in families {
            let FontFamilyName::Named(name) = &family else {
                resolved.push(family);
                continue;
            };
            let family_key = font_family_name_key(name);
            if !self.web_font_families.contains_key(&family_key) {
                resolved.push(family);
                continue;
            }

            let groups = &self.web_font_families[&family_key].groups;
            let requested = font_selection_capabilities(attributes);
            let Some(best) = groups
                .iter()
                .min_by(|first, second| first.capabilities.compare(second.capabilities, requested))
            else {
                resolved.push(family);
                continue;
            };
            // Overlapping ranges can tie even when their endpoints differ.
            // Keep all best matches in this Document's FontFaceSet order.
            let mut faces = groups
                .iter()
                .filter(|group| {
                    group
                        .capabilities
                        .compare(best.capabilities, requested)
                        .is_eq()
                })
                .flat_map(|group| &group.faces)
                .collect::<Vec<_>>();
            faces.sort_by(|first, second| first.priority_key().cmp(&second.priority_key()));
            let default_uses_latin_metrics =
                character.is_none() && faces.iter().any(|face| face.contains('x'));
            resolved.extend(
                faces
                    .into_iter()
                    .rev()
                    .filter(|face| match character {
                        Some(character) => face.contains(character),
                        None if default_uses_latin_metrics => face.contains('x'),
                        None => true,
                    })
                    .map(|face| {
                        FontFamilyName::Named(Cow::Owned(face.internal_family_name.clone()))
                    }),
            );
        }
        style.font_family = FontFamily::List(Cow::Owned(resolved));
    }

    pub(crate) fn inline_font_metrics(
        &mut self,
        style: &TextStyle<'static, 'static, TextBrush>,
        sample: Option<char>,
    ) -> Option<InlineFontMetrics> {
        let cache_key = inline_font_metrics_cache_key(style);
        if let Some((_, metrics)) = self
            .inline_font_metrics_cache
            .iter()
            .find(|(cached, _)| cached == &cache_key)
        {
            // A font such as Baidu's icon font may not contain `x`. A later
            // call carrying one of the style's real characters must be able
            // to retry a previous sample-free miss.
            if metrics.is_some() || sample.is_none() {
                return *metrics;
            }
        }

        // Shape a character from the selected primary face rather than
        // borrowing metrics from an arbitrary fallback run. Most fonts cover
        // `x`; icon fonts often do not, so the owning text contributes one
        // additional candidate and the font identity verifies the result.
        let primary_font = self.primary_font_identity(style);
        let metrics = ['x'].into_iter().chain(sample).find_map(|candidate| {
            let candidate = candidate.to_string();
            let mut builder = self.layout_context.style_run_builder(
                &mut self.font_context,
                &candidate,
                1.0,
                true,
            );
            let style_index = builder.push_style(style.clone());
            builder.push_style_run(style_index, ..);
            let mut layout = builder.build(&candidate);
            layout.break_all_lines(None);
            let run = layout.lines().next()?.runs().next()?;
            if primary_font.is_some_and(|identity| {
                identity != (run.font().font.data.id(), run.font().font.index)
            }) {
                return None;
            }
            let metrics = *run.font_metrics();
            Some(InlineFontMetrics {
                ascent: metrics.ascent,
                descent: metrics.descent,
                line_height: run.line_height(),
                x_height: resolved_inline_x_height(metrics.ascent, metrics.x_height),
            })
        });
        if let Some((_, cached)) = self
            .inline_font_metrics_cache
            .iter_mut()
            .find(|(cached, _)| cached == &cache_key)
        {
            *cached = metrics;
        } else {
            if self.inline_font_metrics_cache.len() >= MAX_INLINE_FONT_METRICS_CACHE_ENTRIES {
                self.inline_font_metrics_cache.clear();
            }
            self.inline_font_metrics_cache.push((cache_key, metrics));
        }
        metrics
    }

    fn primary_font_identity(
        &mut self,
        style: &TextStyle<'static, 'static, TextBrush>,
    ) -> Option<(u64, u32)> {
        let parsed_source;
        let families = match &style.font_family {
            FontFamily::Single(family) => vec![family],
            FontFamily::List(families) => families.iter().collect(),
            FontFamily::Source(source) => {
                parsed_source = FontFamilyName::parse_css_list(source)
                    .filter_map(Result::ok)
                    .collect::<Vec<_>>();
                parsed_source.iter().collect()
            }
        };
        let query_families = families.iter().map(|family| match family {
            FontFamilyName::Named(name) => QueryFamily::Named(name),
            FontFamilyName::Generic(family) => QueryFamily::Generic(*family),
        });
        let FontContext {
            collection,
            source_cache,
        } = &mut self.font_context;
        let mut query = collection.query(source_cache);
        query.set_families(query_families);
        query.set_attributes(Attributes::new(
            style.font_width,
            style.font_style,
            style.font_weight,
        ));
        let mut identity = None;
        query.matches_with(|font| {
            identity = Some((font.blob.id(), font.index));
            QueryStatus::Stop
        });
        identity
    }
}

fn inline_font_metrics_cache_key(
    style: &TextStyle<'static, 'static, TextBrush>,
) -> TextStyle<'static, 'static, TextBrush> {
    TextStyle {
        font_family: style.font_family.clone().into_owned(),
        font_size: style.font_size,
        font_width: style.font_width,
        font_style: style.font_style,
        font_weight: style.font_weight,
        font_variations: style.font_variations.clone(),
        font_features: style.font_features.clone(),
        locale: style.locale,
        line_height: style.line_height,
        ..TextStyle::default()
    }
}

/// Whether a document's font collection may discover platform fonts.
///
/// Tests and deterministic differential runners disable this and register a
/// fixed web font set. Product documents enable it by default so CSS generic
/// families still have platform fallback when no downloadable font covers a
/// character.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SystemFontPolicy {
    /// Discover fonts through Fontique's platform backend.
    #[default]
    Enabled,
    /// Restrict shaping to explicitly registered web fonts.
    Disabled,
}

impl SystemFontPolicy {
    const fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

/// CSS `font-style` metadata attached to one downloadable font face.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WebFontStyle {
    Normal,
    Italic,
    Oblique(Option<f32>),
}

/// One inclusive CSS `unicode-range` interval attached to a downloadable
/// font face.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebFontUnicodeRange {
    start: u32,
    end: u32,
}

impl WebFontUnicodeRange {
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    pub const fn start(self) -> u32 {
        self.start
    }

    pub const fn end(self) -> u32 {
        self.end
    }

    pub const fn contains(self, character: char) -> bool {
        let codepoint = character as u32;
        self.start <= codepoint && codepoint <= self.end
    }

    const fn is_valid(self) -> bool {
        self.start <= self.end && self.end <= char::MAX as u32
    }
}

impl WebFontStyle {
    fn to_fontique(self) -> FontStyle {
        match self {
            Self::Normal => FontStyle::Normal,
            Self::Italic => FontStyle::Italic,
            Self::Oblique(angle) => FontStyle::Oblique(angle),
        }
    }
}

/// Fontique metadata overrides derived from a CSS `@font-face` rule.
///
/// `weight` uses the CSS numeric range and `stretch` is a percentage where
/// `100.0` is normal width. Missing descriptors leave the font's own metadata
/// intact.
#[derive(Clone, Debug, PartialEq)]
pub struct WebFontFace {
    family_name: String,
    family_name_key: String,
    weight: Option<f32>,
    stretch: Option<f32>,
    style: Option<WebFontStyle>,
    capabilities: Option<WebFontCapabilities>,
    unicode_ranges: Vec<WebFontUnicodeRange>,
}

impl WebFontFace {
    pub fn new(family_name: impl Into<String>) -> Self {
        let family_name = family_name.into();
        let family_name_key = font_family_name_key(&family_name);
        Self {
            family_name,
            family_name_key,
            weight: None,
            stretch: None,
            style: None,
            capabilities: None,
            unicode_ranges: Vec::new(),
        }
    }

    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = Some(weight);
        if let Some(capabilities) = &mut self.capabilities {
            capabilities.weight = FontSelectionRange::new(weight, weight);
        }
        self
    }

    pub fn with_stretch(mut self, percentage: f32) -> Self {
        self.stretch = Some(percentage);
        if let Some(capabilities) = &mut self.capabilities {
            capabilities.width = FontSelectionRange::new(percentage, percentage);
        }
        self
    }

    pub fn with_style(mut self, style: WebFontStyle) -> Self {
        self.style = Some(style);
        if let Some(capabilities) = &mut self.capabilities {
            let slope = match style {
                WebFontStyle::Normal => 0.0,
                WebFontStyle::Italic => 14.0,
                WebFontStyle::Oblique(angle) => angle.unwrap_or(14.0),
            };
            capabilities.slope = FontSelectionRange::new(slope, slope);
        }
        self
    }

    /// Retains full computed CSS capabilities for selection before shaping.
    /// Fontique's scalar overrides describe the physical internal family;
    /// they must not reduce a CSS range to one point for family selection.
    pub fn with_css_descriptors(mut self, weight: &str, stretch: &str, style: &str) -> Self {
        let capabilities = WebFontCapabilities::new(weight, stretch, style);
        self.weight = Some(capabilities.weight.min);
        self.stretch = Some(capabilities.width.min);
        self.style = Some(match style.split_ascii_whitespace().next() {
            Some("italic") => WebFontStyle::Italic,
            Some("oblique") => WebFontStyle::Oblique(Some(capabilities.slope.min)),
            _ => WebFontStyle::Normal,
        });
        self.capabilities = Some(capabilities);
        self
    }

    /// Sets the face's CSS `unicode-range` intervals. An empty list means the
    /// full Unicode range, matching the descriptor's initial value.
    pub fn with_unicode_ranges(
        mut self,
        ranges: impl IntoIterator<Item = WebFontUnicodeRange>,
    ) -> Self {
        self.unicode_ranges = ranges.into_iter().collect();
        self
    }

    pub fn family_name(&self) -> &str {
        &self.family_name
    }

    /// Chromium-compatible CSS family matching key, computed with the descriptor.
    pub fn family_name_key(&self) -> &str {
        &self.family_name_key
    }

    /// Full computed CSS ranges, when descriptors override font metadata.
    pub const fn css_capabilities(&self) -> Option<WebFontCapabilities> {
        self.capabilities
    }

    pub const fn weight(&self) -> Option<f32> {
        self.weight
    }

    pub const fn stretch(&self) -> Option<f32> {
        self.stretch
    }

    pub const fn style(&self) -> Option<WebFontStyle> {
        self.style
    }

    pub fn unicode_ranges(&self) -> &[WebFontUnicodeRange] {
        &self.unicode_ranges
    }

    fn validate(&self) -> Result<(), WebFontRegistrationError> {
        if self.family_name.trim().is_empty() {
            return Err(WebFontRegistrationError::InvalidDescriptor {
                detail: "font-family must not be empty".to_owned(),
            });
        }
        if self
            .weight
            .is_some_and(|value| !value.is_finite() || !(1.0..=1000.0).contains(&value))
        {
            return Err(WebFontRegistrationError::InvalidDescriptor {
                detail: "font-weight must be a finite value from 1 through 1000".to_owned(),
            });
        }
        if self
            .stretch
            .is_some_and(|value| !value.is_finite() || value <= 0.0)
        {
            return Err(WebFontRegistrationError::InvalidDescriptor {
                detail: "font-stretch must be a positive finite percentage".to_owned(),
            });
        }
        if self.style.is_some_and(
            |style| matches!(style, WebFontStyle::Oblique(Some(angle)) if !angle.is_finite()),
        ) {
            return Err(WebFontRegistrationError::InvalidDescriptor {
                detail: "font-style oblique angle must be finite".to_owned(),
            });
        }
        if self.unicode_ranges.iter().any(|range| !range.is_valid()) {
            return Err(WebFontRegistrationError::InvalidDescriptor {
                detail: "unicode-range intervals must be ordered Unicode scalar bounds".to_owned(),
            });
        }
        Ok(())
    }

    fn fontique_override_for_family<'a>(&self, family_name: &'a str) -> FontInfoOverride<'a> {
        FontInfoOverride {
            family_name: Some(family_name),
            width: self.stretch.map(FontWidth::from_percentage),
            style: self.style.map(WebFontStyle::to_fontique),
            weight: self.weight.map(FontWeight::new),
            axes: None,
        }
    }
}

/// One owner-validated downloadable font response.
///
/// `slot` is chosen by the stylesheet/resource owner. Reusing it replaces the
/// old face atomically after the new payload has decoded and validated.
#[derive(Clone, Debug, PartialEq)]
pub struct WebFontRegistration {
    slot: String,
    face: WebFontFace,
    payload: WebFontPayload,
    source_key: Option<String>,
    scope: WebFontRegistrationScope,
}

#[derive(Clone, Debug)]
enum WebFontPayload {
    Encoded(Vec<u8>),
    Validated(FontFaceData),
}

impl WebFontPayload {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Encoded(bytes) => bytes,
            Self::Validated(data) => data.bytes(),
        }
    }
}

impl PartialEq for WebFontPayload {
    fn eq(&self, other: &Self) -> bool {
        self.bytes() == other.bytes()
    }
}

impl WebFontRegistration {
    pub fn new(slot: impl Into<String>, face: WebFontFace, bytes: Vec<u8>) -> Self {
        Self {
            slot: slot.into(),
            face,
            payload: WebFontPayload::Encoded(bytes),
            source_key: None,
            scope: WebFontRegistrationScope::FontFaceSetManual,
        }
    }

    /// Retains already validated font data without copying or decoding its
    /// payload again. Descriptors are still validated for this registration.
    pub fn from_font_data(slot: impl Into<String>, face: WebFontFace, data: FontFaceData) -> Self {
        Self {
            slot: slot.into(),
            face,
            payload: WebFontPayload::Validated(data),
            source_key: None,
            scope: WebFontRegistrationScope::FontFaceSetManual,
        }
    }

    /// Restricts this registration to passes where its stylesheet declaration
    /// is active. Other native/programmatic registrations remain available.
    pub fn with_stylesheet_scope(mut self) -> Self {
        self.scope = WebFontRegistrationScope::Stylesheet;
        self
    }

    /// Identifies shareable source data, independently of this declaration's slot.
    pub fn with_source_key(mut self, source_key: Option<String>) -> Self {
        self.source_key = source_key;
        self
    }

    pub fn with_scope(mut self, scope: WebFontRegistrationScope) -> Self {
        self.scope = scope;
        self
    }

    pub fn slot(&self) -> &str {
        &self.slot
    }

    pub fn face(&self) -> &WebFontFace {
        &self.face
    }

    pub fn bytes(&self) -> &[u8] {
        self.payload.bytes()
    }
}

/// One declaration in the current native stylesheet snapshot. A new declaration
/// may use cached source data without reusing another face's mutable metadata.
pub struct WebFontDeclaration<'a> {
    pub slot: &'a str,
    pub source_key: &'a str,
    pub face: &'a WebFontFace,
}

/// The current registration relation in one Document, independently of where
/// the FontFace or its shared source was originally created.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebFontRegistrationScope {
    Stylesheet,
    FontFaceSetConnected,
    FontFaceSetManual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FontFaceSetMembership {
    Connected,
    Manual,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct WebFontMembership {
    stylesheet: bool,
    font_face_set: Option<FontFaceSetMembership>,
}

impl WebFontMembership {
    fn with(mut self, scope: WebFontRegistrationScope) -> Self {
        match scope {
            WebFontRegistrationScope::Stylesheet => self.stylesheet = true,
            WebFontRegistrationScope::FontFaceSetConnected => {
                self.font_face_set = Some(FontFaceSetMembership::Connected);
            }
            WebFontRegistrationScope::FontFaceSetManual => {
                self.font_face_set = Some(FontFaceSetMembership::Manual);
            }
        }
        self
    }

    fn is_empty(self) -> bool {
        !self.stylesheet && self.font_face_set.is_none()
    }

    fn is_stylesheet_only(self) -> bool {
        self.font_face_set != Some(FontFaceSetMembership::Manual)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebFontRegistrationOutcome {
    Added,
    Replaced,
    Unchanged,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum WebFontRegistrationError {
    #[error("web font slot must not be empty")]
    EmptySlot,
    #[error("invalid web font descriptor: {detail}")]
    InvalidDescriptor { detail: String },
    #[error("failed to decode {format} web font")]
    DecodeFailed { format: &'static str },
    #[error("web font payload contains no supported OpenType font")]
    UnsupportedPayload,
}

#[derive(Clone, Debug)]
struct RegisteredWebFont {
    face: WebFontFace,
    sfnt_bytes: Blob<u8>,
    source_key: Option<String>,
    membership: WebFontMembership,
}

impl RegisteredWebFont {
    fn priority(&self, slot: &str, order: &WebFontOrder) -> WebFontPriority {
        order.get(slot).copied().unwrap_or_else(|| {
            if self.membership.is_stylesheet_only() {
                WebFontPriority::stylesheet(usize::MAX)
            } else {
                WebFontPriority::manual(usize::MAX)
            }
        })
    }
}

/// Lazily initialized text resources reused by successive layout demands for
/// one committed Document.
///
/// The renderer owns this sidecar. A one-shot [`LayoutWorld`] only borrows the
/// contexts while building pass-local Parley layouts; neither context escapes
/// in [`crate::PaintSnapshot`].
pub struct DocumentLayoutServices {
    // FontContext and LayoutContext are both large. Keep them off the stack so
    // embedding this sidecar in ScriptVm does not inflate every VM frame.
    parley: Option<Box<ParleyDocumentServices>>,
    local_font_sources: Option<Box<font_source::LocalFontSources>>,
    system_font_policy: SystemFontPolicy,
    web_fonts: BTreeMap<String, RegisteredWebFont>,
    // Resource slots retain identity across removals and shared FontFaces.
    // Selection priority belongs to this Document's current FontFaceSet.
    web_font_order: WebFontOrder,
    has_font_face_set_order: bool,
    pub(crate) text_layout_passes: u64,
    #[cfg(test)]
    font_collection_rebuilds: usize,
}

/// Restores a temporary native pass's candidate order, including unwinding.
/// Physical Fontique families and both text contexts remain untouched.
struct ScopedWebFontOrder<'a> {
    services: &'a mut DocumentLayoutServices,
    previous: WebFontOrder,
}

impl Drop for ScopedWebFontOrder<'_> {
    fn drop(&mut self) {
        self.services.web_font_order = std::mem::take(&mut self.previous);
        self.services.refresh_web_font_priorities();
        if let Some(parley) = self.services.parley.as_mut() {
            parley.invalidate_font_selection();
        }
    }
}

impl Default for DocumentLayoutServices {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentLayoutServices {
    /// Creates an uninitialized document sidecar.
    pub const fn new() -> Self {
        Self {
            parley: None,
            local_font_sources: None,
            system_font_policy: SystemFontPolicy::Enabled,
            web_fonts: BTreeMap::new(),
            web_font_order: BTreeMap::new(),
            has_font_face_set_order: false,
            text_layout_passes: 0,
            #[cfg(test)]
            font_collection_rebuilds: 0,
        }
    }

    /// Creates a document sidecar with an explicit platform-font policy.
    pub const fn with_system_font_policy(system_font_policy: SystemFontPolicy) -> Self {
        Self {
            parley: None,
            local_font_sources: None,
            system_font_policy,
            web_fonts: BTreeMap::new(),
            web_font_order: BTreeMap::new(),
            has_font_face_set_order: false,
            text_layout_passes: 0,
            #[cfg(test)]
            font_collection_rebuilds: 0,
        }
    }

    pub const fn system_font_policy(&self) -> SystemFontPolicy {
        self.system_font_policy
    }

    /// Returns whether a demand with non-empty text has initialized Parley.
    pub const fn is_initialized(&self) -> bool {
        self.parley.is_some()
    }

    /// Counts text-bearing one-shot passes served by these reused contexts.
    pub const fn text_layout_passes(&self) -> u64 {
        self.text_layout_passes
    }

    pub fn web_font_count(&self) -> usize {
        self.web_fonts.len()
    }

    /// Publishes CSS-connected order followed by manual insertion order for
    /// this owner. Unloaded faces keep their place for a later completion.
    /// Returns true only when registered candidates actually change priority.
    pub fn set_web_font_order(&mut self, stylesheet: &[String], manual: &[String]) -> bool {
        self.has_font_face_set_order = true;
        let order = stylesheet
            .iter()
            .enumerate()
            .map(|(index, slot)| (slot.clone(), WebFontPriority::stylesheet(index)))
            .chain(
                manual
                    .iter()
                    .enumerate()
                    .map(|(index, slot)| (slot.clone(), WebFontPriority::manual(index))),
            )
            .collect::<BTreeMap<_, _>>();
        self.update_web_font_order(order)
    }

    /// Publishes native stylesheet order until a JS FontFaceSet is exposed.
    /// Its complete order also includes local-only CSS faces, which have no
    /// fetch-manifest entry. Later native fetch updates must preserve it.
    pub fn set_stylesheet_web_font_order(&mut self, stylesheet: &[String]) -> bool {
        if self.has_font_face_set_order {
            return false;
        }
        let order = stylesheet
            .iter()
            .enumerate()
            .map(|(index, slot)| (slot.clone(), WebFontPriority::stylesheet(index)))
            .collect();
        self.update_web_font_order(order)
    }

    fn update_web_font_order(&mut self, order: WebFontOrder) -> bool {
        if self.web_font_order == order {
            return false;
        }
        let previous = ordered_web_fonts(&self.web_fonts, &self.web_font_order)
            .into_iter()
            .map(|(slot, _)| slot.clone())
            .collect::<Vec<_>>();
        self.web_font_order = order;
        let current = ordered_web_fonts(&self.web_fonts, &self.web_font_order)
            .into_iter()
            .map(|(slot, _)| slot.clone())
            .collect::<Vec<_>>();
        // Physical families are private per slot. Changing CSS selection order
        // only updates candidate priorities, never Fontique registrations.
        self.refresh_web_font_priorities();
        let changed = current != previous;
        if changed && let Some(parley) = self.parley.as_mut() {
            parley.invalidate_font_selection();
        }
        changed
    }

    fn refresh_web_font_priorities(&mut self) {
        let Some(parley) = self.parley.as_mut() else {
            return;
        };
        for family in parley.web_font_families.values_mut() {
            for group in &mut family.groups {
                for face in &mut group.faces {
                    face.source_priority = self.web_fonts[&face.source_slot]
                        .priority(&face.source_slot, &self.web_font_order);
                }
                group
                    .faces
                    .sort_by(|first, second| first.priority_key().cmp(&second.priority_key()));
            }
        }
    }

    fn rebuild_font_collection(&mut self) {
        if self.parley.is_some() {
            self.parley = Some(Box::new(build_parley_services(
                self.system_font_policy,
                &self.web_fonts,
                &self.web_font_order,
            )));
            #[cfg(test)]
            {
                self.font_collection_rebuilds += 1;
            }
        }
    }

    /// Adds or replaces one owner-validated font face.
    ///
    /// This API performs no document/generation check: the resource owner must
    /// call it only after matching its stable document, rule/slot, and request
    /// identity. Invalid new bytes leave an existing slot untouched.
    pub fn register_web_font(
        &mut self,
        registration: WebFontRegistration,
    ) -> Result<WebFontRegistrationOutcome, WebFontRegistrationError> {
        if registration.slot.trim().is_empty() {
            return Err(WebFontRegistrationError::EmptySlot);
        }
        registration.face.validate()?;
        let membership = self
            .web_fonts
            .get(&registration.slot)
            .map_or(WebFontMembership::default(), |font| font.membership)
            .with(registration.scope);
        let (sfnt_bytes, validated) = match registration.payload {
            WebFontPayload::Encoded(bytes) => (decode_web_font_bytes(bytes)?, false),
            WebFontPayload::Validated(data) => (data.bytes, true),
        };
        let font = RegisteredWebFont {
            face: registration.face,
            sfnt_bytes,
            source_key: registration.source_key,
            membership,
        };
        if let Some(current) = self.web_fonts.get_mut(&registration.slot)
            && current.face == font.face
            && (current.sfnt_bytes.id() == font.sfnt_bytes.id()
                || current.sfnt_bytes.as_ref() == font.sfnt_bytes.as_ref())
        {
            if current.membership == font.membership && current.source_key == font.source_key {
                return Ok(WebFontRegistrationOutcome::Unchanged);
            }
            current.membership = font.membership;
            current.source_key = font.source_key;
            self.refresh_web_font_priorities();
            if let Some(parley) = self.parley.as_mut() {
                parley.invalidate_font_selection();
            }
            return Ok(WebFontRegistrationOutcome::Replaced);
        }
        let outcome = if self.web_fonts.contains_key(&registration.slot) {
            WebFontRegistrationOutcome::Replaced
        } else {
            WebFontRegistrationOutcome::Added
        };
        // Fontique's successful registration is the validation for an added
        // face. Keep it in the live collection instead of trial-registering in
        // a throwaway collection and then rebuilding every existing family.
        if outcome == WebFontRegistrationOutcome::Added
            && let Some(parley) = self.parley.as_mut()
        {
            if !validated {
                validate_sfnt_table_bounds(font.sfnt_bytes.as_ref())?;
            }
            parley.register_web_font(&registration.slot, &font, &self.web_font_order)?;
            self.web_fonts.insert(registration.slot, font);
            parley.invalidate_font_selection();
            return Ok(outcome);
        }
        if !validated {
            validate_registered_font(&font.face, font.sfnt_bytes.clone())?;
        }
        self.web_fonts.insert(registration.slot, font);
        // Replacing bytes/descriptors must rebuild: unregistering by Fontique
        // attributes could also remove another slot with the same attributes.
        self.rebuild_font_collection();
        Ok(outcome)
    }

    /// Removes a font slot. Returns whether a registered face was removed.
    pub fn remove_web_font(&mut self, slot: &str) -> bool {
        if self.web_fonts.remove(slot).is_none() {
            return false;
        }
        self.rebuild_font_collection();
        true
    }

    pub fn remove_stylesheet_web_font(&mut self, slot: &str) -> bool {
        self.remove_stylesheet_web_fonts([slot])
    }

    /// Retires a CSS manifest update in one collection transaction.
    pub fn remove_stylesheet_web_fonts<'a>(
        &mut self,
        slots: impl IntoIterator<Item = &'a str>,
    ) -> bool {
        self.remove_web_font_memberships(slots, WebFontRegistrationScope::Stylesheet)
    }

    pub fn remove_font_face_set_web_font(&mut self, slot: &str) -> bool {
        self.remove_font_face_set_web_fonts([slot])
    }

    /// Retires a FontFaceSet membership update in one collection transaction.
    pub fn remove_font_face_set_web_fonts<'a>(
        &mut self,
        slots: impl IntoIterator<Item = &'a str>,
    ) -> bool {
        self.remove_web_font_memberships(slots, WebFontRegistrationScope::FontFaceSetManual)
    }

    fn remove_web_font_memberships<'a>(
        &mut self,
        slots: impl IntoIterator<Item = &'a str>,
        scope: WebFontRegistrationScope,
    ) -> bool {
        let mut changed = false;
        let mut removed = false;
        for slot in slots {
            let Some(font) = self.web_fonts.get_mut(slot) else {
                continue;
            };
            let retired = match scope {
                WebFontRegistrationScope::Stylesheet => {
                    std::mem::take(&mut font.membership.stylesheet)
                }
                WebFontRegistrationScope::FontFaceSetConnected
                | WebFontRegistrationScope::FontFaceSetManual => {
                    font.membership.font_face_set.take().is_some()
                }
            };
            changed |= retired;
            if retired && font.membership.is_empty() {
                self.web_fonts.remove(slot);
                removed = true;
            }
        }
        if removed {
            self.rebuild_font_collection();
        } else if changed {
            self.refresh_web_font_priorities();
            if let Some(parley) = self.parley.as_mut() {
                parley.invalidate_font_selection();
            }
        }
        changed
    }

    pub fn has_stylesheet_web_fonts(&self) -> bool {
        self.web_fonts
            .values()
            .any(|font| font.membership.is_stylesheet_only())
    }

    /// Selects loaded CSS fonts for one pass without changing the document's
    /// resource registrations. Temporary media/viewports and synchronous rule
    /// changes must not discard bytes that another view can still use.
    ///
    /// When every registered CSS face is active, reuse the existing contexts.
    /// Otherwise the temporary service shares font bytes and drops its contexts
    /// on return (including an error result); only the pass counter is retained.
    pub fn with_active_stylesheet_fonts<R>(
        &mut self,
        is_active: impl Fn(&str) -> bool,
        consume: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.with_font_view(is_active, Vec::new(), self.web_font_order.clone(), consume)
    }

    /// Selects this pass's declaration identities and CSS order. Reinserted
    /// rules can reuse loaded source bytes while keeping independent metadata;
    /// temporary view/media changes do not mutate authoritative registrations.
    pub fn with_stylesheet_font_declarations<'a, R>(
        &mut self,
        declarations: impl IntoIterator<Item = WebFontDeclaration<'a>>,
        consume: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let mut active = std::collections::BTreeSet::new();
        let mut additions = Vec::new();
        let mut order = self.web_font_order.clone();
        order.retain(|_, priority| priority.group == WebFontPriorityGroup::Manual);
        for (index, declaration) in declarations.into_iter().enumerate() {
            active.insert(declaration.slot);
            order.insert(
                declaration.slot.to_owned(),
                WebFontPriority::stylesheet(index),
            );
            if self.web_fonts.contains_key(declaration.slot) {
                continue;
            }
            let Some(cached) = self.web_fonts.values().find(|font| {
                font.membership.stylesheet
                    && font.source_key.as_deref() == Some(declaration.source_key)
            }) else {
                continue;
            };
            // Only typed, validated descriptors may expose a cached payload.
            if declaration.face.validate().is_err() {
                continue;
            }
            additions.push((
                declaration.slot.to_owned(),
                RegisteredWebFont {
                    face: declaration.face.clone(),
                    sfnt_bytes: cached.sfnt_bytes.clone(),
                    source_key: cached.source_key.clone(),
                    membership: WebFontMembership::default()
                        .with(WebFontRegistrationScope::Stylesheet),
                },
            ));
        }
        self.with_font_view(|slot| active.contains(slot), additions, order, consume)
    }

    fn with_font_view<R>(
        &mut self,
        is_active: impl Fn(&str) -> bool,
        additions: Vec<(String, RegisteredWebFont)>,
        order: WebFontOrder,
        consume: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let excluded = self
            .web_fonts
            .iter()
            .filter_map(|(slot, font)| {
                (font.membership.is_stylesheet_only() && !is_active(slot)).then_some(slot.as_str())
            })
            .collect::<Vec<_>>();
        if excluded.is_empty() && additions.is_empty() {
            if order == self.web_font_order {
                return consume(self);
            }
            let previous = std::mem::replace(&mut self.web_font_order, order);
            self.refresh_web_font_priorities();
            if let Some(parley) = self.parley.as_mut() {
                parley.invalidate_font_selection();
            }
            let scoped = ScopedWebFontOrder {
                services: self,
                previous,
            };
            return consume(scoped.services);
        }
        // BTreeMap iteration keeps the excluded slots sorted. Font bytes are
        // shared Arcs; a temporary selection never copies their payloads.
        let web_fonts = self
            .web_fonts
            .iter()
            .filter(|(slot, _)| excluded.binary_search(&slot.as_str()).is_err())
            .map(|(slot, font)| (slot.clone(), font.clone()))
            .chain(additions)
            .collect();
        let mut temporary = Self {
            parley: None,
            local_font_sources: None,
            system_font_policy: self.system_font_policy,
            web_fonts,
            web_font_order: order,
            has_font_face_set_order: self.has_font_face_set_order,
            text_layout_passes: 0,
            #[cfg(test)]
            font_collection_rebuilds: 0,
        };
        let result = consume(&mut temporary);
        self.text_layout_passes = self
            .text_layout_passes
            .saturating_add(temporary.text_layout_passes);
        result
    }

    /// Resolves a CSS local() source by its actual full or PostScript name.
    /// This never substitutes a generic font or a document web-font alias.
    pub fn local_font_source(&mut self, name: &str) -> Option<FontFaceData> {
        self.local_font_sources
            .get_or_insert_with(|| {
                Box::new(font_source::LocalFontSources::new(
                    self.system_font_policy.is_enabled(),
                ))
            })
            .find(name)
    }

    pub(crate) fn parley_mut(&mut self) -> &mut ParleyDocumentServices {
        if self.parley.is_none() {
            self.parley = Some(Box::new(build_parley_services(
                self.system_font_policy,
                &self.web_fonts,
                &self.web_font_order,
            )));
        }
        self.parley.as_deref_mut().expect("Parley was initialized")
    }
}

fn ordered_web_fonts<'a>(
    fonts: &'a BTreeMap<String, RegisteredWebFont>,
    order: &WebFontOrder,
) -> Vec<(&'a String, &'a RegisteredWebFont)> {
    let mut fonts = fonts.iter().collect::<Vec<_>>();
    fonts.sort_by_key(|(slot, font)| font.priority(slot, order));
    fonts
}

fn build_parley_services(
    system_font_policy: SystemFontPolicy,
    web_fonts: &BTreeMap<String, RegisteredWebFont>,
    web_font_order: &WebFontOrder,
) -> ParleyDocumentServices {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: system_font_policy.is_enabled(),
    });
    let system_font_family_resolver = system_font_policy
        .is_enabled()
        .then(|| SystemFontFamilyResolver::new(&mut collection));
    let font_context = FontContext {
        collection,
        source_cache: Default::default(),
    };
    let mut services = ParleyDocumentServices {
        font_context,
        layout_context: LayoutContext::new(),
        system_font_family_resolver,
        web_font_families: BTreeMap::new(),
        font_family_resolution_plans: Vec::new(),
        inline_font_metrics_cache: Vec::new(),
        #[cfg(test)]
        font_family_resolution_miss_count: 0,
    };
    for (slot, font) in ordered_web_fonts(web_fonts, web_font_order) {
        services
            .register_web_font(slot, font, web_font_order)
            .expect("retained web fonts passed Fontique registration");
    }
    services
}

impl ParleyDocumentServices {
    /// Initial population and incremental arrivals use the owner's selection
    /// order. Stable resource identity never determines FontFaceSet priority.
    fn register_web_font(
        &mut self,
        slot: &str,
        font: &RegisteredWebFont,
        web_font_order: &WebFontOrder,
    ) -> Result<(), WebFontRegistrationError> {
        let blob = font.sfnt_bytes.clone();
        // Unlike an enumerated index, a stable slot never renames existing
        // families when an earlier-sorting resource finishes later.
        let internal_family_name = format!("\0moli-web-font:{slot}");
        let internal_fonts = self.font_context.collection.register_fonts(
            blob.clone(),
            Some(
                font.face
                    .fontique_override_for_family(&internal_family_name),
            ),
        );
        if internal_fonts.is_empty() {
            return Err(WebFontRegistrationError::UnsupportedPayload);
        }
        // CSS matching belongs to the document registry below. Registering a
        // second, public family alias would let Fontique's broader name matching
        // bypass a rejected CSS family and select that face during fallback.

        let source_priority = font.priority(slot, web_font_order);
        let family = self
            .web_font_families
            .entry(font.face.family_name_key().to_owned())
            .or_default();
        for (_, fonts) in internal_fonts {
            for selector_font in fonts {
                let capabilities = font.face.capabilities.unwrap_or_else(|| {
                    font_selection_capabilities(Attributes::new(
                        selector_font.width(),
                        selector_font.style(),
                        selector_font.weight(),
                    ))
                });
                let group_index = family
                    .groups
                    .iter()
                    .position(|group| group.capabilities == capabilities)
                    .unwrap_or_else(|| {
                        let index = family.groups.len();
                        family.groups.push(SegmentedWebFontCapabilityGroup {
                            capabilities,
                            faces: Vec::new(),
                        });
                        index
                    });
                let group = &mut family.groups[group_index];
                if let Err(index) = group
                    .faces
                    .binary_search_by(|face| face.priority_key().cmp(&(source_priority, slot)))
                {
                    group.faces.insert(
                        index,
                        SegmentedWebFontFace {
                            source_slot: slot.to_owned(),
                            source_priority,
                            internal_family_name: internal_family_name.clone(),
                            unicode_ranges: font.face.unicode_ranges.clone(),
                        },
                    );
                }
            }
        }
        Ok(())
    }

    fn invalidate_font_selection(&mut self) {
        self.font_family_resolution_plans.clear();
        self.inline_font_metrics_cache.clear();
        // A family which previously needed a platform substitution may now
        // have its own downloaded face. Keep the platform inventory, but do
        // not let the old substitution bypass that face.
        if let Some(resolver) = self.system_font_family_resolver.as_mut() {
            resolver.invalidate_substitutions();
        }
    }
}

fn validate_sfnt_table_bounds(sfnt_bytes: &[u8]) -> Result<(), WebFontRegistrationError> {
    // Fontique reads metadata lazily and can register a font whose unused
    // tables extend past the supplied bytes. Validate every directory entry
    // before accepting either a single SFNT font or a font collection.
    let file = read_fonts::FileRef::new(sfnt_bytes)
        .map_err(|_| WebFontRegistrationError::UnsupportedPayload)?;
    for font in file.fonts() {
        let font = font.map_err(|_| WebFontRegistrationError::UnsupportedPayload)?;
        for table in font.table_directory().table_records() {
            let start = table.offset() as usize;
            let end = start
                .checked_add(table.length() as usize)
                .ok_or(WebFontRegistrationError::UnsupportedPayload)?;
            if sfnt_bytes.get(start..end).is_none() {
                return Err(WebFontRegistrationError::UnsupportedPayload);
            }
        }
    }
    Ok(())
}

fn validate_registered_font(
    face: &WebFontFace,
    sfnt_bytes: Blob<u8>,
) -> Result<(), WebFontRegistrationError> {
    validate_sfnt_table_bounds(sfnt_bytes.as_ref())?;
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    (!collection
        .register_fonts(
            sfnt_bytes,
            Some(face.fontique_override_for_family(face.family_name())),
        )
        .is_empty())
    .then_some(())
    .ok_or(WebFontRegistrationError::UnsupportedPayload)
}

fn decode_web_font_bytes<'a>(
    bytes: impl Into<Cow<'a, [u8]>>,
) -> Result<Blob<u8>, WebFontRegistrationError> {
    let bytes = bytes.into();
    let decoded = match bytes.get(..4) {
        Some(b"wOFF") => wuff::decompress_woff1(&bytes)
            .map_err(|_| WebFontRegistrationError::DecodeFailed { format: "WOFF" })?,
        Some(b"wOF2") => {
            wuff::decompress_woff2_with_custom_brotli(&bytes, &mut decompress_woff2_brotli)
                .map_err(|_| WebFontRegistrationError::DecodeFailed { format: "WOFF2" })?
        }
        _ => bytes.into_owned(),
    };
    // Own the Vec, not an Arc<[u8]> made by copying it. An owned SFNT response
    // moves here unchanged; WOFF decoders likewise donate their output buffer.
    Ok(Blob::new(Arc::new(decoded)))
}

fn decompress_woff2_brotli(
    compressed: &[u8],
    expected_size: usize,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // wuff validates this exact size before calling us. A fixed output slice
    // preserves its decompression limit while sharing the native Brotli codec.
    let mut output = vec![0; expected_size];
    let written = brotlic::decompress(compressed, &mut output)?;
    if written != expected_size {
        return Err(wuff::WuffErr::GenericError.into());
    }
    Ok(output)
}

/// Validates a downloadable font payload without retaining it in a document.
///
/// `FontFace` BufferSource and data-URL inputs must use the same decoder and
/// OpenType registration checks as stylesheet-backed fonts. A MIME or magic
/// byte sniff alone accepts truncated WOFF/WOFF2 payloads that can never be
/// shaped.
/// Owned inputs transfer their buffer without a payload copy. Borrowed inputs
/// remain supported for callers which need to retain the original bytes.
pub fn validate_web_font_bytes<'a>(
    bytes: impl Into<Cow<'a, [u8]>>,
) -> Result<(), WebFontRegistrationError> {
    let face = WebFontFace::new("__moli-font-validation__");
    let sfnt_bytes = decode_web_font_bytes(bytes)?;
    validate_registered_font(&face, sfnt_bytes)
}

#[cfg(test)]
mod benchmarks;

#[cfg(test)]
mod tests {
    use super::*;

    mod registration;

    const TEST_TTF: &[u8] = include_bytes!("../tests/fixtures/moli-ahem.ttf");
    const TEST_CJK_TTF: &[u8] = include_bytes!("../tests/fixtures/moli-cjk.ttf");
    const TEST_WOFF: &[u8] = include_bytes!("../tests/fixtures/moli-ahem.woff");
    const TEST_WOFF2: &[u8] = include_bytes!("../tests/fixtures/moli-ahem.woff2");

    #[test]
    fn font_validation_checks_decoding_and_every_table_extent() {
        for bytes in [TEST_TTF, TEST_WOFF, TEST_WOFF2] {
            assert!(validate_web_font_bytes(bytes).is_ok());
            for size in [0, 4, 12, 44, bytes.len() - 10] {
                assert!(
                    validate_web_font_bytes(&bytes[..size]).is_err(),
                    "length {size}"
                );
            }
        }

        // Corrupt a table offset and length independently, including values
        // whose sum would overflow u32. Metadata-only registration must not
        // make an out-of-bounds table acceptable.
        for field in [20, 24] {
            let mut bytes = TEST_TTF.to_vec();
            bytes[field..field + 4].copy_from_slice(&u32::MAX.to_be_bytes());
            assert!(validate_web_font_bytes(&bytes).is_err());
        }
    }

    #[test]
    fn font_validation_checks_all_faces_in_a_collection() {
        let mut collection = b"ttcf\0\x01\0\0\0\0\0\x02\0\0\0\0\0\0\0\0".to_vec();
        for index in 0..2 {
            let offset = collection.len() as u32;
            collection[12 + index * 4..16 + index * 4].copy_from_slice(&offset.to_be_bytes());
            let mut font = TEST_TTF.to_vec();
            let count = u16::from_be_bytes([font[4], font[5]]) as usize;
            for table in 0..count {
                let start = 20 + table * 16;
                let relative = u32::from_be_bytes(font[start..start + 4].try_into().unwrap());
                font[start..start + 4].copy_from_slice(&(relative + offset).to_be_bytes());
            }
            collection.extend(font);
        }
        assert!(validate_web_font_bytes(&collection).is_ok());
        for index in 0..2 {
            let mut corrupt = collection.clone();
            let offset =
                u32::from_be_bytes(corrupt[12 + index * 4..16 + index * 4].try_into().unwrap())
                    as usize;
            corrupt[offset + 20..offset + 24].copy_from_slice(&u32::MAX.to_be_bytes());
            assert!(validate_web_font_bytes(&corrupt).is_err());
        }
    }

    #[test]
    fn missing_x_height_uses_the_blink_ascent_fallback() {
        assert!((resolved_inline_x_height(10.0, None) - 5.6).abs() < f32::EPSILON);
        assert_eq!(resolved_inline_x_height(10.0, Some(4.25)), 4.25);
    }

    #[test]
    fn standalone_web_font_validation_rejects_truncated_magic_payloads() {
        assert_eq!(
            validate_web_font_bytes(b"wOF2garbage"),
            Err(WebFontRegistrationError::DecodeFailed { format: "WOFF2" })
        );
        assert_eq!(
            validate_web_font_bytes(b"wOFFgarbage"),
            Err(WebFontRegistrationError::DecodeFailed { format: "WOFF" })
        );
        assert_eq!(
            validate_web_font_bytes(b"garbage"),
            Err(WebFontRegistrationError::UnsupportedPayload)
        );
        assert!(validate_web_font_bytes(TEST_WOFF2).is_ok());
    }

    #[test]
    fn repeated_font_family_resolution_reuses_the_document_plan() {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
        let style = TextStyle::default();
        let parley = services.parley_mut();

        assert!(!parley.requires_character_font_resolution(&style));
        assert!(!parley.requires_character_font_resolution(&style));
        for _ in 0..2 {
            let mut candidate = style.clone();
            parley.resolve_font_families(&mut candidate, None);
        }

        assert_eq!(parley.font_family_resolution_plans.len(), 1);
        assert_eq!(parley.font_family_resolution_miss_count, 1);
    }

    #[test]
    fn system_family_substitution_preserves_css_fallback_order() {
        let mut parley = build_parley_services(
            SystemFontPolicy::Disabled,
            &BTreeMap::new(),
            &BTreeMap::new(),
        );
        parley.system_font_family_resolver = Some(SystemFontFamilyResolver::new(
            &mut parley.font_context.collection,
        ));
        let face = WebFontFace::new("Moli Ahem");
        let registered = parley.font_context.collection.register_fonts(
            decode_web_font_bytes(TEST_TTF).unwrap(),
            Some(face.fontique_override_for_family(face.family_name())),
        );
        assert!(!registered.is_empty());
        let families = FontFamily::List(Cow::Owned(vec![
            FontFamilyName::Named(Cow::Borrowed("__moli_missing_css_family_order_3d16__")),
            FontFamilyName::Named(Cow::Borrowed("Moli Ahem")),
            FontFamilyName::Generic(parley::fontique::GenericFamily::Serif),
        ]));
        let mut style = TextStyle {
            font_family: families.clone(),
            ..TextStyle::default()
        };

        parley.resolve_font_families(&mut style, None);

        assert_eq!(style.font_family, families);
    }

    #[test]
    fn font_registration_discards_selection_caches_without_rebuilding_context() {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
        let context_address = std::ptr::from_ref(&services.parley_mut().font_context);
        {
            let parley = services.parley_mut();
            let mut style = TextStyle::default();
            parley.resolve_font_families(&mut style, None);
            let _ = parley.inline_font_metrics(&style, None);
            assert_eq!(parley.font_family_resolution_plans.len(), 1);
            assert_eq!(parley.inline_font_metrics_cache.len(), 1);
        }

        services
            .register_web_font(WebFontRegistration::new(
                "cache-reset",
                WebFontFace::new("Cache Reset"),
                TEST_TTF.to_vec(),
            ))
            .unwrap();
        let parley = services.parley_mut();
        assert!(parley.font_family_resolution_plans.is_empty());
        assert!(parley.inline_font_metrics_cache.is_empty());
        assert_eq!(parley.font_family_resolution_miss_count, 1);
        assert_eq!(context_address, std::ptr::from_ref(&parley.font_context));
    }

    #[test]
    fn inline_font_metrics_cache_ignores_paint_only_style_changes() {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
        let parley = services.parley_mut();
        let style = TextStyle::default();
        let _ = parley.inline_font_metrics(&style, None);
        let mut unpainted = style;
        unpainted.brush.paint = false;
        let _ = parley.inline_font_metrics(&unpainted, None);

        assert_eq!(parley.inline_font_metrics_cache.len(), 1);
    }

    #[test]
    fn primary_metrics_retry_with_an_owning_character_when_x_is_missing() {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
        services
            .register_web_font(WebFontRegistration::new(
                "cjk-face",
                WebFontFace::new("Moli CJK"),
                TEST_CJK_TTF.to_vec(),
            ))
            .unwrap();
        services
            .register_web_font(WebFontRegistration::new(
                "latin-face",
                WebFontFace::new("Moli Latin"),
                TEST_TTF.to_vec(),
            ))
            .unwrap();
        let mut style = TextStyle {
            font_family: FontFamily::List(std::borrow::Cow::Owned(vec![
                FontFamilyName::Named(std::borrow::Cow::Borrowed("Moli CJK")),
                FontFamilyName::Named(std::borrow::Cow::Borrowed("Moli Latin")),
            ])),
            ..TextStyle::default()
        };
        let parley = services.parley_mut();
        parley.resolve_font_families(&mut style, None);

        assert!(parley.inline_font_metrics(&style, None).is_none());
        assert!(
            parley.inline_font_metrics(&style, Some('中')).is_some(),
            "the style's actual glyph should recover metrics from a primary font without x"
        );
    }

    fn web_font_style(family: &'static str, weight: f32) -> TextStyle<'static, 'static, TextBrush> {
        TextStyle {
            font_family: FontFamily::Single(FontFamilyName::Named(Cow::Borrowed(family))),
            font_weight: FontWeight::new(weight),
            ..TextStyle::default()
        }
    }

    fn shape_one_character(
        parley: &mut ParleyDocumentServices,
        mut style: TextStyle<'static, 'static, TextBrush>,
        character: char,
    ) -> Vec<u8> {
        parley.resolve_font_families(&mut style, Some(character));
        let text = character.to_string();
        let mut builder =
            parley
                .layout_context
                .style_run_builder(&mut parley.font_context, &text, 1.0, true);
        let style_index = builder.push_style(style);
        builder.push_style_run(style_index, ..);
        let mut layout = builder.build(&text);
        layout.break_all_lines(None);
        layout
            .lines()
            .next()
            .expect("one line")
            .runs()
            .next()
            .expect("one shaped run")
            .font()
            .font
            .data
            .as_ref()
            .to_vec()
    }

    #[test]
    fn stylesheet_fetch_order_preserves_projected_local_font_selection() {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
        let narrow = include_bytes!("../tests/fixtures/moli-ahem-narrow.ttf");
        let wide = include_bytes!("../tests/fixtures/moli-ahem-wide.ttf");
        // Local-only CSS wrappers have a script resource slot rather than a
        // stylesheet fetch entry. The complete FontFaceSet gives their order.
        services
            .register_web_font(WebFontRegistration::new(
                "local-css",
                WebFontFace::new("CSS Local Order"),
                narrow.to_vec(),
            ))
            .unwrap();
        services
            .register_web_font(
                WebFontRegistration::new(
                    "url-css",
                    WebFontFace::new("CSS Local Order"),
                    wide.to_vec(),
                )
                .with_stylesheet_scope(),
            )
            .unwrap();
        services.set_web_font_order(&["local-css".into(), "url-css".into()], &[]);
        let selected = |services: &mut DocumentLayoutServices| {
            shape_one_character(
                services.parley_mut(),
                web_font_style("CSS Local Order", 400.0),
                'A',
            )
        };
        assert_eq!(selected(&mut services), wide);
        services.set_stylesheet_web_font_order(&["url-css".into(), "new-css".into()]);
        assert_eq!(selected(&mut services), wide);
        services.set_web_font_order(&["url-css".into(), "local-css".into()], &[]);
        assert_eq!(selected(&mut services), narrow);
        services.set_stylesheet_web_font_order(&["url-css".into()]);
        assert_eq!(selected(&mut services), narrow);
    }

    #[test]
    fn stylesheet_font_selection_preserves_native_faces_and_restores_after_error() {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
        services
            .register_web_font(WebFontRegistration::new(
                "native",
                WebFontFace::new("Native CJK"),
                TEST_CJK_TTF.to_vec(),
            ))
            .unwrap();
        services
            .register_web_font(
                WebFontRegistration::new("css", WebFontFace::new("CSS Latin"), TEST_TTF.to_vec())
                    .with_stylesheet_scope(),
            )
            .unwrap();
        assert_eq!(
            shape_one_character(
                services.parley_mut(),
                web_font_style("CSS Latin", 400.0),
                'R'
            ),
            TEST_TTF
        );
        let original = services.parley.as_deref().unwrap() as *const _;
        let error: Result<(), &str> = services.with_active_stylesheet_fonts(
            |_| false,
            |selected| {
                assert_eq!(
                    shape_one_character(
                        selected.parley_mut(),
                        web_font_style("Native CJK", 400.0),
                        '中'
                    ),
                    TEST_CJK_TTF
                );
                assert!(
                    !selected
                        .parley_mut()
                        .web_font_families
                        .contains_key("css latin")
                );
                Err("pass failed")
            },
        );
        assert_eq!(error, Err("pass failed"));
        services.with_active_stylesheet_fonts(
            |_| true,
            |selected| {
                assert_eq!(
                    selected.parley.as_deref().unwrap() as *const _,
                    original,
                    "matching passes reuse their text services"
                );
                assert_eq!(
                    shape_one_character(
                        selected.parley_mut(),
                        web_font_style("CSS Latin", 400.0),
                        'R'
                    ),
                    TEST_TTF
                );
            },
        );
        assert_eq!(services.web_font_count(), 2);
    }

    #[test]
    fn segmented_web_font_faces_select_the_range_for_each_character() {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
        services
            .register_web_font(WebFontRegistration::new(
                "latin",
                WebFontFace::new("Moli Segmented")
                    .with_unicode_ranges([WebFontUnicodeRange::new(0x0000, 0x00ff)]),
                TEST_TTF.to_vec(),
            ))
            .unwrap();
        services
            .register_web_font(WebFontRegistration::new(
                "cjk",
                WebFontFace::new("Moli Segmented")
                    .with_unicode_ranges([WebFontUnicodeRange::new(0x4e00, 0x9fff)]),
                TEST_CJK_TTF.to_vec(),
            ))
            .unwrap();
        let parley = services.parley_mut();

        assert_eq!(
            shape_one_character(parley, web_font_style("Moli Segmented", 400.0), 'R'),
            TEST_TTF
        );
        assert_eq!(
            shape_one_character(parley, web_font_style("moli segmented", 400.0), '中'),
            TEST_CJK_TTF,
            "CSS family matching and segmented range selection must both be case-insensitive"
        );
    }

    #[test]
    fn capability_matching_precedes_unicode_range_selection_like_blink() {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
        services
            .register_web_font(WebFontRegistration::new(
                "regular-latin",
                WebFontFace::new("Moli Capability First")
                    .with_weight(400.0)
                    .with_unicode_ranges([WebFontUnicodeRange::new(0x0000, 0x00ff)]),
                TEST_TTF.to_vec(),
            ))
            .unwrap();
        services
            .register_web_font(WebFontRegistration::new(
                "bold-cjk",
                WebFontFace::new("Moli Capability First")
                    .with_weight(700.0)
                    .with_unicode_ranges([WebFontUnicodeRange::new(0x4e00, 0x9fff)]),
                TEST_CJK_TTF.to_vec(),
            ))
            .unwrap();
        let parley = services.parley_mut();
        let mut style = web_font_style("Moli Capability First", 700.0);
        parley.resolve_font_families(&mut style, Some('R'));

        assert!(
            matches!(&style.font_family, FontFamily::List(families) if families.is_empty()),
            "the regular face must not be used after the bold capability group wins but does not cover the character"
        );
    }

    fn registration(slot: &str, family: &str, bytes: &[u8]) -> WebFontRegistration {
        WebFontRegistration::new(
            slot,
            WebFontFace::new(family)
                .with_weight(625.0)
                .with_stretch(87.5)
                .with_style(WebFontStyle::Italic),
            bytes.to_vec(),
        )
    }

    fn resolved_registered_family_name(
        parley: &mut ParleyDocumentServices,
        family: &str,
    ) -> Option<String> {
        let mut style = TextStyle {
            font_family: FontFamily::Single(FontFamilyName::Named(Cow::Owned(family.to_owned()))),
            ..TextStyle::default()
        };
        parley.resolve_segmented_web_font_families(&mut style, None);
        let FontFamily::List(families) = style.font_family else {
            return None;
        };
        families.iter().find_map(|family| match family {
            FontFamilyName::Named(name) => Some(name.to_string()),
            _ => None,
        })
    }

    fn has_family(services: &mut DocumentLayoutServices, family: &str) -> bool {
        let parley = services.parley_mut();
        let Some(family) = resolved_registered_family_name(parley, family) else {
            return false;
        };
        parley.font_context.collection.family_id(&family).is_some()
    }

    #[test]
    fn fixed_font_policy_registers_ttf_under_css_alias() {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
        assert_eq!(services.system_font_policy(), SystemFontPolicy::Disabled);
        assert!(!services.is_initialized());

        assert_eq!(
            services.register_web_font(registration("face-1", "Phase Three Alias", TEST_TTF)),
            Ok(WebFontRegistrationOutcome::Added)
        );
        assert_eq!(services.web_font_count(), 1);
        assert!(!services.is_initialized());
        assert!(has_family(&mut services, "Phase Three Alias"));
    }

    #[test]
    fn woff_and_woff2_are_decoded_before_fontique_registration() {
        for (slot, family, bytes) in [
            ("woff", "Moli WOFF", TEST_WOFF),
            ("woff2", "Moli WOFF2", TEST_WOFF2),
        ] {
            let mut services =
                DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
            assert_eq!(
                services.register_web_font(registration(slot, family, bytes)),
                Ok(WebFontRegistrationOutcome::Added),
                "{slot} should decode and register"
            );
            assert!(has_family(&mut services, family));
        }
    }

    #[test]
    fn woff2_brotli_requires_exact_decoded_size() {
        let compressed = [11, 2, 128, b'h', b'e', b'l', b'l', b'o', 3];
        assert_eq!(decompress_woff2_brotli(&compressed, 5).unwrap(), b"hello");
        for expected_size in [0, 4, 6] {
            assert!(
                decompress_woff2_brotli(&compressed, expected_size).is_err(),
                "decoded length must match {expected_size} exactly"
            );
        }
    }

    #[test]
    fn woff2_brotli_rejects_incomplete_and_corrupt_streams() {
        let compressed = [11, 2, 128, b'h', b'e', b'l', b'l', b'o', 3];
        for end in 0..compressed.len() {
            assert!(
                decompress_woff2_brotli(&compressed[..end], 5).is_err(),
                "a {end}-byte prefix must not decode successfully"
            );
        }
        assert!(decompress_woff2_brotli(&[0; 9], 5).is_err());
    }

    #[test]
    fn woff2_accepts_padding_inside_declared_compressed_block() {
        let expected = decode_web_font_bytes(TEST_WOFF2).unwrap();
        let compressed_size = u32::from_be_bytes(TEST_WOFF2[20..24].try_into().unwrap());
        assert_eq!(compressed_size, 573);
        // The fixture has three padding bytes after its Brotli stream. Some
        // encoders count this padding in totalCompressedSize; wuff accepts it.
        for padding in 1..=3 {
            let mut padded_font = TEST_WOFF2.to_vec();
            padded_font[20..24].copy_from_slice(&(compressed_size + padding).to_be_bytes());
            assert_eq!(
                decode_web_font_bytes(&padded_font).unwrap().as_ref(),
                expected.as_ref()
            );
        }
    }

    #[test]
    fn stable_slot_replacement_rebuilds_initialized_font_collection() {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
        services
            .register_web_font(registration("rule-7", "Old Alias", TEST_TTF))
            .unwrap();
        assert!(has_family(&mut services, "Old Alias"));

        assert_eq!(
            services.register_web_font(registration("rule-7", "New Alias", TEST_WOFF2)),
            Ok(WebFontRegistrationOutcome::Replaced)
        );
        assert!(!has_family(&mut services, "Old Alias"));
        assert!(has_family(&mut services, "New Alias"));
        assert_eq!(services.web_font_count(), 1);
    }

    #[test]
    fn invalid_replacement_does_not_poison_existing_slot() {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
        services
            .register_web_font(registration("rule-9", "Stable Alias", TEST_TTF))
            .unwrap();
        assert!(has_family(&mut services, "Stable Alias"));

        assert_eq!(
            services.register_web_font(registration("rule-9", "Broken Alias", b"not a font")),
            Err(WebFontRegistrationError::UnsupportedPayload)
        );
        assert!(has_family(&mut services, "Stable Alias"));
        assert!(!has_family(&mut services, "Broken Alias"));
        assert_eq!(services.web_font_count(), 1);
    }

    #[test]
    fn corrupt_woff2_returns_decode_error_and_preserves_existing_font() {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
        services
            .register_web_font(registration("woff2-slot", "Stable Alias", TEST_WOFF2))
            .unwrap();
        assert!(has_family(&mut services, "Stable Alias"));

        let mut corrupt_font = TEST_WOFF2.to_vec();
        // This fixture's Brotli stream occupies bytes 76..649. Preserve its
        // header and table directory so the failure comes from decompression.
        assert_eq!(&corrupt_font[20..24], &573_u32.to_be_bytes());
        corrupt_font[76..649].fill(0);
        assert_eq!(
            services.register_web_font(registration("woff2-slot", "Broken Alias", &corrupt_font,)),
            Err(WebFontRegistrationError::DecodeFailed { format: "WOFF2" })
        );
        assert!(has_family(&mut services, "Stable Alias"));
        assert!(!has_family(&mut services, "Broken Alias"));
        assert_eq!(services.web_font_count(), 1);
    }

    #[test]
    fn unchanged_registration_does_not_rebuild_and_remove_is_explicit() {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
        let registration = registration("rule-11", "Stable Alias", TEST_TTF);
        services.register_web_font(registration.clone()).unwrap();
        let font_context_address = std::ptr::from_ref(&services.parley_mut().font_context);

        assert_eq!(
            services.register_web_font(registration),
            Ok(WebFontRegistrationOutcome::Unchanged)
        );
        assert_eq!(
            font_context_address,
            std::ptr::from_ref(&services.parley_mut().font_context)
        );
        assert!(services.remove_web_font("rule-11"));
        assert!(!services.remove_web_font("rule-11"));
        assert!(!has_family(&mut services, "Stable Alias"));
    }

    #[test]
    fn descriptor_validation_rejects_non_css_metadata() {
        let mut services = DocumentLayoutServices::new();
        for face in [
            WebFontFace::new(""),
            WebFontFace::new("Bad Weight").with_weight(0.0),
            WebFontFace::new("Bad Stretch").with_stretch(f32::NAN),
            WebFontFace::new("Bad Style").with_style(WebFontStyle::Oblique(Some(f32::INFINITY))),
            WebFontFace::new("Bad Range")
                .with_unicode_ranges([WebFontUnicodeRange::new(0x100, 0xff)]),
            WebFontFace::new("Too High")
                .with_unicode_ranges([WebFontUnicodeRange::new(0, 0x11_0000)]),
        ] {
            let error = services
                .register_web_font(WebFontRegistration::new("slot", face, TEST_TTF.to_vec()))
                .unwrap_err();
            assert!(matches!(
                error,
                WebFontRegistrationError::InvalidDescriptor { .. }
            ));
        }
        assert_eq!(services.web_font_count(), 0);
    }
}
