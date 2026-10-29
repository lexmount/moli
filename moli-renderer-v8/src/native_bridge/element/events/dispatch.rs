use crate::{
    document_runtime::{DomHandle, EventTargetHandle},
    host::PublicEventDispatchResult,
    util::v8str,
};

use super::super::JsContextHost;
use super::{TextEditInputType, construct_input_event};

/// The cancelable boundary before an editing default action. Callers must not
/// mutate the value or selection until this returns true, and must re-read
/// state that listeners may have changed.
pub(crate) fn dispatch_beforeinput(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    target: DomHandle,
    input_type: TextEditInputType,
    data: Option<&str>,
) -> bool {
    let Some(event) = construct_input_event(scope, "beforeinput", input_type, data) else {
        return false;
    };
    dispatch_public_event(scope, runtime_ptr, target, event).allows_default()
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NodePublicEventDispatchOutcome {
    pub default_prevented: bool,
    pub had_exception: bool,
}

impl NodePublicEventDispatchOutcome {
    pub(crate) fn allows_default(self) -> bool {
        !self.default_prevented
    }

    pub(crate) fn had_exception(self) -> bool {
        self.had_exception
    }
}

pub(crate) fn dispatch_public_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    target: DomHandle,
    event: v8::Local<'s, v8::Object>,
) -> NodePublicEventDispatchOutcome {
    match dispatch_public_event_result(scope, runtime_ptr, target, event) {
        Ok(result) => NodePublicEventDispatchOutcome {
            default_prevented: result.default_prevented,
            had_exception: false,
        },
        Err(_) => NodePublicEventDispatchOutcome {
            default_prevented: event_default_prevented(scope, event),
            had_exception: true,
        },
    }
}

fn dispatch_public_event_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    target: DomHandle,
    event: v8::Local<'s, v8::Object>,
) -> std::result::Result<PublicEventDispatchResult, String> {
    let runtime = unsafe { &mut *runtime_ptr };
    if crate::web_api_interfaces::UIEvent::is_instance(scope, event) {
        // Native UI events identify the target's Document Window. The producer
        // may be running in another realm, or the Document may be windowless.
        let view = runtime
            .owner_dispatch_scope_for_node(target)
            .and_then(|target| {
                let owner = runtime.current_window_execution_context_owner(target)?;
                runtime.window_execution_context(scope, owner, target)
            })
            .map(|(_, context)| context.global(scope).into())
            .unwrap_or_else(|| v8::null(scope).into());
        let backing = crate::context_bootstrap::event_backing(scope, event);
        if backing.create_data_property(scope, v8str(scope, "view").into(), view) != Some(true) {
            return Err("failed to initialize native UIEvent view".to_owned());
        }
    }
    runtime.dispatch_public_event_best_effort(
        scope,
        runtime_ptr,
        EventTargetHandle::Node(target),
        event,
        "node public event",
    )
}

fn event_default_prevented(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
) -> bool {
    crate::context_bootstrap::event_backing(scope, event)
        .get(scope, v8str(scope, "defaultPrevented").into())
        .is_some_and(|value| value.boolean_value(scope))
}
