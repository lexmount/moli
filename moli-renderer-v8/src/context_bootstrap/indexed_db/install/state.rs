use super::*;
use crate::{util::context_host_ptr_from_window_object, web_api_interfaces, webidl};
use moli_webapi_declare::WebApiObject;

#[derive(Default, WebApiObject)]
#[webapi(
    fragment,
    prototype = "WorkerGlobalScope",
    enumerable,
    receiver = web_api_interfaces::WorkerGlobalScope::is_instance
)]
struct WorkerIndexedDbPrototypeDeclaration {
    #[webapi(accessor_property = "indexedDB", getter = worker_indexed_db_getter)]
    indexed_db: (),
}

pub(in crate::context_bootstrap) fn ensure_indexed_db_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    indexed_db_runtime_factory(scope)
}

pub(crate) fn install_worker_indexed_db_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<()> {
    let prototype = crate::util::global_constructor_prototype(scope, "WorkerGlobalScope")
        .ok_or_else(|| anyhow!("WorkerGlobalScope prototype missing during IndexedDB bootstrap"))?;
    WorkerIndexedDbPrototypeDeclaration::default()
        .initialize(scope, prototype)
        .map_err(|error| anyhow!("failed to install WorkerGlobalScope indexedDB getter: {error}"))
}

fn worker_indexed_db_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(context) = args.this().get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    // Laziness and SameObject identity belong to the realm's private cache,
    // independently of author properties shadowing the prototype getter.
    match ensure_indexed_db_runtime_state(scope) {
        Some(factory) => rv.set(factory.into()),
        None => rv.set(v8::undefined(scope).into()),
    }
}

pub(crate) fn window_indexed_db_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if context_host_ptr_from_window_object(scope, args.this()).is_none() {
        webidl::throw_type_error(
            scope,
            "Window.indexedDB getter called on incompatible receiver.",
        );
        return;
    }
    match ensure_indexed_db_runtime_state(scope) {
        Some(factory) => rv.set(factory.into()),
        None => rv.set(v8::undefined(scope).into()),
    }
}
