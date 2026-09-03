use super::*;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(
    interface = "Object",
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

pub(in crate::context_bootstrap) fn file_reader_add_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !file_reader_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    simple_object_event_target_add_listener(scope, &args, FILE_READER_LISTENERS_SLOT);
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn file_reader_remove_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !file_reader_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    simple_object_event_target_remove_listener(scope, &args, FILE_READER_LISTENERS_SLOT);
    rv.set_undefined();
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
    let _ = dispatch_simple_event_target_event(
        scope,
        reader,
        FILE_READER_LISTENERS_SLOT,
        event_type,
        event,
    );
}
