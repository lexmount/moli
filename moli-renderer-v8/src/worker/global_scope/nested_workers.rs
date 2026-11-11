//! Nested worker ownership, creation snapshots and parent event delivery.

use crate::content_security_policy::ContentSecurityPolicy;

use super::*;

pub(in crate::worker) struct NestedWorker {
    wrapper: v8::Global<v8::Object>,
    handle: Option<crate::worker::WorkerHandle>,
    script_load_cancel: Option<moli_fetch::FetchCancelHandle>,
    script_load: Option<crate::context_bootstrap::NestedWorkerScriptLoad>,
    pending_tasks: std::collections::VecDeque<crate::worker::WorkerMessage>,
}

impl Drop for NestedWorker {
    fn drop(&mut self) {
        if let Some(cancel) = self.script_load_cancel.take() {
            cancel.cancel();
        }
    }
}

#[derive(Clone)]
pub(crate) struct NestedWorkerContext {
    pub(crate) worker_id: DedicatedWorkerId,
    pub(crate) base_url: Url,
    pub(crate) loader: crate::network::context::WorkerResourceLoader,
    pub(crate) worker_context_runtime: crate::runtime::RendererWorkerContextRuntime,
    pub(crate) service_worker_runtime:
        Option<crate::service_worker_runtime::ServiceWorkerRuntimeService>,
    pub(crate) service_worker_client_id:
        Option<crate::service_worker_runtime::ServiceWorkerClientId>,
    pub(crate) storage_key_top_level_site: String,
    pub(crate) creator_storage_key: MoliStorageKey,
    pub(crate) creator_message_security: crate::context_bootstrap::RuntimeMessageSourceSecurity,
    pub(crate) indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    pub(crate) storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
    pub(crate) module_static_import_content_security_policies: Vec<ContentSecurityPolicy>,
    pub(crate) content_security_policy_snapshot:
        crate::content_security_policy::InheritedContentSecurityPolicy,
    pub(crate) referrer_policy: Option<String>,
    pub(crate) require_trusted_types_for_script: bool,
    pub(crate) network_policy: crate::worker::handle::WorkerNetworkPolicy,
    pub(crate) policy_context: crate::types::SubresourcePolicyContext,
    pub(crate) shared_memory_agent_cluster: crate::structured_clone::SharedMemoryAgentCluster,
    pub(crate) wake_tx: mpsc::UnboundedSender<crate::worker::handle::WorkerMessage>,
}

pub(crate) fn reserve_nested_worker_context(
    scope: &mut v8::PinScope<'_, '_>,
    worker: v8::Local<'_, v8::Object>,
) -> Option<NestedWorkerContext> {
    let state = get_worker_state(scope)?;
    let mut state = state.borrow_mut();
    let base_url = state.current_script_url.clone()?;
    let worker_id = DedicatedWorkerId::new(state.next_nested_worker_id);
    state.next_nested_worker_id = state
        .next_nested_worker_id
        .checked_add(1)
        .expect("nested worker id space exhausted");
    state.nested_workers.insert(
        worker_id,
        NestedWorker {
            wrapper: v8::Global::new(scope, worker),
            handle: None,
            script_load_cancel: None,
            script_load: None,
            pending_tasks: Default::default(),
        },
    );
    Some(NestedWorkerContext {
        worker_id,
        base_url,
        loader: state.loader.clone(),
        worker_context_runtime: state.worker_context_runtime.clone(),
        service_worker_runtime: state.service_worker_runtime.clone(),
        service_worker_client_id: state.service_worker_client_id,
        storage_key_top_level_site: state.storage_key.top_level_site().to_owned(),
        creator_storage_key: state.storage_key.clone(),
        creator_message_security: state.message_source_security.clone(),
        indexed_db_manager: state.indexed_db_manager.clone(),
        storage_bucket_store: state.storage_bucket_store.clone(),
        module_static_import_content_security_policies: state.content_security_policies.clone(),
        content_security_policy_snapshot: content_security_policy::worker_policy_snapshot(&state),
        referrer_policy: state.referrer_policy.clone(),
        require_trusted_types_for_script:
            crate::content_security_policy::content_security_policy_requires_trusted_types_for_script(
                &state.content_security_policies,
            ),
        network_policy: crate::worker::handle::WorkerNetworkPolicy {
            secure_context: state.secure_context,
            permission_overrides: state.permission_overrides.clone(),
            extra_http_headers: state.extra_http_headers.clone(),
            network_offline: state.network_offline,
            blocked_url_patterns: state.blocked_url_patterns.clone(),
            network_partition_key: state.network_partition_key.clone(),
            fetch_subresource_interception_enabled: state.fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type: state
                .fetch_subresource_interception_resource_type,
        },
        policy_context: state.policy_context,
        shared_memory_agent_cluster: state.shared_memory_agent_cluster.clone(),
        wake_tx: state.worker_wake_tx.clone(),
    })
}

pub(in crate::worker) struct NestedWorkerUnhandledError {
    pub(in crate::worker) message: String,
    pub(in crate::worker) filename: String,
    pub(in crate::worker) lineno: u32,
    pub(in crate::worker) colno: u32,
    pub(in crate::worker) event_kind: crate::worker::handle::WorkerParentErrorEventKind,
}

pub(in crate::worker) struct NestedWorkerDispatchResult {
    pub(in crate::worker) dispatched: bool,
    pub(in crate::worker) unhandled_error: Option<NestedWorkerUnhandledError>,
}

pub(crate) fn register_nested_worker_handle(
    scope: &mut v8::PinScope<'_, '_>,
    worker_id: DedicatedWorkerId,
    handle: crate::worker::WorkerHandle,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };
    let mut state = state.borrow_mut();
    let Some(worker) = state.nested_workers.get_mut(&worker_id) else {
        return false;
    };
    if worker.handle.is_some() {
        return false;
    }
    worker.script_load_cancel = None;
    for task in worker.pending_tasks.drain(..) {
        handle.queue_task(task);
    }
    worker.handle = Some(handle);
    true
}

pub(crate) fn begin_nested_worker_script_load(
    scope: &mut v8::PinScope<'_, '_>,
    worker_id: DedicatedWorkerId,
    cancel: moli_fetch::FetchCancelHandle,
    load: crate::context_bootstrap::NestedWorkerScriptLoad,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };
    let mut state = state.borrow_mut();
    let Some(worker) = state.nested_workers.get_mut(&worker_id) else {
        return false;
    };
    worker.script_load_cancel = Some(cancel);
    worker.script_load = Some(load);
    true
}

pub(crate) fn take_nested_worker_script_load(
    scope: &mut v8::PinScope<'_, '_>,
    worker_id: DedicatedWorkerId,
) -> Option<crate::context_bootstrap::NestedWorkerScriptLoad> {
    let state = get_worker_state(scope)?;
    state
        .borrow_mut()
        .nested_workers
        .get_mut(&worker_id)?
        .script_load
        .take()
}

pub(crate) fn nested_worker_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker_id: DedicatedWorkerId,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    let worker = state.nested_workers.get(&worker_id)?;
    Some(v8::Local::new(scope, &worker.wrapper))
}

pub(crate) fn fail_nested_worker_script_load(
    scope: &mut v8::PinScope<'_, '_>,
    worker_id: DedicatedWorkerId,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    let mut state = state.borrow_mut();
    let Some(worker) = state.nested_workers.get_mut(&worker_id) else {
        return;
    };
    if let Some(cancel) = worker.script_load_cancel.take() {
        cancel.cancel();
    }
    let tasks = std::mem::take(&mut worker.pending_tasks);
    let load = worker.script_load.take();
    drop(state);
    drop(tasks);
    drop(load);
}

pub(crate) fn queue_nested_worker_task(
    scope: &mut v8::PinScope<'_, '_>,
    worker_id: DedicatedWorkerId,
    task: crate::worker::WorkerMessage,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };
    let mut state = state.borrow_mut();
    let Some(worker) = state.nested_workers.get_mut(&worker_id) else {
        return false;
    };
    if let Some(handle) = worker.handle.as_ref() {
        handle.queue_task(task);
    } else if worker.script_load_cancel.is_some() {
        worker.pending_tasks.push_back(task);
    } else {
        return false;
    }
    true
}

pub(crate) fn terminate_nested_worker(
    scope: &mut v8::PinScope<'_, '_>,
    worker_id: DedicatedWorkerId,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };
    let worker = state.borrow_mut().nested_workers.remove(&worker_id);
    worker.is_some()
}

pub(in crate::worker) fn retire_nested_workers(state: &Rc<RefCell<WorkerGlobalState>>) {
    // Drop native handles after releasing the state borrow. Their existing
    // termination path wakes each child without synchronously joining it.
    let workers = std::mem::take(&mut state.borrow_mut().nested_workers);
    drop(workers);
}

pub(in crate::worker) fn dispatch_nested_worker_event(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    worker_id: DedicatedWorkerId,
    message: &crate::worker::handle::WorkerToParentMessage,
) -> NestedWorkerDispatchResult {
    let Some(worker) = state
        .borrow()
        .nested_workers
        .get(&worker_id)
        .map(|worker| v8::Local::new(scope, &worker.wrapper))
    else {
        return NestedWorkerDispatchResult {
            dispatched: false,
            unhandled_error: None,
        };
    };

    match message {
        crate::worker::handle::WorkerToParentMessage::Post(_) => {
            let has_message_delivery_listener =
                crate::context_bootstrap::worker_has_message_delivery_listener(scope, worker);
            let _ = crate::context_bootstrap::dispatch_worker_event(scope, worker, message);
            NestedWorkerDispatchResult {
                dispatched: has_message_delivery_listener,
                unhandled_error: None,
            }
        }
        crate::worker::handle::WorkerToParentMessage::Error {
            message: error_message,
            filename,
            lineno,
            colno,
            event_kind,
            phase,
            ..
        } => {
            let unhandled = crate::context_bootstrap::dispatch_worker_event(scope, worker, message);
            // Bootstrap failures only fire an Event at the child Worker. Only
            // uncanceled runtime errors propagate to its owner's global scope.
            let propagate = unhandled && *phase == crate::worker::handle::WorkerErrorPhase::Runtime;
            NestedWorkerDispatchResult {
                dispatched: true,
                unhandled_error: propagate.then(|| NestedWorkerUnhandledError {
                    message: error_message.clone(),
                    filename: filename.clone(),
                    lineno: *lineno,
                    colno: *colno,
                    event_kind: *event_kind,
                }),
            }
        }
        _ => NestedWorkerDispatchResult {
            dispatched: crate::context_bootstrap::dispatch_worker_event(scope, worker, message),
            unhandled_error: None,
        },
    }
}

pub(crate) fn check_and_queue_nested_worker_constructor_csp(
    scope: &mut v8::PinScope<'_, '_>,
    request_url: &Url,
) -> Result<(), String> {
    let state = get_worker_state(scope)
        .expect("nested Worker construction requires an installed worker global state");
    let (wake_tx, report_only_violation, enforce_violation) = {
        let state = state.borrow();
        let protected_url = state
            .current_script_url
            .as_ref()
            .expect("nested Worker construction requires a current worker script URL");
        (
            state.worker_wake_tx.clone(),
            worker_content_security_policy_report_only_violation(
                &state,
                protected_url,
                request_url,
                crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConstructor,
            ),
            worker_content_security_policy_violation(
                &state,
                protected_url,
                request_url,
                crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConstructor,
            ),
        )
    };

    if let Some(violation) = report_only_violation {
        let _ = wake_tx.send(WorkerMessage::DispatchContentSecurityPolicyViolation(
            Box::new(violation),
        ));
    }
    let Some(violation) = enforce_violation else {
        return Ok(());
    };
    let message = worker_content_security_policy_error_message(&violation, "Worker");
    let _ = wake_tx.send(WorkerMessage::DispatchContentSecurityPolicyViolation(
        Box::new(violation),
    ));
    Err(message)
}
