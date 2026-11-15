use super::*;

const WORKER_SERVICE_WORKER_CONTAINER_SLOT: &str = "__moliWorkerServiceWorkerContainer";

pub(crate) fn retain_worker_service_worker_container<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    container: v8::Local<'s, v8::Object>,
) {
    if get_worker_state(scope).is_some() {
        let global = scope.get_current_context().global(scope);
        set_private_value(
            scope,
            global,
            WORKER_SERVICE_WORKER_CONTAINER_SLOT,
            container.into(),
        );
    }
}

pub(crate) fn worker_service_worker_container<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_worker_state(scope)?;
    let global = scope.get_current_context().global(scope);
    // The actual container survives author shadowing of navigator properties.
    get_private_value(scope, global, WORKER_SERVICE_WORKER_CONTAINER_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(crate) fn dispatch_service_worker_message_from_worker_client(
    scope: &mut v8::PinScope<'_, '_>,
    version: ServiceWorkerVersionId,
    payload: crate::structured_clone::V8StructuredClonePayload,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };
    let (runtime, client) = {
        let state = state.borrow();
        let Some(runtime) = state.service_worker_runtime.clone() else {
            return false;
        };
        let Some(client) = state.service_worker_client_id else {
            return false;
        };
        (runtime, client)
    };
    runtime.dispatch_message_to_version(
        version,
        client,
        worker_global_origin(scope).map(|origin| origin.ascii_serialization().to_owned()),
        payload,
    )
}
