use super::window_document_tasks::{PendingWindowTask, WindowTaskLedger};
use super::{
    JsContextHost, RuntimeObservableContextToken, WindowExecutionContextIdentity,
    WindowExecutionContextOwner,
};
use crate::context_bootstrap::webtransport::{
    backend::ConnectionContext,
    networking::{Pending, Producer},
};
use crate::network::loads::{ResourceLoadDisposition, ResourceLoadKind};
use crate::page_task_queue::{RendererPageWebTransportTaskId, RendererPageWebTransportTaskKind};

pub(super) type WebTransportTaskState = WindowTaskLedger<
    RendererPageWebTransportTaskId,
    RendererPageWebTransportTaskKind,
    Pending,
    WindowExecutionContextIdentity,
>;

impl JsContextHost {
    pub(crate) fn webtransport_connection_context(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        url: &url::Url,
    ) -> Option<ConnectionContext> {
        let target = self.current_window_document_task_target(scope)?;
        let loader = self.document_resource_loader_for_dispatch_scope(target.dispatch_scope())?;
        let lease = loader.register_load(
            ResourceLoadKind::WebTransport,
            ResourceLoadDisposition::Ordinary,
            None,
        )?;
        Some(ConnectionContext {
            runner: loader.task_runner(),
            connector: loader.request_client().webtransport_connector(),
            lease,
            origin: self.window_document_origin(target.dispatch_scope())?,
            blocked: self.network_offline() || self.is_url_blocked(url),
        })
    }

    pub(crate) fn register_webtransport_task<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: v8::Local<'s, v8::Object>,
        payload: v8::Local<'s, v8::Object>,
    ) -> Option<Producer> {
        let context = owner.get_creation_context(scope)?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let target = self.current_runtime_window_execution_context_identity(scope)?;
        let sender = self.page_task_capabilities.get()?.webtransport().clone();
        let id = self
            .webtransport_tasks
            .allocate_task_id(RendererPageWebTransportTaskId::from_raw);
        let (pending, result) = Pending::new(scope, payload);
        self.webtransport_tasks.push(PendingWindowTask::new(
            id,
            target,
            RendererPageWebTransportTaskKind::Complete,
            pending,
        ));
        Some(Producer::window(result, sender, target, id))
    }

    pub(crate) fn current_pending_webtransport_task(
        &self,
        id: RendererPageWebTransportTaskId,
    ) -> Option<(
        WindowExecutionContextIdentity,
        RendererPageWebTransportTaskKind,
    )> {
        let pending = self.webtransport_tasks.pending(id)?;
        self.window_execution_context_identity_is_current(pending.target())
            .then_some((pending.target(), pending.kind()))
    }

    pub(crate) fn apply_authorized_webtransport(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        _host_ptr: *mut JsContextHost,
        id: RendererPageWebTransportTaskId,
        target: WindowExecutionContextIdentity,
        kind: RendererPageWebTransportTaskKind,
    ) -> Option<bool> {
        let pending = self
            .webtransport_tasks
            .remove_exact(id, target, kind)?
            .into_payload();
        if !self.window_execution_context_identity_is_current(target) {
            return Some(false);
        }
        let context = pending.context(scope)?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let dispatch = target.dispatch_scope();
        let previous = dispatch.enter(scope);
        let applied = pending.apply(scope);
        dispatch.restore(scope, previous);
        Some(applied)
    }

    pub(crate) fn discard_pending_webtransport_task(
        &mut self,
        id: RendererPageWebTransportTaskId,
    ) -> bool {
        self.webtransport_tasks.remove(id).is_some()
    }

    pub(crate) fn retire_webtransport_execution_context_owner(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) {
        self.webtransport_tasks
            .retain(|pending| pending.target().owner() != owner);
    }

    pub(crate) fn retire_webtransport_context_token(
        &mut self,
        token: RuntimeObservableContextToken,
    ) {
        self.webtransport_tasks
            .retain(|pending| pending.target().realm_token() != token);
    }
}
