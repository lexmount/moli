use super::ensure_intrinsic_interface_prototype;
use crate::{web_api_interfaces, webidl};
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = web_api_interfaces::AbortSignal)]
struct AbortSignalObjectDeclaration<'scope> {
    #[webapi(prototype)]
    prototype: v8::Local<'scope, v8::Object>,
}

pub(crate) fn new_signal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = ensure_intrinsic_interface_prototype(scope, "AbortSignal").ok()?;
    AbortSignalObjectDeclaration { prototype }.bind(scope).ok()
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AbortSignal.timeout")]
pub(crate) struct TimeoutArgs {
    #[webidl(required, converter = "enforce_range_unsigned_long_long")]
    pub(crate) milliseconds: u64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AbortSignal.any")]
pub(crate) struct AnyArgs<'scope> {
    #[webidl(required, sequence, interface = web_api_interfaces::AbortSignal)]
    pub(crate) signals: Vec<v8::Local<'scope, v8::Object>>,
}
