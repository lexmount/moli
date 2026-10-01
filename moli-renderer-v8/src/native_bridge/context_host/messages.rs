use super::{
    JsContextHost, OwnerDispatchScope, RuntimeObservableContextToken, WindowExecutionContextOwner,
    WindowTaskTarget,
};
use crate::page_task_queue::RendererPageWindowMessageTaskId;
use crate::{document_runtime::DomHandle, structured_clone::V8StructuredClonePayload};

pub(crate) struct PendingWindowMessage {
    pub(crate) target: WindowTaskTarget,
    pub(crate) source: PendingWindowMessageSource,
    // Page-local endpoint ids cannot identify a Window in another Page. Keep
    // the accepted browsing context proxy through delivery, including a later
    // navigation or close of its Document.
    pub(crate) source_window: Option<v8::Global<v8::Object>>,
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
    owner: WindowExecutionContextOwner,
    realm_token: RuntimeObservableContextToken,
}

impl PendingWindowMessageSource {
    pub(crate) fn new(
        endpoint: PendingWindowMessageEndpoint,
        owner: WindowExecutionContextOwner,
        realm_token: RuntimeObservableContextToken,
    ) -> Self {
        Self {
            endpoint,
            owner,
            realm_token,
        }
    }

    pub(crate) fn endpoint(self) -> PendingWindowMessageEndpoint {
        self.endpoint
    }

    pub(crate) fn owner(self) -> WindowExecutionContextOwner {
        self.owner
    }

    pub(crate) fn realm_token(self) -> RuntimeObservableContextToken {
        self.realm_token
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PendingWindowMessageEndpoint {
    TopWindow,
    ChildWindow(DomHandle),
    LightweightPopup(u64),
}

impl PendingWindowMessageEndpoint {
    pub(crate) fn dispatch_scope(self) -> OwnerDispatchScope {
        match self {
            Self::TopWindow => OwnerDispatchScope::Top,
            Self::ChildWindow(handle) => OwnerDispatchScope::Child(handle),
            Self::LightweightPopup(popup_id) => OwnerDispatchScope::LightweightPopup(popup_id),
        }
    }

    pub(crate) const fn from_dispatch_scope(dispatch_scope: OwnerDispatchScope) -> Self {
        match dispatch_scope {
            OwnerDispatchScope::Top => Self::TopWindow,
            OwnerDispatchScope::Child(handle) => Self::ChildWindow(handle),
            OwnerDispatchScope::LightweightPopup(popup_id) => Self::LightweightPopup(popup_id),
        }
    }
}

impl JsContextHost {
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
        for port_id in message.data.transferred_message_ports() {
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

    pub(crate) fn defer_active_lightweight_popup_restore_after_microtasks(
        &mut self,
        previous: Option<u64>,
    ) {
        self.pending_active_lightweight_popup_restore = Some(previous);
    }

    pub(crate) fn take_deferred_active_lightweight_popup_restore(&mut self) -> Option<Option<u64>> {
        self.pending_active_lightweight_popup_restore.take()
    }
}
