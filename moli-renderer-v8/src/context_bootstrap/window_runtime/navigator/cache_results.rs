//! Result construction runs in the receiver's realm after WebIDL conversion.
use super::*;
use moli_webapi_declare::WebApiObject;

pub(super) fn owner_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    callee_resolver: v8::Local<'s, v8::PromiseResolver>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) -> (
    v8::Local<'s, v8::Context>,
    v8::Local<'s, v8::PromiseResolver>,
) {
    let target = moli_webapi_declare::web_api_object_target(scope, receiver)
        .expect("branded Cache receiver has native identity");
    let owner = target
        .get_creation_context(scope)
        .expect("Cache receiver has an owner realm");
    if owner == scope.get_current_context() {
        return (owner, callee_resolver);
    }
    let scope = &mut v8::ContextScope::new(scope, owner);
    let resolver = v8::PromiseResolver::new(scope).expect("Cache result Promise should allocate");
    rv.set(resolver.get_promise(scope).into());
    (owner, resolver)
}

pub(super) fn freeze_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Array>,
) -> Option<v8::Local<'s, v8::Array>> {
    array
        .set_integrity_level(scope, v8::IntegrityLevel::Frozen)?
        .then_some(array)
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct CachedResponseInitDeclaration<'s> {
    status: u32,
    status_text: v8::Local<'s, v8::String>,
}

fn without_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    object
        .set_prototype(scope, v8::null(scope).into())?
        .then_some(object)
}

pub(super) fn response_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    status: u16,
    status_text: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let status_text = v8_string(scope, status_text)?;
    let init = CachedResponseInitDeclaration::new(status as u32, status_text)
        .bind(scope)
        .ok()?;
    without_prototype(scope, init)
}
