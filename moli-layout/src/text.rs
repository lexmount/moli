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

use parley::{
    FontContext, FontFamily, FontFamilyName, LayoutContext, TextStyle,
    fontique::{
        Attributes, Blob, Collection, CollectionOptions, FontInfoOverride, FontStyle, FontWeight,
        FontWidth, QueryFamily, QueryStatus,
    },
};
use thiserror::Error;

use crate::stylo_to_parley::TextBrush;
use crate::system_fonts::SystemFontFamilyResolver;

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

#[derive(Clone, Copy, Debug, PartialEq)]
struct WebFontCapabilities {
    width: FontWidth,
    style: FontStyle,
    weight: FontWeight,
}

#[derive(Clone, Debug)]
struct SegmentedWebFontFace {
    internal_family_name: String,
    unicode_ranges: Vec<WebFontUnicodeRange>,
}

impl SegmentedWebFontFace {
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
    selector_font_identities: Vec<(u64, u32)>,
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
            resolver.resolve_text_style(&mut self.font_context.collection, style);
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
                .get(&normalized_web_font_family_name(name))
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
            let family_key = normalized_web_font_family_name(name);
            if !self.web_font_families.contains_key(&family_key) {
                resolved.push(family);
                continue;
            }

            let selected_identity = {
                let FontContext {
                    collection,
                    source_cache,
                } = &mut self.font_context;
                let mut query = collection.query(source_cache);
                query.set_families([QueryFamily::Named(name)]);
                query.set_attributes(attributes);
                let mut identity = None;
                query.matches_with(|font| {
                    identity = Some((font.blob.id(), font.index));
                    QueryStatus::Stop
                });
                identity
            };
            let Some(selected_identity) = selected_identity else {
                resolved.push(family);
                continue;
            };
            let Some(group) = self.web_font_families[&family_key]
                .groups
                .iter()
                .find(|group| group.selector_font_identities.contains(&selected_identity))
            else {
                resolved.push(family);
                continue;
            };
            let default_uses_latin_metrics =
                character.is_none() && group.faces.iter().any(|face| face.contains('x'));
            resolved.extend(
                group
                    .faces
                    .iter()
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
            if primary_font
                .is_some_and(|identity| identity != (run.font().data.id(), run.font().index))
            {
                return None;
            }
            let metrics = *run.metrics();
            Some(InlineFontMetrics {
                ascent: metrics.ascent,
                descent: metrics.descent,
                line_height: metrics.line_height,
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
    weight: Option<f32>,
    stretch: Option<f32>,
    style: Option<WebFontStyle>,
    unicode_ranges: Vec<WebFontUnicodeRange>,
}

impl WebFontFace {
    pub fn new(family_name: impl Into<String>) -> Self {
        Self {
            family_name: family_name.into(),
            weight: None,
            stretch: None,
            style: None,
            unicode_ranges: Vec::new(),
        }
    }

    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = Some(weight);
        self
    }

    pub fn with_stretch(mut self, percentage: f32) -> Self {
        self.stretch = Some(percentage);
        self
    }

    pub fn with_style(mut self, style: WebFontStyle) -> Self {
        self.style = Some(style);
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
    bytes: Vec<u8>,
}

impl WebFontRegistration {
    pub fn new(slot: impl Into<String>, face: WebFontFace, bytes: Vec<u8>) -> Self {
        Self {
            slot: slot.into(),
            face,
            bytes,
        }
    }

    pub fn slot(&self) -> &str {
        &self.slot
    }

    pub fn face(&self) -> &WebFontFace {
        &self.face
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
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

#[derive(Clone, Debug, PartialEq)]
struct RegisteredWebFont {
    face: WebFontFace,
    sfnt_bytes: Arc<[u8]>,
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
    system_font_policy: SystemFontPolicy,
    web_fonts: BTreeMap<String, RegisteredWebFont>,
    pub(crate) text_layout_passes: u64,
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
            system_font_policy: SystemFontPolicy::Enabled,
            web_fonts: BTreeMap::new(),
            text_layout_passes: 0,
        }
    }

    /// Creates a document sidecar with an explicit platform-font policy.
    pub const fn with_system_font_policy(system_font_policy: SystemFontPolicy) -> Self {
        Self {
            parley: None,
            system_font_policy,
            web_fonts: BTreeMap::new(),
            text_layout_passes: 0,
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
        let sfnt_bytes = decode_web_font_bytes(&registration.bytes)?;
        validate_registered_font(&registration.face, Arc::clone(&sfnt_bytes))?;
        let font = RegisteredWebFont {
            face: registration.face,
            sfnt_bytes,
        };
        let outcome = match self.web_fonts.get(&registration.slot) {
            Some(current) if current == &font => WebFontRegistrationOutcome::Unchanged,
            Some(_) => WebFontRegistrationOutcome::Replaced,
            None => WebFontRegistrationOutcome::Added,
        };
        if outcome == WebFontRegistrationOutcome::Unchanged {
            return Ok(outcome);
        }
        self.web_fonts.insert(registration.slot, font);
        if self.parley.is_some() {
            self.parley = Some(Box::new(build_parley_services(
                self.system_font_policy,
                &self.web_fonts,
            )));
        }
        Ok(outcome)
    }

    /// Removes a font slot. Returns whether a registered face was removed.
    pub fn remove_web_font(&mut self, slot: &str) -> bool {
        if self.web_fonts.remove(slot).is_none() {
            return false;
        }
        if self.parley.is_some() {
            self.parley = Some(Box::new(build_parley_services(
                self.system_font_policy,
                &self.web_fonts,
            )));
        }
        true
    }

    pub(crate) fn parley_mut(&mut self) -> &mut ParleyDocumentServices {
        if self.parley.is_none() {
            self.parley = Some(Box::new(build_parley_services(
                self.system_font_policy,
                &self.web_fonts,
            )));
        }
        self.parley.as_deref_mut().expect("Parley was initialized")
    }
}

fn build_parley_services(
    system_font_policy: SystemFontPolicy,
    web_fonts: &BTreeMap<String, RegisteredWebFont>,
) -> ParleyDocumentServices {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: system_font_policy.is_enabled(),
    });
    let system_font_family_resolver = system_font_policy
        .is_enabled()
        .then(|| SystemFontFamilyResolver::new(&mut collection));
    let mut font_context = FontContext {
        collection,
        source_cache: Default::default(),
    };
    let mut web_font_families = BTreeMap::<String, SegmentedWebFontFamily>::new();
    for (source_order, font) in web_fonts.values().enumerate() {
        let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(Arc::clone(&font.sfnt_bytes));
        let blob = Blob::new(data);
        let selector_fonts = font_context.collection.register_fonts(
            blob.clone(),
            Some(
                font.face
                    .fontique_override_for_family(font.face.family_name()),
            ),
        );
        if selector_fonts.is_empty() {
            continue;
        }

        let internal_family_name = format!("\0moli-web-font:{source_order}");
        if font_context
            .collection
            .register_fonts(
                blob.clone(),
                Some(
                    font.face
                        .fontique_override_for_family(&internal_family_name),
                ),
            )
            .is_empty()
        {
            continue;
        }

        let family = web_font_families
            .entry(normalized_web_font_family_name(font.face.family_name()))
            .or_default();
        for (_, fonts) in selector_fonts {
            for selector_font in fonts {
                let capabilities = WebFontCapabilities {
                    width: selector_font.width(),
                    style: selector_font.style(),
                    weight: selector_font.weight(),
                };
                let group_index = family
                    .groups
                    .iter()
                    .position(|group| group.capabilities == capabilities)
                    .unwrap_or_else(|| {
                        let index = family.groups.len();
                        family.groups.push(SegmentedWebFontCapabilityGroup {
                            capabilities,
                            selector_font_identities: Vec::new(),
                            faces: Vec::new(),
                        });
                        index
                    });
                let group = &mut family.groups[group_index];
                group
                    .selector_font_identities
                    .push((blob.id(), selector_font.index()));
                if !group
                    .faces
                    .iter()
                    .any(|face| face.internal_family_name == internal_family_name)
                {
                    group.faces.push(SegmentedWebFontFace {
                        internal_family_name: internal_family_name.clone(),
                        unicode_ranges: font.face.unicode_ranges.clone(),
                    });
                }
            }
        }
    }
    ParleyDocumentServices {
        font_context,
        layout_context: LayoutContext::new(),
        system_font_family_resolver,
        web_font_families,
        font_family_resolution_plans: Vec::new(),
        inline_font_metrics_cache: Vec::new(),
        #[cfg(test)]
        font_family_resolution_miss_count: 0,
    }
}

fn register_font(font_context: &mut FontContext, font: &RegisteredWebFont) -> bool {
    let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(Arc::clone(&font.sfnt_bytes));
    !font_context
        .collection
        .register_fonts(
            Blob::new(data),
            Some(
                font.face
                    .fontique_override_for_family(font.face.family_name()),
            ),
        )
        .is_empty()
}

fn normalized_web_font_family_name(name: &str) -> String {
    name.chars().flat_map(char::to_lowercase).collect()
}

fn validate_registered_font(
    face: &WebFontFace,
    sfnt_bytes: Arc<[u8]>,
) -> Result<(), WebFontRegistrationError> {
    let mut font_context = FontContext {
        collection: Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        }),
        source_cache: Default::default(),
    };
    let font = RegisteredWebFont {
        face: face.clone(),
        sfnt_bytes,
    };
    register_font(&mut font_context, &font)
        .then_some(())
        .ok_or(WebFontRegistrationError::UnsupportedPayload)
}

fn decode_web_font_bytes(bytes: &[u8]) -> Result<Arc<[u8]>, WebFontRegistrationError> {
    let decoded = match bytes.get(..4) {
        Some(b"wOFF") => wuff::decompress_woff1(bytes)
            .map_err(|_| WebFontRegistrationError::DecodeFailed { format: "WOFF" })?,
        Some(b"wOF2") => wuff::decompress_woff2(bytes)
            .map_err(|_| WebFontRegistrationError::DecodeFailed { format: "WOFF2" })?,
        _ => bytes.to_vec(),
    };
    Ok(Arc::from(decoded))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_TTF: &[u8] = include_bytes!("../tests/fixtures/moli-ahem.ttf");
    const TEST_CJK_TTF: &[u8] = include_bytes!("../tests/fixtures/moli-cjk.ttf");
    const TEST_WOFF: &[u8] = include_bytes!("../tests/fixtures/moli-ahem.woff");
    const TEST_WOFF2: &[u8] = include_bytes!("../tests/fixtures/moli-ahem.woff2");

    #[test]
    fn missing_x_height_uses_the_blink_ascent_fallback() {
        assert!((resolved_inline_x_height(10.0, None) - 5.6).abs() < f32::EPSILON);
        assert_eq!(resolved_inline_x_height(10.0, Some(4.25)), 4.25);
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
    fn font_collection_rebuild_discards_resolution_and_metrics_caches() {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
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
        assert_eq!(parley.font_family_resolution_miss_count, 0);
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
        let style = TextStyle {
            font_family: FontFamily::List(std::borrow::Cow::Owned(vec![
                FontFamilyName::Named(std::borrow::Cow::Borrowed("Moli CJK")),
                FontFamilyName::Named(std::borrow::Cow::Borrowed("Moli Latin")),
            ])),
            ..TextStyle::default()
        };
        let parley = services.parley_mut();

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
            .data
            .as_ref()
            .to_vec()
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

    fn has_family(services: &mut DocumentLayoutServices, family: &str) -> bool {
        services
            .parley_mut()
            .font_context
            .collection
            .family_id(family)
            .is_some()
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
