use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::{
        LazyLock,
        atomic::{AtomicU64, Ordering},
    },
};

use super::{
    css_style::{
        CssStyleDeclarationItemArgs, CssStyleDeclarationPropertyArgs,
        CssStyleDeclarationSetPropertyArgs, CssStyleEntry as StyleEntry,
        camel_case_style_property_name, canonical_style_property_identifier,
        canonical_style_property_name, mask_compat_property_name, mask_compat_value_is_supported,
        parse_css_declaration_list, serialize_css_style_entries,
        serialize_css_style_entries_with_pdb_block, stylo_declaration_block_property_names,
        stylo_mask_property_name, top_level_comma_separated_component_values,
        webkit_transform_origin_compat_property_name,
        webkit_transform_origin_compat_value_is_supported,
    },
    util::{
        call_script_visible_function, get_private_value, serialize_v8_array, set_private_value,
        throw_type_error, v8_string,
    },
    webidl,
};
use crate::native_bridge::element::{
    cssom_text_decoration_line_value_is_compat, serialize_animation_range_shorthand,
    serialize_animation_shorthand_from_longhands, serialize_transition_shorthand_from_longhands,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const STYLE_NAMES_SLOT: &str = "__moliStyleNames";
const STYLE_INDEXED_LENGTH_SLOT: &str = "__moliStyleIndexedLength";
const STYLE_INTERFACE_SLOT: &str = "__moliStyleInterface";
const STYLE_CHANGE_CALLBACK_SLOT: &str = "__moliStyleChangeCallback";
const STYLE_STYLO_DECLARATION_BLOCK_SLOT: &str = "__moliStyleStyloDeclarationBlock";
const STYLE_STYLO_DECLARATION_BLOCK_ID_SLOT: &str = "__moliStyleStyloDeclarationBlockId";
const STYLE_PRIORITY_PREFIX: &str = "__moliStylePriority:";
const STYLE_VALUE_PREFIX: &str = "__moliStyleValue:";
const OVERSCROLL_BEHAVIOR_LONGHANDS: [&str; 2] = ["overscroll-behavior-x", "overscroll-behavior-y"];

thread_local! {
    static STYLO_DECLARATION_BLOCKS: RefCell<HashMap<u64, moli_css_parse::CssDeclarationBlock>> =
        RefCell::new(HashMap::new());
}

#[cfg(test)]
thread_local! {
    static RAW_STYLE_ENTRIES_SNAPSHOT_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
pub(crate) fn reset_raw_style_entries_snapshot_count_for_test() {
    RAW_STYLE_ENTRIES_SNAPSHOT_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn raw_style_entries_snapshot_count_for_test() -> usize {
    RAW_STYLE_ENTRIES_SNAPSHOT_COUNT.with(std::cell::Cell::get)
}

#[cfg(test)]
fn note_raw_style_entries_snapshot() {
    RAW_STYLE_ENTRIES_SNAPSHOT_COUNT.with(|count| count.set(count.get() + 1));
}

#[cfg(not(test))]
fn note_raw_style_entries_snapshot() {}

static NEXT_STYLO_DECLARATION_BLOCK_ID: AtomicU64 = AtomicU64::new(1);

fn next_stylo_declaration_block_id() -> u64 {
    let id = NEXT_STYLO_DECLARATION_BLOCK_ID
        .fetch_add(1, Ordering::Relaxed)
        .max(1);
    if id == u64::MAX {
        NEXT_STYLO_DECLARATION_BLOCK_ID.store(1, Ordering::Relaxed);
    }
    id
}

fn remove_stylo_declaration_block(id: u64) {
    STYLO_DECLARATION_BLOCKS.with(|blocks| {
        blocks.borrow_mut().remove(&id);
    });
}

pub(crate) fn create_lightweight_css_style_stylo_declaration_block(
    block: &moli_css_parse::CssDeclarationBlock,
) -> u64 {
    let id = next_stylo_declaration_block_id();
    STYLO_DECLARATION_BLOCKS.with(|blocks| {
        blocks.borrow_mut().insert(id, block.clone());
    });
    id
}

pub(crate) fn store_lightweight_css_style_stylo_declaration_block(
    id: u64,
    block: &moli_css_parse::CssDeclarationBlock,
) {
    STYLO_DECLARATION_BLOCKS.with(|blocks| {
        blocks.borrow_mut().insert(id, block.clone());
    });
}

pub(crate) fn lightweight_css_style_stylo_declaration_block_css_text(id: u64) -> Option<String> {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    STYLO_DECLARATION_BLOCKS.with(|blocks| blocks.borrow().get(&id).map(|block| block.css_text()))
}

pub(crate) fn remove_lightweight_css_style_stylo_declaration_block(id: u64) {
    remove_stylo_declaration_block(id);
}

pub(crate) fn set_lightweight_css_style_stylo_declaration_block_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    id: u64,
) -> bool {
    if !is_lightweight_style_object(scope, style)
        || !lightweight_style_interface_uses_standard_declarations(scope, style)
    {
        return false;
    }
    if let Some(previous) = stylo_declaration_block_id(scope, style)
        && previous != id
    {
        remove_stylo_declaration_block(previous);
    }
    let value = v8::Boolean::new(scope, true);
    set_style_private_value(
        scope,
        style,
        STYLE_STYLO_DECLARATION_BLOCK_SLOT,
        value.into(),
    );
    set_stylo_declaration_block_id(scope, style, id);
    true
}

#[derive(WebApiObject)]
#[webapi(
    interface = "CSSStyleProperties",
    own_to_string_tag = "CSSStyleProperties"
)]
struct LightweightCssStylePropertiesDeclaration<'scope> {
    #[webapi(slot = STYLE_INTERFACE_SLOT)]
    interface: &'static str,
    #[webapi(slot = STYLE_NAMES_SLOT)]
    names: v8::Local<'scope, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = "CSSFontFaceDescriptors",
    own_to_string_tag = "CSSFontFaceDescriptors"
)]
struct LightweightCssFontFaceDescriptorsDeclaration<'scope> {
    #[webapi(slot = STYLE_INTERFACE_SLOT)]
    interface: &'static str,
    #[webapi(slot = STYLE_NAMES_SLOT)]
    names: v8::Local<'scope, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = "CSSPageDescriptors",
    own_to_string_tag = "CSSPageDescriptors"
)]
struct LightweightCssPageDescriptorsDeclaration<'scope> {
    #[webapi(slot = STYLE_INTERFACE_SLOT)]
    interface: &'static str,
    #[webapi(slot = STYLE_NAMES_SLOT)]
    names: v8::Local<'scope, v8::Array>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "CSSStyleProperties")]
struct LightweightCssStylePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = style_length_getter_callback,
        enumerable
    )]
    length: (),
    #[webapi(
        accessor_property = "cssText",
        getter = style_css_text_getter_callback,
        setter = style_css_text_setter_callback,
        enumerable
    )]
    css_text: (),
    #[webapi(method = "setProperty", length = 2, callback = style_set_property_callback)]
    set_property: (),
    #[webapi(
        method = "getPropertyValue",
        length = 1,
        callback = style_get_property_value_callback
    )]
    get_property_value: (),
    #[webapi(
        method = "removeProperty",
        length = 1,
        callback = style_remove_property_callback
    )]
    remove_property: (),
    #[webapi(
        method = "getPropertyPriority",
        length = 1,
        callback = style_get_property_priority_callback
    )]
    get_property_priority: (),
    #[webapi(method, length = 1, callback = style_item_callback)]
    item: (),
}

const LIGHTWEIGHT_STYLE_PROPERTIES: &[&str] = &[
    "accent-color",
    "all",
    "align-content",
    "align-items",
    "align-self",
    "alignment-baseline",
    "aspect-ratio",
    "baseline-shift",
    "animation",
    "animation-delay",
    "animation-direction",
    "animation-duration",
    "animation-fill-mode",
    "animation-iteration-count",
    "animation-name",
    "animation-play-state",
    "animation-range",
    "animation-range-end",
    "animation-range-start",
    "animation-timeline",
    "animation-timing-function",
    "appearance",
    "backface-visibility",
    "background",
    "background-attachment",
    "background-blend-mode",
    "background-clip",
    "background-color",
    "background-image",
    "background-origin",
    "background-position",
    "background-repeat",
    "background-size",
    "baseline-source",
    "block-size",
    "bookmark-level",
    "bookmark-state",
    "border",
    "border-block-end-color",
    "border-block-start-color",
    "border-bottom",
    "border-bottom-color",
    "border-bottom-left-radius",
    "border-bottom-right-radius",
    "border-bottom-style",
    "border-bottom-width",
    "border-collapse",
    "border-color",
    "border-image",
    "border-image-outset",
    "border-image-repeat",
    "border-image-slice",
    "border-image-source",
    "border-image-width",
    "border-inline-end-color",
    "border-inline-start-color",
    "border-left",
    "border-left-color",
    "border-left-style",
    "border-left-width",
    "border-radius",
    "border-right",
    "border-right-color",
    "border-right-style",
    "border-right-width",
    "border-spacing",
    "border-style",
    "border-top",
    "border-top-color",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-top-style",
    "border-top-width",
    "border-width",
    "bottom",
    "box-shadow",
    "box-sizing",
    "caption-side",
    "caret-color",
    "clear",
    "clip",
    "clip-path",
    "color",
    "color-scheme",
    "column-gap",
    "column-rule-width",
    "column-span",
    "column-width",
    "container",
    "container-name",
    "container-type",
    "content",
    "cursor",
    "direction",
    "display",
    "empty-cells",
    "filter",
    "flex",
    "flex-basis",
    "flex-direction",
    "flex-flow",
    "flex-grow",
    "flex-shrink",
    "flex-wrap",
    "float",
    "font",
    "font-family",
    "font-kerning",
    "font-size",
    "font-style",
    "font-variant",
    "font-variant-alternates",
    "font-variant-caps",
    "font-variant-east-asian",
    "font-variant-emoji",
    "font-variant-ligatures",
    "font-variant-numeric",
    "font-variant-position",
    "font-weight",
    "forced-color-adjust",
    "gap",
    "grid-column",
    "grid-column-end",
    "grid-column-start",
    "height",
    "inset",
    "isolation",
    "justify-content",
    "justify-self",
    "left",
    "letter-spacing",
    "line-height",
    "link-parameters",
    "list-style",
    "list-style-image",
    "list-style-position",
    "list-style-type",
    "margin",
    "margin-block",
    "margin-block-end",
    "margin-block-start",
    "margin-bottom",
    "margin-inline",
    "margin-inline-end",
    "margin-inline-start",
    "margin-left",
    "margin-right",
    "margin-top",
    "mask",
    "mask-clip",
    "mask-composite",
    "mask-image",
    "mask-mode",
    "mask-origin",
    "mask-position",
    "mask-repeat",
    "mask-size",
    "max-height",
    "max-width",
    "min-height",
    "min-width",
    "mix-blend-mode",
    "object-position",
    "opacity",
    "order",
    "orphans",
    "overscroll-behavior",
    "overscroll-behavior-block",
    "overscroll-behavior-inline",
    "overscroll-behavior-x",
    "overscroll-behavior-y",
    "overflow",
    "overflow-x",
    "overflow-y",
    "outline",
    "outline-color",
    "outline-style",
    "outline-width",
    "padding",
    "padding-bottom",
    "padding-block-end",
    "padding-block-start",
    "padding-inline-end",
    "padding-inline-start",
    "padding-left",
    "padding-right",
    "padding-top",
    "page-break-after",
    "page-break-before",
    "page-break-inside",
    "perspective",
    "perspective-origin",
    "place-content",
    "pointer-events",
    "position",
    "print-color-adjust",
    "quotes",
    "reading-flow",
    "reading-order",
    "right",
    "rotate",
    "row-gap",
    "scale",
    "scroll-margin-top",
    "scroll-padding-bottom",
    "scroll-snap-align",
    "scrollbar-color",
    "scrollbar-width",
    "shape-margin",
    "table-layout",
    "text-align",
    "text-decoration",
    "text-decoration-color",
    "text-decoration-fill",
    "text-decoration-inset",
    "text-decoration-line",
    "text-decoration-skip-ink",
    "text-decoration-skip-spaces",
    "text-decoration-stroke",
    "text-decoration-style",
    "text-decoration-thickness",
    "text-emphasis",
    "text-emphasis-color",
    "text-emphasis-position",
    "text-emphasis-style",
    "text-indent",
    "text-shadow",
    "text-size-adjust",
    "text-transform",
    "text-underline-offset",
    "text-underline-position",
    "tab-size",
    "top",
    "transform",
    "transform-origin",
    "transform-style",
    "transition",
    "transition-behavior",
    "transition-delay",
    "transition-duration",
    "transition-property",
    "transition-timing-function",
    "unicode-bidi",
    "user-select",
    "visibility",
    "widows",
    "will-change",
    "zoom",
    "-webkit-text-fill-color",
    "-webkit-text-stroke",
    "-webkit-text-stroke-color",
    "-webkit-text-stroke-width",
    "white-space",
    "width",
    "z-index",
    "word-spacing",
    "writing-mode",
];
const CSS_STYLE_DECLARATION_PROPERTY_ALIASES: &[&str] = &["color-adjust"];
const CSS_STYLE_DECLARATION_WEBKIT_ALIASES: &[&str] = &[
    "webkitAlignContent",
    "webkitAlignItems",
    "webkitAlignSelf",
    "webkitAnimation",
    "webkitAnimationDelay",
    "webkitAnimationDirection",
    "webkitAnimationDuration",
    "webkitAnimationFillMode",
    "webkitAnimationIterationCount",
    "webkitAnimationName",
    "webkitAnimationPlayState",
    "webkitAnimationTimingFunction",
    "webkitAppearance",
    "webkitBackfaceVisibility",
    "WebKitBackgroundClip",
    "webkitBackgroundOrigin",
    "webkitBackgroundSize",
    "webkitBorderBottomLeftRadius",
    "webkitBorderBottomRightRadius",
    "webkitBorderRadius",
    "webkitBorderTopLeftRadius",
    "webkitBorderTopRightRadius",
    "webkitBoxShadow",
    "webkitBoxSizing",
    "webkitFilter",
    "webkitFlex",
    "webkitFlexBasis",
    "webkitFlexDirection",
    "webkitFlexFlow",
    "webkitFlexGrow",
    "webkitFlexShrink",
    "webkitFlexWrap",
    "webkitJustifyContent",
    "webkitMask",
    "webkitMaskBoxImage",
    "webkitMaskBoxImageOutset",
    "webkitMaskBoxImageRepeat",
    "webkitMaskBoxImageSlice",
    "webkitMaskBoxImageSource",
    "webkitMaskBoxImageWidth",
    "webkitMaskClip",
    "webkitMaskComposite",
    "webkitMaskImage",
    "webkitMaskOrigin",
    "webkitMaskPosition",
    "webkitMaskRepeat",
    "webkitMaskSize",
    "webkitOrder",
    "webkitPerspective",
    "webkitPerspectiveOrigin",
    "webkitTransform",
    "webkitTransformOrigin",
    "webkitTransformStyle",
    "webkitTransition",
    "webkitTransitionDelay",
    "webkitTransitionDuration",
    "webkitTransitionProperty",
    "webkitTransitionTimingFunction",
    "webkitUserSelect",
];

static CSS_STYLE_DECLARATION_EXPOSED_PROPERTY_NAMES: LazyLock<HashSet<String>> =
    LazyLock::new(|| {
        let mut names = HashSet::new();
        for_each_css_style_declaration_exposed_property_name(&mut |name| {
            names.insert(name.to_owned());
        });
        names
    });

static CSS_STYLE_DECLARATION_STANDARD_PROPERTY_NAMES: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| {
        crate::style_engine::ensure_stylo_browser_compat_prefs();
        let mut names = Vec::from(LIGHTWEIGHT_STYLE_PROPERTIES);
        let mut seen = names.iter().copied().collect::<HashSet<_>>();
        for name in moli_css_parse::stylo_enabled_style_rule_property_names() {
            if stylo_property_is_chromium_exposed(name) && seen.insert(name) {
                names.push(name);
            }
        }
        names
    });

fn stylo_property_is_chromium_exposed(name: &str) -> bool {
    !name.starts_with("-moz-")
        && !name.starts_with("-x-")
        && !matches!(name, "mask-position-x" | "mask-position-y")
}

pub(crate) fn css_style_declaration_exposes_property_name(property: &str) -> bool {
    CSS_STYLE_DECLARATION_EXPOSED_PROPERTY_NAMES.contains(property)
}

pub(crate) fn css_style_declaration_standard_property_names() -> &'static [&'static str] {
    &CSS_STYLE_DECLARATION_STANDARD_PROPERTY_NAMES
}

fn for_each_css_style_declaration_exposed_property_name(visit: &mut impl FnMut(&str)) {
    let mut seen = HashSet::new();
    for name in css_style_declaration_standard_property_names() {
        for_each_css_property_accessor_name(name, &mut |name| {
            if seen.insert(name.to_owned()) {
                visit(name);
            }
        });
    }
    for_each_css_style_declaration_compat_alias_name_with_seen(&mut seen, visit);
}

fn for_each_css_style_declaration_compat_alias_name(visit: &mut impl FnMut(&str)) {
    let mut seen = HashSet::new();
    for_each_css_style_declaration_compat_alias_name_with_seen(&mut seen, visit);
}

fn for_each_css_style_declaration_compat_alias_name_with_seen(
    seen: &mut HashSet<String>,
    visit: &mut impl FnMut(&str),
) {
    for name in CSS_STYLE_DECLARATION_PROPERTY_ALIASES {
        for_each_css_property_accessor_name(name, &mut |name| {
            if seen.insert(name.to_owned()) {
                visit(name);
            }
        });
    }
    for alias in CSS_STYLE_DECLARATION_WEBKIT_ALIASES {
        if seen.insert((*alias).to_owned()) {
            visit(alias);
        }
        if let Some(property_name) = webkit_css_property_name_for_alias(alias) {
            for_each_css_property_accessor_name(&property_name, &mut |name| {
                if seen.insert(name.to_owned()) {
                    visit(name);
                }
            });
        }
    }
}

fn for_each_css_property_accessor_name(css_property_name: &str, visit: &mut impl FnMut(&str)) {
    visit(css_property_name);
    if let Some(camel_name) = camel_case_style_property_name(css_property_name)
        && camel_name != css_property_name
    {
        visit(&camel_name);
    }
    if let Some(webkit_name) = webkit_cased_style_property_name(css_property_name) {
        visit(&webkit_name);
    }
}

fn webkit_cased_style_property_name(property: &str) -> Option<String> {
    let suffix = property.strip_prefix("-webkit-")?;
    let camel = camel_case_style_property_name(suffix)?;
    Some(format!("webkit{}", uppercase_ascii_head(&camel)))
}

fn webkit_css_property_name_for_alias(alias: &str) -> Option<String> {
    for prefix in ["WebKit", "Webkit", "webkit"] {
        let Some(rest) = alias.strip_prefix(prefix) else {
            continue;
        };
        if rest
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
        {
            return Some(format!(
                "-webkit-{}",
                moli_css_parse::camel_to_kebab(&lowercase_ascii_head(rest))
            ));
        }
    }
    None
}

fn uppercase_ascii_head(input: &str) -> String {
    let mut chars = input.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out = String::new();
    out.push(first.to_ascii_uppercase());
    out.extend(chars);
    out
}

fn lowercase_ascii_head(input: &str) -> String {
    let mut chars = input.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out = String::new();
    out.push(first.to_ascii_lowercase());
    out.extend(chars);
    out
}

fn all_shorthand_applies_to(property: &str) -> bool {
    !property.starts_with("--")
        && !matches!(property, "all" | "direction" | "unicode-bidi")
        && css_style_declaration_standard_property_names().contains(&property)
}

fn css_wide_keyword(value: &str) -> Option<String> {
    let lowered = value.trim().to_ascii_lowercase();
    matches!(
        lowered.as_str(),
        "initial" | "inherit" | "unset" | "revert" | "revert-layer" | "revert-rule"
    )
    .then_some(lowered)
}

pub(crate) fn build_lightweight_css_style_declaration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    build_lightweight_css_style_declaration_with_interface(scope, "CSSStyleProperties", true)
}

pub(crate) fn build_lightweight_detached_css_style_declaration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let style = build_lightweight_css_style_declaration(scope);
    set_style_uses_stylo_declaration_block(scope, style);
    style
}

pub(crate) fn build_lightweight_css_rule_style_declaration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let style = build_lightweight_css_style_declaration(scope);
    set_style_uses_stylo_declaration_block(scope, style);
    style
}

pub(crate) fn build_lightweight_css_keyframe_style_declaration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let style = build_lightweight_css_style_declaration(scope);
    set_style_interface(scope, style, "CSSKeyframeProperties");
    set_style_uses_stylo_declaration_block(scope, style);
    style
}

pub(crate) fn build_lightweight_css_font_face_descriptors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    build_lightweight_css_style_declaration_with_interface(scope, "CSSFontFaceDescriptors", true)
}

pub(crate) fn build_lightweight_css_page_descriptors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    build_lightweight_css_style_declaration_with_interface(scope, "CSSPageDescriptors", false)
}

fn build_lightweight_css_style_declaration_with_interface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    interface_name: &str,
    _install_standard_properties: bool,
) -> v8::Local<'s, v8::Object> {
    let names = v8::Array::new(scope, 0);
    match interface_name {
        "CSSFontFaceDescriptors" => LightweightCssFontFaceDescriptorsDeclaration {
            interface: "CSSFontFaceDescriptors",
            names,
        }
        .bind(scope)
        .expect("CSSFontFaceDescriptors declaration should bind"),
        "CSSPageDescriptors" => LightweightCssPageDescriptorsDeclaration {
            interface: "CSSPageDescriptors",
            names,
        }
        .bind(scope)
        .expect("CSSPageDescriptors declaration should bind"),
        _ => LightweightCssStylePropertiesDeclaration {
            interface: "CSSStyleProperties",
            names,
        }
        .bind(scope)
        .expect("CSSStyleProperties declaration should bind"),
    }
}

pub(crate) fn install_css_style_declaration_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "CSSStyleDeclaration" => {
            install_lightweight_style_template_getter(
                scope,
                prototype,
                "parentRule",
                None,
                style_parent_rule_getter_callback,
            );
            for_each_css_style_declaration_compat_alias_name(&mut |name| {
                install_lightweight_style_named_property_template_accessor(scope, prototype, name);
            });
        }
        "CSSStyleProperties" => {
            install_lightweight_css_style_prototype_template(scope, prototype, true);
        }
        "CSSFontFaceDescriptors" => {
            install_lightweight_css_style_prototype_template(scope, prototype, true);
            install_descriptor_prototype_template_properties(
                scope,
                prototype,
                moli_css_parse::font_face_descriptor_property_names_with_stylo(),
            );
        }
        "CSSPageDescriptors" => {
            install_lightweight_css_style_prototype_template(scope, prototype, false);
            install_descriptor_prototype_template_properties(
                scope,
                prototype,
                moli_css_parse::page_descriptor_property_names_with_stylo(),
            );
        }
        _ => {}
    }
}

pub(crate) fn set_lightweight_css_style_change_callback(
    scope: &mut v8::PinScope<'_, '_>,
    style: v8::Local<'_, v8::Object>,
    callback: v8::Local<'_, v8::Function>,
) {
    set_style_private_value(scope, style, STYLE_CHANGE_CALLBACK_SLOT, callback.into());
}

pub(crate) fn lightweight_css_style_css_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Option<String> {
    if !is_lightweight_style_object(scope, style) {
        return None;
    }
    if let Some(css_text) = stylo_declaration_block_css_text_for_getter(scope, style) {
        return Some(css_text);
    }
    Some(serialize_css_style_entries(&style_entries(scope, style)))
}

pub(crate) fn set_lightweight_css_style_css_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    css_text: &str,
) -> bool {
    set_lightweight_css_style_css_text_internal(scope, style, css_text, true)
}

pub(crate) fn set_lightweight_css_style_css_text_without_notify<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    css_text: &str,
) -> bool {
    set_lightweight_css_style_css_text_internal(scope, style, css_text, false)
}

fn set_lightweight_css_style_css_text_internal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    css_text: &str,
    notify: bool,
) -> bool {
    if !is_lightweight_style_object(scope, style) {
        return false;
    }
    let Some(style) = lightweight_style_receiver(scope, style) else {
        return false;
    };
    clear_style_entries(scope, style);
    let use_stylo_declaration_block = style_uses_stylo_declaration_block(scope, style);
    if use_stylo_declaration_block
        && set_css_text_as_stylo_declaration_block(scope, style, css_text).is_some()
    {
        if notify {
            notify_style_changed(scope, style);
        }
        return true;
    }
    for entry in parse_css_text(use_stylo_declaration_block, css_text) {
        set_style_entry(scope, style, &entry.name, &entry.value, entry.priority);
    }
    if notify {
        notify_style_changed(scope, style);
    }
    true
}

pub(crate) fn lightweight_css_style_uses_only_stylo_declaration_block<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> bool {
    is_lightweight_style_object(scope, style)
        && style_uses_stylo_declaration_block(scope, style)
        && lightweight_style_interface_uses_standard_declarations(scope, style)
        && raw_style_entries(scope, style).is_empty()
        && stored_stylo_declaration_block(scope, style).is_some()
}

pub(crate) fn lightweight_css_style_has_pdb_side_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> bool {
    is_lightweight_style_object(scope, style)
        && style_uses_stylo_declaration_block(scope, style)
        && lightweight_style_interface_uses_standard_declarations(scope, style)
        && raw_style_entries(scope, style).iter().any(|entry| {
            crate::native_bridge::element::cssom_style_entry_is_pdb_supplemental_side_entry(entry)
        })
}

pub(crate) fn lightweight_css_rule_declaration_write_uses_pdb(name: &str, value: &str) -> bool {
    detached_style_property_write_uses_pdb(name, value)
}

pub(crate) fn lightweight_css_keyframe_declaration_write_uses_pdb(name: &str, value: &str) -> bool {
    keyframe_style_property_name_uses_pdb(name)
        && lightweight_css_rule_declaration_write_uses_pdb(name, value)
}

fn priority_key(name: &str) -> String {
    format!("{STYLE_PRIORITY_PREFIX}{name}")
}

fn value_key(name: &str) -> String {
    format!("{STYLE_VALUE_PREFIX}{name}")
}

fn style_private_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, style, slot)
}

fn set_style_private_value(
    scope: &mut v8::PinScope<'_, '_>,
    style: v8::Local<'_, v8::Object>,
    slot: &str,
    value: v8::Local<'_, v8::Value>,
) {
    set_private_value(scope, style, slot, value);
}

fn install_lightweight_css_style_prototype_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    install_standard_properties: bool,
) {
    LightweightCssStylePrototypeDeclaration::initialize_prototype_template(scope, prototype);
    if install_standard_properties {
        install_lightweight_style_property_template_accessors(scope, prototype);
    }
}

fn install_lightweight_style_template_getter<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    name: &str,
    data: Option<v8::Local<'s, v8::Value>>,
    getter: impl v8::MapFnTo<v8::FunctionCallback>,
) {
    let getter = build_lightweight_style_accessor_template(scope, getter, data, 0);
    set_lightweight_style_accessor_template_name(scope, getter, &format!("get {name}"));
    define_lightweight_style_template_accessor(scope, prototype, name, getter, None);
}

fn install_lightweight_style_template_accessor<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    name: &str,
    data: Option<v8::Local<'s, v8::Value>>,
    getter: impl v8::MapFnTo<v8::FunctionCallback>,
    setter: impl v8::MapFnTo<v8::FunctionCallback>,
) {
    let getter = build_lightweight_style_accessor_template(scope, getter, data, 0);
    set_lightweight_style_accessor_template_name(scope, getter, &format!("get {name}"));
    let setter = build_lightweight_style_accessor_template(scope, setter, data, 1);
    set_lightweight_style_accessor_template_name(scope, setter, &format!("set {name}"));
    define_lightweight_style_template_accessor(scope, prototype, name, getter, Some(setter));
}

fn define_lightweight_style_template_accessor<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    name: &str,
    getter: v8::Local<'s, v8::FunctionTemplate>,
    setter: Option<v8::Local<'s, v8::FunctionTemplate>>,
) {
    let Some(key) = v8_string(scope, name) else {
        return;
    };
    prototype.set_accessor_property(
        key.into(),
        Some(getter),
        setter,
        v8::PropertyAttribute::NONE,
    );
}

fn build_lightweight_style_accessor_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
    data: Option<v8::Local<'s, v8::Value>>,
    length: i32,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let builder = v8::FunctionTemplate::builder(callback).length(length);
    match data {
        Some(data) => builder.data(data).build(scope),
        None => builder.build(scope),
    }
}

fn set_lightweight_style_accessor_template_name(
    scope: &mut v8::PinScope<'_, '_, ()>,
    function: v8::Local<'_, v8::FunctionTemplate>,
    name: &str,
) {
    if let Some(name) = v8_string(scope, name) {
        function.set_class_name(name);
    }
}

fn install_lightweight_style_property_template_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    target: v8::Local<'s, v8::ObjectTemplate>,
) {
    for_each_css_style_declaration_exposed_property_name(&mut |name| {
        install_lightweight_style_named_property_template_accessor(scope, target, name);
    });
}

fn install_lightweight_style_named_property_template_accessor<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    target: v8::Local<'s, v8::ObjectTemplate>,
    name: &str,
) {
    if let Some(name_value) = v8_string(scope, name) {
        let data = name_value.into();
        install_lightweight_style_template_accessor(
            scope,
            target,
            name,
            Some(data),
            style_named_property_getter_callback,
            style_named_property_setter_callback,
        );
    }
}

fn install_descriptor_prototype_template_properties<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    properties: &[&str],
) {
    for name in properties {
        install_lightweight_style_named_property_template_accessor(scope, prototype, name);
        if let Some(camel_name) = camel_case_style_property_name(name)
            && camel_name != *name
        {
            install_lightweight_style_named_property_template_accessor(
                scope,
                prototype,
                &camel_name,
            );
        }
    }
}

fn set_style_interface(
    scope: &mut v8::PinScope<'_, '_>,
    style: v8::Local<'_, v8::Object>,
    interface_name: &str,
) {
    let Some(interface_name) = v8_string(scope, interface_name) else {
        return;
    };
    set_style_private_value(scope, style, STYLE_INTERFACE_SLOT, interface_name.into());
}

fn set_style_uses_stylo_declaration_block<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) {
    let value = v8::Boolean::new(scope, true);
    set_style_private_value(
        scope,
        style,
        STYLE_STYLO_DECLARATION_BLOCK_SLOT,
        value.into(),
    );
    ensure_stylo_declaration_block(scope, style);
}

fn style_uses_stylo_declaration_block<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> bool {
    style_private_value(scope, style, STYLE_STYLO_DECLARATION_BLOCK_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn stylo_declaration_block_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    style_private_value(scope, style, STYLE_STYLO_DECLARATION_BLOCK_ID_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .and_then(|value| {
            let (id, lossless) = value.u64_value();
            (lossless && id != 0).then_some(id)
        })
}

fn set_stylo_declaration_block_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    id: u64,
) {
    let value = v8::BigInt::new_from_u64(scope, id);
    set_style_private_value(
        scope,
        style,
        STYLE_STYLO_DECLARATION_BLOCK_ID_SLOT,
        value.into(),
    );
}

fn ensure_stylo_declaration_block<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) {
    if stylo_declaration_block_id(scope, style).is_some() {
        return;
    }
    let id = next_stylo_declaration_block_id();
    STYLO_DECLARATION_BLOCKS.with(|blocks| {
        blocks
            .borrow_mut()
            .insert(id, moli_css_parse::CssDeclarationBlock::default());
    });
    set_stylo_declaration_block_id(scope, style, id);
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, style, move || {
        remove_stylo_declaration_block(id)
    });
}

fn stored_stylo_declaration_block<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Option<moli_css_parse::CssDeclarationBlock> {
    if !style_uses_stylo_declaration_block(scope, style)
        || !lightweight_style_interface_uses_standard_declarations(scope, style)
    {
        return None;
    }
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let id = stylo_declaration_block_id(scope, style)?;
    STYLO_DECLARATION_BLOCKS.with(|blocks| blocks.borrow().get(&id).cloned())
}

fn mutable_stylo_declaration_block<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Option<moli_css_parse::CssDeclarationBlock> {
    if !style_uses_stylo_declaration_block(scope, style)
        || !lightweight_style_interface_uses_standard_declarations(scope, style)
    {
        return None;
    }
    ensure_stylo_declaration_block(scope, style);
    stored_stylo_declaration_block(scope, style)
}

fn store_stylo_declaration_block<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    block: &moli_css_parse::CssDeclarationBlock,
) {
    ensure_stylo_declaration_block(scope, style);
    let Some(id) = stylo_declaration_block_id(scope, style) else {
        return;
    };
    STYLO_DECLARATION_BLOCKS.with(|blocks| {
        blocks.borrow_mut().insert(id, block.clone());
    });
}

fn set_css_text_as_stylo_declaration_block<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    css_text: &str,
) -> Option<()> {
    if !style_uses_stylo_declaration_block(scope, style)
        || !lightweight_style_interface_uses_standard_declarations(scope, style)
    {
        return None;
    }
    clear_stylo_declaration_block(scope, style);
    for entry in parse_css_text(false, css_text) {
        set_style_entry(scope, style, &entry.name, &entry.value, entry.priority);
    }
    Some(())
}

fn clear_stylo_declaration_block<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) {
    if let Some(id) = stylo_declaration_block_id(scope, style) {
        STYLO_DECLARATION_BLOCKS.with(|blocks| {
            blocks
                .borrow_mut()
                .insert(id, moli_css_parse::CssDeclarationBlock::default());
        });
    }
}

fn lightweight_style_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if is_lightweight_style_object(scope, style) {
        Some(style)
    } else {
        throw_type_error(scope, "Illegal invocation");
        None
    }
}

fn is_lightweight_style_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> bool {
    style_private_value(scope, style, STYLE_NAMES_SLOT).is_some()
}

fn is_css_style_declaration_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> bool {
    is_lightweight_style_object(scope, style)
        || crate::native_bridge::element::is_live_style_declaration_object(scope, style)
}

fn style_interface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> String {
    style_private_value(scope, style, STYLE_INTERFACE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default()
}

fn style_names<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Vec<String> {
    let Some(value) = style_private_value(scope, style, STYLE_NAMES_SLOT) else {
        return Vec::new();
    };
    let Ok(array) = v8::Local::<v8::Array>::try_from(value) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for index in 0..array.length() {
        let Some(value) = array.get_index(scope, index) else {
            continue;
        };
        let Some(value) = value.to_string(scope) else {
            continue;
        };
        names.push(value.to_rust_string_lossy(scope));
    }
    names
}

fn style_property_names<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Vec<String> {
    let names = style_names(scope, style);
    if let Some(block) = stored_stylo_declaration_block(scope, style)
        && raw_style_entries(scope, style).is_empty()
    {
        return stylo_declaration_block_property_names(&block);
    }
    names
}

fn set_style_names<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    names: &[String],
) {
    let array = serialize_v8_array(scope, names).unwrap_or_else(|| v8::Array::new(scope, 0));
    set_style_private_value(scope, style, STYLE_NAMES_SLOT, array.into());
    let indexed_names = style_property_names(scope, style);
    sync_style_indexed_properties(scope, style, &indexed_names);
}

fn sync_style_indexed_properties<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    names: &[String],
) {
    let old_length = style_private_value(scope, style, STYLE_INDEXED_LENGTH_SLOT)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    for index in 0..old_length {
        let _ = style.delete_index(scope, index);
    }
    for (index, name) in names.iter().enumerate() {
        let Some(key) = v8_string(scope, &index.to_string()) else {
            continue;
        };
        let Some(value) = v8_string(scope, name) else {
            continue;
        };
        let _ = style.define_own_property(
            scope,
            key.into(),
            value.into(),
            v8::PropertyAttribute::READ_ONLY,
        );
    }
    let length = v8::Integer::new_from_unsigned(scope, names.len() as u32);
    set_style_private_value(scope, style, STYLE_INDEXED_LENGTH_SLOT, length.into());
}

fn set_style_property_value(
    scope: &mut v8::PinScope<'_, '_>,
    style: v8::Local<'_, v8::Object>,
    name: &str,
    value: &str,
) {
    let key = value_key(name);
    if value.is_empty() {
        let undefined = v8::undefined(scope);
        set_style_private_value(scope, style, &key, undefined.into());
        return;
    }
    let Some(value) = v8_string(scope, value) else {
        return;
    };
    set_style_private_value(scope, style, &key, value.into());
}

fn style_property_priority<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> bool {
    let key = priority_key(name);
    style_private_value(scope, style, &key)
        .and_then(|value| value.boolean_value(scope).then_some(true))
        .unwrap_or(false)
}

fn set_style_property_priority(
    scope: &mut v8::PinScope<'_, '_>,
    style: v8::Local<'_, v8::Object>,
    name: &str,
    priority: bool,
) {
    let key = priority_key(name);
    if priority {
        let value = v8::Boolean::new(scope, true);
        set_style_private_value(scope, style, &key, value.into());
    } else {
        let value = v8::undefined(scope);
        set_style_private_value(scope, style, &key, value.into());
    }
}

fn notify_style_changed<'s>(scope: &mut v8::PinScope<'s, '_>, style: v8::Local<'s, v8::Object>) {
    let Some(callback) = style_private_value(scope, style, STYLE_CHANGE_CALLBACK_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    let _ = call_script_visible_function(
        scope,
        callback,
        style.into(),
        &[style.into()],
        "lightweight CSS style change callback",
    );
}

fn style_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> String {
    let stylo_value = stylo_style_property_value_for_query(scope, style, name);
    style_property_value_with_stylo_value(scope, style, name, stylo_value)
}

fn style_property_value_with_stylo_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
    stylo_value: Option<String>,
) -> String {
    if let Some(value) = stylo_value {
        return value;
    }
    if name == "all" {
        return all_style_property_value(scope, style);
    }
    if name == "animation" {
        let value = raw_style_property_value(scope, style, name);
        if !value.is_empty() {
            return value;
        }
        return animation_shorthand_property_value(scope, style);
    }
    if name == "animation-range" {
        let value = raw_style_property_value(scope, style, name);
        if !value.is_empty() {
            return value;
        }
        return animation_range_shorthand_property_value(scope, style);
    }
    if name == "transition" {
        return transition_shorthand_property_value(scope, style);
    }
    if name == "overscroll-behavior" {
        return two_value_shorthand_property_value(scope, style, OVERSCROLL_BEHAVIOR_LONGHANDS);
    }
    if let Some(longhands) = box_shorthand_longhands(name) {
        let value = if all_shorthand_applies_to(name) {
            declared_style_property_value_after_all(scope, style, name)
        } else {
            raw_style_property_value(scope, style, name)
        };
        if !value.is_empty() {
            return value;
        }
        return box_shorthand_property_value(scope, style, longhands);
    }
    let value = if all_shorthand_applies_to(name) {
        declared_style_property_value_after_all(scope, style, name)
    } else {
        raw_style_property_value(scope, style, name)
    };
    if !value.is_empty() {
        return value;
    }
    border_css_wide_keyword_property_value(scope, style, name).unwrap_or_default()
}

fn stylo_style_property_value_for_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<String> {
    if !detached_style_property_query_uses_pdb(scope, style, name) {
        return None;
    }
    let block = stored_stylo_declaration_block(scope, style);
    let raw_entries = raw_style_entries(scope, style);
    stylo_style_property_value_for_query_from_snapshot(name, block.as_ref(), &raw_entries)
}

fn stylo_style_property_value_for_query_from_snapshot(
    name: &str,
    block: Option<&moli_css_parse::CssDeclarationBlock>,
    raw_entries: &[StyleEntry],
) -> Option<String> {
    if let Some(block) = block {
        if raw_legacy_entries_block_pdb_property_query(block, raw_entries, name) {
            return None;
        }
        if let Some(supplemental) = raw_exact_pdb_supplemental_side_entry(raw_entries, name) {
            return Some(supplemental.value);
        }
        if detached_pdb_supplemental_query_can_return_directly(name)
            && let Some(supplemental) =
                raw_pdb_supplemental_side_entry_for_property(raw_entries, name, None)
        {
            return Some(supplemental.value);
        }
        if let Some(value) =
            crate::native_bridge::element::pdb_property_value_for_cssom_query_with_side_entries(
                block,
                name,
                raw_entries,
            )
        {
            return Some(value);
        }
        return None;
    }
    let value =
        crate::native_bridge::element::style_entries_property_value_with_pdb(raw_entries, name)?;
    (!value.is_empty()).then_some(value)
}

fn stylo_style_property_priority_for_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<bool> {
    if !detached_style_property_query_uses_pdb(scope, style, name) {
        return None;
    }
    let block = stored_stylo_declaration_block(scope, style);
    let raw_entries = raw_style_entries(scope, style);
    stylo_style_property_priority_for_query_from_snapshot(name, block.as_ref(), &raw_entries)
}

fn stylo_style_property_priority_for_query_from_snapshot(
    name: &str,
    block: Option<&moli_css_parse::CssDeclarationBlock>,
    raw_entries: &[StyleEntry],
) -> Option<bool> {
    if let Some(block) = block {
        if raw_legacy_entries_block_pdb_property_query(block, raw_entries, name) {
            return None;
        }
        if let Some(supplemental) = raw_exact_pdb_supplemental_side_entry(raw_entries, name) {
            return Some(supplemental.priority);
        }
        if detached_pdb_supplemental_query_can_return_directly(name)
            && let Some(supplemental) =
                raw_pdb_supplemental_side_entry_for_property(raw_entries, name, None)
        {
            return Some(supplemental.priority);
        }
        if let Some(priority) =
            crate::native_bridge::element::pdb_property_priority_for_cssom_query_with_side_entries(
                block,
                name,
                raw_entries,
            )
        {
            return Some(priority);
        }
        return None;
    }
    crate::native_bridge::element::style_entries_property_priority_with_pdb(raw_entries, name)
}

fn stylo_style_property_affected_names_for_removal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<Vec<String>> {
    if !detached_style_property_query_uses_pdb(scope, style, name) {
        return None;
    }
    crate::native_bridge::element::cssom_style_property_mutation_affected_names_with_pdb(name)
}

fn raw_legacy_entries_block_pdb_property_query(
    block: &moli_css_parse::CssDeclarationBlock,
    raw_entries: &[StyleEntry],
    name: &str,
) -> bool {
    let affected_names =
        crate::native_bridge::element::cssom_style_property_affected_names_with_pdb(name)
            .unwrap_or_default();
    let block_entries = block
        .entries()
        .into_iter()
        .map(StyleEntry::from)
        .collect::<Vec<_>>();
    raw_entries.iter().any(|entry| {
        legacy_style_entry_affects_property_query(entry, name, &affected_names)
            && !block_entries.iter().any(|block_entry| {
                block_entry.name == entry.name
                    && block_entry.value == entry.value
                    && block_entry.priority == entry.priority
            })
            && !crate::native_bridge::element::cssom_style_entry_is_pdb_supplemental_side_entry(
                entry,
            )
    })
}

fn raw_pdb_supplemental_side_entry_for_property(
    raw_entries: &[StyleEntry],
    name: &str,
    priority: Option<bool>,
) -> Option<StyleEntry> {
    let affected_names =
        crate::native_bridge::element::cssom_style_property_affected_names_with_pdb(name)
            .unwrap_or_default();
    raw_entries.iter().cloned().rev().find(|entry| {
        legacy_style_entry_affects_property_query(entry, name, &affected_names)
            && crate::native_bridge::element::cssom_style_entry_is_pdb_supplemental_side_entry(
                entry,
            )
            && priority.is_none_or(|priority| entry.priority == priority)
    })
}

fn raw_exact_pdb_supplemental_side_entry(
    raw_entries: &[StyleEntry],
    name: &str,
) -> Option<StyleEntry> {
    raw_entries.iter().cloned().rev().find(|entry| {
        entry.name == name
            && crate::native_bridge::element::cssom_style_entry_is_pdb_supplemental_side_entry(
                entry,
            )
    })
}

fn detached_pdb_supplemental_query_can_return_directly(name: &str) -> bool {
    crate::native_bridge::element::cssom_style_property_affected_names_with_pdb(name)
        .is_some_and(|affected_names| affected_names.len() == 1 && affected_names[0] == name)
}

fn legacy_style_entry_affects_property_query(
    entry: &StyleEntry,
    property: &str,
    affected_names: &[String],
) -> bool {
    if prefixed_style_entry_is_independent_of_unprefixed_property(&entry.name, property) {
        return false;
    }
    if entry.name == property || affected_names.iter().any(|name| name == &entry.name) {
        return true;
    }
    if entry.name == "all" && all_shorthand_applies_to(property) {
        return true;
    }
    if let Some(longhands) = box_shorthand_longhands(&entry.name)
        && longhands.iter().any(|longhand| {
            longhand == &property || affected_names.iter().any(|name| name == longhand)
        })
    {
        return true;
    }
    if entry.name == "overscroll-behavior"
        && OVERSCROLL_BEHAVIOR_LONGHANDS.iter().any(|longhand| {
            longhand == &property || affected_names.iter().any(|name| name == longhand)
        })
    {
        return true;
    }
    if let Some(entry_affected_names) =
        crate::native_bridge::element::cssom_style_property_affected_names_with_pdb(&entry.name)
    {
        return entry_affected_names.iter().any(|name| {
            name == property || affected_names.iter().any(|affected| affected == name)
        });
    }
    false
}

fn prefixed_style_entry_is_independent_of_unprefixed_property(
    entry_name: &str,
    property: &str,
) -> bool {
    entry_name.starts_with("-webkit-") && !property.starts_with("-webkit-")
}

fn animation_shorthand_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> String {
    let Some((values, shared_css_wide_keyword)) = shorthand_property_values(
        scope,
        style,
        &[
            "animation-duration",
            "animation-timing-function",
            "animation-delay",
            "animation-iteration-count",
            "animation-direction",
            "animation-fill-mode",
            "animation-play-state",
            "animation-name",
        ],
        &[
            "animation-timeline",
            "animation-range-start",
            "animation-range-end",
        ],
    ) else {
        return String::new();
    };
    if let Some(keyword) = shared_css_wide_keyword {
        return keyword;
    }

    let mut longhands: [Vec<String>; 8] = Default::default();
    for (index, value) in values.into_iter().take(longhands.len()).enumerate() {
        longhands[index] =
            top_level_comma_separated_component_values(&value).unwrap_or_else(|| vec![value]);
    }
    serialize_animation_shorthand_from_longhands(longhands)
}

fn animation_range_shorthand_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> String {
    let Some((values, shared_css_wide_keyword)) = shorthand_property_values(
        scope,
        style,
        &["animation-range-start", "animation-range-end"],
        &[],
    ) else {
        return String::new();
    };
    if let Some(keyword) = shared_css_wide_keyword {
        return keyword;
    }
    serialize_animation_range_shorthand(&values[0], &values[1])
}

fn transition_shorthand_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> String {
    let Some((values, shared_css_wide_keyword)) = shorthand_property_values(
        scope,
        style,
        &[
            "transition-property",
            "transition-duration",
            "transition-timing-function",
            "transition-delay",
            "transition-behavior",
        ],
        &[],
    ) else {
        return String::new();
    };
    if let Some(keyword) = shared_css_wide_keyword {
        return keyword;
    }

    let mut longhands: [Vec<String>; 5] = Default::default();
    for (index, value) in values.into_iter().enumerate() {
        longhands[index] =
            top_level_comma_separated_component_values(&value).unwrap_or_else(|| vec![value]);
    }
    serialize_transition_shorthand_from_longhands(longhands)
}

fn shorthand_property_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    longhands: &[&str],
    reset_only_longhands: &[&str],
) -> Option<(Vec<String>, Option<String>)> {
    let mut values = Vec::with_capacity(longhands.len() + reset_only_longhands.len());
    let mut priority = None;
    for name in longhands.iter().chain(reset_only_longhands.iter()) {
        let value = style_longhand_property_value_for_shorthand(scope, style, name);
        if value.is_empty() {
            return None;
        }
        let entry_priority = style_longhand_property_priority_for_shorthand(scope, style, name);
        if priority.is_some_and(|current| current != entry_priority) {
            return None;
        }
        priority = Some(entry_priority);
        values.push(value);
    }

    let css_wide_keywords = values
        .iter()
        .map(|value| css_wide_keyword(value))
        .collect::<Option<Vec<_>>>();
    if values.iter().any(|value| css_wide_keyword(value).is_some()) {
        let keywords = css_wide_keywords?;
        let first = keywords.first()?.clone();
        if keywords.iter().all(|keyword| keyword == &first) {
            return Some((values, Some(first)));
        }
        return None;
    }

    Some((values, None))
}

fn style_longhand_property_value_for_shorthand<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> String {
    stylo_style_property_value_for_query(scope, style, name)
        .unwrap_or_else(|| raw_style_property_value(scope, style, name))
}

fn style_longhand_property_priority_for_shorthand<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> bool {
    stylo_style_property_priority_for_query(scope, style, name)
        .unwrap_or_else(|| style_property_priority(scope, style, name))
}

fn two_value_shorthand_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    longhands: [&str; 2],
) -> String {
    let mut values = Vec::with_capacity(longhands.len());
    let mut priority = None;
    for name in longhands {
        let value = raw_style_property_value(scope, style, name);
        if value.is_empty() {
            return String::new();
        }
        let entry_priority = style_property_priority(scope, style, name);
        if priority.is_some_and(|current| current != entry_priority) {
            return String::new();
        }
        priority = Some(entry_priority);
        values.push(value);
    }
    if values.iter().any(|value| css_wide_keyword(value).is_some())
        && values.first() != values.get(1)
    {
        return String::new();
    }
    if values.first() == values.get(1) {
        values.remove(0)
    } else {
        values.join(" ")
    }
}

fn box_shorthand_longhands(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "margin" => Some(&["margin-top", "margin-right", "margin-bottom", "margin-left"]),
        "margin-inline" => Some(&["margin-inline-start", "margin-inline-end"]),
        "margin-block" => Some(&["margin-block-start", "margin-block-end"]),
        "padding" => Some(&[
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left",
        ]),
        _ => None,
    }
}

fn box_shorthand_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    longhands: &[&str],
) -> String {
    let Some((values, shared_css_wide_keyword)) =
        shorthand_property_values(scope, style, longhands, &[])
    else {
        return String::new();
    };
    if let Some(keyword) = shared_css_wide_keyword {
        return keyword;
    }
    compress_box_components(&values).unwrap_or_default()
}

fn compress_box_components(values: &[String]) -> Option<String> {
    match values {
        [start, end] if start == end => Some(start.clone()),
        [start, end] => Some(format!("{start} {end}")),
        [top, right, bottom, left] if top == right && top == bottom && top == left => {
            Some(top.clone())
        }
        [top, right, bottom, left] if top == bottom && right == left => {
            Some(format!("{top} {right}"))
        }
        [top, right, bottom, left] if right == left => Some(format!("{top} {right} {bottom}")),
        [top, right, bottom, left] => Some(format!("{top} {right} {bottom} {left}")),
        _ => None,
    }
}

fn raw_style_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> String {
    let key = value_key(name);
    style_private_value(scope, style, &key)
        .filter(|value| !value.is_null_or_undefined())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default()
}

fn declared_style_property_value_after_all<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> String {
    style_names(scope, style)
        .into_iter()
        .filter(|candidate| candidate == name || candidate == "all")
        .filter_map(|candidate| {
            let value = if candidate == name {
                stylo_declaration_block_property_value_for_query(scope, style, name)
                    .unwrap_or_else(|| raw_style_property_value(scope, style, &candidate))
            } else {
                raw_style_property_value(scope, style, &candidate)
            };
            (!value.is_empty()).then_some(value)
        })
        .last()
        .unwrap_or_default()
}

fn declared_style_property_priority_after_all<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<bool> {
    style_names(scope, style)
        .into_iter()
        .filter(|candidate| candidate == name || candidate == "all")
        .filter_map(|candidate| {
            let value = if candidate == name {
                stylo_declaration_block_property_value(scope, style, name)
                    .unwrap_or_else(|| raw_style_property_value(scope, style, &candidate))
            } else {
                raw_style_property_value(scope, style, &candidate)
            };
            if value.is_empty() {
                return None;
            }
            let priority = if candidate == name {
                stylo_declaration_block_property_priority_for_query(scope, style, name)
                    .unwrap_or_else(|| style_property_priority(scope, style, &candidate))
            } else {
                style_property_priority(scope, style, &candidate)
            };
            Some(priority)
        })
        .last()
}

fn stylo_declaration_block_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<String> {
    let value = stored_stylo_declaration_block(scope, style)?.property_value(name)?;
    (!value.is_empty()).then_some(value)
}

fn stylo_declaration_block_property_value_for_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<String> {
    detached_style_property_query_uses_pdb(scope, style, name)
        .then(|| stylo_declaration_block_property_value(scope, style, name))
        .flatten()
}

fn stylo_declaration_block_property_priority<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<bool> {
    Some(stored_stylo_declaration_block(scope, style)?.property_priority(name))
}

fn stylo_declaration_block_property_priority_for_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<bool> {
    detached_style_property_query_uses_pdb(scope, style, name)
        .then(|| stylo_declaration_block_property_priority(scope, style, name))
        .flatten()
}

fn all_style_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> String {
    let names = style_names(scope, style);
    let Some(all_index) = names.iter().rposition(|name| name == "all") else {
        return String::new();
    };
    let all_value = raw_style_property_value(scope, style, "all");
    let all_priority = style_property_priority(scope, style, "all");
    let overridden = names.iter().skip(all_index + 1).any(|name| {
        all_shorthand_applies_to(name)
            && (style_property_priority(scope, style, name) != all_priority
                || raw_style_property_value(scope, style, name) != all_value)
    });
    if overridden { String::new() } else { all_value }
}

fn border_css_wide_keyword_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<String> {
    if !border_shorthand_applies_to(name) {
        return None;
    }
    css_wide_keyword(&raw_style_property_value(scope, style, "border"))
}

fn border_shorthand_applies_to(name: &str) -> bool {
    matches!(
        name,
        "border"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "border-width"
            | "border-top-width"
            | "border-right-width"
            | "border-bottom-width"
            | "border-left-width"
            | "border-style"
            | "border-top-style"
            | "border-right-style"
            | "border-bottom-style"
            | "border-left-style"
            | "border-color"
            | "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color"
    )
}

fn set_stylo_style_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
    priority: bool,
) -> bool {
    if moli_css_parse::escape_top_level_semicolons(value) != value
        || moli_css_parse::split_important_priority(value).1
    {
        return false;
    }
    let Some(mut block) = mutable_stylo_declaration_block(scope, style) else {
        return false;
    };
    let Some(affected_names) =
        crate::native_bridge::element::cssom_style_property_mutation_affected_names_with_pdb(name)
    else {
        return false;
    };
    let Some(parsed) = crate::native_bridge::element::parse_cssom_style_property_entries_for_write(
        name, value, priority, None,
    ) else {
        return false;
    };
    let supplemental_entries = parsed
        .entries
        .iter()
        .filter(|entry| {
            crate::native_bridge::element::cssom_style_entry_is_pdb_supplemental_side_entry(entry)
        })
        .cloned()
        .collect::<Vec<_>>();

    let mut names = style_names(scope, style);
    clear_style_property_names(scope, style, &mut names, name, &affected_names);
    if parsed.entries.iter().all(|entry| {
        crate::native_bridge::element::cssom_style_entry_is_pdb_supplemental_side_entry(entry)
    }) {
        for affected in &affected_names {
            let _ = block.remove_property(affected);
        }
        for entry in parsed.entries {
            set_style_property_value(scope, style, &entry.name, &entry.value);
            set_style_property_priority(scope, style, &entry.name, entry.priority);
            names.push(entry.name);
        }
    } else {
        let uses_preferred_supplemental_entries =
            crate::native_bridge::element::cssom_style_property_uses_preferred_pdb_supplemental_entries(
                name, value, priority,
            );
        let projection_entries: Vec<StyleEntry> =
            match crate::native_bridge::element::set_pdb_block_property_collecting_entries(
                &mut block,
                name,
                value,
                priority,
                &parsed,
                uses_preferred_supplemental_entries,
            ) {
                Some(entries) => entries,
                None => return false,
            };
        for affected in
            crate::native_bridge::element::cssom_style_property_mutation_cleanup_names_with_pdb(
                name,
            )
        {
            let _ = block.remove_property(&affected);
        }
        let entries = if uses_preferred_supplemental_entries {
            parsed.entries.clone()
        } else if (projection_entries.is_empty()
            || projection_entries
                .iter()
                .any(|entry| entry.value.is_empty()))
            && (moli_css_parse::css_value_may_contain_var_function(value)
                || moli_css_parse::css_value_may_contain_env_function(value))
        {
            parsed
                .entries
                .iter()
                .filter(|entry| {
                    !crate::native_bridge::element::cssom_style_entry_is_pdb_supplemental_side_entry(
                        entry,
                    )
                })
                .cloned()
                .collect::<Vec<_>>()
        } else {
            projection_entries
        };
        for entry in entries {
            let is_supplemental =
                crate::native_bridge::element::cssom_style_entry_is_pdb_supplemental_side_entry(
                    &entry,
                );
            if is_supplemental {
                set_style_property_value(scope, style, &entry.name, &entry.value);
                set_style_property_priority(scope, style, &entry.name, entry.priority);
            }
            if is_supplemental && !uses_preferred_supplemental_entries {
                continue;
            }
            names.push(canonical_style_property_name(&entry.name));
        }
        if !uses_preferred_supplemental_entries {
            for entry in supplemental_entries {
                set_style_property_value(scope, style, &entry.name, &entry.value);
                set_style_property_priority(scope, style, &entry.name, entry.priority);
                names.push(entry.name);
            }
        }
    }
    store_stylo_declaration_block(scope, style, &block);
    set_style_names(scope, style, &names);
    true
}

fn remove_stylo_style_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) {
    let Some(mut block) = mutable_stylo_declaration_block(scope, style) else {
        return;
    };
    let _ = block.remove_property(name);
    store_stylo_declaration_block(scope, style, &block);
}

fn set_style_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
    priority: bool,
) {
    let name = canonical_style_property_name(name);
    if name.is_empty() {
        return;
    }
    if style_interface(scope, style) == "CSSFontFaceDescriptors" {
        set_font_face_descriptor_style_entry(scope, style, &name, value, priority);
        return;
    }
    if style_interface(scope, style) == "CSSPageDescriptors" {
        set_page_descriptor_style_entry(scope, style, &name, value, priority);
        return;
    }
    if keyframe_style_property_write_is_ignored(scope, style, &name) {
        if value.is_empty() {
            let mut names = style_names(scope, style);
            clear_style_property_names(
                scope,
                style,
                &mut names,
                &name,
                std::slice::from_ref(&name),
            );
            remove_stylo_style_entry(scope, style, &name);
            set_style_names(scope, style, &names);
        }
        return;
    }
    if lightweight_style_property_uses_standard_declarations(scope, style, &name)
        && value.is_empty()
        && let Some(affected_names) =
            stylo_style_property_affected_names_for_removal(scope, style, &name)
    {
        let mut names = style_names(scope, style);
        clear_style_property_names(scope, style, &mut names, &name, &affected_names);
        remove_stylo_style_entry(scope, style, &name);
        set_style_names(scope, style, &names);
        return;
    }
    if lightweight_style_interface_uses_standard_declarations(scope, style)
        && value.is_empty()
        && let Some(longhands) = box_shorthand_longhands(&name)
    {
        let mut names = style_names(scope, style);
        let affected_names = longhands
            .iter()
            .map(|longhand| (*longhand).to_owned())
            .collect::<Vec<_>>();
        clear_style_property_names(scope, style, &mut names, &name, &affected_names);
        remove_stylo_style_entry(scope, style, &name);
        set_style_names(scope, style, &names);
        return;
    }
    let write_uses_pdb =
        lightweight_style_property_write_uses_pdb(scope, style, &name, value) && !value.is_empty();
    if write_uses_pdb {
        if set_stylo_style_entry(scope, style, &name, value, priority) {
            return;
        }
        if lightweight_style_property_uses_standard_declarations(scope, style, &name) {
            return;
        }
    }
    if lightweight_style_interface_uses_standard_declarations(scope, style)
        && !value.is_empty()
        && mask_compat_property_name(&name)
        && !stylo_mask_property_name(&name)
        && !mask_compat_value_is_supported(&name, value)
    {
        return;
    }
    if lightweight_style_interface_uses_standard_declarations(scope, style)
        && !value.is_empty()
        && webkit_transform_origin_compat_property_name(&name)
        && !webkit_transform_origin_compat_value_is_supported(&name, value)
    {
        return;
    }
    if lightweight_style_interface_uses_standard_declarations(scope, style)
        && !value.is_empty()
        && cssom_style_entry_requires_structured_parser(&name)
    {
        if let Some(parsed) =
            crate::native_bridge::element::parse_cssom_style_property_entries_with_base(
                &name, value, priority, None,
            )
        {
            let mut names = style_names(scope, style);
            clear_style_property_names(scope, style, &mut names, &name, &parsed.affected_names);
            for entry in parsed.entries {
                set_style_property_value(scope, style, &entry.name, &entry.value);
                set_style_property_priority(scope, style, &entry.name, entry.priority);
                names.push(entry.name);
            }
            set_style_names(scope, style, &names);
        }
        return;
    }
    let value = normalize_style_entry_value(scope, style, &name, value);
    let Some(value) = value.as_deref() else {
        return;
    };
    let mut names = style_names(scope, style);
    if value.is_empty() {
        set_style_property_value(scope, style, &name, "");
        set_style_property_priority(scope, style, &name, false);
        names.retain(|existing| existing != &name);
        set_style_names(scope, style, &names);
        return;
    }
    set_style_property_value(scope, style, &name, value);
    set_style_property_priority(scope, style, &name, priority);
    names.retain(|existing| existing != &name);
    names.push(name);
    set_style_names(scope, style, &names);
}

fn set_font_face_descriptor_style_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
    priority: bool,
) {
    if value.is_empty() {
        let mut names = style_names(scope, style);
        clear_style_property_names(scope, style, &mut names, name, &[name.to_owned()]);
        set_style_names(scope, style, &names);
        return;
    }
    let Some(entry) = moli_css_parse::parse_font_face_descriptor_entry_with_stylo(name, value)
    else {
        return;
    };
    let affected_names = [entry.name.clone()];
    let mut names = style_names(scope, style);
    clear_style_property_names(scope, style, &mut names, name, &affected_names);
    set_style_property_value(scope, style, &entry.name, &entry.value);
    set_style_property_priority(scope, style, &entry.name, priority);
    names.push(entry.name);
    set_style_names(scope, style, &names);
}

fn lightweight_style_interface_uses_standard_declarations<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> bool {
    matches!(
        style_interface(scope, style).as_str(),
        "CSSStyleProperties" | "CSSKeyframeProperties"
    )
}

fn lightweight_style_property_uses_standard_declarations<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> bool {
    lightweight_style_interface_uses_standard_declarations(scope, style)
        || style_interface(scope, style) == "CSSStyleDeclaration"
            && text_decoration_property_uses_standard_declarations(name)
}

fn text_decoration_property_uses_standard_declarations(name: &str) -> bool {
    name == "text-decoration" || name.starts_with("text-decoration-")
}

fn lightweight_style_property_write_uses_pdb<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
) -> bool {
    style_uses_stylo_declaration_block(scope, style)
        && lightweight_style_property_uses_standard_declarations(scope, style, name)
        && keyframe_style_property_uses_pdb(scope, style, name)
        && detached_style_property_write_uses_pdb(name, value)
}

fn keyframe_style_property_uses_pdb<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> bool {
    style_interface(scope, style) != "CSSKeyframeProperties"
        || keyframe_style_property_name_uses_pdb(name)
}

fn keyframe_style_property_name_uses_pdb(name: &str) -> bool {
    !keyframe_style_property_name_is_ignored(name)
}

fn keyframe_style_property_write_is_ignored<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> bool {
    style_interface(scope, style) == "CSSKeyframeProperties"
        && keyframe_style_property_name_is_ignored(name)
}

fn keyframe_style_property_name_is_ignored(name: &str) -> bool {
    name == "animation" || (name.starts_with("animation-") && name != "animation-timing-function")
}

fn detached_style_property_write_uses_pdb(name: &str, value: &str) -> bool {
    let name = canonical_style_property_name(name);
    name != "all"
        && crate::native_bridge::element::cssom_style_property_write_can_use_pdb_storage(
            &name, value,
        )
}

fn detached_style_property_query_uses_pdb<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> bool {
    style_uses_stylo_declaration_block(scope, style)
        && lightweight_style_property_uses_standard_declarations(scope, style, name)
        && keyframe_style_property_uses_pdb(scope, style, name)
        && name != "all"
        // A CSS-wide value is valid for every native Stylo property. Asking
        // the shared write gate keeps detached/rule query ownership aligned
        // with the PDB instead of maintaining a second WebKit allowlist here.
        && detached_style_property_write_uses_pdb(name, "initial")
}

fn cssom_style_entry_requires_structured_parser(name: &str) -> bool {
    crate::native_bridge::element::cssom_style_entry_requires_structured_parser(name)
}

fn clear_style_property_names<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    names: &mut Vec<String>,
    property: &str,
    affected_names: &[String],
) {
    let existing = std::mem::take(names);
    for name in existing {
        if name == property || affected_names.iter().any(|affected| affected == &name) {
            set_style_property_value(scope, style, &name, "");
            set_style_property_priority(scope, style, &name, false);
        } else {
            names.push(name);
        }
    }
}

fn normalize_style_entry_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
) -> Option<String> {
    if style_interface(scope, style) != "CSSPageDescriptors" || value.is_empty() {
        if keyframe_style_property_write_is_ignored(scope, style, name) {
            return None;
        }
        if name == "all" {
            return css_wide_keyword(value);
        }
        if !name.starts_with("--") && moli_css_parse::css_value_may_contain_env_function(value) {
            return moli_css_parse::normalize_cssom_component_value_serialization(value);
        }
        return Some(value.to_owned());
    }
    None
}

fn set_page_descriptor_style_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
    priority: bool,
) {
    if value.is_empty() {
        let affected_names = if name == "margin" {
            ["margin-top", "margin-right", "margin-bottom", "margin-left"]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        } else {
            vec![name.to_owned()]
        };
        let mut names = style_names(scope, style);
        clear_style_property_names(scope, style, &mut names, name, &affected_names);
        set_style_names(scope, style, &names);
        return;
    }
    let Some(entries) = moli_css_parse::parse_page_descriptor_entries_with_stylo(name, value)
    else {
        return;
    };
    set_page_descriptor_entries(scope, style, entries, priority);
}

fn set_page_descriptor_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    entries: Vec<moli_css_parse::CssPageDescriptorEntryView>,
    priority: bool,
) {
    let affected_names = entries
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<Vec<_>>();
    let mut names = style_names(scope, style);
    for name in &affected_names {
        set_style_property_value(scope, style, name, "");
        set_style_property_priority(scope, style, name, false);
    }
    names.retain(|name| !affected_names.contains(name));
    for entry in entries {
        set_style_property_value(scope, style, &entry.name, &entry.value);
        set_style_property_priority(scope, style, &entry.name, priority);
        names.push(entry.name);
    }
    set_style_names(scope, style, &names);
}

fn style_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Vec<StyleEntry> {
    style_property_names(scope, style)
        .into_iter()
        .filter_map(|name| {
            let value = style_property_value(scope, style, &name);
            (!value.is_empty()).then(|| StyleEntry {
                priority: style_property_priority_for_query(scope, style, &name),
                name,
                value,
            })
        })
        .collect()
}

fn style_entries_from_stylo_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    block: &moli_css_parse::CssDeclarationBlock,
    raw_entries: &[StyleEntry],
) -> Vec<StyleEntry> {
    style_names(scope, style)
        .into_iter()
        .filter_map(|name| {
            let query_uses_pdb = detached_style_property_query_uses_pdb(scope, style, &name);
            let stylo_value = query_uses_pdb
                .then(|| {
                    stylo_style_property_value_for_query_from_snapshot(
                        &name,
                        Some(block),
                        raw_entries,
                    )
                })
                .flatten();
            let value = style_property_value_with_stylo_value(scope, style, &name, stylo_value);
            (!value.is_empty()).then(|| {
                let stylo_priority = query_uses_pdb
                    .then(|| {
                        stylo_style_property_priority_for_query_from_snapshot(
                            &name,
                            Some(block),
                            raw_entries,
                        )
                    })
                    .flatten();
                StyleEntry {
                    priority: style_property_priority_for_query_with_stylo_priority(
                        scope,
                        style,
                        &name,
                        stylo_priority,
                    ),
                    name,
                    value,
                }
            })
        })
        .collect()
}

fn raw_style_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Vec<StyleEntry> {
    note_raw_style_entries_snapshot();
    style_names(scope, style)
        .into_iter()
        .filter_map(|name| {
            let value = raw_style_property_value(scope, style, &name);
            (!value.is_empty()).then(|| StyleEntry {
                priority: style_property_priority(scope, style, &name),
                name,
                value,
            })
        })
        .collect()
}

fn clear_style_entries<'s>(scope: &mut v8::PinScope<'s, '_>, style: v8::Local<'s, v8::Object>) {
    let names = style_names(scope, style);
    for name in names {
        set_style_property_value(scope, style, &name, "");
        set_style_property_priority(scope, style, &name, false);
    }
    clear_stylo_declaration_block(scope, style);
    set_style_names(scope, style, &[]);
}

fn parse_css_text(use_stylo_declaration_block: bool, css_text: &str) -> Vec<StyleEntry> {
    if use_stylo_declaration_block {
        return crate::native_bridge::element::parse_inline_css_text_with_base(css_text, None);
    }

    let mut entries = Vec::new();
    for declaration in parse_css_declaration_list(css_text) {
        let name = canonical_style_property_name(&declaration.name);
        if !name.is_empty() && !declaration.value.is_empty() {
            let value = if name == "src" {
                moli_css_parse::normalize_font_face_src(&declaration.value)
                    .unwrap_or(declaration.value)
            } else {
                declaration.value
            };
            entries.push(StyleEntry {
                name,
                value,
                priority: declaration.priority,
            });
        }
    }
    entries
}

fn style_length_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_lightweight_style_object(scope, args.this()) {
        crate::native_bridge::element::style_length_getter_callback(scope, args, rv);
        return;
    }
    let Some(style) = lightweight_style_receiver(scope, args.this()) else {
        return;
    };
    let length = style_property_names(scope, style).len() as i32;
    rv.set(v8::Integer::new(scope, length).into());
}

fn style_parent_rule_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if crate::native_bridge::element::is_live_style_declaration_object(scope, args.this()) {
        rv.set_null();
        return;
    }
    let Some(_) = lightweight_style_receiver(scope, args.this()) else {
        return;
    };
    rv.set_null();
}

fn style_css_text_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_lightweight_style_object(scope, args.this()) {
        crate::native_bridge::element::style_css_text_getter_callback(scope, args, rv);
        return;
    }
    let Some(style) = lightweight_style_receiver(scope, args.this()) else {
        return;
    };
    let css_text = lightweight_css_style_css_text(scope, style).unwrap_or_default();
    if let Some(value) = v8_string(scope, &css_text) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

fn stylo_declaration_block_css_text_for_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Option<String> {
    if !style_uses_stylo_declaration_block(scope, style)
        || !lightweight_style_interface_uses_standard_declarations(scope, style)
    {
        return None;
    }
    if let Some(block) = stored_stylo_declaration_block(scope, style) {
        let side_entries = raw_style_entries(scope, style);
        if side_entries.is_empty() {
            return Some(block.css_text());
        }
        let entries = style_entries_from_stylo_snapshot(scope, style, &block, &side_entries);
        return serialize_css_style_entries_with_pdb_block(&entries, &side_entries, &block);
    }
    let entries = raw_style_entries(scope, style);
    if !entries
        .iter()
        .all(|entry| detached_style_entry_css_text_uses_pdb(scope, style, &entry.name))
    {
        return None;
    }
    crate::native_bridge::element::style_entries_css_text_with_pdb(&entries)
}

fn detached_style_entry_css_text_uses_pdb<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> bool {
    detached_style_property_query_uses_pdb(scope, style, name)
}

fn style_css_text_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_lightweight_style_object(scope, args.this()) {
        crate::native_bridge::element::style_css_text_setter_callback(scope, args, _rv);
        return;
    }
    let value = args.get(0);
    let Some(style) = lightweight_style_receiver(scope, args.this()) else {
        return;
    };
    let css_text = if value.is_null_or_undefined() {
        String::new()
    } else {
        value
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_default()
    };
    let _ = set_lightweight_css_style_css_text(scope, style, &css_text);
}

fn style_named_property_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = args.data().to_string(scope) else {
        rv.set_empty_string();
        return;
    };
    let name = canonical_style_property_identifier(&name.to_rust_string_lossy(scope));
    if !is_lightweight_style_object(scope, args.this()) {
        if !crate::native_bridge::element::is_live_style_declaration_object(scope, args.this()) {
            throw_type_error(scope, "Illegal invocation");
            return;
        }
        let value = crate::native_bridge::element::live_style_named_property_value(
            scope,
            args.this(),
            &name,
        )
        .unwrap_or_default();
        if let Some(value) = v8_string(scope, &value) {
            rv.set(value.into());
        } else {
            rv.set_empty_string();
        }
        return;
    }
    let Some(style) = lightweight_style_receiver(scope, args.this()) else {
        return;
    };
    let value = style_property_value(scope, style, &name);
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

fn style_named_property_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = args.data().to_string(scope) else {
        return;
    };
    let value = args.get(0);
    let name = name.to_rust_string_lossy(scope);
    let name = canonical_style_property_identifier(&name);
    if !is_lightweight_style_object(scope, args.this()) {
        if !crate::native_bridge::element::is_live_style_declaration_object(scope, args.this()) {
            throw_type_error(scope, "Illegal invocation");
            return;
        }
        let _ = crate::native_bridge::element::set_live_style_named_property_value(
            scope,
            args.this(),
            &name,
            value,
        );
        return;
    }
    let value = if value.is_null_or_undefined() {
        String::new()
    } else {
        value
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_default()
    };
    let Some(style) = lightweight_style_receiver(scope, args.this()) else {
        return;
    };
    set_style_entry(scope, style, &name, &value, false);
    notify_style_changed(scope, style);
}

fn style_set_property_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_css_style_declaration_receiver(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if crate::native_bridge::element::is_live_style_declaration_object(scope, args.this()) {
        crate::native_bridge::element::style_set_property_callback(scope, args, _rv);
        return;
    }
    if args.length() > 1 && args.get(1).is_undefined() {
        return;
    }
    let Some(parsed) = webidl::parse_args::<CssStyleDeclarationSetPropertyArgs>(scope, &args)
    else {
        return;
    };
    if !parsed.priority.is_empty() && !parsed.priority.eq_ignore_ascii_case("important") {
        return;
    }
    let Some(style) = lightweight_style_receiver(scope, args.this()) else {
        return;
    };
    set_style_entry(
        scope,
        style,
        &parsed.property,
        &parsed.value,
        parsed.priority.eq_ignore_ascii_case("important"),
    );
    notify_style_changed(scope, style);
}

fn style_get_property_value_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_css_style_declaration_receiver(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if crate::native_bridge::element::is_live_style_declaration_object(scope, args.this()) {
        crate::native_bridge::element::style_get_property_value_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<CssStyleDeclarationPropertyArgs>(scope, &args) else {
        rv.set_empty_string();
        return;
    };
    let name = canonical_style_property_name(&parsed.property);
    let Some(style) = lightweight_style_receiver(scope, args.this()) else {
        return;
    };
    let value = style_property_value(scope, style, &name);
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

fn style_remove_property_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_css_style_declaration_receiver(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if crate::native_bridge::element::is_live_style_declaration_object(scope, args.this()) {
        crate::native_bridge::element::style_remove_property_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<CssStyleDeclarationPropertyArgs>(scope, &args) else {
        rv.set_empty_string();
        return;
    };
    let name = canonical_style_property_name(&parsed.property);
    let Some(style) = lightweight_style_receiver(scope, args.this()) else {
        return;
    };
    let previous = style_property_value(scope, style, &name);
    let previous = if name == "text-decoration"
        && cssom_text_decoration_line_value_is_compat(&style_property_value(
            scope,
            style,
            "text-decoration-line",
        )) {
        String::new()
    } else {
        previous
    };
    if name == "all" {
        let names = style_names(scope, style);
        for name in names {
            if name == "all" || all_shorthand_applies_to(&name) {
                set_style_entry(scope, style, &name, "", false);
            }
        }
    } else {
        set_style_entry(scope, style, &name, "", false);
    }
    notify_style_changed(scope, style);
    if let Some(previous) = v8_string(scope, &previous) {
        rv.set(previous.into());
    } else {
        rv.set_empty_string();
    }
}

fn style_get_property_priority_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_css_style_declaration_receiver(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if crate::native_bridge::element::is_live_style_declaration_object(scope, args.this()) {
        crate::native_bridge::element::style_get_property_priority_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<CssStyleDeclarationPropertyArgs>(scope, &args) else {
        rv.set(v8::String::empty(scope).into());
        return;
    };
    let name = canonical_style_property_name(&parsed.property);
    let Some(style) = lightweight_style_receiver(scope, args.this()) else {
        return;
    };
    let priority = if style_property_priority_for_query(scope, style, &name) {
        "important"
    } else {
        ""
    };
    if let Some(priority) = v8_string(scope, priority) {
        rv.set(priority.into());
    } else {
        rv.set(v8::String::empty(scope).into());
    }
}

fn style_property_priority_for_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
) -> bool {
    let stylo_priority = stylo_style_property_priority_for_query(scope, style, name);
    style_property_priority_for_query_with_stylo_priority(scope, style, name, stylo_priority)
}

fn style_property_priority_for_query_with_stylo_priority<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    name: &str,
    stylo_priority: Option<bool>,
) -> bool {
    if let Some(priority) = stylo_priority {
        return priority;
    }
    if name == "animation" {
        return shorthand_property_priority(
            scope,
            style,
            &[
                "animation-duration",
                "animation-timing-function",
                "animation-delay",
                "animation-iteration-count",
                "animation-direction",
                "animation-fill-mode",
                "animation-play-state",
                "animation-name",
            ],
            &[
                "animation-timeline",
                "animation-range-start",
                "animation-range-end",
            ],
        )
        .unwrap_or(false);
    }
    if name == "animation-range" {
        return shorthand_property_priority(
            scope,
            style,
            &["animation-range-start", "animation-range-end"],
            &[],
        )
        .unwrap_or(false);
    }
    if all_shorthand_applies_to(name)
        && let Some(priority) = declared_style_property_priority_after_all(scope, style, name)
    {
        return priority;
    }
    if let Some(longhands) = box_shorthand_longhands(name) {
        let mut priority = None;
        for longhand in longhands {
            if style_longhand_property_value_for_shorthand(scope, style, longhand).is_empty() {
                return false;
            }
            let longhand_priority =
                style_longhand_property_priority_for_shorthand(scope, style, longhand);
            if priority.is_some_and(|current| current != longhand_priority) {
                return false;
            }
            priority = Some(longhand_priority);
        }
        return priority.unwrap_or(false);
    }
    style_property_priority(scope, style, name)
}

fn shorthand_property_priority<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    longhands: &[&str],
    reset_only_longhands: &[&str],
) -> Option<bool> {
    let mut priority = None;
    for name in longhands.iter().chain(reset_only_longhands.iter()) {
        if style_longhand_property_value_for_shorthand(scope, style, name).is_empty() {
            return None;
        }
        let entry_priority = style_longhand_property_priority_for_shorthand(scope, style, name);
        if priority.is_some_and(|current| current != entry_priority) {
            return None;
        }
        priority = Some(entry_priority);
    }
    priority
}

fn style_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_css_style_declaration_receiver(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if crate::native_bridge::element::is_live_style_declaration_object(scope, args.this()) {
        crate::native_bridge::element::style_item_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<CssStyleDeclarationItemArgs>(scope, &args) else {
        return;
    };
    let Some(style) = lightweight_style_receiver(scope, args.this()) else {
        return;
    };
    let Some(name) = style_property_names(scope, style)
        .get(parsed.index as usize)
        .cloned()
    else {
        rv.set(v8::String::empty(scope).into());
        return;
    };
    if let Some(name) = v8_string(scope, &name) {
        rv.set(name.into());
    } else {
        rv.set(v8::String::empty(scope).into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scanned_css_property_accessor_exposes_name(
        css_property_name: &str,
        exposed_name: &str,
    ) -> bool {
        css_property_name == exposed_name
            || camel_case_style_property_name(css_property_name)
                .as_deref()
                .is_some_and(|camel_name| camel_name == exposed_name)
            || webkit_cased_style_property_name(css_property_name)
                .as_deref()
                .is_some_and(|webkit_name| webkit_name == exposed_name)
    }

    fn scanned_css_style_declaration_exposes_property_name(property: &str) -> bool {
        css_style_declaration_standard_property_names()
            .iter()
            .any(|name| scanned_css_property_accessor_exposes_name(name, property))
            || CSS_STYLE_DECLARATION_PROPERTY_ALIASES
                .iter()
                .any(|name| scanned_css_property_accessor_exposes_name(name, property))
            || CSS_STYLE_DECLARATION_WEBKIT_ALIASES.contains(&property)
            || CSS_STYLE_DECLARATION_WEBKIT_ALIASES.iter().any(|alias| {
                webkit_css_property_name_for_alias(alias)
                    .as_deref()
                    .is_some_and(|name| scanned_css_property_accessor_exposes_name(name, property))
            })
    }

    fn add_accessor_name_probes(probes: &mut HashSet<String>, property: &str) {
        probes.insert(property.to_owned());
        probes.insert(format!("{property}-unknown"));
        probes.insert(uppercase_ascii_head(property));
        if let Some(camel_name) = camel_case_style_property_name(property) {
            probes.insert(camel_name.clone());
            probes.insert(format!("{camel_name}Unknown"));
            probes.insert(uppercase_ascii_head(&camel_name));
        }
        if let Some(webkit_name) = webkit_cased_style_property_name(property) {
            probes.insert(webkit_name.clone());
            probes.insert(format!("{webkit_name}Unknown"));
            probes.insert(uppercase_ascii_head(&webkit_name));
        }
    }

    #[test]
    fn css_style_declaration_exposed_name_registry_matches_scan_semantics() {
        let mut probes = HashSet::new();
        for property in css_style_declaration_standard_property_names()
            .iter()
            .chain(CSS_STYLE_DECLARATION_PROPERTY_ALIASES)
        {
            add_accessor_name_probes(&mut probes, property);
        }
        for alias in CSS_STYLE_DECLARATION_WEBKIT_ALIASES {
            probes.insert((*alias).to_owned());
            probes.insert(format!("{alias}Unknown"));
            probes.insert(uppercase_ascii_head(alias));
            if let Some(property) = webkit_css_property_name_for_alias(alias) {
                add_accessor_name_probes(&mut probes, &property);
            }
        }
        probes.extend(
            [
                "",
                "notAProperty",
                "FontSize",
                "font_size",
                "WebKitTransform",
                "WebkitTransform",
                "webkitTransform",
                "WebKitBackgroundClip",
                "-webkit-transform",
                "webkit-text-fill-color",
                "webkitTextFillColor",
                "color-adjust",
                "colorAdjust",
            ]
            .into_iter()
            .map(str::to_owned),
        );

        for property in probes {
            assert_eq!(
                css_style_declaration_exposes_property_name(&property),
                scanned_css_style_declaration_exposes_property_name(&property),
                "exposure changed for {property:?}"
            );
        }
    }

    #[test]
    fn css_style_declaration_registry_is_the_accessor_name_set() {
        let mut accessor_names = HashSet::new();
        for_each_css_style_declaration_exposed_property_name(&mut |name| {
            accessor_names.insert(name.to_owned());
        });

        assert_eq!(
            &*CSS_STYLE_DECLARATION_EXPOSED_PROPERTY_NAMES,
            &accessor_names
        );
    }

    #[test]
    fn css_style_declaration_exposes_standard_mask_longhands() {
        for property in [
            "mask-clip",
            "mask-composite",
            "mask-image",
            "mask-mode",
            "mask-origin",
            "mask-position",
            "mask-repeat",
            "mask-size",
        ] {
            assert!(css_style_declaration_exposes_property_name(property));
            let camel = camel_case_style_property_name(property).unwrap();
            assert!(css_style_declaration_exposes_property_name(&camel));
        }

        for property in ["mask-position-x", "mask-position-y"] {
            assert!(!css_style_declaration_exposes_property_name(property));
            let prefixed = format!("-webkit-{property}");
            assert!(css_style_declaration_exposes_property_name(&prefixed));
            let camel = webkit_cased_style_property_name(&prefixed).unwrap();
            assert!(css_style_declaration_exposes_property_name(&camel));
        }
    }

    #[test]
    fn stylo_property_exposure_matches_chromium_for_former_gecko_gates() {
        for property in crate::chromium_property_surface::FORMER_GECKO_GATED_SUPPORTED_PROPERTIES {
            assert!(
                css_style_declaration_exposes_property_name(property),
                "Chromium-supported Stylo property should be exposed: {property}"
            );
        }

        for property in crate::chromium_property_surface::GECKO_ONLY_UNSUPPORTED_PROPERTIES {
            assert!(
                !css_style_declaration_exposes_property_name(property),
                "Gecko-only property must stay hidden: {property}"
            );
        }
    }
}
