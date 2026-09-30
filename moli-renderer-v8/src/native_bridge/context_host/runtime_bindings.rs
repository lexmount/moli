use super::{JsContextHost, RuntimeBindingExecutionContext, RuntimeObservableContextToken};
use crate::{
    protocol_types::{PendingRuntimeBindingCall, RuntimeBindingRegistration},
    runtime_binding_data::{build_runtime_binding_data, runtime_binding_callback},
    {document_runtime::DomHandle, util::v8_string},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RuntimeBindingOwnerTransitionOutcome {
    retired_execution_context_count: usize,
    rebound_execution_context_count: usize,
}

impl RuntimeBindingOwnerTransitionOutcome {
    pub(crate) fn retired_execution_context_count(self) -> usize {
        self.retired_execution_context_count
    }

    pub(crate) fn rebound_execution_context_count(self) -> usize {
        self.rebound_execution_context_count
    }
}

impl JsContextHost {
    pub(crate) fn document_task_owner_is_current(
        &self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> bool {
        self.frame_owner_store.document_task_owner_is_current(owner)
    }
    pub(crate) fn set_stored_runtime_bindings(&mut self, bindings: &[RuntimeBindingRegistration]) {
        self.stored_runtime_bindings = bindings.to_vec();
    }

    pub(crate) fn stored_runtime_bindings(&self) -> Vec<RuntimeBindingRegistration> {
        self.stored_runtime_bindings.clone()
    }

    pub(crate) fn install_default_runtime_bindings_for_child_window(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        window: v8::Local<'_, v8::Object>,
    ) {
        let binding_names = self.stored_default_native_runtime_binding_names();
        if binding_names.is_empty() {
            return;
        }
        let execution_context_id = self.child_default_execution_context_id(handle).unwrap_or(0);
        let Some(document_owner) = self.current_child_document_task_owner(handle) else {
            return;
        };
        let Some(context_token) = super::current_runtime_observable_context_token(scope) else {
            return;
        };
        let execution_context =
            RuntimeBindingExecutionContext::new(document_owner.local_window_id, context_token);
        if !self.register_runtime_binding_execution_context(execution_context, document_owner) {
            return;
        }
        for binding_name in binding_names {
            let Some(key) = v8_string(scope, &binding_name) else {
                continue;
            };
            let Some(value) = Self::build_runtime_binding_function(
                scope,
                self as *mut JsContextHost,
                &binding_name,
                execution_context_id,
                execution_context,
            ) else {
                continue;
            };
            let _ = window.set(scope, key.into(), value.into());
        }
    }

    pub(crate) fn refresh_default_runtime_bindings_for_child_window(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
    ) {
        let Some(window) = self.child_window_proxy_records.live_window(scope, handle) else {
            return;
        };
        self.install_default_runtime_bindings_for_child_window(scope, handle, window);
    }

    fn build_runtime_binding_function<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        name: &str,
        execution_context_id: i64,
        execution_context: RuntimeBindingExecutionContext,
    ) -> Option<v8::Local<'s, v8::Function>> {
        let data = build_runtime_binding_data(
            scope,
            host_ptr.cast::<std::ffi::c_void>(),
            v8_string(scope, name)?,
            execution_context_id,
            execution_context,
        )
        .ok()?;
        v8::Function::builder(runtime_binding_callback)
            .data(data.into())
            .build(scope)
    }

    pub(crate) fn register_runtime_binding_execution_context(
        &mut self,
        execution_context: RuntimeBindingExecutionContext,
        document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> bool {
        if execution_context.local_window_id() != document_owner.local_window_id
            || !self.document_task_owner_is_current(document_owner)
        {
            tracing::debug!(
                ?execution_context,
                ?document_owner,
                "refused to register Runtime binding for stale execution context"
            );
            return false;
        }
        if self
            .runtime_binding_execution_context_owners
            .get(&execution_context)
            .is_some_and(|registered| *registered != document_owner)
        {
            tracing::warn!(
                ?execution_context,
                registered_owner = ?self.runtime_binding_execution_context_owners.get(&execution_context),
                ?document_owner,
                "refused to overwrite Runtime binding execution-context owner"
            );
            return false;
        }
        self.runtime_binding_execution_context_owners
            .insert(execution_context, document_owner);
        true
    }

    pub(crate) fn record_runtime_binding_call(
        &mut self,
        execution_context: RuntimeBindingExecutionContext,
        call: PendingRuntimeBindingCall,
    ) -> bool {
        if call.source != execution_context.binding_call_source_identity() {
            tracing::warn!(
                ?execution_context,
                call_source = ?call.source,
                "refused Runtime binding call with mismatched realm identity"
            );
            return false;
        }
        let Some(owner) = self
            .runtime_binding_execution_context_owners
            .get(&execution_context)
            .copied()
        else {
            return false;
        };
        if !self.document_task_owner_is_current(owner) {
            return false;
        }
        // The invocation is a historical observation once this call-time
        // authorization succeeds. Freeze it at this exact producer boundary:
        // a command-local call stays ahead of the command response, while an
        // ordinary Page turn appends to that Page's concrete output FIFO.
        if let Some(recorder) = &self.command_turn_output {
            recorder.push_runtime_binding_call(call);
            return true;
        }
        if let Some(output_journal) = &self.output_journal {
            output_journal.append(crate::runtime::PendingRendererOutputRecord::observation(
                None,
                crate::runtime::RendererProtocolObservation::RuntimeBinding(call),
            ));
            return true;
        }
        #[cfg(test)]
        self.pending_runtime_binding_calls.push(call);
        #[cfg(test)]
        {
            true
        }
        #[cfg(not(test))]
        {
            let _ = call;
            panic!("a production Runtime binding call must have a concrete renderer output sink");
        }
    }

    #[cfg(test)]
    pub(crate) fn take_runtime_binding_calls(&mut self) -> Vec<PendingRuntimeBindingCall> {
        std::mem::take(&mut self.pending_runtime_binding_calls)
    }

    pub(crate) fn rebind_runtime_binding_document_owner(
        &mut self,
        retired_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        current_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> RuntimeBindingOwnerTransitionOutcome {
        let mut outcome = RuntimeBindingOwnerTransitionOutcome::default();
        self.runtime_binding_execution_context_owners
            .retain(|execution_context, owner| {
                if *owner != retired_owner {
                    return true;
                }
                if execution_context.local_window_id() == current_owner.local_window_id {
                    *owner = current_owner;
                    outcome.rebound_execution_context_count += 1;
                    true
                } else {
                    outcome.retired_execution_context_count += 1;
                    false
                }
            });
        outcome
    }

    pub(crate) fn retire_runtime_bindings_for_document_owner(
        &mut self,
        retired_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> RuntimeBindingOwnerTransitionOutcome {
        let previous_count = self.runtime_binding_execution_context_owners.len();
        self.runtime_binding_execution_context_owners
            .retain(|_, owner| *owner != retired_owner);
        RuntimeBindingOwnerTransitionOutcome {
            retired_execution_context_count: previous_count
                - self.runtime_binding_execution_context_owners.len(),
            rebound_execution_context_count: 0,
        }
    }

    pub(crate) fn retire_runtime_binding_context_token(
        &mut self,
        context_token: RuntimeObservableContextToken,
    ) -> RuntimeBindingOwnerTransitionOutcome {
        let previous_context_count = self.runtime_binding_execution_context_owners.len();
        self.runtime_binding_execution_context_owners
            .retain(|execution_context, _| execution_context.context_token() != context_token);
        RuntimeBindingOwnerTransitionOutcome {
            retired_execution_context_count: previous_context_count
                - self.runtime_binding_execution_context_owners.len(),
            rebound_execution_context_count: 0,
        }
    }
}
