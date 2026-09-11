use super::*;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct KeyboardEventInitDeclaration<'scope> {
    view: v8::Local<'scope, v8::Value>,
    detail: f64,
    key: v8::Local<'scope, v8::String>,
    code: v8::Local<'scope, v8::String>,
    location: f64,
    char_code: f64,
    key_code: f64,
    which: f64,
    repeat: bool,
    is_composing: bool,
    ctrl_key: bool,
    shift_key: bool,
    alt_key: bool,
    meta_key: bool,
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_keyboard_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let Ok(view) = init_window_view_property(scope, init, "KeyboardEvent") else {
        return false;
    };
    let detail = init_number_property(scope, init, "detail", 0.0);
    let key = init_string_property(scope, init, "key", "");
    let code = init_string_property(scope, init, "code", "");
    let key = v8_string(scope, &key).expect("KeyboardEvent key");
    let code = v8_string(scope, &code).expect("KeyboardEvent code");
    let location = init_number_property(scope, init, "location", 0.0);
    KeyboardEventInitDeclaration::new(
        view,
        detail,
        key,
        code,
        location,
        init_number_property(scope, init, "charCode", 0.0),
        init_number_property(scope, init, "keyCode", 0.0),
        init_number_property(scope, init, "which", 0.0),
        init_bool_property(scope, init, "repeat", false),
        init_bool_property(scope, init, "isComposing", false),
        init_bool_property(scope, init, "ctrlKey", false),
        init_bool_property(scope, init, "shiftKey", false),
        init_bool_property(scope, init, "altKey", false),
        init_bool_property(scope, init, "metaKey", false),
    )
    .initialize(scope, event)
    .expect("KeyboardEvent init declaration should initialize");
    true
}
