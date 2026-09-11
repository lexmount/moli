use super::*;
use crate::web_api_interfaces;
use crate::webidl;
use moli_selector::StyleRuleNamespaceContext;
use moli_webapi_declare::WebApiObject;

pub const CSS_STYLE_SHEET_RULES_SLOT: &str = "__moliCssStyleSheetRules";

pub const CSS_STYLE_SHEET_OWNER_RULE_SLOT: &str = "__moliCssStyleSheetOwnerRule";

pub const CSS_STYLE_SHEET_OWNER_NODE_SLOT: &str = "__moliCssStyleSheetOwnerNode";

pub const CSS_STYLE_SHEET_CONSTRUCTED_SLOT: &str = "__moliCssStyleSheetConstructed";

pub const CSS_STYLE_SHEET_CONSTRUCTOR_DOCUMENT_HANDLE_SLOT: &str =
    "__moliCssStyleSheetConstructorDocumentHandle";

pub const CSS_STYLE_SHEET_BASE_URL_SLOT: &str = "__moliCssStyleSheetBaseUrl";

pub const CSS_STYLE_SHEET_HREF_SLOT: &str = "__moliCssStyleSheetHref";

pub const CSS_STYLE_SHEET_ADOPTED_OWNER_KEYS_SLOT: &str = "__moliCssStyleSheetAdoptedOwnerKeys";

pub const CSS_STYLE_SHEET_ADOPTED_OWNER_ARRAYS_SLOT: &str = "__moliCssStyleSheetAdoptedOwnerArrays";

pub const CSS_STYLE_SHEET_ID_SLOT: &str = "__moliCssStyleSheetId";

pub const CSS_STYLE_SHEET_WRAPPER_LEASE_ID_SLOT: &str = "__moliCssStyleSheetWrapperLeaseId";

pub const CSS_RULE_DETACHED_SNAPSHOT_TEXT_SLOT: &str = "__moliCssRuleDetachedSnapshotText";

pub const CSS_RULE_DETACHED_CHILD_SNAPSHOTS_SLOT: &str = "__moliCssRuleDetachedChildSnapshots";

pub const CSS_RULE_NATIVE_WRAPPER_LEASE_ID_SLOT: &str = "__moliCssRuleNativeWrapperLeaseId";

pub const CSS_RULE_STYLO_DECLARATION_BLOCK_ID_SLOT: &str = "__moliCssRuleStyloDeclarationBlockId";

pub const CSS_RULE_STYLO_DECLARATION_BLOCK_VALID_SLOT: &str =
    "__moliCssRuleStyloDeclarationBlockValid";

pub const CSS_AT_RULE_TYPE_SLOT: &str = "__moliCssRuleType";

pub const CSS_RULE_PARENT_RULE_SLOT: &str = "__moliCssRuleParentRule";

pub const CSS_RULE_PARENT_STYLE_SHEET_SLOT: &str = "__moliCssRuleParentStyleSheet";

pub const CSS_AT_RULE_NESTED_STYLE_TEXT_SLOT: &str = "__moliCssAtRuleNestedStyleText";

pub const CSS_AT_RULE_NESTED_RULES_SLOT: &str = "__moliCssAtRuleNestedRules";

pub const CSS_AT_RULE_STYLE_OBJECT_SLOT: &str = "__moliCssAtRuleStyleObject";

pub const CSS_KEYFRAMES_RULE_RULES_SLOT: &str = "__moliCssKeyframesRuleRules";

pub const CSS_KEYFRAME_RULE_KEY_TEXT_SLOT: &str = "__moliCssKeyframeRuleKeyText";

pub const CSS_KEYFRAME_RULE_STYLE_TEXT_SLOT: &str = "__moliCssKeyframeRuleStyleText";

pub const CSS_KEYFRAME_RULE_STYLE_OBJECT_SLOT: &str = "__moliCssKeyframeRuleStyleObject";

pub const CSS_MARGIN_RULE_NAME_SLOT: &str = "__moliCssMarginRuleName";

pub const CSS_MARGIN_RULE_STYLE_TEXT_SLOT: &str = "__moliCssMarginRuleStyleText";

pub const CSS_MARGIN_RULE_STYLE_OBJECT_SLOT: &str = "__moliCssMarginRuleStyleObject";

pub const CSS_STYLE_RULE_SELECTOR_TEXT_SLOT: &str = "__moliCssStyleRuleSelectorText";

pub const CSS_STYLE_RULE_STYLE_TEXT_SLOT: &str = "__moliCssStyleRuleStyleText";

pub const CSS_STYLE_RULE_STYLE_OBJECT_SLOT: &str = "__moliCssStyleRuleStyleObject";

pub const CSS_STYLE_RULE_NESTED_RULES_SLOT: &str = "__moliCssStyleRuleNestedRules";

pub const CSS_FONT_FEATURE_VALUES_RULE_FONT_FAMILY_SLOT: &str =
    "__moliCssFontFeatureValuesRuleFontFamily";

pub const CSS_FONT_FEATURE_VALUES_RULE_ANNOTATION_SLOT: &str =
    "__moliCssFontFeatureValuesRuleAnnotation";

pub const CSS_FONT_FEATURE_VALUES_RULE_ORNAMENTS_SLOT: &str =
    "__moliCssFontFeatureValuesRuleOrnaments";

pub const CSS_FONT_FEATURE_VALUES_RULE_STYLISTIC_SLOT: &str =
    "__moliCssFontFeatureValuesRuleStylistic";

pub const CSS_FONT_FEATURE_VALUES_RULE_STYLESET_SLOT: &str =
    "__moliCssFontFeatureValuesRuleStyleset";

pub const CSS_FONT_FEATURE_VALUES_RULE_CHARACTER_VARIANT_SLOT: &str =
    "__moliCssFontFeatureValuesRuleCharacterVariant";

pub const CSS_FONT_FEATURE_VALUES_RULE_SWASH_SLOT: &str = "__moliCssFontFeatureValuesRuleSwash";

pub const CSS_FONT_FEATURE_VALUES_MAP_OWNER_RULE_SLOT: &str =
    "__moliCssFontFeatureValuesMapOwnerRule";

pub const CSS_FONT_FEATURE_VALUES_MAP_GROUP_SLOT: &str = "__moliCssFontFeatureValuesMapGroup";

pub const CSS_FONT_FEATURE_VALUES_MAP_BACKING_SLOT: &str = "__moliCssFontFeatureValuesMapBacking";

pub const CSS_PROPERTY_RULE_NAME_SLOT: &str = "__moliCssPropertyRuleName";

pub const CSS_PROPERTY_RULE_SYNTAX_SLOT: &str = "__moliCssPropertyRuleSyntax";

pub const CSS_PROPERTY_RULE_INHERITS_SLOT: &str = "__moliCssPropertyRuleInherits";

pub const CSS_PROPERTY_RULE_INITIAL_VALUE_SLOT: &str = "__moliCssPropertyRuleInitialValue";

pub const CSS_NESTED_DECLARATIONS_STYLE_TEXT_SLOT: &str = "__moliCssNestedDeclarationsStyleText";

pub const CSS_NESTED_DECLARATIONS_STYLE_OBJECT_SLOT: &str =
    "__moliCssNestedDeclarationsStyleObject";

pub const CSS_STYLE_SHEET_MEDIA_LIST_SLOT: &str = "__moliCssStyleSheetMediaList";

pub const CSS_MEDIA_RULE_MEDIA_LIST_SLOT: &str = "__moliCssMediaRuleMediaList";

pub const CSS_IMPORT_RULE_MEDIA_LIST_SLOT: &str = "__moliCssImportRuleMediaList";

pub const CSS_IMPORT_RULE_STYLE_SHEET_SLOT: &str = "__moliCssImportRuleStyleSheet";

pub const CSS_MEDIA_LIST_OWNER_RULE_SLOT: &str = "__moliCssMediaListOwnerRule";

pub const CSS_MEDIA_LIST_OWNER_KIND_SLOT: &str = "__moliCssMediaListOwnerKind";

pub const CSS_MEDIA_LIST_LENGTH_SLOT: &str = "__moliCssMediaListLength";

pub const STYLE_SHEET_LIST_LENGTH_SLOT: &str = "__moliStyleSheetListLength";

pub const CSS_RULE_LIST_LENGTH_SLOT: &str = "__moliCssRuleListLength";

pub const CSS_RULE_LIST_MATERIALIZED_ITEMS_SLOT: &str = "__moliCssRuleListMaterializedItems";

pub const CSS_RULE_LIST_DETACHED_SNAPSHOTS_SLOT: &str = "__moliCssRuleListDetachedSnapshots";

pub const CSS_RULE_LIST_PARENT_STYLE_SHEET_SLOT: &str = "__moliCssRuleListParentStyleSheet";

pub const CSS_RULE_LIST_PARENT_RULE_SLOT: &str = "__moliCssRuleListParentRule";

pub const CSS_MEDIA_LIST_OWNER_MEDIA_RULE: &str = "mediaRule";

pub const CSS_MEDIA_LIST_OWNER_IMPORT_RULE: &str = "importRule";

pub const CSS_MEDIA_LIST_OWNER_STYLE_SHEET: &str = "styleSheet";

pub const ADOPTED_STYLE_SHEETS_ARRAY_TRACKED_SHEETS_SLOT: &str =
    "__moliAdoptedStyleSheetsArrayTrackedSheets";

pub const CSS_RULE_UNKNOWN_RULE_TYPE: u32 = 0;

pub const CSS_RULE_STYLE_RULE_TYPE: u32 = 1;

pub const CSS_RULE_IMPORT_RULE_TYPE: u32 = 3;

pub const CSS_RULE_MEDIA_RULE_TYPE: u32 = 4;

pub const CSS_RULE_FONT_FACE_RULE_TYPE: u32 = 5;

pub const CSS_RULE_PAGE_RULE_TYPE: u32 = 6;

pub const CSS_RULE_KEYFRAMES_RULE_TYPE: u32 = 7;

pub const CSS_RULE_KEYFRAME_RULE_TYPE: u32 = 8;

pub const CSS_RULE_MARGIN_RULE_TYPE: u32 = 9;

pub const CSS_RULE_NAMESPACE_RULE_TYPE: u32 = 10;

pub const CSS_RULE_COUNTER_STYLE_RULE_TYPE: u32 = 11;

pub const CSS_RULE_SUPPORTS_RULE_TYPE: u32 = 12;

pub const CSS_RULE_FONT_FEATURE_VALUES_RULE_TYPE: u32 = 14;

pub const CSS_RULE_CONTAINER_RULE_TYPE: u32 = 17;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CssAtRuleKind {
    Unknown,
    Import,
    Layer,
    Media,
    Scope,
    FontFace,
    FontFeatureValues,
    Keyframes,
    Page,
    Namespace,
    CounterStyle,
    Supports,
    Container,
    StartingStyle,
    Property,
    Function,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssStyleRuleTextParts {
    pub css_text: String,
    pub selector_text: String,
    pub style_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssAtRuleTextParts {
    pub kind: CssAtRuleKind,
    pub prelude: String,
    pub block: Option<String>,
}

#[derive(Clone, Copy)]
pub enum CssRulePdbDeclarationKind {
    StyleRule,
    KeyframeRule,
    NestedDeclarations,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSRule)]
pub struct CssRuleConstantsDeclaration {
    #[webapi(constant = "UNKNOWN_RULE", value = CSS_RULE_UNKNOWN_RULE_TYPE)]
    pub unknown_rule: (),

    #[webapi(constant = "STYLE_RULE", value = CSS_RULE_STYLE_RULE_TYPE)]
    pub style_rule: (),

    #[webapi(constant = "CHARSET_RULE", value = 2u32)]
    pub charset_rule: (),

    #[webapi(constant = "IMPORT_RULE", value = CSS_RULE_IMPORT_RULE_TYPE)]
    pub import_rule: (),

    #[webapi(constant = "MEDIA_RULE", value = CSS_RULE_MEDIA_RULE_TYPE)]
    pub media_rule: (),

    #[webapi(constant = "FONT_FACE_RULE", value = CSS_RULE_FONT_FACE_RULE_TYPE)]
    pub font_face_rule: (),

    #[webapi(constant = "PAGE_RULE", value = CSS_RULE_PAGE_RULE_TYPE)]
    pub page_rule: (),

    #[webapi(constant = "KEYFRAMES_RULE", value = CSS_RULE_KEYFRAMES_RULE_TYPE)]
    pub keyframes_rule: (),

    #[webapi(constant = "KEYFRAME_RULE", value = CSS_RULE_KEYFRAME_RULE_TYPE)]
    pub keyframe_rule: (),

    #[webapi(constant = "MARGIN_RULE", value = CSS_RULE_MARGIN_RULE_TYPE)]
    pub margin_rule: (),

    #[webapi(constant = "NAMESPACE_RULE", value = CSS_RULE_NAMESPACE_RULE_TYPE)]
    pub namespace_rule: (),

    #[webapi(
        constant = "COUNTER_STYLE_RULE",
        value = CSS_RULE_COUNTER_STYLE_RULE_TYPE
    )]
    pub counter_style_rule: (),

    #[webapi(constant = "SUPPORTS_RULE", value = CSS_RULE_SUPPORTS_RULE_TYPE)]
    pub supports_rule: (),

    #[webapi(
        constant = "FONT_FEATURE_VALUES_RULE",
        value = CSS_RULE_FONT_FEATURE_VALUES_RULE_TYPE
    )]
    pub font_feature_values_rule: (),

    #[webapi(constant = "VIEWPORT_RULE", value = 15u32)]
    pub viewport_rule: (),

    #[webapi(constant = "REGION_STYLE_RULE", value = 16u32)]
    pub region_style_rule: (),

    #[webapi(constant = "CONTAINER_RULE", value = CSS_RULE_CONTAINER_RULE_TYPE)]
    pub container_rule: (),
}

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

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::StyleSheet, enumerable)]
pub struct StyleSheetBasePrototypeDeclaration {
    #[webapi(accessor_property, getter = css_style_sheet_type_getter_callback)]
    r#type: (),
    #[webapi(
        accessor_property,
        getter = css_style_sheet_disabled_getter_callback,
        setter = css_style_sheet_disabled_setter_callback
    )]
    pub disabled: (),
    #[webapi(accessor_property, getter = css_style_sheet_owner_node_getter_callback)]
    pub owner_node: (),
    #[webapi(accessor_property, getter = css_style_sheet_parent_style_sheet_getter_callback)]
    pub parent_style_sheet: (),
    #[webapi(accessor_property, getter = css_style_sheet_href_getter_callback)]
    pub href: (),
    #[webapi(accessor_property, getter = css_style_sheet_title_getter_callback)]
    pub title: (),
    #[webapi(
        accessor_property,
        getter = css_style_sheet_media_getter_callback,
        setter = css_style_sheet_media_setter_callback
    )]
    pub media: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSStyleSheet, enumerable)]
pub struct CssStyleSheetPrototypeDeclaration {
    #[webapi(accessor_property, getter = css_style_sheet_css_rules_getter_callback)]
    pub css_rules: (),
    #[webapi(accessor_property, getter = css_style_sheet_css_rules_getter_callback)]
    pub rules: (),
    #[webapi(accessor_property, getter = css_style_sheet_owner_rule_getter_callback)]
    pub owner_rule: (),
    #[webapi(method, length = 1, callback = css_style_sheet_insert_rule_callback)]
    pub insert_rule: (),
    #[webapi(method, length = 1, callback = css_style_sheet_delete_rule_callback)]
    pub delete_rule: (),
    #[webapi(method, length = 1, callback = css_style_sheet_replace_callback)]
    pub replace: (),
    #[webapi(method, length = 1, callback = css_style_sheet_replace_sync_callback)]
    pub replace_sync: (),
    #[webapi(method, length = 0, callback = css_style_sheet_remove_rule_callback)]
    pub remove_rule: (),
    #[webapi(method, length = 0, callback = css_style_sheet_add_rule_callback)]
    pub add_rule: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::StyleSheetList, enumerable)]
pub struct StyleSheetListPrototypeDeclaration {
    #[webapi(accessor_property, getter = style_sheet_list_length_getter_callback)]
    pub length: (),
    #[webapi(method, length = 1, callback = style_sheet_list_item_callback)]
    pub item: (),
    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    pub iterator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSRuleList, enumerable)]
pub struct CssRuleListPrototypeDeclaration {
    #[webapi(accessor_property, getter = css_rule_list_length_getter_callback)]
    pub length: (),
    #[webapi(method, length = 1, callback = css_rule_list_item_callback)]
    pub item: (),
    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    pub iterator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaList, enumerable)]
pub struct MediaListPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = media_list_media_text_getter_callback,
        setter = media_list_media_text_setter_callback
    )]
    pub media_text: (),
    #[webapi(accessor_property, getter = media_list_length_getter_callback)]
    pub length: (),
    #[webapi(method, length = 1, callback = media_list_item_callback)]
    pub item: (),
    #[webapi(method, length = 1, callback = media_list_delete_medium_callback)]
    pub delete_medium: (),
    #[webapi(method, length = 1, callback = media_list_append_medium_callback)]
    pub append_medium: (),
    #[webapi(method = "toString", length = 0, callback = media_list_to_string_callback)]
    pub to_string: (),
    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    pub iterator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "LinkStyle", enumerable)]
pub struct LinkStylePrototypeDeclaration {
    #[webapi(accessor_property, getter = crate::native_bridge::element::style_sheet_getter_function)]
    pub sheet: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSRule, enumerable)]
pub struct CssRulePrototypeDeclaration {
    #[webapi(accessor_property, getter = css_rule_type_getter_callback)]
    r#type: (),
    #[webapi(
        accessor_property,
        getter = css_rule_css_text_getter_callback,
        setter = css_rule_css_text_setter_callback
    )]
    pub css_text: (),
    #[webapi(accessor_property, getter = css_rule_parent_rule_getter_callback)]
    pub parent_rule: (),
    #[webapi(accessor_property, getter = css_rule_parent_style_sheet_getter_callback)]
    pub parent_style_sheet: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSGroupingRule, enumerable)]
pub struct CssGroupingRulePrototypeDeclaration {
    #[webapi(accessor_property, getter = css_grouping_rule_css_rules_getter_callback)]
    pub css_rules: (),
    #[webapi(method, length = 1, callback = css_grouping_rule_insert_rule_callback)]
    pub insert_rule: (),
    #[webapi(method, length = 1, callback = css_grouping_rule_delete_rule_callback)]
    pub delete_rule: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSConditionRule, enumerable)]
pub struct CssConditionRulePrototypeDeclaration {
    #[webapi(accessor_property, getter = css_condition_rule_condition_text_getter_callback)]
    pub condition_text: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSMediaRule, enumerable)]
pub struct CssMediaRulePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = css_media_rule_media_getter_callback,
        setter = css_media_rule_media_setter_callback
    )]
    pub media: (),
    #[webapi(accessor_property, getter = css_media_rule_matches_getter_callback)]
    pub matches: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSSupportsRule, enumerable)]
pub struct CssSupportsRulePrototypeDeclaration {
    #[webapi(accessor_property, getter = css_supports_rule_matches_getter_callback)]
    pub matches: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSContainerRule, enumerable)]
pub struct CssContainerRulePrototypeDeclaration {
    #[webapi(accessor_property, getter = css_container_rule_container_name_getter_callback)]
    pub container_name: (),
    #[webapi(accessor_property, getter = css_container_rule_container_query_getter_callback)]
    pub container_query: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSLayerBlockRule, enumerable)]
pub struct CssLayerBlockRulePrototypeDeclaration {
    #[webapi(accessor_property, getter = css_layer_block_rule_name_getter_callback)]
    pub name: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSLayerStatementRule, enumerable)]
pub struct CssLayerStatementRulePrototypeDeclaration {
    #[webapi(accessor_property, getter = css_layer_statement_rule_name_list_getter_callback)]
    pub name_list: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSScopeRule, enumerable)]
pub struct CssScopeRulePrototypeDeclaration {
    #[webapi(accessor_property, getter = css_scope_rule_start_getter_callback)]
    pub start: (),
    #[webapi(accessor_property, getter = css_scope_rule_end_getter_callback)]
    pub end: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSImportRule, enumerable)]
pub struct CssImportRulePrototypeDeclaration {
    #[webapi(accessor_property, getter = css_import_rule_href_getter_callback)]
    pub href: (),
    #[webapi(
        accessor_property,
        getter = css_import_rule_media_getter_callback,
        setter = css_import_rule_media_setter_callback
    )]
    pub media: (),
    #[webapi(accessor_property, getter = css_import_rule_style_sheet_getter_callback)]
    pub style_sheet: (),
    #[webapi(accessor_property, getter = css_import_rule_layer_name_getter_callback)]
    pub layer_name: (),
    #[webapi(accessor_property, getter = css_import_rule_supports_text_getter_callback)]
    pub supports_text: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSFontFaceRule, enumerable)]
pub struct CssFontFaceRulePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = css_font_face_rule_style_getter_callback,
        setter = css_font_face_rule_style_setter_callback
    )]
    pub style: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSMarginRule, enumerable)]
pub struct CssMarginRulePrototypeDeclaration {
    #[webapi(accessor_property, getter = css_margin_rule_name_getter_callback)]
    pub name: (),
    #[webapi(
        accessor_property,
        getter = css_margin_rule_style_getter_callback,
        setter = css_margin_rule_style_setter_callback
    )]
    pub style: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSFontFeatureValuesRule, enumerable)]
pub struct CssFontFeatureValuesRulePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = css_font_feature_values_rule_font_family_getter_callback,
        setter = css_font_feature_values_rule_font_family_setter_callback
    )]
    pub font_family: (),
    #[webapi(accessor_property, getter = css_font_feature_values_rule_annotation_getter_callback)]
    pub annotation: (),
    #[webapi(accessor_property, getter = css_font_feature_values_rule_ornaments_getter_callback)]
    pub ornaments: (),
    #[webapi(accessor_property, getter = css_font_feature_values_rule_stylistic_getter_callback)]
    pub stylistic: (),
    #[webapi(accessor_property, getter = css_font_feature_values_rule_styleset_getter_callback)]
    pub styleset: (),
    #[webapi(
        accessor_property,
        getter = css_font_feature_values_rule_character_variant_getter_callback
    )]
    pub character_variant: (),
    #[webapi(accessor_property, getter = css_font_feature_values_rule_swash_getter_callback)]
    pub swash: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSPropertyRule, enumerable)]
pub struct CssPropertyRulePrototypeDeclaration {
    #[webapi(accessor_property, getter = css_property_rule_name_getter_callback)]
    pub name: (),
    #[webapi(accessor_property, getter = css_property_rule_syntax_getter_callback)]
    pub syntax: (),
    #[webapi(accessor_property, getter = css_property_rule_inherits_getter_callback)]
    pub inherits: (),
    #[webapi(
        accessor_property = "initialValue",
        getter = css_property_rule_initial_value_getter_callback
    )]
    pub initial_value: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSPageRule, enumerable)]
pub struct CssPageRulePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = css_page_rule_style_getter_callback,
        setter = css_page_rule_style_setter_callback
    )]
    pub style: (),
    #[webapi(
        accessor_property,
        getter = css_page_rule_selector_text_getter_callback,
        setter = css_page_rule_selector_text_setter_callback
    )]
    pub selector_text: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSKeyframesRule, enumerable)]
pub struct CssKeyframesRulePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = css_keyframes_rule_name_getter_callback,
        setter = css_keyframes_rule_name_setter_callback
    )]
    pub name: (),
    #[webapi(accessor_property, getter = css_keyframes_rule_css_rules_getter_callback)]
    pub css_rules: (),
    #[webapi(accessor_property, getter = css_keyframes_rule_length_getter_callback)]
    pub length: (),
    #[webapi(method, length = 1, callback = css_keyframes_rule_append_rule_callback)]
    pub append_rule: (),
    #[webapi(method, length = 1, callback = css_keyframes_rule_delete_rule_callback)]
    pub delete_rule: (),
    #[webapi(method, length = 1, callback = css_keyframes_rule_find_rule_callback)]
    pub find_rule: (),
    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    pub iterator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSKeyframeRule, enumerable)]
pub struct CssKeyframeRulePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = css_keyframe_rule_key_text_getter_callback,
        setter = css_keyframe_rule_key_text_setter_callback
    )]
    pub key_text: (),
    #[webapi(
        accessor_property,
        getter = css_keyframe_rule_style_getter_callback,
        setter = css_keyframe_rule_style_setter_callback
    )]
    pub style: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSNamespaceRule, enumerable)]
pub struct CssNamespaceRulePrototypeDeclaration {
    #[webapi(
        accessor_property = "namespaceURI",
        getter = css_namespace_rule_namespace_uri_getter_callback
    )]
    pub namespace_uri: (),
    #[webapi(accessor_property, getter = css_namespace_rule_prefix_getter_callback)]
    pub prefix: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSCounterStyleRule, enumerable)]
pub struct CssCounterStyleRulePrototypeDeclaration {
    #[webapi(accessor_property, getter = css_counter_style_rule_name_getter_callback)]
    pub name: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSStyleRule, enumerable)]
pub struct CssStyleRulePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = css_style_rule_selector_text_getter_callback,
        setter = css_style_rule_selector_text_setter_callback
    )]
    pub selector_text: (),
    #[webapi(
        accessor_property,
        getter = css_style_rule_style_getter_callback,
        setter = css_style_rule_style_setter_callback
    )]
    pub style: (),
    #[webapi(accessor_property, getter = css_style_rule_css_rules_getter_callback)]
    pub css_rules: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSNestedDeclarations, enumerable)]
pub struct CssNestedDeclarationsPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = css_nested_declarations_style_getter_callback,
        setter = css_nested_declarations_style_setter_callback
    )]
    pub style: (),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CssStyleSheetAdoptedOwnerKey {
    Document(DomHandle),
    ShadowRoot(DomHandle),
}

#[derive(Clone, Copy)]
pub struct CssStyleSheetAdoptedOwner<'s> {
    pub key: CssStyleSheetAdoptedOwnerKey,
    pub array: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSStyleSheet.insertRule")]
pub struct CssStyleSheetInsertRuleArgs {
    #[webidl(required)]
    pub rule: String,
    #[webidl(default = 0)]
    pub index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSStyleSheet.deleteRule")]
pub struct CssStyleSheetDeleteRuleArgs {
    #[webidl(required)]
    pub index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSStyleSheet.removeRule")]
pub struct CssStyleSheetRemoveRuleArgs {
    #[webidl(default = 0)]
    pub index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSGroupingRule.insertRule")]
pub struct CssGroupingRuleInsertRuleArgs {
    #[webidl(required)]
    pub rule: String,
    #[webidl(default = 0)]
    pub index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSGroupingRule.deleteRule")]
pub struct CssGroupingRuleDeleteRuleArgs {
    #[webidl(required)]
    pub index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSKeyframesRule.appendRule")]
pub struct CssKeyframesRuleAppendRuleArgs {
    #[webidl(required)]
    pub rule: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSKeyframesRule.deleteRule")]
pub struct CssKeyframesRuleDeleteRuleArgs {
    #[webidl(required)]
    pub key: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSKeyframesRule.findRule")]
pub struct CssKeyframesRuleFindRuleArgs {
    #[webidl(required)]
    pub key: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSKeyframeRule.keyText")]
pub struct CssKeyframeRuleKeyTextArgs {
    #[webidl(required)]
    pub key_text: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSStyleSheet.replace")]
pub struct CssStyleSheetReplaceArgs {
    #[webidl(required, converter = "usv_string")]
    pub text: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "StyleSheetList.item")]
pub struct StyleSheetListItemArgs {
    #[webidl(required)]
    pub index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSRuleList.item")]
pub struct CssRuleListItemArgs {
    #[webidl(required)]
    pub index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaList.item")]
pub struct MediaListItemArgs {
    #[webidl(required)]
    pub index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaList.deleteMedium")]
pub struct MediaListDeleteMediumArgs {
    #[webidl(required)]
    pub medium: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaList.appendMedium")]
pub struct MediaListAppendMediumArgs {
    #[webidl(required)]
    pub medium: String,
}

#[derive(Clone, Default)]
pub struct CssomSelectorNamespaceContext {
    pub default_namespace_uri: Option<String>,
    pub namespace_prefixes: Vec<(String, String)>,
}

impl CssomSelectorNamespaceContext {
    pub(crate) fn record_rule_text(&mut self, css_text: &str) {
        let Some(namespace) = parse_namespace_rule_view_with_stylo(css_text) else {
            return;
        };
        self.record_namespace_rule_view(&namespace);
    }

    pub(crate) fn record_namespace_rule_view(&mut self, namespace: &CssNamespaceRuleView) {
        if namespace.prefix.is_empty() {
            self.default_namespace_uri = Some(namespace.namespace_uri.clone());
        } else {
            self.namespace_prefixes
                .push((namespace.prefix.clone(), namespace.namespace_uri.clone()));
        }
    }

    pub(crate) fn style_rule_namespace_context(&self) -> StyleRuleNamespaceContext {
        StyleRuleNamespaceContext {
            default_namespace_uri: self.default_namespace_uri.clone(),
            namespace_prefixes: self.namespace_prefixes.clone(),
        }
    }

    pub(crate) fn stylo_parent_rule_texts(&self) -> Vec<String> {
        let mut rule_texts = Vec::new();
        if let Some(uri) = &self.default_namespace_uri {
            rule_texts.push(css_namespace_rule_text_for_stylo_context(None, uri));
        }
        rule_texts.extend(
            self.namespace_prefixes
                .iter()
                .map(|(prefix, uri)| css_namespace_rule_text_for_stylo_context(Some(prefix), uri)),
        );
        rule_texts
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CssRuleOrderKind {
    Import,
    Namespace,
    Other,
}

#[derive(Clone)]
pub struct CssGroupingRuleTextParts {
    pub kind: CssAtRuleKind,
    pub prelude: String,
}
