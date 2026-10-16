use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::ProgressEvent, prototype = "Object",
    scope_lifetime = 'scope,
    data_properties,
    enumerable
)]
struct FileReaderEventFallbackDeclaration<'scope, 'event> {
    r#type: &'event str,
    target: v8::Local<'scope, v8::Object>,
    bubbles: bool,
    cancelable: bool,
    loaded: f64,
    total: f64,
    length_computable: bool,
}

pub(in crate::context_bootstrap::file_api::file_reader) fn dispatch_file_reader_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
    event_type: &str,
    loaded: f64,
    total: f64,
) {
    let event = FileReaderEventFallbackDeclaration {
        r#type: event_type,
        target: reader,
        bubbles: false,
        cancelable: false,
        loaded,
        total,
        length_computable: true,
    }
    .bind(scope)
    .expect("FileReader event fallback declaration should bind");
    let _ = dispatch_simple_event_target_event(scope, reader, event_type, event);
}
