use super::{
    JsContextHost, OwnerDispatchScope, RuntimeObservableContextToken, WindowExecutionContextOwner,
    WindowTaskTarget,
};
use crate::page_task_queue::RendererPageWindowMessageTaskId;
use crate::{document_runtime::DomHandle, structured_clone::V8StructuredClonePayload};

pub(crate) struct PendingWindowMessage {
    pub(crate) target: WindowTaskTarget,
    pub(crate) source: PendingWindowMessageSource,
    pub(crate) source_window_proxy: Option<v8::Global<v8::Object>>,
    /// Whether the source endpoint is the target's current opener. Its event
    /// projection can then reuse the target realm's canonical `window.opener`
    /// wrapper instead of exposing a second wrapper for the same endpoint.
    pub(crate) source_is_target_opener: bool,
    pub(crate) data: V8StructuredClonePayload,
    pub(crate) origin: String,
    pub(crate) intended_target_origin: Option<String>,
}

pub(super) struct QueuedWindowMessage {
    task_id: RendererPageWindowMessageTaskId,
    message: PendingWindowMessage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingWindowMessageSource {
    endpoint: PendingWindowMessageEndpoint,
    owner: Option<WindowExecutionContextOwner>,
    realm_token: Option<RuntimeObservableContextToken>,
}

impl PendingWindowMessageSource {
    pub(crate) fn new(
        endpoint: PendingWindowMessageEndpoint,
        owner: WindowExecutionContextOwner,
        realm_token: RuntimeObservableContextToken,
    ) -> Self {
        Self {
            endpoint,
            owner: Some(owner),
            realm_token: Some(realm_token),
        }
    }

    pub(crate) fn remote_top_level() -> Self {
        Self {
            endpoint: PendingWindowMessageEndpoint::TopWindow,
            owner: None,
            realm_token: None,
        }
    }

    pub(crate) fn endpoint(self) -> PendingWindowMessageEndpoint {
        self.endpoint
    }

    pub(crate) fn owner(self) -> Option<WindowExecutionContextOwner> {
        self.owner
    }

    pub(crate) fn realm_token(self) -> Option<RuntimeObservableContextToken> {
        self.realm_token
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PendingWindowMessageEndpoint {
    TopWindow,
    ChildWindow(DomHandle),
}

impl PendingWindowMessageEndpoint {
    pub(crate) fn dispatch_scope(self) -> OwnerDispatchScope {
        match self {
            Self::TopWindow => OwnerDispatchScope::Top,
            Self::ChildWindow(handle) => OwnerDispatchScope::Child(handle),
        }
    }

    pub(crate) const fn from_dispatch_scope(dispatch_scope: OwnerDispatchScope) -> Self {
        match dispatch_scope {
            OwnerDispatchScope::Top => Self::TopWindow,
            OwnerDispatchScope::Child(handle) => Self::ChildWindow(handle),
        }
    }
}

impl JsContextHost {
    fn remote_window_message_source_projection<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        source: &crate::runtime::RendererRemoteWindowProxySource,
    ) -> Option<(v8::Local<'s, v8::Object>, bool)> {
        let source_endpoint = source.endpoint();
        let source_target = self
            .page_script_environment
            .as_ref()?
            .remote_top_level_target_snapshot(source_endpoint)?;
        if source_target.residence != source.page() {
            return None;
        }
        if let Some(frame) = source.frame() {
            if frame.endpoint != source_endpoint
                || self
                    .page_script_environment
                    .as_ref()?
                    .remote_frame_snapshot(frame)
                    .is_none()
            {
                return None;
            }
            return self
                .remote_frame_window_proxy_for_token(scope, frame)
                .map(|proxy| (proxy, false));
        }

        let source_is_target_opener =
            self.page_script_environment
                .as_ref()
                .is_some_and(|environment| {
                    environment.top_level_opener_endpoint() == Some(source_endpoint)
                });
        let opener_source = source_is_target_opener
            .then(|| {
                scope
                    .get_current_context()
                    .global(scope)
                    .get(scope, crate::util::v8str(scope, "opener").into())
            })
            .flatten()
            .and_then(|opener| v8::Local::<v8::Object>::try_from(opener).ok());
        opener_source
            .or_else(|| self.remote_top_level_window_proxy_for_endpoint(scope, source_endpoint))
            .map(|proxy| (proxy, source_is_target_opener))
    }

    pub(crate) fn queue_remote_top_level_window_message(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        message: crate::runtime::RendererRemoteWindowProxyMessage,
    ) -> bool {
        let Some((source_window_proxy, source_is_target_opener)) =
            self.remote_window_message_source_projection(scope, &message.source)
        else {
            return false;
        };
        let dispatch_scope = OwnerDispatchScope::Top;
        let Some(owner) = self.current_window_execution_context_owner(dispatch_scope) else {
            return false;
        };
        let target = WindowTaskTarget::new(dispatch_scope, owner);
        let task_id = self.queue_window_message(PendingWindowMessage {
            target,
            source: PendingWindowMessageSource::remote_top_level(),
            source_window_proxy: Some(v8::Global::new(scope, source_window_proxy)),
            source_is_target_opener,
            data: message.payload,
            origin: message.source.serialized_origin().to_owned(),
            intended_target_origin: message.intended_target_origin,
        });
        let sender = self.page_window_message_sender().clone();
        if sender.send(target, task_id).is_err() {
            let discarded = self.discard_pending_window_message_task(task_id);
            assert!(
                discarded,
                "closed remote Window.postMessage route lost its local payload"
            );
            return false;
        }
        true
    }

    pub(crate) fn queue_remote_frame_window_message(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        message: crate::runtime::RendererRemoteWindowProxyMessage,
    ) -> bool {
        if self
            .ensure_prebootstrapped_child_default_context(scope, handle)
            .is_err()
        {
            return false;
        }
        let Some((source_window_proxy, _)) =
            self.remote_window_message_source_projection(scope, &message.source)
        else {
            return false;
        };
        let dispatch_scope = OwnerDispatchScope::Child(handle);
        let Some(owner) = self.current_window_execution_context_owner(dispatch_scope) else {
            return false;
        };
        let target = WindowTaskTarget::new(dispatch_scope, owner);
        let task_id = self.queue_window_message(PendingWindowMessage {
            target,
            source: PendingWindowMessageSource::remote_top_level(),
            source_window_proxy: Some(v8::Global::new(scope, source_window_proxy)),
            source_is_target_opener: false,
            data: message.payload,
            origin: message.source.serialized_origin().to_owned(),
            intended_target_origin: message.intended_target_origin,
        });
        let sender = self.page_window_message_sender().clone();
        if sender.send(target, task_id).is_err() {
            let discarded = self.discard_pending_window_message_task(task_id);
            assert!(
                discarded,
                "closed remote-frame Window.postMessage route lost its local payload"
            );
            return false;
        }
        true
    }

    pub(crate) fn enter_window_message_source_scope(
        &mut self,
        source: PendingWindowMessageEndpoint,
    ) -> Option<PendingWindowMessageEndpoint> {
        let previous = self.current_window_message_source;
        self.current_window_message_source = Some(source);
        previous
    }

    pub(crate) fn restore_window_message_source_scope(
        &mut self,
        previous: Option<PendingWindowMessageEndpoint>,
    ) {
        self.current_window_message_source = previous;
    }

    pub(crate) fn current_window_message_source(&self) -> Option<PendingWindowMessageEndpoint> {
        self.current_window_message_source
    }

    pub(crate) fn queue_window_message(
        &mut self,
        message: PendingWindowMessage,
    ) -> RendererPageWindowMessageTaskId {
        let task_id = self.next_window_message_task_id;
        self.next_window_message_task_id = task_id
            .checked_next()
            .expect("Window.postMessage task id overflow");
        self.pending_window_messages
            .push_back(QueuedWindowMessage { task_id, message });
        task_id
    }

    pub(crate) fn retire_window_messages_for_execution_context_owner(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) -> usize {
        let retired_count =
            self.retire_window_messages_for_execution_context_owner_without_signal(owner);
        self.signal_retired_window_message_tasks(retired_count);
        retired_count
    }

    fn retire_window_messages_for_execution_context_owner_without_signal(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) -> usize {
        let mut retained =
            std::collections::VecDeque::with_capacity(self.pending_window_messages.len());
        let mut retired_count = 0;
        while let Some(queued) = self.pending_window_messages.pop_front() {
            if queued.message.target.owner() == owner {
                self.retire_transferred_window_message_ports(&queued.message);
                retired_count += 1;
            } else {
                retained.push_back(queued);
            }
        }
        self.pending_window_messages = retained;
        retired_count
    }

    fn signal_retired_window_message_tasks(&self, retired_count: usize) {
        if retired_count != 0 {
            // The corresponding stable Page tasks intentionally outlive this
            // PageVm-local payload. Readmit the ready source so the Page
            // arbiter can dequeue those now-stale tickets even when their
            // original readiness wake was already consumed while blocked.
            self.page_window_message_sender().signal_reconsideration();
        }
    }

    pub(crate) fn retire_window_messages_for_context_token(
        &mut self,
        context_token: RuntimeObservableContextToken,
    ) -> usize {
        let owners = self
            .window_execution_contexts
            .iter()
            .filter_map(|(owner, binding)| {
                (binding.realm_token() == context_token).then_some(*owner)
            })
            .collect::<Vec<_>>();
        let retired_count = owners
            .into_iter()
            .map(|owner| {
                self.retire_window_messages_for_execution_context_owner_without_signal(owner)
            })
            .sum();
        self.signal_retired_window_message_tasks(retired_count);
        retired_count
    }

    pub(crate) fn retire_transferred_window_message_ports(
        &mut self,
        message: &PendingWindowMessage,
    ) {
        self.retire_transferred_window_message_payload(&message.data);
    }

    pub(crate) fn retire_transferred_window_message_payload(
        &mut self,
        payload: &V8StructuredClonePayload,
    ) {
        for port_id in payload.transferred_message_ports() {
            if !self.retire_message_port(*port_id) {
                self.message_port_registry.close_message_port(*port_id);
            }
        }
    }

    pub(crate) fn has_pending_window_messages(&self) -> bool {
        !self.pending_window_messages.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn pending_window_message_endpoints_for_test(
        &self,
    ) -> Vec<(PendingWindowMessageEndpoint, PendingWindowMessageEndpoint)> {
        self.pending_window_messages
            .iter()
            .map(|queued| {
                (
                    PendingWindowMessageEndpoint::from_dispatch_scope(
                        queued.message.target.dispatch_scope(),
                    ),
                    queued.message.source.endpoint(),
                )
            })
            .collect()
    }

    pub(crate) fn has_pending_window_message_task(
        &self,
        task_id: RendererPageWindowMessageTaskId,
    ) -> bool {
        self.pending_window_messages
            .iter()
            .any(|queued| queued.task_id == task_id)
    }

    pub(crate) fn take_pending_window_message_task(
        &mut self,
        task_id: RendererPageWindowMessageTaskId,
    ) -> Option<PendingWindowMessage> {
        let index = self
            .pending_window_messages
            .iter()
            .position(|queued| queued.task_id == task_id)?;
        self.pending_window_messages
            .remove(index)
            .map(|queued| queued.message)
    }

    pub(crate) fn discard_pending_window_message_task(
        &mut self,
        task_id: RendererPageWindowMessageTaskId,
    ) -> bool {
        let Some(message) = self.take_pending_window_message_task(task_id) else {
            return false;
        };
        self.retire_transferred_window_message_ports(&message);
        true
    }

    pub(crate) fn window_message_target_is_materialized(&self, target: WindowTaskTarget) -> bool {
        self.window_execution_contexts
            .get(&target.owner())
            .is_some_and(|binding| binding.dispatch_scope() == target.dispatch_scope())
    }

    pub(crate) fn signal_pending_window_message_reconsideration(&self) {
        if self.has_pending_window_messages() {
            self.page_window_message_sender().signal_reconsideration();
        }
    }

    pub(crate) fn defer_active_child_window_restore_after_microtasks(
        &mut self,
        previous: Option<DomHandle>,
    ) {
        self.pending_active_child_window_restore = Some(previous);
    }

    pub(crate) fn take_deferred_active_child_window_restore(
        &mut self,
    ) -> Option<Option<DomHandle>> {
        self.pending_active_child_window_restore.take()
    }
}
