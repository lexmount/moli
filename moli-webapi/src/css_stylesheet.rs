//! CSSOM object declarations for rules, sheets, and their lists.

use crate::interfaces as web_api_interfaces;
use moli_v8_util::get_private_value;
use moli_webapi_declare::WebApiObject;

pub const CSS_AT_RULE_TYPE_SLOT: &str = "__moliCssRuleType";

pub const CSS_FONT_FEATURE_VALUES_RULE_FONT_FAMILY_SLOT: &str =
    "__moliCssFontFeatureValuesRuleFontFamily";

pub const CSS_KEYFRAME_RULE_KEY_TEXT_SLOT: &str = "__moliCssKeyframeRuleKeyText";

pub const CSS_KEYFRAME_RULE_STYLE_TEXT_SLOT: &str = "__moliCssKeyframeRuleStyleText";

pub const CSS_MARGIN_RULE_NAME_SLOT: &str = "__moliCssMarginRuleName";

pub const CSS_MARGIN_RULE_STYLE_TEXT_SLOT: &str = "__moliCssMarginRuleStyleText";

pub const CSS_MEDIA_LIST_OWNER_KIND_SLOT: &str = "__moliCssMediaListOwnerKind";

pub const CSS_MEDIA_LIST_OWNER_RULE_SLOT: &str = "__moliCssMediaListOwnerRule";

pub const CSS_NESTED_DECLARATIONS_STYLE_TEXT_SLOT: &str = "__moliCssNestedDeclarationsStyleText";

pub const CSS_PROPERTY_RULE_INHERITS_SLOT: &str = "__moliCssPropertyRuleInherits";

pub const CSS_PROPERTY_RULE_INITIAL_VALUE_SLOT: &str = "__moliCssPropertyRuleInitialValue";

pub const CSS_PROPERTY_RULE_NAME_SLOT: &str = "__moliCssPropertyRuleName";

pub const CSS_PROPERTY_RULE_SYNTAX_SLOT: &str = "__moliCssPropertyRuleSyntax";

pub const CSS_RULE_DETACHED_SNAPSHOT_TEXT_SLOT: &str = "__moliCssRuleDetachedSnapshotText";

pub const CSS_RULE_LIST_LENGTH_SLOT: &str = "__moliCssRuleListLength";

pub const CSS_RULE_PARENT_RULE_SLOT: &str = "__moliCssRuleParentRule";

pub const CSS_RULE_PARENT_STYLE_SHEET_SLOT: &str = "__moliCssRuleParentStyleSheet";

pub const CSS_STYLE_RULE_SELECTOR_TEXT_SLOT: &str = "__moliCssStyleRuleSelectorText";

pub const CSS_STYLE_RULE_STYLE_TEXT_SLOT: &str = "__moliCssStyleRuleStyleText";

pub const CSS_STYLE_SHEET_RULES_SLOT: &str = "__moliCssStyleSheetRules";

pub const STYLE_SHEET_LIST_LENGTH_SLOT: &str = "__moliStyleSheetListLength";

const CSS_FONT_FEATURE_VALUES_MAP_BACKING_SLOT: &str = "__moliCssFontFeatureValuesMapBacking";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSStyleSheet)]
pub struct CssStyleSheetDeclaration<'scope> {
    #[webapi(slot = CSS_STYLE_SHEET_RULES_SLOT)]
    pub rules: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::StyleSheetList)]
pub struct StyleSheetListDeclaration {
    #[webapi(slot = STYLE_SHEET_LIST_LENGTH_SLOT, init = 0)]
    pub length: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSRuleList)]
pub struct CssRuleListDeclaration {
    #[webapi(slot = CSS_RULE_LIST_LENGTH_SLOT)]
    pub length: u32,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSRule)]
pub struct CssAtRuleDeclaration<'scope> {
    #[webapi(slot = CSS_RULE_DETACHED_SNAPSHOT_TEXT_SLOT)]
    pub css_text: String,
    #[webapi(slot = CSS_AT_RULE_TYPE_SLOT)]
    pub rule_type: u32,
    #[webapi(slot = CSS_RULE_PARENT_RULE_SLOT)]
    pub parent_rule: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(slot = CSS_RULE_PARENT_STYLE_SHEET_SLOT)]
    pub parent_style_sheet: Option<v8::Local<'scope, v8::Object>>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSMarginRule)]
pub struct CssMarginRuleDeclaration<'scope> {
    #[webapi(slot = CSS_RULE_DETACHED_SNAPSHOT_TEXT_SLOT)]
    pub css_text: String,
    #[webapi(slot = CSS_MARGIN_RULE_NAME_SLOT)]
    pub name: String,
    #[webapi(slot = CSS_MARGIN_RULE_STYLE_TEXT_SLOT)]
    pub style_text: String,
    #[webapi(slot = CSS_RULE_PARENT_RULE_SLOT)]
    pub parent_rule: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(slot = CSS_RULE_PARENT_STYLE_SHEET_SLOT)]
    pub parent_style_sheet: Option<v8::Local<'scope, v8::Object>>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSKeyframeRule)]
pub struct CssKeyframeRuleDeclaration<'scope> {
    #[webapi(slot = CSS_RULE_DETACHED_SNAPSHOT_TEXT_SLOT)]
    pub css_text: String,
    #[webapi(slot = CSS_KEYFRAME_RULE_KEY_TEXT_SLOT)]
    pub key_text: String,
    #[webapi(slot = CSS_KEYFRAME_RULE_STYLE_TEXT_SLOT)]
    pub style_text: String,
    #[webapi(slot = CSS_RULE_PARENT_RULE_SLOT)]
    pub parent_rule: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(slot = CSS_RULE_PARENT_STYLE_SHEET_SLOT)]
    pub parent_style_sheet: Option<v8::Local<'scope, v8::Object>>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSFontFeatureValuesRule)]
pub struct CssFontFeatureValuesRuleDeclaration<'scope> {
    #[webapi(slot = CSS_RULE_DETACHED_SNAPSHOT_TEXT_SLOT)]
    pub css_text: String,
    #[webapi(slot = CSS_FONT_FEATURE_VALUES_RULE_FONT_FAMILY_SLOT)]
    pub font_family: String,
    #[webapi(slot = CSS_RULE_PARENT_RULE_SLOT)]
    pub parent_rule: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(slot = CSS_RULE_PARENT_STYLE_SHEET_SLOT)]
    pub parent_style_sheet: Option<v8::Local<'scope, v8::Object>>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSPropertyRule)]
pub struct CssPropertyRuleDeclaration<'scope> {
    #[webapi(slot = CSS_RULE_DETACHED_SNAPSHOT_TEXT_SLOT)]
    pub css_text: String,
    #[webapi(slot = CSS_AT_RULE_TYPE_SLOT)]
    pub rule_type: u32,
    #[webapi(slot = CSS_PROPERTY_RULE_NAME_SLOT)]
    pub name: String,
    #[webapi(slot = CSS_PROPERTY_RULE_SYNTAX_SLOT)]
    pub syntax: String,
    #[webapi(slot = CSS_PROPERTY_RULE_INHERITS_SLOT)]
    pub inherits: bool,
    #[webapi(slot = CSS_PROPERTY_RULE_INITIAL_VALUE_SLOT)]
    pub initial_value: Option<String>,
    #[webapi(slot = CSS_RULE_PARENT_RULE_SLOT)]
    pub parent_rule: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(slot = CSS_RULE_PARENT_STYLE_SHEET_SLOT)]
    pub parent_style_sheet: Option<v8::Local<'scope, v8::Object>>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSStyleRule)]
pub struct CssStyleRuleDeclaration<'scope> {
    #[webapi(slot = CSS_RULE_DETACHED_SNAPSHOT_TEXT_SLOT)]
    pub css_text: String,
    #[webapi(slot = CSS_STYLE_RULE_SELECTOR_TEXT_SLOT)]
    pub selector_text: String,
    #[webapi(slot = CSS_STYLE_RULE_STYLE_TEXT_SLOT)]
    pub style_text: String,
    #[webapi(slot = CSS_RULE_PARENT_RULE_SLOT)]
    pub parent_rule: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(slot = CSS_RULE_PARENT_STYLE_SHEET_SLOT)]
    pub parent_style_sheet: Option<v8::Local<'scope, v8::Object>>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSNestedDeclarations)]
pub struct CssNestedDeclarationsRuleDeclaration<'scope> {
    #[webapi(slot = CSS_RULE_DETACHED_SNAPSHOT_TEXT_SLOT)]
    pub css_text: String,
    #[webapi(slot = CSS_NESTED_DECLARATIONS_STYLE_TEXT_SLOT)]
    pub style_text: String,
    #[webapi(slot = CSS_RULE_PARENT_RULE_SLOT)]
    pub parent_rule: v8::Local<'scope, v8::Object>,
    #[webapi(slot = CSS_RULE_PARENT_STYLE_SHEET_SLOT)]
    pub parent_style_sheet: Option<v8::Local<'scope, v8::Object>>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MediaList)]
pub struct MediaListDeclaration<'scope> {
    #[webapi(slot = CSS_MEDIA_LIST_OWNER_RULE_SLOT)]
    pub owner: v8::Local<'scope, v8::Object>,
    #[webapi(slot = CSS_MEDIA_LIST_OWNER_KIND_SLOT)]
    pub owner_kind: &'static str,
}

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = web_api_interfaces::CSSFontFeatureValuesMap)]
struct CssFontFeatureValuesMapObjectDeclaration<'s> {
    #[webapi(slot = CSS_FONT_FEATURE_VALUES_MAP_BACKING_SLOT)]
    backing: v8::Local<'s, v8::Map>,
}

pub fn new_css_font_feature_values_map<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backing: v8::Local<'s, v8::Map>,
    prototype: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let map = CssFontFeatureValuesMapObjectDeclaration::new(backing)
        .bind(scope)
        .ok()?;
    (map.set_prototype(scope, prototype.into()) == Some(true)).then_some(map)
}

pub fn css_font_feature_values_map_backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Map>> {
    get_private_value(scope, map, CSS_FONT_FEATURE_VALUES_MAP_BACKING_SLOT)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
}
