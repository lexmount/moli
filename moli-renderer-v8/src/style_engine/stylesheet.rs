use std::{
    collections::HashSet,
    ptr::NonNull,
    sync::{Arc as StdArc, LazyLock},
};

use euclid::{Scale, Size2D};
use moli_selector::StyloSourceDependencySummary;
use style::{
    context::QuirksMode,
    device::{Device, servo::FontMetricsProvider},
    font_face::{
        FontFaceRule, FontFaceSourceFormat, FontFaceSourceFormatKeyword, Source, SourceList,
    },
    font_metrics::FontMetrics,
    properties::{ComputedValues, style_structs::Font},
    servo::media_features::PointerCapabilities,
    servo_arc::Arc as ServoArc,
    shared_lock::{Locked, SharedRwLock, ToCssWithGuard},
    stylesheets::{
        AllowImportRules, CssRule, CustomMediaMap, DocumentStyleSheet, Origin, Stylesheet,
        StylesheetInDocument, UrlExtraData, scope_rule::ImplicitScopeRoot,
    },
    stylist::{CascadeData, Stylist},
    values::{
        computed::{
            CSSPixelLength, Length,
            font::{GenericFontFamily, SingleFontFamily},
        },
        specified::font::QueryFontMetricsFlags,
    },
};
use style_traits::{CSSPixel, CssWriter, DevicePixel, ToCss};

use crate::{document_runtime::DomHandle, dom::native::DomHost};

use super::{
    StyleViewport, StyloStyleEnvironment,
    active_stylesheets::{ActiveStylesheet, ActiveStylesheetCollection, ActiveWebFontResource},
    media_list::parse_media_query_list_with_context,
    source::store::{StyleSourceMetadata, StyloStylesheetSource},
    source_id::{StyleSourceId, StyleSourceKind},
    ua::HTML_STYLESHEET as MOLI_UA_STYLESHEET,
    world_key::{DEFAULT_VIEWPORT_HEIGHT, DEFAULT_VIEWPORT_WIDTH},
};

static MOLI_UA_STYLESHEET_BASE_URL: LazyLock<url::Url> =
    LazyLock::new(|| url::Url::parse("about:blank").expect("valid built-in stylesheet base URL"));

static MOLI_UA_SOURCE_METADATA: LazyLock<StyleSourceMetadata> = LazyLock::new(|| {
    style_source_metadata_for_css_text_with_origin(
        MOLI_UA_STYLESHEET,
        moli_ua_stylesheet_base_url(),
        Origin::UserAgent,
    )
});

static MOLI_UA_SOURCE_DEPENDENCY_SUMMARY: LazyLock<StdArc<StyloSourceDependencySummary>> =
    LazyLock::new(|| StdArc::new(MOLI_UA_SOURCE_METADATA.dependency_summary.clone()));

#[cfg(test)]
thread_local! {
    static AUTHOR_SOURCE_TEXT_PARSE_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
pub(crate) fn reset_author_source_text_parse_count_for_test() {
    AUTHOR_SOURCE_TEXT_PARSE_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn author_source_text_parse_count_for_test() -> usize {
    AUTHOR_SOURCE_TEXT_PARSE_COUNT.with(std::cell::Cell::get)
}

pub(super) fn moli_ua_stylesheet_base_url() -> &'static url::Url {
    &MOLI_UA_STYLESHEET_BASE_URL
}

#[derive(Debug)]
struct HeadlessFontMetricsProvider;

pub(super) fn install_active_stylesheets(
    host: &DomHost,
    shared_lock: &SharedRwLock,
    sources: &[StyloStylesheetSource],
    quirks_mode: QuirksMode,
) -> ActiveStylesheetCollection {
    ActiveStylesheetCollection::new(
        sources
            .iter()
            .map(|source| install_active_stylesheet(host, shared_lock, source, quirks_mode))
            .collect(),
    )
}

pub(super) fn install_active_stylesheet(
    host: &DomHost,
    shared_lock: &SharedRwLock,
    source: &StyloStylesheetSource,
    quirks_mode: QuirksMode,
) -> ActiveStylesheet {
    let stylesheet = author_stylesheet_for_source(shared_lock, source, quirks_mode);
    let web_font_resources = native_font_face_rules_for_stylesheet(&stylesheet)
        .into_iter()
        .filter_map(|rule| {
            let resource =
                crate::css_resource_urls::stylesheet_web_font_resource_with_resolved_url(
                    &rule.rule_fingerprint,
                    rule.request_url?,
                )?;
            Some(ActiveWebFontResource::new(rule.rule, resource))
        })
        .collect::<Vec<_>>();
    ActiveStylesheet::new(
        source.clone(),
        document_stylesheet_for_source(host, source, stylesheet),
        StdArc::from(web_font_resources),
        source.import_urls(),
    )
}

pub(super) fn style_source_metadata_for_css_text(
    css_text: &str,
    base_url: &url::Url,
) -> StyleSourceMetadata {
    style_source_metadata_for_css_text_with_origin(css_text, base_url, Origin::Author)
}

pub(in crate::style_engine) fn style_source_metadata_for_stylesheet(
    stylesheet: &ServoArc<Stylesheet>,
) -> StyleSourceMetadata {
    let guard = stylesheet.shared_lock.read();
    let quirks_mode = stylesheet.contents.read_with(&guard).quirks_mode;
    let stylist = new_stylist_with_viewport_bits(
        DEFAULT_VIEWPORT_WIDTH.to_bits(),
        DEFAULT_VIEWPORT_HEIGHT.to_bits(),
        DEFAULT_VIEWPORT_WIDTH.to_bits(),
        DEFAULT_VIEWPORT_HEIGHT.to_bits(),
        StyloStyleEnvironment::default(),
        quirks_mode,
    );
    let mut cascade_data = CascadeData::new();
    let document_stylesheet = DocumentStyleSheet::new(stylesheet.clone());
    if cascade_data
        .add_stylesheet_for_moli_source_metadata(
            stylist.device(),
            quirks_mode,
            &document_stylesheet,
            0,
            &guard,
        )
        .is_err()
    {
        return StyleSourceMetadata::default();
    }
    style_source_metadata_from_cascade_data(&cascade_data)
}

#[derive(Clone, Debug)]
pub(crate) struct StylesheetFontFaceRuleProjection {
    pub(crate) rule_identity: u64,
    pub(crate) rule_fingerprint: String,
    pub(crate) descriptor: moli_css_parse::CssFontFace,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct StylesheetFontFaceProjection {
    pub(crate) all_rules: Vec<StylesheetFontFaceRuleProjection>,
    pub(crate) effective_rule_identities: HashSet<u64>,
}

#[derive(Clone, Debug)]
pub(crate) struct NativeStylesheetFontFaceRuleProjection {
    pub(crate) rule: ServoArc<Locked<FontFaceRule>>,
    pub(crate) rule_fingerprint: String,
    pub(crate) descriptor: moli_css_parse::CssFontFace,
    /// First supported network source, resolved by Stylo in the exact parser
    /// context that owns this rule. Imported rules therefore retain the base
    /// URL of their imported stylesheet rather than inheriting the root base.
    pub(crate) request_url: Option<url::Url>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct NativeStylesheetFontFaceProjection {
    pub(crate) all_rules: Vec<NativeStylesheetFontFaceRuleProjection>,
    pub(crate) effective_rule_addresses: HashSet<usize>,
}

pub(crate) fn native_font_face_projection_for_stylesheet(
    stylesheet: &ServoArc<Stylesheet>,
    environment: StyloStyleEnvironment,
    viewport: StyleViewport,
) -> NativeStylesheetFontFaceProjection {
    let guard = stylesheet.shared_lock.read();
    let quirks_mode = stylesheet.contents.read_with(&guard).quirks_mode;
    let viewport_width = viewport.width.unwrap_or(DEFAULT_VIEWPORT_WIDTH as f64) as f32;
    let viewport_height = viewport.height.unwrap_or(DEFAULT_VIEWPORT_HEIGHT as f64) as f32;
    let screen_width = viewport.screen_width.unwrap_or(f64::from(viewport_width)) as f32;
    let screen_height = viewport.screen_height.unwrap_or(f64::from(viewport_height)) as f32;
    let stylist = new_stylist_with_viewport_bits(
        viewport_width.to_bits(),
        viewport_height.to_bits(),
        screen_width.to_bits(),
        screen_height.to_bits(),
        environment,
        quirks_mode,
    );

    let mut projection = NativeStylesheetFontFaceProjection::default();
    let contents = stylesheet.contents(&guard);
    collect_font_face_rule_projections(contents.rules(&guard), &guard, &mut projection.all_rules);

    let document_stylesheet = DocumentStyleSheet::new(stylesheet.clone());
    projection.effective_rule_addresses = native_effective_font_face_rule_addresses_with_guard(
        &document_stylesheet,
        stylist.device(),
        &CustomMediaMap::default(),
        &guard,
    );
    projection
}

pub(super) fn native_effective_font_face_rule_addresses(
    stylesheet: &DocumentStyleSheet,
    device: &Device,
    custom_media: &CustomMediaMap,
) -> HashSet<usize> {
    let guard = stylesheet.0.shared_lock.read();
    native_effective_font_face_rule_addresses_with_guard(stylesheet, device, custom_media, &guard)
}

fn native_effective_font_face_rule_addresses_with_guard(
    stylesheet: &DocumentStyleSheet,
    device: &Device,
    custom_media: &CustomMediaMap,
    guard: &style::shared_lock::SharedRwLockReadGuard<'_>,
) -> HashSet<usize> {
    if !stylesheet.enabled() || !stylesheet.is_effective_for_device(device, custom_media, guard) {
        return HashSet::new();
    }
    stylesheet
        .contents(guard)
        .effective_rules(device, custom_media, guard)
        .filter_map(|rule| match rule {
            CssRule::FontFace(rule) => Some(rule.raw_ptr().as_ptr() as usize),
            _ => None,
        })
        .collect()
}

pub(crate) fn native_font_face_rules_for_stylesheet(
    stylesheet: &ServoArc<Stylesheet>,
) -> Vec<NativeStylesheetFontFaceRuleProjection> {
    let guard = stylesheet.shared_lock.read();
    let mut rules = Vec::new();
    collect_font_face_rule_projections(
        stylesheet.contents(&guard).rules(&guard),
        &guard,
        &mut rules,
    );
    rules
}

fn collect_font_face_rule_projections(
    rules: &[CssRule],
    guard: &style::shared_lock::SharedRwLockReadGuard<'_>,
    projections: &mut Vec<NativeStylesheetFontFaceRuleProjection>,
) {
    for rule in rules {
        if let CssRule::FontFace(rule) = rule {
            let locked_rule = rule.read_with(guard);
            let Some(family) = locked_rule.descriptors.font_family.as_ref() else {
                continue;
            };
            let Some(source) = locked_rule.descriptors.src.as_ref() else {
                continue;
            };
            let mut serialized_source = String::new();
            if source
                .to_css(&mut CssWriter::new(&mut serialized_source))
                .is_ok()
            {
                projections.push(NativeStylesheetFontFaceRuleProjection {
                    rule: rule.clone(),
                    rule_fingerprint: locked_rule.to_css_string(guard),
                    descriptor: moli_css_parse::CssFontFace {
                        family: family.name.to_string(),
                        source: serialized_source,
                    },
                    request_url: preferred_native_font_source_url(source),
                });
            }
            continue;
        }
        collect_font_face_rule_projections(rule.children(guard), guard, projections);
    }
}

fn preferred_native_font_source_url(source_list: &SourceList) -> Option<url::Url> {
    source_list.0.iter().find_map(|source| {
        let Source::Url(source) = source else {
            return None;
        };
        if !native_font_source_format_is_supported(source.format_hint.as_ref()) {
            return None;
        }
        let url = source.url.url()?;
        matches!(url.scheme(), "http" | "https" | "data" | "blob").then(|| (**url).clone())
    })
}

fn native_font_source_format_is_supported(format: Option<&FontFaceSourceFormat>) -> bool {
    match format {
        None => true,
        Some(FontFaceSourceFormat::Keyword(format)) => matches!(
            format,
            FontFaceSourceFormatKeyword::Collection
                | FontFaceSourceFormatKeyword::Opentype
                | FontFaceSourceFormatKeyword::Truetype
                | FontFaceSourceFormatKeyword::Woff
                | FontFaceSourceFormatKeyword::Woff2
        ),
        Some(FontFaceSourceFormat::String(format)) => matches!(
            format.to_ascii_lowercase().as_str(),
            "collection" | "opentype" | "otf" | "truetype" | "ttf" | "woff" | "woff2"
        ),
    }
}

pub(super) fn moli_user_agent_source_dependency_summary() -> StdArc<StyloSourceDependencySummary> {
    StdArc::clone(&MOLI_UA_SOURCE_DEPENDENCY_SUMMARY)
}

fn style_source_metadata_for_css_text_with_origin(
    css_text: &str,
    base_url: &url::Url,
    origin: Origin,
) -> StyleSourceMetadata {
    let shared_lock = SharedRwLock::new();
    let stylesheet = DocumentStyleSheet::new(ServoArc::new(parse_stylesheet(
        &shared_lock,
        base_url,
        css_text,
        origin,
        QuirksMode::NoQuirks,
        "",
    )));
    let guard = shared_lock.read();
    let stylist = new_stylist_with_viewport_bits(
        DEFAULT_VIEWPORT_WIDTH.to_bits(),
        DEFAULT_VIEWPORT_HEIGHT.to_bits(),
        DEFAULT_VIEWPORT_WIDTH.to_bits(),
        DEFAULT_VIEWPORT_HEIGHT.to_bits(),
        StyloStyleEnvironment::default(),
        QuirksMode::NoQuirks,
    );
    let mut cascade_data = CascadeData::new();
    if cascade_data
        .add_stylesheet_for_moli_source_metadata(
            stylist.device(),
            QuirksMode::NoQuirks,
            &stylesheet,
            0,
            &guard,
        )
        .is_err()
    {
        return StyleSourceMetadata::default();
    }
    style_source_metadata_from_cascade_data(&cascade_data)
}

fn style_source_metadata_from_cascade_data(cascade_data: &CascadeData) -> StyleSourceMetadata {
    StyleSourceMetadata {
        dependency_summary: StyloSourceDependencySummary::from_cascade_data(cascade_data),
    }
}

pub(super) fn new_stylist_with_viewport_bits(
    viewport_width_bits: u32,
    viewport_height_bits: u32,
    screen_width_bits: u32,
    screen_height_bits: u32,
    environment: StyloStyleEnvironment,
    quirks_mode: QuirksMode,
) -> Stylist {
    Stylist::new(
        new_style_device_with_viewport_bits(
            viewport_width_bits,
            viewport_height_bits,
            screen_width_bits,
            screen_height_bits,
            environment,
            quirks_mode,
        ),
        quirks_mode,
    )
}

pub(super) fn new_style_device_with_viewport_bits(
    viewport_width_bits: u32,
    viewport_height_bits: u32,
    screen_width_bits: u32,
    screen_height_bits: u32,
    environment: StyloStyleEnvironment,
    quirks_mode: QuirksMode,
) -> Device {
    let width = f32::from_bits(viewport_width_bits);
    let height = f32::from_bits(viewport_height_bits);
    let screen_width = f32::from_bits(screen_width_bits);
    let screen_height = f32::from_bits(screen_height_bits);
    let initial_style = ComputedValues::initial_values_with_font_override(Font::initial_values());
    let mut device = Device::new(
        environment.stylo_media_type(),
        quirks_mode,
        Size2D::<f32, CSSPixel>::new(width, height),
        Size2D::<f32, DevicePixel>::new(screen_width, screen_height),
        Scale::<f32, CSSPixel, DevicePixel>::new(1.0),
        Box::new(HeadlessFontMetricsProvider),
        initial_style,
        environment.stylo_prefers_color_scheme(),
        PointerCapabilities::default(),
        PointerCapabilities::default(),
    );
    device.set_media_feature_preferences(environment.stylo_media_feature_preferences());
    device.set_page_color_schemes(environment.stylo_page_color_schemes());
    device
}

fn document_stylesheet_for_source(
    host: &DomHost,
    source: &StyloStylesheetSource,
    stylesheet: ServoArc<Stylesheet>,
) -> DocumentStyleSheet {
    implicit_scope_root_for_source(host, source.source_id())
        .map(|root| DocumentStyleSheet::with_implicit_scope_root(stylesheet.clone(), root))
        .unwrap_or_else(|| DocumentStyleSheet::new(stylesheet))
}

fn implicit_scope_root_for_source(
    host: &DomHost,
    source_id: Option<&StyleSourceId>,
) -> Option<ImplicitScopeRoot> {
    let source_id = source_id?;
    match &source_id.kind {
        StyleSourceKind::OwnerStyleSheet { owner }
        | StyleSourceKind::LinkedStyleSheet { owner } => {
            implicit_scope_root_for_stylesheet_owner(host, *owner)
        }
        StyleSourceKind::DocumentAdoptedStyleSheet { .. }
        | StyleSourceKind::ShadowRootAdoptedStyleSheet { .. } => {
            Some(ImplicitScopeRoot::Constructed)
        }
    }
}

fn implicit_scope_root_for_stylesheet_owner(
    host: &DomHost,
    owner: DomHandle,
) -> Option<ImplicitScopeRoot> {
    let parent = host.node(owner)?.parent_node()?;
    if host.is_shadow_root(parent) {
        return host
            .shadow_root_host(parent)
            .and_then(|host_element| opaque_element_for_handle(host, host_element))
            .map(ImplicitScopeRoot::ShadowHost);
    }
    host.node(parent)?.as_element()?;
    let opaque_parent = opaque_element_for_handle(host, parent)?;
    if host.containing_shadow_root(parent).is_some() {
        Some(ImplicitScopeRoot::InShadowTree(opaque_parent))
    } else {
        Some(ImplicitScopeRoot::InLightTree(opaque_parent))
    }
}

fn opaque_element_for_handle(
    host: &DomHost,
    handle: DomHandle,
) -> Option<selectors::OpaqueElement> {
    let node = host.node(handle)?;
    node.as_element()?;
    Some(selectors::OpaqueElement::from_non_null_ptr(
        NonNull::new(node as *const crate::dom::native::Node as *mut ())
            .expect("DOM node pointers are never null"),
    ))
}

pub(super) fn append_stylesheet_to_stylist(
    stylist: &mut Stylist,
    shared_lock: &SharedRwLock,
    css_text: &str,
    base_url: &url::Url,
    origin: Origin,
    quirks_mode: QuirksMode,
) {
    let stylesheet = parse_stylesheet(shared_lock, base_url, css_text, origin, quirks_mode, "");
    let guard = shared_lock.read();
    stylist.append_stylesheet(DocumentStyleSheet::new(ServoArc::new(stylesheet)), &guard);
}

fn author_stylesheet_for_source(
    shared_lock: &SharedRwLock,
    source: &StyloStylesheetSource,
    quirks_mode: QuirksMode,
) -> ServoArc<Stylesheet> {
    if let Some(stylesheet) = source.parsed_stylesheet() {
        return stylesheet;
    }
    #[cfg(test)]
    AUTHOR_SOURCE_TEXT_PARSE_COUNT.with(|count| count.set(count.get() + 1));
    let css_text = source
        .input_css_text()
        .expect("stylesheet without parsed contents must remain text-backed");
    ServoArc::new(parse_stylesheet(
        shared_lock,
        source.base_url(),
        css_text,
        Origin::Author,
        quirks_mode,
        source.media_text(),
    ))
}

fn parse_stylesheet(
    shared_lock: &SharedRwLock,
    base_url: &url::Url,
    css_text: &str,
    origin: Origin,
    quirks_mode: QuirksMode,
    media_text: &str,
) -> Stylesheet {
    let media = parse_media_query_list_with_context(media_text, base_url, quirks_mode);
    let media = ServoArc::new(shared_lock.wrap(media));
    Stylesheet::from_str(
        css_text,
        UrlExtraData::from(base_url.clone()),
        origin,
        media,
        shared_lock.clone(),
        None,
        None,
        quirks_mode,
        AllowImportRules::No,
    )
}

impl FontMetricsProvider for HeadlessFontMetricsProvider {
    fn query_font_metrics(
        &self,
        _vertical: bool,
        font: &Font,
        base_size: CSSPixelLength,
        _flags: QueryFontMetricsFlags,
    ) -> FontMetrics {
        let mut metrics = FontMetrics::default();
        if font_family_list_starts_with_ahem(font) {
            metrics.zero_advance_measure = Some(base_size);
        }
        metrics
    }

    fn base_size_for_generic(&self, _generic: GenericFontFamily) -> Length {
        Length::new(16.0)
    }
}

fn font_family_list_starts_with_ahem(font: &Font) -> bool {
    font.clone_font_family()
        .families
        .iter()
        .next()
        .is_some_and(|family| match family {
            SingleFontFamily::FamilyName(name) => name.name.as_ref().eq_ignore_ascii_case("Ahem"),
            SingleFontFamily::Generic(_) => false,
        })
}
