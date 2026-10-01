//! Worker-owned WebCrypto and OPFS tasks and their promise completions.

use super::*;

/// A WebCrypto primitive dispatched off the worker event loop to the blocking
/// pool. The worker owns the resolver and matches the completion back by the
/// task id allocated for this worker lifetime.
pub(in crate::worker) struct PendingWorkerWebCryptoTask {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
}

/// Completion of a worker WebCrypto blocking task, routed back onto the worker
/// event loop. The worker sink captures these ids at registration just as the
/// page-side typed producer captures its exact Page/Window owner.
pub(crate) struct WorkerWebCryptoCompletion {
    pub(crate) task_id: u64,
    pub(crate) result: Result<WebCryptoTaskResult, WebCryptoRejection>,
}

pub(in crate::worker) struct PendingWorkerOpfsTask {
    pub(in crate::worker) locator: moli_storage_service::StorageBucketLocator,
    pub(in crate::worker) handle_access: Option<crate::opfs_owner_tasks::OpfsHandleAccessContext>,
    pub(in crate::worker) settlement: crate::opfs_owner_tasks::OpfsTaskSettlement,
}

pub(in crate::worker) struct WorkerOpfsOwnerState {
    next_task_id: u64,
    pending_tasks: HashMap<u64, PendingWorkerOpfsTask>,
    handles: crate::opfs_owner_tasks::OpfsHandleRegistry,
    directory_iterators: crate::opfs_owner_tasks::OpfsDirectoryIteratorRegistry,
}

impl Default for WorkerOpfsOwnerState {
    fn default() -> Self {
        Self {
            next_task_id: 1,
            pending_tasks: HashMap::new(),
            handles: crate::opfs_owner_tasks::OpfsHandleRegistry::default(),
            directory_iterators: crate::opfs_owner_tasks::OpfsDirectoryIteratorRegistry::default(),
        }
    }
}

impl WorkerOpfsOwnerState {
    pub(in crate::worker) fn has_pending_tasks(&self) -> bool {
        !self.pending_tasks.is_empty()
    }
}

pub(crate) struct WorkerOpfsCompletion {
    pub(crate) task_id: u64,
    pub(crate) result: OpfsTaskResult,
}

impl WorkerGlobalState {
    /// Register a pending worker WebCrypto task and return the routing tuple the
    /// blocking task needs to report completion. Runs synchronously inside the
    /// V8 callback before control returns to the worker event loop.
    pub(in crate::worker) fn register_pending_webcrypto_task(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> (u64, mpsc::UnboundedSender<WorkerWebCryptoCompletion>) {
        let task_id = self.next_webcrypto_task_id;
        self.next_webcrypto_task_id = task_id
            .checked_add(1)
            .expect("worker WebCrypto task id exhausted");
        self.pending_webcrypto
            .insert(task_id, PendingWorkerWebCryptoTask { resolver });
        (task_id, self.webcrypto_completion_tx.clone())
    }

    pub(in crate::worker) fn take_pending_webcrypto_task(
        &mut self,
        task_id: u64,
    ) -> Option<PendingWorkerWebCryptoTask> {
        self.pending_webcrypto.remove(&task_id)
    }

    pub(in crate::worker) fn register_pending_opfs_task(
        &mut self,
        locator: moli_storage_service::StorageBucketLocator,
        handle_access: Option<crate::opfs_owner_tasks::OpfsHandleAccessContext>,
        settlement: crate::opfs_owner_tasks::OpfsTaskSettlement,
    ) -> (u64, mpsc::UnboundedSender<WorkerOpfsCompletion>) {
        let state = self
            .opfs_owner_state
            .get_or_insert_with(WorkerOpfsOwnerState::default);
        let task_id = state.next_task_id;
        state.next_task_id = task_id
            .checked_add(1)
            .expect("worker OPFS task id exhausted");
        state.pending_tasks.insert(
            task_id,
            PendingWorkerOpfsTask {
                locator,
                handle_access,
                settlement,
            },
        );
        (task_id, self.opfs_completion_tx.clone())
    }

    pub(in crate::worker) fn take_pending_opfs_task(
        &mut self,
        task_id: u64,
    ) -> Option<PendingWorkerOpfsTask> {
        self.opfs_owner_state
            .as_mut()?
            .pending_tasks
            .remove(&task_id)
    }
}

/// Register a pending worker WebCrypto task from a callback scope.
///
/// Returns the routing tuple the blocking task needs to report completion, or
/// `None` when the current scope is not a worker global (e.g. the page runtime
/// or a bare unit-test context). This is the worker-lane analog of the page
/// `register_webcrypto_task`.
pub(crate) fn register_worker_webcrypto_task(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
) -> Option<(u64, mpsc::UnboundedSender<WorkerWebCryptoCompletion>)> {
    let state = get_worker_state(scope)?;
    let resolver = v8::Global::new(scope, resolver);
    let mut state = state.borrow_mut();
    Some(state.register_pending_webcrypto_task(resolver))
}

pub(crate) fn register_worker_opfs_task(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    locator: moli_storage_service::StorageBucketLocator,
    handle_access: Option<crate::opfs_owner_tasks::OpfsHandleAccessContext>,
) -> Option<(u64, mpsc::UnboundedSender<WorkerOpfsCompletion>)> {
    let state = get_worker_state(scope)?;
    let resolver = v8::Global::new(scope, resolver);
    let mut state = state.borrow_mut();
    Some(state.register_pending_opfs_task(
        locator,
        handle_access,
        crate::opfs_owner_tasks::OpfsTaskSettlement::Promise(resolver),
    ))
}

pub(crate) fn register_worker_opfs_iterator_task(
    scope: &mut v8::PinScope<'_, '_>,
    locator: moli_storage_service::StorageBucketLocator,
    registry: crate::opfs_owner_tasks::OpfsDirectoryIteratorRegistry,
    iterator_id: u32,
    keep_alive: v8::Global<v8::Object>,
    handle_access: Option<crate::opfs_owner_tasks::OpfsHandleAccessContext>,
) -> Option<(u64, mpsc::UnboundedSender<WorkerOpfsCompletion>)> {
    let state = get_worker_state(scope)?;
    let mut state = state.borrow_mut();
    Some(state.register_pending_opfs_task(
        locator,
        handle_access,
        crate::opfs_owner_tasks::OpfsTaskSettlement::DirectoryIterator {
            registry,
            iterator_id,
            keep_alive,
        },
    ))
}

pub(crate) fn register_worker_opfs_move_task(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    handle: v8::Local<'_, v8::Object>,
    mutation: crate::opfs_owner_tasks::OpfsHandleMutationGuard,
    locator: moli_storage_service::StorageBucketLocator,
    handle_access: Option<crate::opfs_owner_tasks::OpfsHandleAccessContext>,
) -> Option<(u64, mpsc::UnboundedSender<WorkerOpfsCompletion>)> {
    let state = get_worker_state(scope)?;
    let mut state = state.borrow_mut();
    Some(state.register_pending_opfs_task(
        locator,
        handle_access,
        crate::opfs_owner_tasks::OpfsTaskSettlement::Move {
            resolver: v8::Global::new(scope, resolver),
            handle: v8::Global::new(scope, handle),
            mutation,
        },
    ))
}

pub(crate) fn worker_opfs_handle_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::opfs_owner_tasks::OpfsHandleRegistry> {
    Some(
        get_worker_state(scope)?
            .borrow()
            .opfs_owner_state
            .as_ref()?
            .handles
            .clone(),
    )
}

pub(crate) fn ensure_worker_opfs_handle_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::opfs_owner_tasks::OpfsHandleRegistry> {
    Some(
        get_worker_state(scope)?
            .borrow_mut()
            .opfs_owner_state
            .get_or_insert_with(WorkerOpfsOwnerState::default)
            .handles
            .clone(),
    )
}

pub(crate) fn worker_opfs_directory_iterator_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::opfs_owner_tasks::OpfsDirectoryIteratorRegistry> {
    Some(
        get_worker_state(scope)?
            .borrow()
            .opfs_owner_state
            .as_ref()?
            .directory_iterators
            .clone(),
    )
}

pub(crate) fn ensure_worker_opfs_directory_iterator_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::opfs_owner_tasks::OpfsDirectoryIteratorRegistry> {
    Some(
        get_worker_state(scope)?
            .borrow_mut()
            .opfs_owner_state
            .get_or_insert_with(WorkerOpfsOwnerState::default)
            .directory_iterators
            .clone(),
    )
}

pub(crate) fn cancel_worker_opfs_task(scope: &mut v8::PinScope<'_, '_>, task_id: u64) {
    if let Some(state) = get_worker_state(scope) {
        state.borrow_mut().take_pending_opfs_task(task_id);
    }
}

/// Settle a worker WebCrypto promise once its blocking task reports back on the
/// worker event loop. Terminating a worker drops this state and its completion
/// receiver together, so a separate reset generation is unnecessary.
pub(in crate::worker) fn drain_worker_webcrypto_completion(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    completion: WorkerWebCryptoCompletion,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_webcrypto_task(completion.task_id) else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    crate::script_vm::webcrypto_tasks::settle_webcrypto_task_result(
        scope,
        resolver,
        completion.result,
    );
}

pub(in crate::worker) fn drain_worker_opfs_completion(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    completion: WorkerOpfsCompletion,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_opfs_task(completion.task_id) else {
            return;
        };
        pending
    };
    let handle_access = pending.handle_access;
    match pending.settlement {
        crate::opfs_owner_tasks::OpfsTaskSettlement::Promise(resolver) => {
            let resolver = v8::Local::new(scope, &resolver);
            crate::context_bootstrap::settle_opfs_task_result(
                scope,
                resolver,
                &pending.locator,
                handle_access.as_ref(),
                completion.result,
            );
        }
        crate::opfs_owner_tasks::OpfsTaskSettlement::Move {
            resolver,
            handle,
            mutation,
        } => {
            let resolver = v8::Local::new(scope, &resolver);
            let handle = v8::Local::new(scope, &handle);
            crate::context_bootstrap::settle_opfs_move_task_result(
                scope,
                resolver,
                handle,
                &pending.locator,
                handle_access.as_ref(),
                completion.result,
            );
            drop(mutation);
        }
        crate::opfs_owner_tasks::OpfsTaskSettlement::DirectoryIterator {
            registry,
            iterator_id,
            keep_alive,
        } => {
            crate::context_bootstrap::settle_opfs_directory_iterator_task_result(
                scope,
                &registry,
                iterator_id,
                &pending.locator,
                handle_access.as_ref(),
                completion.result,
            );
            drop(keep_alive);
        }
    }
}
