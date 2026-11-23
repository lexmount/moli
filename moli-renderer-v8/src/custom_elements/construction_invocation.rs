use super::reactions::enter_custom_element_reaction;

use super::super::native_bridge::PageScriptHost;
use crate::script_vm::perform_microtask_checkpoint_and_report_pending_promise_rejections;

pub(super) enum CustomElementConstructorInvocation {
    Created(v8::Global<v8::Object>),
    Exception(v8::Global<v8::Value>),
    Empty,
}

pub(super) fn invoke_custom_element_constructor(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    constructor: v8::Local<'_, v8::Function>,
) -> CustomElementConstructorInvocation {
    let invocation = call_custom_element_constructor(scope, host_ptr, constructor);
    // Direct creation must validate the element after constructor microtasks.
    checkpoint_after_custom_element_constructor(scope);
    invocation
}

pub(super) fn call_custom_element_constructor(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    constructor: v8::Local<'_, v8::Function>,
) -> CustomElementConstructorInvocation {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let created = {
        let _reaction = enter_custom_element_reaction(host_ptr);
        crate::script_execution::construct(&mut scope, constructor, &[])
    };
    match created {
        Some(object) => {
            CustomElementConstructorInvocation::Created(v8::Global::new(&scope, object))
        }
        None if scope.has_caught() => scope
            .exception()
            .map(|exception| {
                CustomElementConstructorInvocation::Exception(v8::Global::new(&scope, exception))
            })
            .unwrap_or(CustomElementConstructorInvocation::Empty),
        None => CustomElementConstructorInvocation::Empty,
    }
}

pub(super) fn checkpoint_after_custom_element_constructor(scope: &mut v8::PinScope<'_, '_>) {
    if can_perform_custom_element_microtask_checkpoint(scope) {
        perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    }
}

pub(super) fn can_perform_custom_element_microtask_checkpoint(
    scope: &v8::PinScope<'_, '_>,
) -> bool {
    !scope.is_execution_terminating()
        && scope
            .get_current_context()
            .get_microtask_queue()
            .is_some_and(|queue| {
                queue.get_microtasks_scope_depth() == 0 && !queue.is_running_microtasks()
            })
}
