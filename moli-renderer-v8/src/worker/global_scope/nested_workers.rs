//! Nested worker ownership, creation snapshots and parent event delivery.

use crate::content_security_policy::ContentSecurityPolicy;
use crate::diagnostics::ErrorReportOutcome;
use crate::worker::handle::{
    WorkerErrorPhase, WorkerErrorReport, WorkerMessage, WorkerToParentMessage,
};

use super::*;

pub(crate) struct NestedWorkerContext {
    pub(crate) interception_context:
        Option<crate::network::interception_output::ResourceInterceptionContext>,
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
    pub(crate) wake_tx: mpsc::UnboundedSender<WorkerMessage>,
    pub(crate) closed_owner_error_tx: Option<mpsc::UnboundedSender<WorkerToParentMessage>>,
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
    state
        .nested_worker_wrappers
        .insert(worker_id, v8::Global::new(scope, worker));
    Some(NestedWorkerContext {
        interception_context: state.interception_context.clone(),
        worker_id,
        base_url,
        loader: state.loader.clone(),
        worker_context_runtime: state.worker_context_runtime.clone().for_nested_dedicated_worker(),
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
            secure_context: crate::worker::WorkerSecureContextPolicy::Inherited(state.secure_context),
            permission_overrides: state.permission_overrides.clone(),
            extra_http_headers: state.extra_http_headers.clone(),
            network_offline: state.network_offline,
            blocked_url_patterns: state.blocked_url_patterns.clone(),
            network_partition_key: state.network_partition_key.clone(),

        },
        policy_context: state.policy_context,
        wake_tx: state.worker_wake_tx.clone(),
        closed_owner_error_tx: matches!(state.global_kind, WorkerGlobalKind::Dedicated { .. })
            .then(|| state.parent_tx.clone()),
    })
}

pub(in crate::worker) struct NestedWorkerDispatchResult {
    pub(in crate::worker) dispatched: bool,
    pub(in crate::worker) unhandled_error: Option<WorkerErrorReport>,
}

pub(crate) fn forget_nested_worker_context(
    scope: &mut v8::PinScope<'_, '_>,
    worker_id: DedicatedWorkerId,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };
    let mut state = state.borrow_mut();
    state.nested_worker_wrappers.remove(&worker_id).is_some()
}

pub(in crate::worker) fn close_worker_owned_nested_workers(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
) {
    let wrappers = std::mem::take(&mut state.borrow_mut().nested_worker_wrappers);
    for wrapper in wrappers.into_values() {
        let worker = v8::Local::new(scope, wrapper);
        crate::context_bootstrap::terminate_worker_handle(scope, worker);
    }
}

/// A stopped DedicatedWorker has no callbacks to cancel a descendant's
/// report. Forward the existing exception to its owner.
pub(crate) fn forward_nested_worker_error_after_owner_close(
    message: WorkerMessage,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
) {
    if let WorkerMessage::NestedWorkerEvent { message, .. } = message
        && matches!(
            message.as_ref(),
            WorkerToParentMessage::Error(WorkerErrorReport {
                phase: WorkerErrorPhase::Runtime,
                ..
            })
        )
    {
        let _ = parent_tx.send(*message);
    }
}

pub(in crate::worker) fn dispatch_nested_worker_event(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    worker_id: DedicatedWorkerId,
    message: WorkerToParentMessage,
) -> NestedWorkerDispatchResult {
    let worker = state
        .borrow()
        .nested_worker_wrappers
        .get(&worker_id)
        .map(|worker| v8::Local::new(scope, worker));

    match (worker, message) {
        (Some(worker), message @ WorkerToParentMessage::Post(_)) => {
            let has_message_delivery_listener =
                crate::context_bootstrap::worker_has_message_delivery_listener(scope, worker);
            let _ = crate::context_bootstrap::dispatch_worker_event(scope, worker, &message);
            NestedWorkerDispatchResult {
                dispatched: has_message_delivery_listener,
                unhandled_error: None,
            }
        }
        (worker, WorkerToParentMessage::Error(mut error)) => {
            let unhandled = if let Some(worker) = worker {
                let omitted_error = v8::null(scope).into();
                crate::context_bootstrap::dispatch_worker_error_event_with_kind(
                    scope,
                    worker,
                    &error.message,
                    &error.filename,
                    error.lineno,
                    error.colno,
                    omitted_error,
                    error.event_kind,
                )
            } else if error.phase == WorkerErrorPhase::Runtime {
                Some(true)
            } else {
                None
            };
            // Bootstrap failures only fire an Event at the child Worker. Only
            // uncanceled runtime errors propagate to its owner's global scope.
            let unhandled_error =
                if unhandled == Some(true) && error.phase == WorkerErrorPhase::Runtime {
                    Some(error)
                } else {
                    if let Some(unhandled) = unhandled {
                        error.complete(|| ErrorReportOutcome::from_default_prevented(!unhandled));
                    }
                    None
                };
            NestedWorkerDispatchResult {
                dispatched: worker.is_some() || unhandled_error.is_some(),
                unhandled_error,
            }
        }
        (worker, message) => NestedWorkerDispatchResult {
            dispatched: worker.is_some_and(|worker| {
                crate::context_bootstrap::dispatch_worker_event(scope, worker, &message)
            }),
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
