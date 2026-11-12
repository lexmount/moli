use crate::{
    page_task_queue::{
        PageServiceWorkerQueryTargetEffect, RendererPageServiceWorkerQueryTaskId,
        RendererPageServiceWorkerQueryTaskKind,
    },
    types::ServiceWorkerRegistrationQueryResult,
};

use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{JsContextHost, WindowDocumentTaskTarget};

pub(crate) struct PendingServiceWorkerQuery {
    resolver: v8::Global<v8::PromiseResolver>,
    container: v8::Global<v8::Object>,
    result: ServiceWorkerRegistrationQueryResult,
}

pub(super) type ServiceWorkerQueryState = ExactWindowDocumentTaskLedger<
    RendererPageServiceWorkerQueryTaskId,
    RendererPageServiceWorkerQueryTaskKind,
    PendingServiceWorkerQuery,
>;

impl JsContextHost {
    /// Successful lookups, including no match, settle in the Promise realm's
    /// exact Document on the shared DOM-manipulation task source.
    pub(crate) fn queue_service_worker_query_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        resolver: v8::Local<'_, v8::PromiseResolver>,
        container: v8::Local<'_, v8::Object>,
        result: ServiceWorkerRegistrationQueryResult,
    ) {
        let Some(target) = self.current_window_document_task_target(scope) else {
            return;
        };
        let kind = RendererPageServiceWorkerQueryTaskKind::Resolve;
        let task_id = self
            .service_worker_queries
            .allocate_task_id(RendererPageServiceWorkerQueryTaskId::from_raw);
        self.service_worker_queries
            .push(PendingExactWindowDocumentTask::new(
                task_id,
                target,
                kind,
                PendingServiceWorkerQuery {
                    resolver: v8::Global::new(scope, resolver),
                    container: v8::Global::new(scope, container),
                    result,
                },
            ));
        if self
            .page_service_worker_query_sender()
            .send(target, task_id, kind)
            .is_err()
        {
            self.service_worker_queries
                .remove_exact(task_id, target, kind);
        }
    }

    pub(crate) fn current_pending_service_worker_query_task(
        &self,
        task_id: RendererPageServiceWorkerQueryTaskId,
    ) -> Option<(
        WindowDocumentTaskTarget,
        RendererPageServiceWorkerQueryTaskKind,
    )> {
        let pending = self.service_worker_queries.pending(task_id)?;
        let current = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((current, pending.kind()))
    }

    pub(crate) fn take_pending_service_worker_query_for_exact_target(
        &mut self,
        task_id: RendererPageServiceWorkerQueryTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageServiceWorkerQueryTaskKind,
    ) -> Option<PendingServiceWorkerQuery> {
        self.service_worker_queries
            .remove_exact(task_id, target, kind)
            .map(PendingExactWindowDocumentTask::into_payload)
    }

    pub(crate) fn discard_pending_service_worker_query_task(
        &mut self,
        task_id: RendererPageServiceWorkerQueryTaskId,
    ) -> bool {
        self.service_worker_queries.remove(task_id).is_some()
    }

    pub(crate) fn dispatch_authorized_service_worker_query(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        target: WindowDocumentTaskTarget,
        query: PendingServiceWorkerQuery,
    ) -> PageServiceWorkerQueryTargetEffect {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return PageServiceWorkerQueryTargetEffect::CurrentOwnerResolverRetired;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let resolver = v8::Local::new(scope, &query.resolver);
        let container = v8::Local::new(scope, &query.container);
        crate::context_bootstrap::settle_service_worker_query(
            scope,
            resolver,
            container,
            query.result,
        );
        dispatch_scope.restore(scope, previous_scope);
        PageServiceWorkerQueryTargetEffect::PromiseSettledForCurrentOwner
    }
}
