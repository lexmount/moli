//! Promise-settlement bodies for browser-context ServiceWorker internal tasks.
//!
//! These helpers authorize the exact pending request inside `ScriptVm`, settle
//! its resolver, and deliberately leave Promise reactions pending. The
//! selected Page-task dispatcher owns the task-end microtask checkpoint.

use anyhow::Result;

use super::{ScriptVm, ServiceWorkerInternalBodyEffect};
use crate::types::{
    ServiceWorkerReadyCompletion, ServiceWorkerRegisterCompletion,
    ServiceWorkerUnregisterCompletion,
};

impl ScriptVm {
    pub(crate) fn apply_service_worker_register_body(
        &mut self,
        completion: ServiceWorkerRegisterCompletion,
    ) -> Result<ServiceWorkerInternalBodyEffect> {
        let Some(pending) = self
            ._context_host
            .borrow_mut()
            .take_pending_service_worker_register(completion.request_id)
        else {
            return Ok(ServiceWorkerInternalBodyEffect::ExactTargetUnavailable);
        };
        let owner = pending.owner;
        let owner_is_current = self
            ._context_host
            .borrow()
            .service_worker_window_owner_is_current(owner);
        if owner.window_document_owner() != completion.document_owner || !owner_is_current {
            tracing::debug!(
                dispatch_scope = ?owner.dispatch_scope(),
                document_owner = ?owner.document_owner(),
                owner_is_current,
                pending_document_owner = ?owner.window_document_owner(),
                completion_document_owner = ?completion.document_owner,
                request_id = completion.request_id,
                "dropped stale service worker register completion"
            );
            return Ok(ServiceWorkerInternalBodyEffect::ExactTargetUnavailable);
        }

        let context = pending.context;
        let resolver = pending.resolver;
        let context_ptr: *const v8::Global<v8::Context> = &context;
        self.with_context_scope_by_ptr(context_ptr, move |scope, _host_ptr| {
            let resolver = v8::Local::new(scope, &resolver);
            crate::context_bootstrap::settle_service_worker_register_completion(
                scope,
                resolver,
                owner.dispatch_scope(),
                completion.result,
            );
            Ok(())
        })?;
        Ok(ServiceWorkerInternalBodyEffect::PromiseSettled)
    }

    pub(crate) fn apply_service_worker_ready_body(
        &mut self,
        completion: ServiceWorkerReadyCompletion,
    ) -> Result<ServiceWorkerInternalBodyEffect> {
        let Some(pending) = self
            ._context_host
            .borrow_mut()
            .take_pending_service_worker_ready(completion.request_id)
        else {
            return Ok(ServiceWorkerInternalBodyEffect::ExactTargetUnavailable);
        };
        let owner = pending.request_context.owner();
        let owner_is_current = self
            ._context_host
            .borrow()
            .service_worker_window_owner_is_current(owner);
        if owner.window_document_owner() != completion.document_owner || !owner_is_current {
            tracing::debug!(
                dispatch_scope = ?owner.dispatch_scope(),
                document_owner = ?owner.document_owner(),
                owner_is_current,
                pending_document_owner = ?owner.window_document_owner(),
                completion_document_owner = ?completion.document_owner,
                request_id = completion.request_id,
                "dropped stale service worker ready completion"
            );
            return Ok(ServiceWorkerInternalBodyEffect::ExactTargetUnavailable);
        }

        let context = pending.context;
        let resolver = pending.resolver;
        let registration = completion.registration;
        let context_ptr: *const v8::Global<v8::Context> = &context;
        self.with_context_scope_by_ptr(context_ptr, move |scope, _host_ptr| {
            let resolver = v8::Local::new(scope, &resolver);
            crate::context_bootstrap::settle_service_worker_ready_completion(
                scope,
                resolver,
                owner.dispatch_scope(),
                registration,
            );
            Ok(())
        })?;
        Ok(ServiceWorkerInternalBodyEffect::PromiseSettled)
    }

    pub(crate) fn apply_service_worker_unregister_body(
        &mut self,
        completion: ServiceWorkerUnregisterCompletion,
    ) -> Result<ServiceWorkerInternalBodyEffect> {
        let Some(pending) = self
            ._context_host
            .borrow_mut()
            .take_pending_service_worker_unregister(completion.request_id)
        else {
            return Ok(ServiceWorkerInternalBodyEffect::ExactTargetUnavailable);
        };
        let owner = pending.owner;
        let owner_is_current = self
            ._context_host
            .borrow()
            .service_worker_window_owner_is_current(owner);
        if owner.window_document_owner() != completion.document_owner || !owner_is_current {
            tracing::debug!(
                dispatch_scope = ?owner.dispatch_scope(),
                document_owner = ?owner.document_owner(),
                owner_is_current,
                pending_document_owner = ?owner.window_document_owner(),
                completion_document_owner = ?completion.document_owner,
                request_id = completion.request_id,
                "dropped stale service worker unregister completion"
            );
            return Ok(ServiceWorkerInternalBodyEffect::ExactTargetUnavailable);
        }

        let context = pending.context;
        let resolver = pending.resolver;
        let registration = pending.registration;
        let active_worker = pending.active_worker;
        let removed = completion.result;
        let context_ptr: *const v8::Global<v8::Context> = &context;
        self.with_context_scope_by_ptr(context_ptr, move |scope, _host_ptr| {
            let resolver = v8::Local::new(scope, &resolver);
            let registration = v8::Local::new(scope, &registration);
            let active_worker = active_worker
                .as_ref()
                .map(|worker| v8::Local::new(scope, worker));
            crate::context_bootstrap::settle_service_worker_unregister_completion(
                scope,
                resolver,
                Some(registration),
                active_worker,
                removed,
            );
            Ok(())
        })?;
        Ok(ServiceWorkerInternalBodyEffect::PromiseSettled)
    }
}
