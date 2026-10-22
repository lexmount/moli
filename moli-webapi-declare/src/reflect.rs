use moli_v8_util::{get_private_value, private_key, v8str};

const REFLECT_SET: &str = "moli.webidl.reflectSet";

/// Capture before author script runs. Native exotic objects use Reflect.set
/// for its original receiver semantics, independently of constructor dispatch.
pub fn capture_web_api_reflect_set(scope: &mut v8::PinScope<'_, '_>) -> Option<()> {
    let global = scope.get_current_context().global(scope);
    if web_api_reflect_set(scope).is_some() {
        return Some(());
    }
    let reflect = global.get(scope, v8str(scope, "Reflect").into())?;
    let reflect = v8::Local::<v8::Object>::try_from(reflect).ok()?;
    let function = reflect.get(scope, v8str(scope, "set").into())?;
    let function = v8::Local::<v8::Function>::try_from(function).ok()?;
    let key = private_key(scope, REFLECT_SET)?;
    global
        .set_private(scope, key, function.into())?
        .then_some(())
}

/// The realm's captured Reflect.set, unaffected by author replacements.
pub fn web_api_reflect_set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Function>> {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, REFLECT_SET)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
}
