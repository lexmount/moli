use super::super::{JsContextHost, OwnerDispatchScope, WindowExecutionContextOwner};
use crate::{
    context_bootstrap::{ResourcePerformanceEntry, record_resource_performance_entry},
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::native_bridge::context_host) struct PendingFrameOwnerResourceTiming {
    target: FrameOwnerDocumentTarget,
    request_url: String,
    initiator_type: &'static str,
    start_unix_millis_bits: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::native_bridge::context_host) struct CompletedFrameOwnerResourceTiming {
    child_owner: FrameDocumentTaskOwner,
    target: FrameOwnerDocumentTarget,
    request_url: String,
    initiator_type: &'static str,
    start_unix_millis_bits: u64,
    network: crate::protocol_types::ChildFrameDocumentNetworkResponse,
}

impl PendingFrameOwnerResourceTiming {
    pub(in crate::native_bridge::context_host) fn complete(
        self,
        child_owner: FrameDocumentTaskOwner,
        network: crate::protocol_types::ChildFrameDocumentNetworkResponse,
    ) -> CompletedFrameOwnerResourceTiming {
        CompletedFrameOwnerResourceTiming {
            child_owner,
            target: self.target,
            request_url: self.request_url,
            initiator_type: self.initiator_type,
            start_unix_millis_bits: self.start_unix_millis_bits,
            network,
        }
    }
}

impl CompletedFrameOwnerResourceTiming {
    pub(in crate::native_bridge::context_host) fn child_owner(&self) -> FrameDocumentTaskOwner {
        self.child_owner
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

    fn performance_entry(&self) -> ResourcePerformanceEntry {
        ResourcePerformanceEntry::from_child_frame_document_network(
            self.request_url.clone(),
            self.initiator_type,
            Some(f64::from_bits(self.start_unix_millis_bits)),
            &self.network,
        )
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
        if !timing.parent_owner_is_current(self) {
            return;
        }
        let dispatch_scope = timing.parent_dispatch_scope();
        let execution_context_owner =
            WindowExecutionContextOwner::Frame(timing.target.owner.local_window_id);
        let Some((_, context)) =
            self.window_execution_context(scope, execution_context_owner, dispatch_scope)
        else {
            return;
        };
        {
            let scope = &mut v8::ContextScope::new(scope, context);
            record_resource_performance_entry(scope, timing.performance_entry());
        }
        if let Some(entry) = self.child_browsing_contexts.get_mut(&action.child_handle()) {
            entry.clear_frame_owner_resource_timing_if_owner(action.owner());
        }
    }

    fn frame_owner_resource_initiator_type(&self, handle: DomHandle) -> Option<&'static str> {
        ["iframe", "frame", "embed", "object"]
            .into_iter()
            .find(|name| self.dom_host().is_html_element_named(handle, name))
    }
}
