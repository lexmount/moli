//! Captured worker origin and the replaceable public origin property.

use super::*;

const WORKER_GLOBAL_ORIGIN_SLOT: &str = "__moliWorkerGlobalOrigin";

#[derive(WebApiObject)]
#[webapi(plain)]
pub(super) struct WorkerGlobalOriginDeclaration {
    #[webapi(slot = WORKER_GLOBAL_ORIGIN_SLOT)]
    origin: String,
}

#[derive(Default, WebApiObject)]
#[webapi(fragment, prototype = "WorkerGlobalScope", enumerable)]
pub(super) struct WorkerGlobalOriginPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = worker_global_origin_getter,
        setter = worker_global_origin_setter
    )]
    origin: (),
}

fn worker_global_origin_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let global = scope.get_current_context().global(scope);
    if !args.this().strict_equals(global.into()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let origin = get_private_value(scope, global, WORKER_GLOBAL_ORIGIN_SLOT)
        .unwrap_or_else(|| v8str(scope, "null").into());
    rv.set(origin);
}

fn worker_global_origin_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let global = scope.get_current_context().global(scope);
    if !args.this().strict_equals(global.into()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    // [Replaceable] creates an own data property without converting the value
    // or changing the origin returned by the original getter.
    match global.define_own_property(
        scope,
        v8str(scope, "origin").into(),
        args.get(0),
        v8::PropertyAttribute::NONE,
    ) {
        Some(true) => {}
        Some(false) => throw_type_error(scope, "Cannot redefine WorkerGlobalScope.origin"),
        None => {}
    }
}

pub(crate) fn worker_global_origin(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<moli_url::WebOrigin> {
    let global = scope.get_current_context().global(scope);
    let origin = get_private_value(scope, global, WORKER_GLOBAL_ORIGIN_SLOT)?;
    let origin = v8::Local::<v8::String>::try_from(origin).ok()?;
    Some(moli_url::WebOrigin::from_serialized(
        &origin.to_rust_string_lossy(scope),
    ))
}
