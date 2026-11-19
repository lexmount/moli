//! Initializer dictionaries used when the embedder invokes an event constructor.

use moli_webapi_declare::WebApiObject;

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
