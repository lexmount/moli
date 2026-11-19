//! Initializer dictionaries used when the embedder invokes an event constructor.

use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
pub struct MouseEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    screen_x: f64,
    screen_y: f64,
    client_x: f64,
    client_y: f64,
    detail: i32,
    button: i32,
    buttons: i32,
    alt_key: bool,
    ctrl_key: bool,
    meta_key: bool,
    shift_key: bool,
    related_target: Option<v8::Local<'scope, v8::Value>>,
    view: Option<v8::Local<'scope, v8::Object>>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
pub struct KeyboardEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    key: v8::Local<'scope, v8::Value>,
    code: v8::Local<'scope, v8::Value>,
    location: u32,
    char_code: f64,
    key_code: f64,
    which: f64,
    alt_key: bool,
    ctrl_key: bool,
    meta_key: bool,
    shift_key: bool,
    repeat: bool,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
pub struct InputEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    input_type: v8::Local<'scope, v8::String>,
    data: v8::Local<'scope, v8::Value>,
    data_transfer: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
pub struct SimpleEventInitDeclaration {
    #[webapi(prototype, value = v8::null(scope))]
    prototype: (),
    bubbles: bool,
    cancelable: bool,
    composed: bool,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
pub struct SubmitEventInitDeclaration<'scope> {
    #[webapi(prototype, value = v8::null(scope))]
    prototype: (),
    bubbles: bool,
    cancelable: bool,
    submitter: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
pub struct CommandEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    source: v8::Local<'scope, v8::Value>,
    command: v8::Local<'scope, v8::String>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
pub struct ToggleEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    source: v8::Local<'scope, v8::Value>,
    old_state: v8::Local<'scope, v8::String>,
    new_state: v8::Local<'scope, v8::String>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
pub struct InterestEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    source: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
pub struct FormDataEventInitDeclaration<'s> {
    #[webapi(prototype, value = v8::null(scope))]
    prototype: (),
    form_data: v8::Local<'s, v8::Object>,
    bubbles: bool,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
pub struct FocusEventInitDeclaration<'scope> {
    bubbles: bool,
    #[webapi(constructor_default = false)]
    cancelable: bool,
    #[webapi(constructor_default = true)]
    composed: bool,
    related_target: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
pub struct PointerEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    screen_x: f64,
    screen_y: f64,
    client_x: f64,
    client_y: f64,
    button: i32,
    buttons: i32,
    alt_key: bool,
    ctrl_key: bool,
    meta_key: bool,
    shift_key: bool,
    pointer_type: v8::Local<'scope, v8::String>,
    pressure: f64,
    tangential_pressure: f64,
    tilt_x: f64,
    tilt_y: f64,
    twist: f64,
    pointer_id: f64,
    width: f64,
    height: f64,
    is_primary: bool,
    related_target: Option<v8::Local<'scope, v8::Value>>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
pub struct DragEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    screen_x: f64,
    screen_y: f64,
    client_x: f64,
    client_y: f64,
    alt_key: bool,
    ctrl_key: bool,
    meta_key: bool,
    shift_key: bool,
    view: v8::Local<'scope, v8::Object>,
    data_transfer: v8::Local<'scope, v8::Value>,
    related_target: Option<v8::Local<'scope, v8::Value>>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
pub struct WheelEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    screen_x: f64,
    screen_y: f64,
    client_x: f64,
    client_y: f64,
    delta_x: f64,
    delta_y: f64,
    button: i32,
    buttons: i32,
    alt_key: bool,
    ctrl_key: bool,
    meta_key: bool,
    shift_key: bool,
    view: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
pub struct TrackEventInitDeclaration<'scope> {
    track: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
pub struct DispatchedEventInitDeclaration {
    bubbles: bool,
    cancelable: bool,
}

#[derive(WebApiObject)]
#[webapi(plain)]
pub struct PromiseRejectionEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    cancelable: bool,
    #[webapi(data_property, enumerable)]
    promise: v8::Local<'scope, v8::Promise>,
    #[webapi(data_property, enumerable)]
    reason: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
pub struct ServiceWorkerExtendableMessageEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    data: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    origin: v8::Local<'scope, v8::String>,
    #[webapi(data_property = "lastEventId", enumerable)]
    last_event_id: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    source: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    ports: v8::Local<'scope, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
pub struct ReportedErrorEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    cancelable: bool,
    #[webapi(data_property, enumerable)]
    bubbles: bool,
    #[webapi(data_property, enumerable)]
    message: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    filename: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    lineno: f64,
    #[webapi(data_property, enumerable)]
    colno: f64,
    #[webapi(data_property, enumerable)]
    error: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
pub struct NavigationCurrentEntryChangeEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    pub from: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    pub navigation_type: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
pub struct NavigationErrorEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    pub message: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    pub filename: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    pub lineno: u32,
    #[webapi(data_property, enumerable)]
    pub colno: u32,
    #[webapi(data_property, enumerable)]
    pub error: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
pub struct PopStateEventStateDeclaration<'scope> {
    #[webapi(data_property)]
    state: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
pub struct HashChangeEventStateDeclaration {
    #[webapi(data_property = "oldURL")]
    old_url: String,
    #[webapi(data_property = "newURL")]
    new_url: String,
}

#[derive(WebApiObject)]
#[webapi(plain)]
pub struct NavigateEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    pub navigation_type: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    pub destination: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, enumerable)]
    pub can_intercept: bool,
    #[webapi(data_property, enumerable)]
    pub user_initiated: bool,
    #[webapi(data_property, enumerable)]
    pub hash_change: bool,
    #[webapi(data_property, enumerable)]
    pub signal: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    pub form_data: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    pub download_request: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    pub info: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    pub source_element: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    pub cancelable: bool,
}
