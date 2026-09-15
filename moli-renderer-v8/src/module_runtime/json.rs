use std::pin::pin;

use crate::util::{get_private_value, private_key};

const JSON_MODULE_SOURCE_URL_SLOT: &str = "__moliJsonModuleSourceUrl";

/// Parse before graph evaluation and retain the loader's response URL with
/// the original exception. V8's JSON parser supplies the line and column, but
/// the existing JSON binding does not accept a source URL. A private slot
/// keeps that host metadata independent of author-visible error properties.
pub(crate) fn parse_json_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: &str,
    source_url: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let source = v8::String::new(scope, source)?;
    let try_catch = pin!(v8::TryCatch::new(scope));
    let scope = &mut try_catch.init();
    if let Some(value) = v8::json::parse(scope, source) {
        return Some(value);
    }
    if let Some(exception) = scope.exception()
        && let Ok(exception) = v8::Local::<v8::Object>::try_from(exception)
        && let Some(source_url) = v8::String::new(scope, source_url)
        && let Some(key) = private_key(scope, JSON_MODULE_SOURCE_URL_SLOT)
    {
        let _ = exception.set_private(scope, key, source_url.into());
    }
    let _ = scope.rethrow();
    None
}

pub(crate) fn json_module_exception_source_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    exception: v8::Local<'s, v8::Value>,
) -> Option<String> {
    if !exception.is_native_error() {
        return None;
    }
    let exception = v8::Local::<v8::Object>::try_from(exception).ok()?;
    let value = get_private_value(scope, exception, JSON_MODULE_SOURCE_URL_SLOT)?;
    let value = v8::Local::<v8::String>::try_from(value).ok()?;
    Some(value.to_rust_string_lossy(scope))
}
