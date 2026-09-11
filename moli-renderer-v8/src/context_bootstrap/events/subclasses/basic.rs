use super::*;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct UiEventInitDeclaration<'scope> {
    view: v8::Local<'scope, v8::Value>,
    detail: f64,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct FocusEventInitDeclaration<'scope> {
    view: v8::Local<'scope, v8::Value>,
    detail: f64,
    #[webapi(data_property = "relatedTarget")]
    related_target: v8::Local<'scope, v8::Value>,
    composed: bool,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct TextEventInitDeclaration<'scope> {
    view: v8::Local<'scope, v8::Value>,
    detail: f64,
    data: v8::Local<'scope, v8::String>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct CustomEventInitDeclaration<'scope> {
    detail: v8::Local<'scope, v8::Value>,
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_ui_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let Ok(view) = init_window_view_property(scope, init, "UIEvent") else {
        return false;
    };
    let detail = init_number_property(scope, init, "detail", 0.0);
    UiEventInitDeclaration::new(view, detail)
        .initialize(scope, event)
        .expect("UIEvent init declaration should initialize");
    true
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_focus_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let Ok(view) = init_window_view_property(scope, init, "FocusEvent") else {
        return false;
    };
    let detail = init_number_property(scope, init, "detail", 0.0);
    let related_target =
        init_value_property(scope, init, "relatedTarget").unwrap_or_else(|| v8::null(scope).into());
    let composed = init_bool_property(scope, init, "composed", false);
    FocusEventInitDeclaration::new(view, detail, related_target, composed)
        .initialize(scope, event)
        .expect("FocusEvent init declaration should initialize");
    true
}

pub(in crate::context_bootstrap::events) fn initialize_text_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let Ok(view) = init_window_view_property(scope, init, "TextEvent") else {
        return false;
    };
    let detail = init_number_property(scope, init, "detail", 0.0);
    let data = init_string_property(scope, init, "data", "");
    let data = v8_string(scope, &data).expect("text event data");
    TextEventInitDeclaration::new(view, detail, data)
        .initialize(scope, event)
        .expect("TextEvent init declaration should initialize");
    true
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_composition_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let Ok(view) = init_window_view_property(scope, init, "CompositionEvent") else {
        return false;
    };
    let detail = init_number_property(scope, init, "detail", 0.0);
    let data = init_string_property(scope, init, "data", "");
    let data = v8_string(scope, &data).expect("composition event data");
    TextEventInitDeclaration::new(view, detail, data)
        .initialize(scope, event)
        .expect("CompositionEvent init declaration should initialize");
    true
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_custom_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) {
    let detail =
        init_value_property(scope, init, "detail").unwrap_or_else(|| v8::null(scope).into());
    CustomEventInitDeclaration::new(detail)
        .initialize(scope, event)
        .expect("CustomEvent init declaration should initialize");
}
