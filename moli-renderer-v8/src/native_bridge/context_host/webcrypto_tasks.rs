use super::*;

pub(crate) struct PendingWebCryptoTask {
    pub(crate) execution_context: super::WindowExecutionContextIdentity,
    pub(crate) relevant_context: super::WindowExecutionContextBinding,
    pub(crate) resolver: v8::Global<v8::PromiseResolver>,
}

impl JsContextHost {
    pub(crate) fn register_pending_webcrypto_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        resolver: v8::Local<'_, v8::PromiseResolver>,
    ) -> Option<crate::page_task_queue::RendererPageWebCryptoTaskProducer> {
        let task_id = self.next_webcrypto_task_id;
        self.next_webcrypto_task_id = self
            .next_webcrypto_task_id
            .checked_next()
            .expect("Page WebCrypto task id overflow");
        let execution_context = self.current_runtime_window_execution_context_identity(scope)?;
        let relevant_context = super::WindowExecutionContextBinding::new(
            execution_context.owner(),
            execution_context.dispatch_scope(),
            execution_context.realm_token(),
            v8::Global::new(scope, scope.get_current_context()),
        );
        let producer = self
            .page_webcrypto_task_sender()
            .bind_task(execution_context, task_id);
        let replaced = self.pending_webcrypto_tasks.insert(
            task_id,
            PendingWebCryptoTask {
                execution_context,
                relevant_context,
                resolver: v8::Global::new(scope, resolver),
            },
        );
        assert!(
            replaced.is_none(),
            "Page WebCrypto task ids must never be reused"
        );
        tracing::debug!(
            task_id = task_id.task_id(),
            ?execution_context,
            "registered WebCrypto task with Window execution context"
        );
        Some(producer)
    }

    pub(crate) fn current_pending_webcrypto_task_execution_context(
        &self,
        task: crate::page_task_queue::RendererPageWebCryptoTaskId,
    ) -> Option<super::WindowExecutionContextIdentity> {
        let pending = self.pending_webcrypto_tasks.get(&task)?;
        if !self.window_execution_context_identity_is_current(pending.execution_context) {
            return None;
        }
        Some(pending.execution_context)
    }

    pub(crate) fn take_pending_webcrypto_task_for_exact_owner(
        &mut self,
        execution_context: super::WindowExecutionContextIdentity,
        task: crate::page_task_queue::RendererPageWebCryptoTaskId,
    ) -> Option<PendingWebCryptoTask> {
        let pending = self.pending_webcrypto_tasks.get(&task)?;
        if pending.execution_context != execution_context {
            return None;
        }
        self.pending_webcrypto_tasks.remove(&task)
    }

    pub(crate) fn retire_webcrypto_execution_context_owner(
        &mut self,
        retired_owner: super::WindowExecutionContextOwner,
    ) -> usize {
        let count_before = self.pending_webcrypto_tasks.len();
        self.pending_webcrypto_tasks
            .retain(|_, pending| pending.relevant_context.owner() != retired_owner);
        let retired_count = count_before - self.pending_webcrypto_tasks.len();
        tracing::debug!(
            ?retired_owner,
            retired_count,
            "retired WebCrypto tasks with Window execution context"
        );
        retired_count
    }

    pub(crate) fn retire_webcrypto_context_token(
        &mut self,
        context_token: super::RuntimeObservableContextToken,
    ) -> usize {
        let count_before = self.pending_webcrypto_tasks.len();
        self.pending_webcrypto_tasks
            .retain(|_, pending| pending.relevant_context.realm_token() != context_token);
        let retired_count = count_before - self.pending_webcrypto_tasks.len();
        if retired_count > 0 {
            tracing::debug!(
                ?context_token,
                retired_count,
                "retired WebCrypto tasks with destroyed V8 context"
            );
        }
        retired_count
    }

    pub(crate) fn pending_webcrypto_task_count(&self) -> usize {
        self.pending_webcrypto_tasks.len()
    }

    #[cfg(test)]
    pub(crate) fn pending_webcrypto_execution_contexts_for_test(
        &self,
    ) -> Vec<(
        super::WindowExecutionContextOwner,
        super::RuntimeObservableContextToken,
    )> {
        self.pending_webcrypto_tasks
            .values()
            .map(|pending| {
                (
                    pending.relevant_context.owner(),
                    pending.relevant_context.realm_token(),
                )
            })
            .collect()
    }

    pub(crate) fn has_pending_webcrypto_tasks(&self) -> bool {
        !self.pending_webcrypto_tasks.is_empty()
    }
}
