use super::reactions::enter_custom_element_reaction;

use super::super::native_bridge::JsContextHost;
use crate::script_vm::perform_microtask_checkpoint_and_report_pending_promise_rejections;

pub(super) enum CustomElementConstructorInvocation {
    Created(v8::Global<v8::Object>),
    Exception(v8::Global<v8::Value>),
    Empty,
}

pub(super) fn invoke_custom_element_constructor(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    constructor: v8::Local<'_, v8::Function>,
) -> CustomElementConstructorInvocation {
    let invocation = {
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
                    CustomElementConstructorInvocation::Exception(v8::Global::new(
                        &scope, exception,
                    ))
                })
                .unwrap_or(CustomElementConstructorInvocation::Empty),
            None => CustomElementConstructorInvocation::Empty,
        }
    };

    // Cleaning up a constructor call checkpoints only when its caller left no
    // JavaScript on the stack. Parser construction must do this before result
    // validation, while createElement() and document.write() called from script
    // must leave the queued microtasks for that outer script's completion.
    if can_perform_custom_element_microtask_checkpoint(scope) {
        perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    }
    invocation
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
