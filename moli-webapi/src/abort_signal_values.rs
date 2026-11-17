use crate::interfaces as web_api_interfaces;
use moli_v8_util::global_constructor_prototype;
use moli_webapi_declare::WebApiObject;
use moli_webidl as webidl;

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = web_api_interfaces::AbortSignal)]
struct AbortSignalObjectDeclaration<'scope> {
    #[webapi(prototype)]
    prototype: v8::Local<'scope, v8::Object>,
}

pub fn new_signal<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = global_constructor_prototype(scope, "AbortSignal")?;
    AbortSignalObjectDeclaration { prototype }.bind(scope).ok()
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AbortSignal.timeout")]
pub struct TimeoutArgs {
    #[webidl(required, converter = "enforce_range_unsigned_long_long")]
    pub milliseconds: u64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AbortSignal.any")]
pub struct AnyArgs<'scope> {
    #[webidl(required, sequence, interface = web_api_interfaces::AbortSignal)]
    pub signals: Vec<v8::Local<'scope, v8::Object>>,
}
