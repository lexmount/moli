use super::load_delivery_tasks::FrameDocumentLoadDeliveryAdmissionId;
use super::records::FrameRealmId;
use super::records::{
    BrowsingContextId, DocumentId, DocumentLoadDelayTokenId, FrameNavigationId, FrameRequestId,
    FrameSchedulerLaneId, LocalWindowId, WindowProxyId,
};

#[derive(Debug)]
pub(super) struct FrameOwnerIdAllocator {
    next_nested_browsing_context_id: u64,
    next_window_proxy_id: u64,
    next_local_window_id: u64,
    next_document_id: u64,
    next_document_load_delay_token_id: u64,
    next_document_load_delivery_admission_id: u64,
    next_frame_navigation_id: u64,
    next_frame_realm_id: i64,
    next_frame_request_id: u64,
    next_scheduler_lane_id: u64,
}

impl Default for FrameOwnerIdAllocator {
    fn default() -> Self {
        Self {
            next_nested_browsing_context_id: 1,
            next_window_proxy_id: 1,
            next_local_window_id: 1,
            next_document_id: 1,
            next_document_load_delay_token_id: 1,
            next_document_load_delivery_admission_id: 1,
            next_frame_navigation_id: 1,
            next_frame_realm_id: 1,
            next_frame_request_id: 1,
            next_scheduler_lane_id: 1,
        }
    }
}

impl FrameOwnerIdAllocator {
    pub(super) fn nested_browsing_context(&mut self) -> BrowsingContextId {
        let id = BrowsingContextId::nested(self.next_nested_browsing_context_id);
        self.next_nested_browsing_context_id = self
            .next_nested_browsing_context_id
            .checked_add(1)
            .expect("nested browsing-context id overflow");
        id
    }

    pub(super) fn window_proxy(&mut self) -> WindowProxyId {
        WindowProxyId(take_next_u64(
            &mut self.next_window_proxy_id,
            "WindowProxy id space exhausted",
        ))
    }

    pub(super) fn local_window(&mut self) -> LocalWindowId {
        LocalWindowId(take_next_u64(
            &mut self.next_local_window_id,
            "LocalWindow id space exhausted",
        ))
    }

    pub(super) fn document(&mut self) -> DocumentId {
        DocumentId(take_next_u64(
            &mut self.next_document_id,
            "Document id space exhausted",
        ))
    }

    pub(super) fn document_load_delay_token(&mut self) -> DocumentLoadDelayTokenId {
        DocumentLoadDelayTokenId(take_next_u64(
            &mut self.next_document_load_delay_token_id,
            "Document load-delay token id space exhausted",
        ))
    }

    pub(super) fn document_load_delivery_admission(
        &mut self,
    ) -> FrameDocumentLoadDeliveryAdmissionId {
        FrameDocumentLoadDeliveryAdmissionId(take_next_u64(
            &mut self.next_document_load_delivery_admission_id,
            "Document load-delivery admission id space exhausted",
        ))
    }

    pub(super) fn frame_navigation(&mut self) -> FrameNavigationId {
        FrameNavigationId(take_next_u64(
            &mut self.next_frame_navigation_id,
            "frame navigation id space exhausted",
        ))
    }

    pub(super) fn frame_realm(&mut self) -> FrameRealmId {
        FrameRealmId(take_next_i64(
            &mut self.next_frame_realm_id,
            "frame realm id space exhausted",
        ))
    }

    pub(super) fn frame_request(&mut self) -> FrameRequestId {
        FrameRequestId(take_next_u64(
            &mut self.next_frame_request_id,
            "frame request id space exhausted",
        ))
    }

    pub(super) fn scheduler_lane(&mut self) -> FrameSchedulerLaneId {
        FrameSchedulerLaneId(take_next_u64(
            &mut self.next_scheduler_lane_id,
            "frame scheduler-lane id space exhausted",
        ))
    }
}

fn take_next_u64(next: &mut u64, exhausted: &'static str) -> u64 {
    let id = *next;
    *next = next.checked_add(1).expect(exhausted);
    id
}

fn take_next_i64(next: &mut i64, exhausted: &'static str) -> i64 {
    let id = *next;
    *next = next.checked_add(1).expect(exhausted);
    id
}

#[cfg(test)]
mod tests {
    use super::FrameOwnerIdAllocator;

    #[test]
    #[should_panic(expected = "Document id space exhausted")]
    fn document_ids_never_wrap() {
        let mut ids = FrameOwnerIdAllocator {
            next_document_id: u64::MAX,
            ..FrameOwnerIdAllocator::default()
        };

        let _ = ids.document();
    }
}
