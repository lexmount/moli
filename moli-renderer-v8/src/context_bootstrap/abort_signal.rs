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
    #[webidl(required, with = signal_sequence_arg)]
    pub(crate) signals: Vec<v8::Local<'scope, v8::Object>>,
}

struct SignalReference<'scope>(v8::Local<'scope, v8::Object>);

impl<'scope> webidl::WebIdlConverter<'scope> for SignalReference<'scope> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'scope, '_>,
        value: v8::Local<'scope, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        let signal = webidl::convert::<v8::Local<'scope, v8::Object>>(scope, value, context)?;
        if !web_api_interfaces::AbortSignal::is_instance(scope, signal) {
            return Err(webidl::WebIdlError::custom_message(
                "AbortSignal.any: sequence members must be AbortSignal objects.",
            ));
        }
        Ok(Self(signal))
    }
}

fn signal_sequence_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Vec<v8::Local<'s, v8::Object>>, webidl::WebIdlError> {
    let context = webidl::Context::argument("AbortSignal.any", (index + 1) as usize);
    if args.length() <= index {
        return Err(webidl::WebIdlError::missing_required(context));
    }
    webidl::convert::<webidl::Sequence<SignalReference<'s>>>(scope, args.get(index), context)
        .map(|sequence| sequence.0.into_iter().map(|signal| signal.0).collect())
}
