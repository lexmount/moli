use super::*;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct PromiseRejectionEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    cancelable: bool,
    #[webapi(data_property, enumerable)]
    promise: v8::Local<'scope, v8::Promise>,
    #[webapi(data_property, enumerable)]
    reason: v8::Local<'scope, v8::Value>,
}

pub(crate) fn dispatch_window_promise_rejection_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    dispatch_scope: crate::native_bridge::OwnerDispatchScope,
    event_type: &str,
    promise: v8::Local<'s, v8::Promise>,
    reason: Option<v8::Local<'s, v8::Value>>,
) -> std::result::Result<bool, String> {
    let global = scope.get_current_context().global(scope);
    let Some(event_ctor_value) = global.get(scope, v8str(scope, "PromiseRejectionEvent").into())
    else {
        return Ok(true);
    };
    let Ok(event_ctor) = v8::Local::<v8::Function>::try_from(event_ctor_value) else {
        return Ok(true);
    };

    let init = PromiseRejectionEventInitDeclaration::new(
        event_type == "unhandledrejection",
        promise,
        reason.unwrap_or_else(|| v8::null(scope).into()),
    )
    .bind(scope)
    .expect("PromiseRejectionEvent init declaration should bind");

    let Some(type_value) = v8_string(scope, event_type) else {
        return Ok(true);
    };
    let Some(event) = event_ctor.new_instance(scope, &[type_value.into(), init.into()]) else {
        return Ok(true);
    };

    let runtime = unsafe { &mut *host_ptr };
    match dispatch_scope {
        crate::native_bridge::OwnerDispatchScope::Top => runtime
            .dispatch_public_event(scope, host_ptr, EventTargetHandle::Window, event)
            .map(|dispatch| dispatch.allows_default()),
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            let target = runtime
                .current_child_window_event_target(handle)
                .ok_or_else(|| "promise rejection child Window is no longer current".to_owned())?;
            runtime
                .dispatch_public_event(
                    scope,
                    host_ptr,
                    EventTargetHandle::ChildWindow(target),
                    event,
                )
                .map(|dispatch| dispatch.allows_default())
        }
    }
}
