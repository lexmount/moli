use crate::util::v8str;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(plain)]
struct WorkerHostErrorEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    message: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    filename: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    lineno: u32,
    #[webapi(data_property, enumerable)]
    colno: u32,
    #[webapi(data_property, enumerable)]
    cancelable: bool,
    #[webapi(data_property, enumerable)]
    error: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ErrorEvent, prototype = "Object", scope_lifetime = 'scope)]
struct WorkerHostErrorEventFallbackDeclaration<'scope, 'text> {
    #[webapi(data_property, enumerable)]
    r#type: &'static str,
    #[webapi(data_property, enumerable)]
    message: &'text str,
    #[webapi(data_property, enumerable)]
    filename: &'text str,
    #[webapi(data_property, enumerable)]
    lineno: u32,
    #[webapi(data_property, enumerable)]
    colno: u32,
    #[webapi(data_property, enumerable)]
    error: v8::Local<'scope, v8::Value>,
}

pub(in crate::context_bootstrap) fn new_worker_host_error_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    filename: &str,
    lineno: u32,
    colno: u32,
    error: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Object> {
    let global = scope.get_current_context().global(scope);
    if let Some(error_ctor) = global
        .get(scope, v8str(scope, "ErrorEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let init = WorkerHostErrorEventInitDeclaration::new(
            v8::String::new(scope, message).unwrap(),
            v8::String::new(scope, filename).unwrap(),
            lineno,
            colno,
            true,
            error,
        )
        .bind(scope)
        .expect("worker host ErrorEvent init declaration should bind");
        if let Some(event) = error_ctor.new_instance(
            scope,
            &[v8::String::new(scope, "error").unwrap().into(), init.into()],
        ) {
            return event;
        }
    }

    WorkerHostErrorEventFallbackDeclaration::new("error", message, filename, lineno, colno, error)
        .bind(scope)
        .expect("worker host ErrorEvent fallback declaration should bind")
}
