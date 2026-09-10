use super::*;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = "CSSFontFeatureValuesMap")]
struct CssFontFeatureValuesMapObjectDeclaration<'s> {
    #[webapi(slot = CSS_FONT_FEATURE_VALUES_MAP_BACKING_SLOT)]
    backing: v8::Local<'s, v8::Map>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "CSSFontFeatureValuesMap", enumerable)]
struct CssFontFeatureValuesMapPrototypeDeclaration {
    #[webapi(accessor_property, getter = css_font_feature_values_map_size_getter_callback)]
    size: (),
    #[webapi(
        method,
        callback = css_font_feature_values_map_get_callback,
        length = 1
    )]
    get: (),
    #[webapi(
        method,
        callback = css_font_feature_values_map_has_callback,
        length = 1
    )]
    has: (),
    #[webapi(
        method,
        callback = css_font_feature_values_map_set_callback,
        length = 2
    )]
    set: (),
    #[webapi(
        method,
        callback = css_font_feature_values_map_delete_callback,
        length = 1
    )]
    delete: (),
    #[webapi(
        method,
        callback = css_font_feature_values_map_clear_callback,
        length = 0
    )]
    clear: (),
    #[webapi(
        method,
        callback = css_font_feature_values_map_entries_callback,
        length = 0
    )]
    entries: (),
    #[webapi(
        method,
        callback = css_font_feature_values_map_keys_callback,
        length = 0
    )]
    keys: (),
    #[webapi(
        method,
        callback = css_font_feature_values_map_values_callback,
        length = 0
    )]
    values: (),
    #[webapi(
        method = "forEach",
        callback = css_font_feature_values_map_for_each_callback,
        length = 1
    )]
    for_each: (),
    #[webapi(alias = "entries", symbol = "iterator")]
    iterator: (),
}

pub(in crate::context_bootstrap) fn install_css_stylesheet_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "StyleSheet" => {
            StyleSheetBasePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSStyleSheet" => {
            CssStyleSheetPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "StyleSheetList" => {
            StyleSheetListPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSRuleList" => {
            CssRuleListPrototypeDeclaration::initialize_prototype_template(scope, prototype);
            install_css_rule_list_indexed_property_handler(scope, template);
        }
        "MediaList" => {
            MediaListPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSRule" => {
            CssRuleConstantsDeclaration::initialize_template(scope, template);
            CssRuleConstantsDeclaration::initialize_prototype_template(scope, prototype);
            CssRulePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSGroupingRule" => {
            CssGroupingRulePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSConditionRule" => {
            CssConditionRulePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSMediaRule" => {
            CssMediaRulePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSSupportsRule" => {
            CssSupportsRulePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSContainerRule" => {
            CssContainerRulePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSLayerBlockRule" => {
            CssLayerBlockRulePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSLayerStatementRule" => {
            CssLayerStatementRulePrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "CSSScopeRule" => {
            CssScopeRulePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSImportRule" => {
            CssImportRulePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSFontFaceRule" => {
            CssFontFaceRulePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSMarginRule" => {
            CssMarginRulePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSFontFeatureValuesRule" => {
            CssFontFeatureValuesRulePrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "CSSPropertyRule" => {
            CssPropertyRulePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSPageRule" => {
            CssPageRulePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSKeyframesRule" => {
            CssKeyframesRulePrototypeDeclaration::initialize_prototype_template(scope, prototype);
            install_css_keyframes_rule_indexed_property_handler(scope, template);
        }
        "CSSKeyframeRule" => {
            CssKeyframeRulePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSNamespaceRule" => {
            CssNamespaceRulePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSCounterStyleRule" => {
            CssCounterStyleRulePrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "CSSStyleRule" => {
            CssStyleRulePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSNestedDeclarations" => {
            CssNestedDeclarationsPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "CSSFontFeatureValuesMap" => {
            CssFontFeatureValuesMapPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "HTMLStyleElement" | "HTMLLinkElement" | "SVGStyleElement" | "ProcessingInstruction" => {
            LinkStylePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}

pub(crate) fn install_css_rule_list_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) {
    CssRuleListDeclaration {
        length: css_rule_list_length(scope, list),
    }
    .bind_into(scope, list)
    .expect("CSSRuleList declaration should bind into list");
}

pub(crate) fn new_css_font_feature_values_map<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backing: v8::Local<'s, v8::Map>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = ensure_intrinsic_interface_prototype(scope, "CSSFontFeatureValuesMap").ok()?;
    let map = CssFontFeatureValuesMapObjectDeclaration::new(backing)
        .bind(scope)
        .ok()?;
    (map.set_prototype(scope, prototype.into()) == Some(true)).then_some(map)
}

pub(crate) fn css_font_feature_values_map_backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Map>> {
    get_private_value(scope, map, CSS_FONT_FEATURE_VALUES_MAP_BACKING_SLOT)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
}
