use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{JsContextHost, WindowDocumentTaskTarget};
use crate::page_task_queue::{RendererPageWebRtcTaskId, RendererPageWebRtcTaskKind};

pub(super) type WebRtcTaskState = ExactWindowDocumentTaskLedger<
    RendererPageWebRtcTaskId,
    RendererPageWebRtcTaskKind,
    v8::Global<v8::Object>,
>;

impl JsContextHost {
    pub(crate) fn queue_webrtc_task<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
        kind: RendererPageWebRtcTaskKind,
    ) -> bool {
        let Some(context) = object.get_creation_context(scope) else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let Some(target) = self.current_window_document_task_target(scope) else {
            return false;
        };
        let id = self
            .webrtc_tasks
            .allocate_task_id(RendererPageWebRtcTaskId::from_raw);
        self.webrtc_tasks.push(PendingExactWindowDocumentTask::new(
            id,
            target,
            kind,
            v8::Global::new(scope, object),
        ));
        if self
            .page_task_capabilities
            .get()
            .expect("Page WebRTC task capabilities")
            .webrtc()
            .send(target, id, kind)
        {
            return true;
        }
        self.webrtc_tasks.remove_exact(id, target, kind);
        false
    }

    pub(crate) fn current_pending_webrtc_task(
        &self,
        id: RendererPageWebRtcTaskId,
    ) -> Option<(WindowDocumentTaskTarget, RendererPageWebRtcTaskKind)> {
        let pending = self.webrtc_tasks.pending(id)?;
        let target = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((target, pending.kind()))
    }

    pub(crate) fn apply_authorized_webrtc(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        _host_ptr: *mut JsContextHost,
        id: RendererPageWebRtcTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageWebRtcTaskKind,
    ) -> Option<bool> {
        let object = self
            .webrtc_tasks
            .remove_exact(id, target, kind)?
            .into_payload();
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return Some(false);
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch = target.dispatch_scope();
        let previous = dispatch.enter(scope);
        let object = v8::Local::new(scope, &object);
        let applied = crate::context_bootstrap::apply_webrtc_task(scope, object, kind);
        dispatch.restore(scope, previous);
        Some(applied)
    }

    pub(crate) fn discard_pending_webrtc_task(&mut self, id: RendererPageWebRtcTaskId) -> bool {
        self.webrtc_tasks.remove(id).is_some()
    }
}
