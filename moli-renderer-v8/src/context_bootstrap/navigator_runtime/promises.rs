use crate::util::v8_string;
use crate::webidl;

pub(super) fn try_parse_promise_args<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> std::result::Result<T, v8::Local<'s, v8::Value>>
where
    T: webidl::WebIdlArguments<'s>,
{
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    match webidl::try_parse_args::<T>(&mut scope, args) {
        Ok(parsed) => Ok(parsed),
        Err(error) if error.is_pending_exception() => Err(scope
            .exception()
            .unwrap_or_else(|| v8::undefined(&scope).into())),
        Err(error) => Err(type_error_value(&mut scope, &error.to_string())),
    }
}

pub(super) fn type_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> v8::Local<'s, v8::Value> {
    let message = v8_string(scope, message).unwrap_or_else(|| v8::String::empty(scope));
    v8::Exception::type_error(scope, message)
}

pub(super) fn set_rejected_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    reason: v8::Local<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    let _ = resolver.reject(scope, reason);
    rv.set(promise.into());
}
