use super::super::{OwnerDispatchScope, PageScriptHost, WindowExecutionContextOwner};
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
    BrowsingContext(Option<super::ChildNavigationDownloadPolicy>),
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
    initiator: crate::context_bootstrap::ResourceTimingInitiator,
    request_origin: moli_url::WebOrigin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::native_bridge::context_host) struct CompletedFrameOwnerResourceTiming {
    child_owner: FrameDocumentTaskOwner,
    timing: PendingFrameOwnerResourceTiming,
    network: crate::protocol_types::ChildFrameDocumentNetworkSnapshot,
}

impl PendingFrameOwnerResourceTiming {
    pub(in crate::native_bridge::context_host) fn request_origin(&self) -> moli_url::WebOrigin {
        self.request_origin.clone()
    }

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

    fn parent_owner_is_current(&self, host: &PageScriptHost) -> bool {
        match self.target.parent {
            FrameDocumentDescendantLoadParent::MainDocument => host
                .frame_owner_store
                .main_document_task_owner_is_current(self.target.owner),
            FrameDocumentDescendantLoadParent::ChildDocument(handle) => host
                .frame_owner_store
                .child_document_task_owner_is_current(handle, self.target.owner),
        }
    }

    fn apply_native_timing(
        &self,
        entry: &mut ResourcePerformanceEntry,
        network: &crate::protocol_types::ChildFrameDocumentNetworkSnapshot,
        expose_document_response: bool,
    ) {
        let timing = FetchResourceTiming::from_start(
            self.initiator_type,
            self.started_at,
            self.start_unix_millis(),
        );
        if let Some(native) = network.resource_timing_response.as_deref() {
            timing.observe_native_timing(*native);
        }
        timing.apply_child_document_timing(entry, network, expose_document_response);
        self.initiator
            .apply_child_document(entry, network, expose_document_response);
    }

    fn start_unix_millis(&self) -> f64 {
        f64::from_bits(self.start_unix_millis_bits)
    }
}

impl CompletedFrameOwnerResourceTiming {
    pub(in crate::native_bridge::context_host) fn child_owner(&self) -> FrameDocumentTaskOwner {
        self.child_owner
    }

    fn performance_entry(&self, expose_document_response: bool) -> ResourcePerformanceEntry {
        let mut entry = ResourcePerformanceEntry::from_child_frame_document_network(
            self.timing.request_url.clone(),
            self.timing.initiator_type,
            Some(self.timing.start_unix_millis()),
            &self.network,
        );
        self.timing
            .apply_native_timing(&mut entry, &self.network, expose_document_response);
        entry
    }
}

impl PageScriptHost {
    pub(in crate::native_bridge::context_host) fn pending_frame_owner_resource_timing(
        &self,
        handle: DomHandle,
        target_url: &url::Url,
        initiator: ChildDocumentNavigationInitiator,
        resource_initiator: crate::context_bootstrap::ResourceTimingInitiator,
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
        let request_origin = self
            .document_resource_loader_for_owner(target.owner)?
            .fetch_context()
            .request_origin();
        Some(PendingFrameOwnerResourceTiming {
            target,
            request_url: target_url.as_str().to_owned(),
            initiator_type,
            start_unix_millis_bits: moli_time::unix_epoch_millis().to_bits(),
            started_at: std::time::Instant::now(),
            initiator: resource_initiator,
            request_origin,
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
        let expose_response = self
            .frame_owner_store
            .current_child_document_creation_kind(action.child_handle())
            != Some(crate::frame_owner_model::DocumentCreationKind::Error);
        if self.record_frame_owner_resource_timing(
            scope,
            &timing.timing,
            timing.performance_entry(expose_response),
        ) && let Some(entry) = self.child_browsing_contexts.get_mut(&action.child_handle())
        {
            entry.clear_frame_owner_resource_timing_if_owner(action.owner());
        }
    }

    pub(in crate::native_bridge::context_host) fn record_no_content_frame_owner_resource_timing(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        timing: &PendingFrameOwnerResourceTiming,
        network: &crate::protocol_types::ChildFrameDocumentNetworkSnapshot,
        response_status: u16,
    ) {
        if !network.resource_timing_allowed {
            return;
        }
        let mut entry = ResourcePerformanceEntry::from_child_frame_no_content(
            timing.request_url.clone(),
            timing.initiator_type,
            Some(timing.start_unix_millis()),
            response_status,
            network.cache_state,
            network.preload_state.clone(),
        );
        timing.apply_native_timing(&mut entry, network, true);
        self.record_frame_owner_resource_timing(scope, timing, entry);
    }

    fn record_frame_owner_resource_timing(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        timing: &PendingFrameOwnerResourceTiming,
        entry: ResourcePerformanceEntry,
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
        let scope = &mut v8::ContextScope::new(scope, context);
        record_resource_performance_entry(scope, entry);
        true
    }

    fn frame_owner_resource_initiator_type(&self, handle: DomHandle) -> Option<&'static str> {
        ["iframe", "frame", "embed", "object"]
            .into_iter()
            .find(|name| self.dom_host().is_html_element_named(handle, name))
    }
}
