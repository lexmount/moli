use super::super::{
    BackgroundTarget, BrowserContext, ConnectionNetworkRequestIdAllocator, PausedDocumentTransfer,
    PendingFetchAuthNavigation, PendingFetchNavigation, PendingSubresourceFetchAuthRequest,
    PendingSubresourceFetchRequest, PendingSubresourceFetchResponseRequest,
};
#[cfg(test)]
use super::super::{DocumentBodySource, DocumentNavigationToken, NavigationDispatchState};

fn document_navigation_loader_id(sequence: u64) -> String {
    format!("LID-{sequence:010}")
}

impl BrowserContext {
    pub fn background_target(&self, target_id: &str) -> Option<&BackgroundTarget> {
        self.background_targets
            .iter()
            .find(|target| target.is_target(target_id))
    }

    pub(crate) fn background_target_mut(
        &mut self,
        target_id: &str,
    ) -> Option<&mut BackgroundTarget> {
        self.background_targets
            .iter_mut()
            .find(|target| target.is_target(target_id))
    }

    pub(crate) fn take_active_target_pending_fetch_state(
        &mut self,
    ) -> (
        Vec<PendingFetchNavigation>,
        Vec<PendingFetchAuthNavigation>,
        Vec<PausedDocumentTransfer>,
        Vec<(String, PendingSubresourceFetchRequest)>,
        Vec<(String, PendingSubresourceFetchAuthRequest)>,
        Vec<(String, PendingSubresourceFetchResponseRequest)>,
    ) {
        self.active_target.fetch_owner.drain_pending_requests()
    }

    pub(crate) fn clear_pending_fetch_state(&mut self) {
        self.active_target.fetch_owner.clear_pending();
    }

    #[cfg(test)]
    pub(crate) fn open_pending_fetch_response_body_stream(
        &mut self,
        request_id: &str,
    ) -> Result<Option<String>, String> {
        let handle = self.active_target.runtime_slot.allocate_io_stream_handle();
        self.active_target
            .fetch_owner
            .open_pending_fetch_response_body_stream(
                &mut self.active_target.runtime_slot,
                request_id,
                handle,
            )
    }

    #[cfg(test)]
    pub(crate) fn take_pending_subresource_fetch_request_for_test(
        &mut self,
        request_id: &str,
    ) -> Option<PendingSubresourceFetchRequest> {
        self.active_target
            .fetch_owner
            .take_pending_subresource_fetch_request(request_id, None)
    }

    #[cfg(test)]
    pub(crate) fn register_pending_fetch_response_navigation(
        &mut self,
        request_id: String,
        document_navigation_token: Option<DocumentNavigationToken>,
        navigation: NavigationDispatchState,
        body: DocumentBodySource,
    ) {
        self.active_target
            .fetch_owner
            .register_pending_fetch_response_navigation(
                request_id,
                document_navigation_token,
                navigation,
                body,
            );
    }

    #[cfg(test)]
    pub(crate) fn register_pending_subresource_fetch_request(
        &mut self,
        request_id: String,
        pending: PendingSubresourceFetchRequest,
    ) {
        self.active_target
            .fetch_owner
            .register_pending_subresource_fetch_request(request_id, pending);
    }

    #[cfg(test)]
    pub(crate) fn register_in_flight_subresource_fetch_request(
        &mut self,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
    ) {
        self.active_target
            .fetch_owner
            .register_in_flight_subresource_fetch_request(request_id, pending);
    }

    #[cfg(test)]
    pub(crate) fn allocate_subresource_network_request_id(&mut self) -> String {
        self.active_target
            .runtime_slot
            .request_id_allocator()
            .allocate_network_request_id()
    }

    #[cfg(test)]
    pub(crate) fn record_captured_response_body(
        &mut self,
        request_id: String,
        response_body: String,
        session_ids: impl IntoIterator<Item = Option<String>>,
    ) {
        self.active_target
            .runtime_slot
            .record_captured_response_body(request_id, response_body, session_ids);
    }

    #[cfg(test)]
    pub(crate) fn record_captured_response_body_source(
        &mut self,
        request_id: String,
        response_body: crate::conn::CapturedBody,
        session_ids: impl IntoIterator<Item = Option<String>>,
    ) {
        self.active_target
            .runtime_slot
            .record_captured_response_body_source(request_id, response_body, session_ids);
    }

    #[cfg(test)]
    pub(crate) fn record_pending_response_body(
        &mut self,
        request_id: String,
        session_ids: impl IntoIterator<Item = Option<String>>,
    ) {
        self.active_target
            .runtime_slot
            .record_pending_response_body(request_id, session_ids);
    }

    #[cfg(test)]
    pub(crate) fn captured_response_body(
        &self,
        request_id: &str,
    ) -> Option<&crate::domains::network::CapturedResponseBody> {
        self.active_target
            .runtime_slot
            .captured_response_body(request_id)
    }

    pub(crate) fn clear_captured_response_bodies(&mut self) {
        self.active_target
            .runtime_slot
            .clear_captured_response_bodies();
    }

    pub(crate) fn clear_network_body_artifacts(&mut self) {
        self.active_target
            .runtime_slot
            .clear_network_body_artifacts();
    }

    pub(crate) fn remove_captured_response_body_visibility_for_session(
        &mut self,
        session_id: Option<&str>,
    ) {
        self.active_target
            .runtime_slot
            .remove_captured_response_body_visibility_for_session(session_id);
    }

    #[cfg(test)]
    pub(crate) fn allocate_io_stream_handle(&mut self) -> String {
        self.active_target.runtime_slot.allocate_io_stream_handle()
    }

    #[cfg(test)]
    pub(crate) fn insert_io_stream(&mut self, handle: String, bytes: Vec<u8>, offset: usize) {
        self.active_target
            .runtime_slot
            .insert_io_stream(handle, bytes, offset);
    }

    #[cfg(test)]
    pub(crate) fn read_io_stream(
        &mut self,
        handle: &str,
        offset: Option<usize>,
        size: Option<usize>,
    ) -> Option<crate::domains::network::TargetIoStreamRead> {
        self.active_target
            .runtime_slot
            .read_io_stream(handle, offset, size)
    }

    pub(crate) fn reset_subresource_network_cursor(&mut self) {
        self.active_target.runtime_slot.reset_subresource_cursor();
    }

    pub(crate) fn clear_websocket_network_request_ids(&mut self) {
        self.active_target
            .runtime_slot
            .clear_websocket_request_ids();
    }

    pub(crate) fn clear_websocket_network_artifacts(&mut self) {
        self.active_target.runtime_slot.clear_websocket_artifacts();
    }

    pub(crate) fn initialize_network_listener_observation_cursor(
        &mut self,
        session_id: Option<&str>,
    ) {
        self.active_target
            .runtime_slot
            .initialize_network_session_observation_cursor_at_output_tail(session_id);
    }

    pub(crate) fn remove_network_listener_observation_cursor(&mut self, session_id: Option<&str>) {
        self.active_target
            .runtime_slot
            .remove_network_session_observation_cursor(session_id);
    }

    pub(crate) fn clear_session_scoped_network_observation_artifacts(&mut self) {
        self.active_target
            .runtime_slot
            .clear_session_scoped_network_observation_artifacts();
    }

    pub(crate) fn reset_target_scoped_network_artifacts(&mut self) {
        self.active_target
            .runtime_slot
            .reset_all_target_scoped_network_artifacts();
    }

    #[cfg(test)]
    pub(crate) fn has_captured_response_body_for_test(&self, request_id: &str) -> bool {
        self.active_target
            .runtime_slot
            .has_captured_response_body(request_id)
    }

    #[cfg(test)]
    pub(crate) fn captured_response_bodies_empty_for_test(&self) -> bool {
        self.active_target
            .runtime_slot
            .captured_response_bodies_empty()
    }

    #[cfg(test)]
    pub(crate) fn set_next_network_request_sequence_for_test(&mut self, sequence: u64) {
        self.active_target
            .runtime_slot
            .set_next_network_request_sequence_for_test(sequence);
    }

    #[cfg(test)]
    pub(crate) fn next_network_request_sequence_for_test(&self) -> u64 {
        self.active_target
            .runtime_slot
            .next_network_request_sequence_for_test()
    }

    #[cfg(test)]
    pub(crate) fn io_streams_empty_for_test(&self) -> bool {
        self.active_target.runtime_slot.io_streams_empty_for_test()
    }

    #[cfg(test)]
    pub(crate) fn set_next_io_stream_sequence_for_test(&mut self, sequence: u64) {
        self.active_target
            .runtime_slot
            .set_next_io_stream_sequence_for_test(sequence);
    }

    #[cfg(test)]
    pub(crate) fn next_io_stream_sequence_for_test(&self) -> u64 {
        self.active_target
            .runtime_slot
            .next_io_stream_sequence_for_test()
    }

    #[cfg(test)]
    pub(crate) fn set_subresource_network_emitted_record_count_for_test(&mut self, count: usize) {
        self.active_target
            .runtime_slot
            .set_subresource_emitted_record_count_for_test(count);
    }

    #[cfg(test)]
    pub(crate) fn subresource_network_emitted_record_count_for_test(&self) -> usize {
        self.active_target
            .runtime_slot
            .subresource_emitted_record_count_for_test()
    }

    pub(crate) fn prepare_document_navigation_request_ids(
        &mut self,
        network_request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
        clear_captured_response_bodies: bool,
        observes_document_request: bool,
        needs_fetch_navigation_request_id: bool,
    ) -> (String, Option<String>, Option<String>) {
        if clear_captured_response_bodies {
            self.clear_captured_response_bodies();
        }
        let mut allocator = self.active_target.runtime_slot.request_id_allocator();
        let document_loader_id =
            document_navigation_loader_id(network_request_id_allocator.allocate_sequence());
        let document_request_id = observes_document_request.then(|| document_loader_id.clone());
        let fetch_navigation_request_id = needs_fetch_navigation_request_id
            .then(|| allocator.allocate_fetch_navigation_request_id());
        (
            document_loader_id,
            document_request_id,
            fetch_navigation_request_id,
        )
    }
}

impl BackgroundTarget {
    pub(crate) fn prepare_document_navigation_request_ids(
        &mut self,
        network_request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
        clear_captured_response_bodies: bool,
        observes_document_request: bool,
        needs_fetch_navigation_request_id: bool,
    ) -> (String, Option<String>, Option<String>) {
        if clear_captured_response_bodies {
            self.runtime_slot.clear_captured_response_bodies();
        }
        let mut allocator = self.runtime_slot.request_id_allocator();
        let document_loader_id =
            document_navigation_loader_id(network_request_id_allocator.allocate_sequence());
        let document_request_id = observes_document_request.then(|| document_loader_id.clone());
        let fetch_navigation_request_id = needs_fetch_navigation_request_id
            .then(|| allocator.allocate_fetch_navigation_request_id());
        (
            document_loader_id,
            document_request_id,
            fetch_navigation_request_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{NavigationResultProjection, PendingSubresourceFetchOwnerKind};
    use moli_core::page::SubresourceResourceType;
    use moli_fetch::{RawResponse, ResponseHead};
    use serde_json::json;
    use url::Url;

    fn pending_subresource_fetch(internal_id: u64) -> PendingSubresourceFetchRequest {
        PendingSubresourceFetchRequest {
            residence: crate::conn::PendingSubresourceFetchResidence::InstalledPage(
                crate::conn::TargetPageResidenceIdentity::new_for_test(
                    "BID-fetch-state".to_owned(),
                    Some("TID-1".to_owned()),
                    1,
                ),
            ),
            owner_session_id: None,
            action_session_id: None,
            owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
            internal_id,
            network_request_id: format!("REQ-{internal_id}"),
            network_request_handle: None,
            frame_id: "TID-1".to_owned(),
            document_url: Url::parse("https://example.test/page").unwrap(),
            resource_type: SubresourceResourceType::Fetch,
            websocket_socket_id: None,
            request_stage_chain: None,
        }
    }

    fn navigation_state(url: &Url) -> NavigationDispatchState {
        NavigationDispatchState {
            navigate_id: Some(1),
            navigate_session_id: Some("SID-1".to_owned()),
            result_projection: NavigationResultProjection::Cdp(
                json!({"frameId": "TID-1", "loaderId": "LID-0000000001"}),
            ),
            frame_id: "TID-1".to_owned(),
            session_id: Some("SID-1".to_owned()),
            request_id: Some("REQ-1".to_owned()),
            loader_id: "LID-0000000001".to_owned(),
            request_announced: false,
            requested_url: url.clone(),
            request_method: "GET".to_owned(),
            request_body: None,
            request_body_bytes: None,
            request_headers: Vec::new(),
            request_load_policy: crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
            timestamp: 0.0,
            source_document_security: Default::default(),
        }
    }

    fn buffered_body_source(url: &Url, body: &[u8]) -> DocumentBodySource {
        DocumentBodySource::BufferedRaw {
            requested_url: url.clone(),
            request_method: "GET".to_owned(),
            request_headers: Vec::new(),
            response: RawResponse::from_head_and_body(
                ResponseHead {
                    final_url: url.clone(),
                    status: 200,
                    headers: vec![("content-type".to_owned(), "text/html".to_owned())],
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                },
                body.to_vec(),
            ),
            network_observation_journal: Default::default(),
        }
    }

    #[test]
    fn active_target_pending_fetch_take_clears_in_flight_subresources() {
        let mut bc = BrowserContext::new("BID-1".to_owned());
        bc.register_in_flight_subresource_fetch_request(
            Some("INT-SUB-1".to_owned()),
            pending_subresource_fetch(1),
        );

        assert!(
            bc.active_target
                .fetch_owner
                .has_in_flight_subresource_fetches_for_test()
        );
        let _ = bc.take_active_target_pending_fetch_state();
        assert!(
            !bc.active_target
                .fetch_owner
                .has_in_flight_subresource_fetches_for_test()
        );
    }

    #[test]
    fn subresource_fetch_pending_lifecycle_uses_owner_bookkeeping() {
        let mut bc = BrowserContext::new("BID-1".to_owned());
        bc.register_pending_subresource_fetch_request(
            "INT-SUB-9".to_owned(),
            pending_subresource_fetch(9),
        );

        assert!(
            bc.active_target
                .fetch_owner
                .has_pending_subresource_fetch_for_test("INT-SUB-9")
        );
        assert!(
            bc.active_target
                .fetch_owner
                .has_pending_fetch_request_id_for_test("INT-SUB-9")
        );

        let pending = bc
            .take_pending_subresource_fetch_request_for_test("INT-SUB-9")
            .expect("pending subresource fetch should be found by its protocol request id");
        assert_eq!(pending.network_request_id, "REQ-9");
        assert!(
            !bc.active_target
                .fetch_owner
                .has_pending_subresource_fetch_for_test("INT-SUB-9")
        );
        assert!(
            !bc.active_target
                .fetch_owner
                .has_pending_fetch_request_id_for_test("INT-SUB-9")
        );
    }

    #[test]
    fn fetch_response_body_stream_workflow_buffers_reusable_response_body() {
        let mut bc = BrowserContext::new("BID-1".to_owned());
        let url = Url::parse("https://example.test/page").unwrap();
        bc.register_pending_fetch_response_navigation(
            "INT-1".to_owned(),
            None,
            navigation_state(&url),
            buffered_body_source(&url, b"buffered response"),
        );

        let stream = bc
            .open_pending_fetch_response_body_stream("INT-1")
            .expect("opening buffered response body stream should not fail")
            .expect("buffered response body should produce an IO stream handle");

        assert_eq!(stream, "STREAM-1");
        assert!(
            bc.active_target
                .fetch_owner
                .pending_fetch_response_transfer_is_pending_for_test("INT-1"),
            "buffered body stream reads from IO artifacts and keeps the paused response reusable"
        );
        assert!(
            bc.active_target
                .fetch_owner
                .active_fetch_response_body_stream_request_id_for_test(&stream)
                .is_none()
        );

        let read = bc
            .read_io_stream(&stream, None, None)
            .expect("buffered body bytes should be registered as a target-local IO stream");
        assert_eq!(read.bytes, b"buffered response");
        assert!(read.eof);
    }

    #[tokio::test]
    async fn clearing_session_scoped_state_clears_in_flight_subresources() {
        let mut bc = BrowserContext::new("BID-1".to_owned());
        bc.register_in_flight_subresource_fetch_request(
            Some("INT-SUB-2".to_owned()),
            pending_subresource_fetch(2),
        );

        bc.clear_active_target_session_scoped_state_async()
            .await
            .unwrap();
        assert!(
            !bc.active_target
                .fetch_owner
                .has_in_flight_subresource_fetches_for_test()
        );
    }

    #[test]
    fn network_and_io_stream_ids_cross_u32_max_without_reuse() {
        let mut bc = BrowserContext::new("BID-1".to_owned());

        bc.set_next_network_request_sequence_for_test(u32::MAX as u64);
        assert_eq!(
            bc.allocate_subresource_network_request_id(),
            "REQ-4294967296"
        );
        assert_eq!(bc.next_network_request_sequence_for_test(), 4_294_967_296);

        bc.set_next_io_stream_sequence_for_test(u32::MAX as u64);
        assert_eq!(bc.allocate_io_stream_handle(), "STREAM-4294967296");
        assert_eq!(bc.next_io_stream_sequence_for_test(), 4_294_967_296);
    }

    #[test]
    fn network_and_io_stream_id_allocators_fail_at_u64_exhaustion() {
        let mut bc = BrowserContext::new("BID-1".to_owned());
        bc.set_next_network_request_sequence_for_test(u64::MAX);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = bc.allocate_subresource_network_request_id();
            }))
            .is_err(),
            "network request ids must not silently wrap after u64::MAX"
        );

        let mut bc = BrowserContext::new("BID-2".to_owned());
        bc.set_next_io_stream_sequence_for_test(u64::MAX);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = bc.allocate_io_stream_handle();
            }))
            .is_err(),
            "IO stream ids must not silently wrap after u64::MAX"
        );
    }
}
