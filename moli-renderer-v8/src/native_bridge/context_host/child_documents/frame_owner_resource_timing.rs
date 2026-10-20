use super::super::{JsContextHost, OwnerDispatchScope, WindowExecutionContextOwner};
use crate::{
    context_bootstrap::{
        FetchResourceTiming, ResourcePerformanceEntry, record_resource_performance_entry,
    },
    document_runtime::DomHandle,
    frame_owner_model::{
        FrameDocumentDescendantLoadParent, FrameDocumentLoadDeliveryAction, FrameDocumentTaskOwner,
        FrameOwnerDocumentTarget,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::native_bridge::context_host) enum ChildDocumentNavigationInitiator {
    FrameOwnerElement,
    BrowsingContext,
    /// The history task already checked beforeunload for the retiring subtree.
    HistoryTraversal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::native_bridge::context_host) struct PendingFrameOwnerResourceTiming {
    target: FrameOwnerDocumentTarget,
    request_url: String,
    initiator_type: &'static str,
    start_unix_millis_bits: u64,
    started_at: std::time::Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::native_bridge::context_host) struct CompletedFrameOwnerResourceTiming {
    child_owner: FrameDocumentTaskOwner,
    timing: PendingFrameOwnerResourceTiming,
    network: crate::protocol_types::ChildFrameDocumentNetworkSnapshot,
}

impl PendingFrameOwnerResourceTiming {
    pub(in crate::native_bridge::context_host) fn complete(
        self,
        child_owner: FrameDocumentTaskOwner,
        network: crate::protocol_types::ChildFrameDocumentNetworkSnapshot,
    ) -> CompletedFrameOwnerResourceTiming {
        CompletedFrameOwnerResourceTiming {
            child_owner,
            timing: self,
            network,
        }
    }

    fn parent_dispatch_scope(&self) -> OwnerDispatchScope {
        match self.target.parent {
            FrameDocumentDescendantLoadParent::MainDocument => OwnerDispatchScope::Top,
            FrameDocumentDescendantLoadParent::ChildDocument(handle) => {
                OwnerDispatchScope::Child(handle)
            }
        }
    }

    fn parent_owner_is_current(&self, host: &JsContextHost) -> bool {
        match self.target.parent {
            FrameDocumentDescendantLoadParent::MainDocument => host
                .frame_owner_store
                .main_document_task_owner_is_current(self.target.owner),
            FrameDocumentDescendantLoadParent::ChildDocument(handle) => host
                .frame_owner_store
                .child_document_task_owner_is_current(handle, self.target.owner),
        }
    }

    fn performance_entry(
        &self,
        network: &crate::protocol_types::ChildFrameDocumentNetworkSnapshot,
    ) -> ResourcePerformanceEntry {
        let mut entry = ResourcePerformanceEntry::from_child_frame_document_network(
            self.request_url.clone(),
            self.initiator_type,
            Some(f64::from_bits(self.start_unix_millis_bits)),
            network,
        );
        let mut timing = FetchResourceTiming::from_start(
            self.initiator_type,
            self.started_at,
            f64::from_bits(self.start_unix_millis_bits),
        );
        if let Some(native) = network.resource_timing_response.as_deref() {
            timing.observe_native_timing(*native);
        }
        timing.apply_child_document_timing(&mut entry, network);
        entry
    }
}

impl CompletedFrameOwnerResourceTiming {
    pub(in crate::native_bridge::context_host) fn child_owner(&self) -> FrameDocumentTaskOwner {
        self.child_owner
    }
}

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn pending_frame_owner_resource_timing(
        &self,
        handle: DomHandle,
        target_url: &url::Url,
        initiator: ChildDocumentNavigationInitiator,
    ) -> Option<PendingFrameOwnerResourceTiming> {
        if initiator != ChildDocumentNavigationInitiator::FrameOwnerElement
            || !matches!(target_url.scheme(), "http" | "https")
        {
            return None;
        }
        let target = self
            .frame_owner_store
            .current_frame_owner_document_target(handle)?;
        let initiator_type = self.frame_owner_resource_initiator_type(handle)?;
        Some(PendingFrameOwnerResourceTiming {
            target,
            request_url: target_url.as_str().to_owned(),
            initiator_type,
            start_unix_millis_bits: moli_time::unix_epoch_millis().to_bits(),
            started_at: std::time::Instant::now(),
        })
    }

    pub(in crate::native_bridge::context_host) fn record_frame_owner_resource_timing_before_load(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        action: FrameDocumentLoadDeliveryAction,
    ) {
        let Some(timing) = self
            .child_browsing_contexts
            .get(&action.child_handle())
            .and_then(|entry| entry.frame_owner_resource_timing_for_owner(action.owner()))
        else {
            return;
        };
        if !self.record_frame_owner_resource_timing(scope, &timing.timing, &timing.network) {
            return;
        }
        if let Some(entry) = self.child_browsing_contexts.get_mut(&action.child_handle()) {
            entry.clear_frame_owner_resource_timing_if_owner(action.owner());
        }
    }

    pub(in crate::native_bridge::context_host) fn record_frame_owner_resource_timing_without_document_commit(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        timing: &PendingFrameOwnerResourceTiming,
        network: &crate::protocol_types::ChildFrameDocumentNetworkSnapshot,
    ) {
        // A null-body response can report the completed fetch without a new
        // Document or load event. Downloads and unsupported MIME types do not.
        // A TAO failure has no normal entry and, without load, no fallback entry.
        if matches!(network.status, 204 | 205) && network.resource_timing_allowed {
            self.record_frame_owner_resource_timing(scope, timing, network);
        }
    }

    fn record_frame_owner_resource_timing(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        timing: &PendingFrameOwnerResourceTiming,
        network: &crate::protocol_types::ChildFrameDocumentNetworkSnapshot,
    ) -> bool {
        if !timing.parent_owner_is_current(self) {
            return false;
        }
        let dispatch_scope = timing.parent_dispatch_scope();
        let execution_context_owner =
            WindowExecutionContextOwner::Frame(timing.target.owner.local_window_id);
        let Some((_, context)) =
            self.window_execution_context(scope, execution_context_owner, dispatch_scope)
        else {
            return false;
        };
        {
            let scope = &mut v8::ContextScope::new(scope, context);
            record_resource_performance_entry(scope, timing.performance_entry(network));
        }
        true
    }

    fn frame_owner_resource_initiator_type(&self, handle: DomHandle) -> Option<&'static str> {
        ["iframe", "frame", "embed", "object"]
            .into_iter()
            .find(|name| self.dom_host().is_html_element_named(handle, name))
    }
}
