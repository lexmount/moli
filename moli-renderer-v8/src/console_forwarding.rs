use crate::util::{get_private_value, v8str};

pub(crate) fn captured_console_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    console_slot: &'static str,
    method_name: &'static str,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Function>)> {
    let global = scope.get_current_context().global(scope);
    let console =
        v8::Local::<v8::Object>::try_from(get_private_value(scope, global, console_slot)?).ok()?;
    let method =
        v8::Local::<v8::Function>::try_from(console.get(scope, v8str(scope, method_name).into())?)
            .ok()?;
    Some((console, method))
}
