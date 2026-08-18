use super::*;

/// One exact Window identity paired with the V8 context that realizes it.
///
/// OPFS pending work needs both the copyable identity used by Page
/// authorization and the persistent V8 binding used for settlement. Keeping
/// them behind this constructor prevents those two facts from diverging.
pub(crate) struct PendingOpfsExecutionContext {
    identity: super::WindowExecutionContextIdentity,
    binding: super::WindowExecutionContextBinding,
}

impl PendingOpfsExecutionContext {
    fn new(
        identity: super::WindowExecutionContextIdentity,
        binding: super::WindowExecutionContextBinding,
    ) -> Option<Self> {
        (binding.owner() == identity.owner()
            && binding.dispatch_scope() == identity.dispatch_scope()
            && binding.realm_token() == identity.realm_token())
        .then_some(Self { identity, binding })
    }

    pub(crate) const fn identity(&self) -> super::WindowExecutionContextIdentity {
        self.identity
    }

    pub(crate) fn into_binding(self) -> super::WindowExecutionContextBinding {
        self.binding
    }
}

pub(crate) struct PendingOpfsTask {
    pub(crate) execution_context: PendingOpfsExecutionContext,
    pub(crate) locator: moli_storage_service::StorageBucketLocator,
    pub(crate) handle_access: Option<crate::opfs_owner_tasks::OpfsHandleAccessContext>,
    pub(crate) settlement: crate::opfs_owner_tasks::OpfsTaskSettlement,
}

pub(super) struct WindowOpfsOwnerState {
    next_task_id: crate::page_task_queue::RendererPageOpfsTaskId,
    pending_tasks: HashMap<crate::page_task_queue::RendererPageOpfsTaskId, PendingOpfsTask>,
    handles: crate::opfs_owner_tasks::OpfsHandleRegistry,
    directory_iterators: crate::opfs_owner_tasks::OpfsDirectoryIteratorRegistry,
}

impl Default for WindowOpfsOwnerState {
    fn default() -> Self {
        Self {
            next_task_id: crate::page_task_queue::RendererPageOpfsTaskId::first(),
            pending_tasks: HashMap::new(),
            handles: crate::opfs_owner_tasks::OpfsHandleRegistry::default(),
            directory_iterators: crate::opfs_owner_tasks::OpfsDirectoryIteratorRegistry::default(),
        }
    }
}

impl JsContextHost {
    fn ensure_opfs_owner_state(&mut self) -> &mut WindowOpfsOwnerState {
        self.opfs_owner_state
            .get_or_insert_with(WindowOpfsOwnerState::default)
    }

    pub(crate) fn register_pending_opfs_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        resolver: v8::Local<'_, v8::PromiseResolver>,
        locator: moli_storage_service::StorageBucketLocator,
        handle_access: Option<crate::opfs_owner_tasks::OpfsHandleAccessContext>,
    ) -> Option<(
        crate::page_task_queue::RendererPageOpfsTaskId,
        crate::page_task_queue::RendererPageOpfsTaskProducer,
    )> {
        let settlement =
            crate::opfs_owner_tasks::OpfsTaskSettlement::Promise(v8::Global::new(scope, resolver));
        self.register_pending_opfs_settlement_task(scope, locator, settlement, handle_access)
    }

    pub(crate) fn register_pending_opfs_iterator_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        locator: moli_storage_service::StorageBucketLocator,
        registry: crate::opfs_owner_tasks::OpfsDirectoryIteratorRegistry,
        iterator_id: u32,
        keep_alive: v8::Global<v8::Object>,
        handle_access: Option<crate::opfs_owner_tasks::OpfsHandleAccessContext>,
    ) -> Option<(
        crate::page_task_queue::RendererPageOpfsTaskId,
        crate::page_task_queue::RendererPageOpfsTaskProducer,
    )> {
        self.register_pending_opfs_settlement_task(
            scope,
            locator,
            crate::opfs_owner_tasks::OpfsTaskSettlement::DirectoryIterator {
                registry,
                iterator_id,
                keep_alive,
            },
            handle_access,
        )
    }

    pub(crate) fn register_pending_opfs_move_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        resolver: v8::Local<'_, v8::PromiseResolver>,
        handle: v8::Local<'_, v8::Object>,
        mutation: crate::opfs_owner_tasks::OpfsHandleMutationGuard,
        locator: moli_storage_service::StorageBucketLocator,
        handle_access: Option<crate::opfs_owner_tasks::OpfsHandleAccessContext>,
    ) -> Option<(
        crate::page_task_queue::RendererPageOpfsTaskId,
        crate::page_task_queue::RendererPageOpfsTaskProducer,
    )> {
        self.register_pending_opfs_settlement_task(
            scope,
            locator,
            crate::opfs_owner_tasks::OpfsTaskSettlement::Move {
                resolver: v8::Global::new(scope, resolver),
                handle: v8::Global::new(scope, handle),
                mutation,
            },
            handle_access,
        )
    }

    fn register_pending_opfs_settlement_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        locator: moli_storage_service::StorageBucketLocator,
        settlement: crate::opfs_owner_tasks::OpfsTaskSettlement,
        handle_access: Option<crate::opfs_owner_tasks::OpfsHandleAccessContext>,
    ) -> Option<(
        crate::page_task_queue::RendererPageOpfsTaskId,
        crate::page_task_queue::RendererPageOpfsTaskProducer,
    )> {
        let current_execution_context =
            self.current_runtime_window_execution_context_identity(scope);
        let execution_context = match handle_access.as_ref() {
            Some(access) => {
                let identity = access.window_identity()?;
                if !self.window_execution_context_identity_is_current(identity) {
                    return None;
                }
                identity
            }
            None => current_execution_context?,
        };
        let relevant_context = if current_execution_context == Some(execution_context) {
            super::WindowExecutionContextBinding::new(
                execution_context.owner(),
                execution_context.dispatch_scope(),
                execution_context.realm_token(),
                v8::Global::new(scope, scope.get_current_context()),
            )
        } else {
            let binding = self.clone_window_execution_context_binding(
                scope,
                execution_context.owner(),
                execution_context.dispatch_scope(),
            )?;
            (binding.realm_token() == execution_context.realm_token()).then_some(binding)?
        };
        let execution_context =
            PendingOpfsExecutionContext::new(execution_context, relevant_context)?;
        let execution_context_identity = execution_context.identity();
        let owner = execution_context_identity.owner();
        let realm_token = execution_context_identity.realm_token();
        let state = self.ensure_opfs_owner_state();
        let task_id = state.next_task_id;
        state.next_task_id = state
            .next_task_id
            .checked_next()
            .expect("Page OPFS task id overflow");
        let replaced = state.pending_tasks.insert(
            task_id,
            PendingOpfsTask {
                execution_context,
                locator,
                handle_access,
                settlement,
            },
        );
        assert!(
            replaced.is_none(),
            "Page OPFS task ids must never be reused"
        );
        tracing::debug!(
            task_id = task_id.task_id(),
            ?owner,
            ?realm_token,
            "registered OPFS task with Window execution context"
        );
        let producer = self
            .page_opfs_task_sender()
            .bind_task(execution_context_identity, task_id);
        Some((task_id, producer))
    }

    pub(crate) fn current_pending_opfs_task_execution_context(
        &self,
        task: crate::page_task_queue::RendererPageOpfsTaskId,
    ) -> Option<super::WindowExecutionContextIdentity> {
        let pending = self.opfs_owner_state.as_ref()?.pending_tasks.get(&task)?;
        if !self.window_execution_context_identity_is_current(pending.execution_context.identity())
        {
            return None;
        }
        Some(pending.execution_context.identity())
    }

    pub(crate) fn take_pending_opfs_task_for_exact_owner(
        &mut self,
        execution_context: super::WindowExecutionContextIdentity,
        task: crate::page_task_queue::RendererPageOpfsTaskId,
    ) -> Option<PendingOpfsTask> {
        let state = self.opfs_owner_state.as_mut()?;
        let pending = state.pending_tasks.get(&task)?;
        if pending.execution_context.identity() != execution_context {
            return None;
        }
        state.pending_tasks.remove(&task)
    }

    pub(crate) fn cancel_pending_opfs_task(
        &mut self,
        task_id: crate::page_task_queue::RendererPageOpfsTaskId,
    ) -> bool {
        self.opfs_owner_state
            .as_mut()
            .is_some_and(|state| state.pending_tasks.remove(&task_id).is_some())
    }

    pub(crate) fn retire_opfs_execution_context_owner(
        &mut self,
        retired_owner: super::WindowExecutionContextOwner,
    ) -> usize {
        let Some(state) = self.opfs_owner_state.as_mut() else {
            return 0;
        };
        let count_before = state.pending_tasks.len();
        state
            .pending_tasks
            .retain(|_, pending| pending.execution_context.identity().owner() != retired_owner);
        count_before - state.pending_tasks.len()
    }

    pub(crate) fn retire_opfs_context_token(
        &mut self,
        context_token: super::RuntimeObservableContextToken,
    ) -> usize {
        let Some(state) = self.opfs_owner_state.as_mut() else {
            return 0;
        };
        let count_before = state.pending_tasks.len();
        state.pending_tasks.retain(|_, pending| {
            pending.execution_context.identity().realm_token() != context_token
        });
        count_before - state.pending_tasks.len()
    }

    pub(crate) fn pending_opfs_task_count(&self) -> usize {
        self.opfs_owner_state
            .as_ref()
            .map_or(0, |state| state.pending_tasks.len())
    }

    pub(crate) fn has_pending_opfs_tasks(&self) -> bool {
        self.opfs_owner_state
            .as_ref()
            .is_some_and(|state| !state.pending_tasks.is_empty())
    }

    pub(crate) fn opfs_directory_iterator_registry(
        &self,
    ) -> Option<crate::opfs_owner_tasks::OpfsDirectoryIteratorRegistry> {
        Some(self.opfs_owner_state.as_ref()?.directory_iterators.clone())
    }

    pub(crate) fn ensure_opfs_directory_iterator_registry(
        &mut self,
    ) -> crate::opfs_owner_tasks::OpfsDirectoryIteratorRegistry {
        self.ensure_opfs_owner_state().directory_iterators.clone()
    }

    pub(crate) fn opfs_handle_registry(
        &self,
    ) -> Option<crate::opfs_owner_tasks::OpfsHandleRegistry> {
        Some(self.opfs_owner_state.as_ref()?.handles.clone())
    }

    pub(crate) fn ensure_opfs_handle_registry(
        &mut self,
    ) -> crate::opfs_owner_tasks::OpfsHandleRegistry {
        self.ensure_opfs_owner_state().handles.clone()
    }

    #[cfg(test)]
    pub(crate) fn has_opfs_owner_state(&self) -> bool {
        self.opfs_owner_state.is_some()
    }
}
