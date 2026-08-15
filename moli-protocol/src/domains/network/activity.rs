use crate::conn::CdpConnection;
use crate::domains::activity::{
    PreparedSubresourceContinueAction, ProtocolOutputPayloads, ProtocolOutputProjectionContext,
    ProtocolOutputSink, ProtocolOutputSlot,
    flush_prepared_subresource_continue_actions_background_events_async,
    prepare_subresource_continue_action_for_renderer_record,
};
use crate::domains::fetch;

use super::TargetSubresourceFetchPauseOutput;
use super::output_queue::TargetNetworkBacklogPreparedDelivery;
use super::{
    NetworkBacklogProjectionContext, emit_pending_network_backlog_activity_background_events,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NetworkActivityOutput {
    PendingSubresourceContinueEvents,
    RendererNetworkLive,
    NetworkBacklog,
    SubresourceFetchInterception,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NetworkOutputProjectionStep {
    PendingSubresourceContinueEvents,
    RendererNetworkLive,
    NetworkBacklog,
    SubresourceFetchInterception,
}

#[derive(Debug, Default)]
pub(crate) struct NetworkPreparedOutputs {
    pending_subresource_continue_actions: Vec<PreparedSubresourceContinueAction>,
    renderer_live: TargetNetworkBacklogPreparedDelivery,
    backlog: TargetNetworkBacklogPreparedDelivery,
    subresource_fetch_pauses: Vec<TargetSubresourceFetchPauseOutput>,
}

#[derive(Debug, Default)]
pub(crate) struct NetworkPreparedOutputSlot {
    outputs: NetworkPreparedOutputs,
}

impl NetworkPreparedOutputs {
    pub(crate) fn from_renderer_live_delivery(
        renderer_live: TargetNetworkBacklogPreparedDelivery,
    ) -> Self {
        Self {
            pending_subresource_continue_actions: Vec::new(),
            renderer_live,
            backlog: TargetNetworkBacklogPreparedDelivery::default(),
            subresource_fetch_pauses: Vec::new(),
        }
    }

    pub(crate) fn backlog_projection_context<'a>(
        session_id: Option<&'a str>,
        frame_id: Option<&'a str>,
        base_timestamp: Option<f64>,
        contextual_subresource_request_id: Option<&'a str>,
        prepared_outputs: Option<&'a mut Self>,
    ) -> NetworkBacklogProjectionContext<'a> {
        NetworkBacklogProjectionContext::new(session_id)
            .with_frame_id(frame_id)
            .with_base_timestamp(base_timestamp)
            .with_contextual_subresource_request_id(contextual_subresource_request_id)
            .with_prepared_outputs(prepared_outputs)
    }

    pub(crate) fn from_subresource_fetch_pauses(
        subresource_fetch_pauses: Vec<TargetSubresourceFetchPauseOutput>,
    ) -> Self {
        Self {
            pending_subresource_continue_actions: Vec::new(),
            renderer_live: TargetNetworkBacklogPreparedDelivery::default(),
            backlog: TargetNetworkBacklogPreparedDelivery::default(),
            subresource_fetch_pauses,
        }
    }

    pub(crate) fn from_renderer_subresource_continue(
        conn: &mut CdpConnection,
        session_id: Option<&str>,
        source_document: moli_core::RendererDocumentLifecycleIdentity,
        event: moli_core::page::PendingSubresourceContinueEvent,
    ) -> Self {
        Self {
            pending_subresource_continue_actions:
                prepare_subresource_continue_action_for_renderer_record(
                    conn,
                    session_id,
                    source_document,
                    event,
                )
                .into_iter()
                .collect(),
            renderer_live: TargetNetworkBacklogPreparedDelivery::default(),
            backlog: TargetNetworkBacklogPreparedDelivery::default(),
            subresource_fetch_pauses: Vec::new(),
        }
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.pending_subresource_continue_actions
            .extend(other.pending_subresource_continue_actions);
        self.renderer_live.extend(other.renderer_live);
        self.backlog.extend(other.backlog);
        self.subresource_fetch_pauses
            .extend(other.subresource_fetch_pauses);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pending_subresource_continue_actions.is_empty()
            && !self.renderer_live.has_delivery_output()
            && !self.backlog.has_delivery_output()
            && self.subresource_fetch_pauses.is_empty()
    }

    pub(crate) fn outputs(&self) -> Vec<NetworkActivityOutput> {
        let mut outputs = Vec::new();
        if !self.pending_subresource_continue_actions.is_empty() {
            outputs.push(NetworkActivityOutput::PendingSubresourceContinueEvents);
        }
        if self.renderer_live.has_delivery_output() {
            outputs.push(NetworkActivityOutput::RendererNetworkLive);
        }
        if self.backlog.has_delivery_output() {
            outputs.push(NetworkActivityOutput::NetworkBacklog);
        }
        if !self.subresource_fetch_pauses.is_empty() {
            outputs.push(NetworkActivityOutput::SubresourceFetchInterception);
        }
        outputs
    }

    pub(in crate::domains) fn append_to_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if !self.is_empty() {
            for output in self.outputs() {
                match output {
                    NetworkActivityOutput::PendingSubresourceContinueEvents => {
                        sink.push_produced_slot(SLOT_PENDING_SUBRESOURCE_CONTINUE);
                    }
                    NetworkActivityOutput::RendererNetworkLive => {
                        sink.push_produced_slot(SLOT_RENDERER_NETWORK_LIVE);
                    }
                    NetworkActivityOutput::NetworkBacklog => {
                        sink.push_produced_slot(SLOT_NETWORK_BACKLOG);
                    }
                    NetworkActivityOutput::SubresourceFetchInterception => {
                        sink.push_produced_slot(SLOT_SUBRESOURCE_FETCH_INTERCEPTION);
                    }
                }
            }
            sink.push_prepared_payload(NetworkPreparedOutputSlot::from_outputs(self).into());
        }
    }

    pub(crate) fn take_pending_subresource_continue_actions(
        &mut self,
    ) -> Option<Vec<PreparedSubresourceContinueAction>> {
        (!self.pending_subresource_continue_actions.is_empty())
            .then(|| std::mem::take(&mut self.pending_subresource_continue_actions))
    }

    pub(crate) fn take_subresource_fetch_pauses(
        &mut self,
    ) -> Option<Vec<TargetSubresourceFetchPauseOutput>> {
        (!self.subresource_fetch_pauses.is_empty())
            .then(|| std::mem::take(&mut self.subresource_fetch_pauses))
    }

    #[cfg(test)]
    pub(crate) fn from_prepared_subresource_continue_actions_for_test(
        actions: Vec<PreparedSubresourceContinueAction>,
    ) -> Self {
        Self {
            pending_subresource_continue_actions: actions,
            renderer_live: TargetNetworkBacklogPreparedDelivery::default(),
            backlog: TargetNetworkBacklogPreparedDelivery::default(),
            subresource_fetch_pauses: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_subresource_fetch_interception_for_test(
        page_owner: crate::conn::TargetPageResidenceIdentity,
    ) -> Self {
        use crate::conn::{PendingSubresourceFetchOwnerKind, PendingSubresourceFetchRequest};
        use moli_core::page::{PendingSubresourceFetchInfo, SubresourceResourceType};
        use serde_json::json;
        use url::Url;

        let info = PendingSubresourceFetchInfo {
            internal_id: 7,
            network_request_handle: None,
            frame_id: Some("FRAME-1".to_owned()),
            document_url: Url::parse("https://example.test/page").unwrap(),
            url: Url::parse("https://example.test/api").unwrap(),
            websocket_socket_id: None,
            method: "GET".to_owned(),
            request_headers: Vec::new(),
            request_body: None,
            request_body_bytes: None,
            resource_type: SubresourceResourceType::Fetch,
            request_cookie_report: None,
        };
        let pending = PendingSubresourceFetchRequest {
            residence: crate::conn::PendingSubresourceFetchResidence::InstalledPage(page_owner),
            owner_session_id: None,
            action_session_id: None,
            owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
            internal_id: info.internal_id,
            network_request_id: "REQ-1".to_owned(),
            network_request_handle: None,
            frame_id: "FRAME-1".to_owned(),
            document_url: info.document_url.clone(),
            resource_type: info.resource_type,
            websocket_socket_id: None,
            request_stage_chain: None,
        };
        let network_output =
            super::TargetSubresourceFetchPauseNetworkOutput::from_pending_fetch_info(
                "REQ-1".to_owned(),
                "FRAME-1".to_owned(),
                "LOADER-1".to_owned(),
                12.0,
                info.document_url.clone(),
                &info,
            );
        let payload = json!({
            "requestId": "FETCH-1",
            "frameId": "FRAME-1",
            "request": {
                "url": "https://example.test/api",
                "method": "GET",
                "headers": {},
                "hasPostData": false,
            },
            "resourceType": "Fetch",
            "networkId": "REQ-1",
        });
        Self::from_subresource_fetch_pauses(vec![TargetSubresourceFetchPauseOutput::new(
            network_output,
            Some("FETCH-SID".to_owned()),
            "FETCH-1".to_owned(),
            pending,
            payload,
        )])
    }

    pub(in crate::domains::network) fn backlog_mut(
        &mut self,
    ) -> &mut TargetNetworkBacklogPreparedDelivery {
        &mut self.backlog
    }

    pub(in crate::domains::network) fn renderer_live_mut(
        &mut self,
    ) -> &mut TargetNetworkBacklogPreparedDelivery {
        &mut self.renderer_live
    }
}

impl NetworkPreparedOutputSlot {
    pub(crate) fn from_outputs(outputs: NetworkPreparedOutputs) -> Self {
        Self { outputs }
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.outputs.extend(other.outputs);
    }

    pub(crate) fn take_pending_subresource_continue_actions(
        &mut self,
    ) -> Option<Vec<PreparedSubresourceContinueAction>> {
        self.outputs.take_pending_subresource_continue_actions()
    }

    pub(crate) fn take_subresource_fetch_pauses(
        &mut self,
    ) -> Option<Vec<TargetSubresourceFetchPauseOutput>> {
        self.outputs.take_subresource_fetch_pauses()
    }

    pub(crate) fn backlog_projection_context<'a>(
        &'a mut self,
        session_id: Option<&'a str>,
        frame_id: Option<&'a str>,
        base_timestamp: Option<f64>,
        contextual_subresource_request_id: Option<&'a str>,
    ) -> NetworkBacklogProjectionContext<'a> {
        NetworkPreparedOutputs::backlog_projection_context(
            session_id,
            frame_id,
            base_timestamp,
            contextual_subresource_request_id,
            Some(&mut self.outputs),
        )
    }

    pub(crate) async fn emit_pending_subresource_continue_background_events(
        &mut self,
        conn: &mut CdpConnection,
        out: &mut Vec<crate::conn::BackgroundProtocolEvent>,
        session_id: Option<&str>,
    ) {
        if let Some(actions) = self.take_pending_subresource_continue_actions() {
            flush_prepared_subresource_continue_actions_background_events_async(
                conn, out, session_id, actions,
            )
            .await;
        }
    }

    pub(crate) fn emit_backlog_activity_background_events(
        &mut self,
        conn: &mut CdpConnection,
        out: &mut Vec<crate::conn::BackgroundProtocolEvent>,
        session_id: Option<&str>,
        frame_id: Option<&str>,
        base_timestamp: Option<f64>,
        contextual_subresource_request_id: Option<&str>,
    ) {
        emit_pending_network_backlog_activity_background_events(
            conn,
            out,
            self.backlog_projection_context(
                session_id,
                frame_id,
                base_timestamp,
                contextual_subresource_request_id,
            ),
        );
    }

    pub(crate) fn emit_renderer_live_background_events(
        &mut self,
        conn: &mut CdpConnection,
        out: &mut Vec<crate::conn::BackgroundProtocolEvent>,
        session_id: Option<&str>,
    ) {
        super::emit_prepared_renderer_network_live_background_events(
            conn,
            out,
            session_id,
            self.outputs.renderer_live_mut(),
        );
    }
}

pub(in crate::domains) const SLOT_PENDING_SUBRESOURCE_CONTINUE: ProtocolOutputSlot =
    ProtocolOutputSlot::PendingSubresourceContinueEvents;
pub(in crate::domains) const SLOT_RENDERER_NETWORK_LIVE: ProtocolOutputSlot =
    ProtocolOutputSlot::RendererNetworkLive;
pub(in crate::domains) const SLOT_NETWORK_BACKLOG: ProtocolOutputSlot =
    ProtocolOutputSlot::NetworkBacklog;
pub(in crate::domains) const SLOT_SUBRESOURCE_FETCH_INTERCEPTION: ProtocolOutputSlot =
    ProtocolOutputSlot::SubresourceFetchInterception;

impl NetworkOutputProjectionStep {
    async fn project_async(
        self,
        conn: &mut CdpConnection,
        context: &mut ProtocolOutputProjectionContext<'_>,
        prepared_outputs: Option<&mut ProtocolOutputPayloads>,
    ) {
        match self {
            NetworkOutputProjectionStep::PendingSubresourceContinueEvents => {
                if let Some(slot) = prepared_outputs.and_then(ProtocolOutputPayloads::network_mut) {
                    slot.emit_pending_subresource_continue_background_events(
                        conn,
                        context.command.protocol_events_mut(),
                        context.session_id,
                    )
                    .await;
                }
            }
            NetworkOutputProjectionStep::RendererNetworkLive => {
                if let Some(slot) = prepared_outputs.and_then(ProtocolOutputPayloads::network_mut) {
                    slot.emit_renderer_live_background_events(
                        conn,
                        context.command.protocol_events_mut(),
                        context.session_id,
                    );
                }
            }
            NetworkOutputProjectionStep::NetworkBacklog => {
                if let Some(slot) = prepared_outputs.and_then(ProtocolOutputPayloads::network_mut) {
                    slot.emit_backlog_activity_background_events(
                        conn,
                        context.command.protocol_events_mut(),
                        context.session_id,
                        context.subresource_frame_id,
                        context.subresource_timestamp,
                        context.subresource_network_request_id,
                    );
                }
            }
            NetworkOutputProjectionStep::SubresourceFetchInterception => {
                if let Some(pauses) = prepared_outputs
                    .and_then(ProtocolOutputPayloads::network_mut)
                    .and_then(NetworkPreparedOutputSlot::take_subresource_fetch_pauses)
                {
                    let mut events = Vec::new();
                    let network_session_ids =
                        conn.network_event_session_ids_for_session_owner(context.session_id);
                    fetch::emit_subresource_fetch_pause_outputs(
                        conn,
                        &mut events,
                        context.session_id,
                        &network_session_ids,
                        pauses,
                    );
                    context.command.protocol_events_mut().extend(events);
                }
            }
        }
    }
}

pub(in crate::domains) async fn project_pending_subresource_continue_async(
    conn: &mut CdpConnection,
    context: &mut ProtocolOutputProjectionContext<'_>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    NetworkOutputProjectionStep::PendingSubresourceContinueEvents
        .project_async(conn, context, prepared_outputs)
        .await;
}

pub(in crate::domains) async fn project_network_backlog_async(
    conn: &mut CdpConnection,
    context: &mut ProtocolOutputProjectionContext<'_>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    NetworkOutputProjectionStep::NetworkBacklog
        .project_async(conn, context, prepared_outputs)
        .await;
}

pub(in crate::domains) async fn project_renderer_network_live_async(
    conn: &mut CdpConnection,
    context: &mut ProtocolOutputProjectionContext<'_>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    NetworkOutputProjectionStep::RendererNetworkLive
        .project_async(conn, context, prepared_outputs)
        .await;
}

pub(in crate::domains) async fn project_subresource_fetch_interception_async(
    conn: &mut CdpConnection,
    context: &mut ProtocolOutputProjectionContext<'_>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    NetworkOutputProjectionStep::SubresourceFetchInterception
        .project_async(conn, context, prepared_outputs)
        .await;
}

pub(in crate::domains) fn network_backlog_prepared_outputs(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    preferred_request_id: Option<super::NetworkBacklogPreferredRequestId<'_>>,
) -> NetworkPreparedOutputs {
    let mut outputs = NetworkPreparedOutputs::default();
    let primary_session_id = conn.runtime_session_owner_primary_session_id(session_id);
    if let Some(backlog) = conn.network_backlog_prepared_delivery_for_session_owner(
        session_id,
        session_id,
        primary_session_id.as_deref(),
        preferred_request_id,
    ) {
        outputs.extend(NetworkPreparedOutputs {
            pending_subresource_continue_actions: Vec::new(),
            renderer_live: TargetNetworkBacklogPreparedDelivery::default(),
            backlog,
            subresource_fetch_pauses: Vec::new(),
        });
    }
    outputs
}

#[cfg(test)]
mod tests {
    use crate::conn::{CommandDispatchContext, PendingSubresourceFetchOwnerKind};
    use axum::{
        Router,
        extract::ws::{Message, WebSocketUpgrade},
        response::IntoResponse,
        routing::get,
    };
    use moli_core::page::{
        PendingSubresourceContinueEvent, PendingSubresourceResponseInfo,
        SubresourceNetworkRequestHandle, SubresourceResourceType, SubresourceResponseBody,
    };
    use serde_json::json;
    use tokio::net::TcpListener;
    use url::Url;

    use super::NetworkPreparedOutputs;
    use crate::{
        conn::{BackgroundTarget, BrowserContext, CdpConnection, PendingSubresourceFetchRequest},
        domains::activity::{ProtocolOutputPayloads, ProtocolOutputProjectionContext},
        testing::{TestContext, wait_until_message, wait_until_messages},
    };

    fn pending_request_for_page(
        page_owner: crate::conn::TargetPageResidenceIdentity,
        internal_id: u64,
        network_request_id: &str,
        request_handle: SubresourceNetworkRequestHandle,
    ) -> PendingSubresourceFetchRequest {
        PendingSubresourceFetchRequest {
            residence: crate::conn::PendingSubresourceFetchResidence::InstalledPage(page_owner),
            owner_session_id: None,
            action_session_id: None,
            owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
            internal_id,
            network_request_id: network_request_id.to_owned(),
            network_request_handle: Some(request_handle),
            frame_id: "FRAME-1".to_owned(),
            document_url: Url::parse("https://example.test/page").unwrap(),
            resource_type: SubresourceResourceType::Fetch,
            websocket_socket_id: None,
            request_stage_chain: None,
        }
    }

    async fn websocket_echo_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(|mut socket| async move {
            while let Some(Ok(message)) = socket.recv().await {
                match message {
                    Message::Text(text) => {
                        let _ = socket.send(Message::Text(text)).await;
                    }
                    Message::Binary(bytes) => {
                        let _ = socket.send(Message::Binary(bytes)).await;
                    }
                    Message::Close(frame) => {
                        let _ = socket.send(Message::Close(frame)).await;
                        break;
                    }
                    Message::Ping(bytes) => {
                        let _ = socket.send(Message::Pong(bytes)).await;
                    }
                    Message::Pong(_) => {}
                }
            }
        })
    }

    #[test]
    fn network_prepared_outputs_build_backlog_projection_context() {
        let mut prepared = NetworkPreparedOutputs::default();
        let context = NetworkPreparedOutputs::backlog_projection_context(
            Some("SID-1"),
            Some("FRAME-1"),
            Some(11.0),
            Some("REQ-1"),
            Some(&mut prepared),
        );

        assert_eq!(context.session_id, Some("SID-1"));
        assert_eq!(context.frame_id, Some("FRAME-1"));
        assert_eq!(context.base_timestamp, Some(11.0));
        assert!(
            context.prepared_outputs.is_some(),
            "Network captured outputs should own Network backlog projection-context construction"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pending_subresource_continue_drain_consumes_prepared_events_without_page_readback() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
        conn.browser_context = Some(bc);
        let page_owner = conn
            .target_page_residence_identity_for_session(Some("SID-1"))
            .expect("test target should expose a Page residence identity");
        let pending = PendingSubresourceFetchRequest {
            residence: crate::conn::PendingSubresourceFetchResidence::InstalledPage(page_owner),
            owner_session_id: None,
            action_session_id: None,
            owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
            internal_id: 7,
            network_request_id: "network-req-1".to_owned(),
            network_request_handle: None,
            frame_id: "FRAME-1".to_owned(),
            document_url: Url::parse("https://example.test/page").unwrap(),
            resource_type: SubresourceResourceType::Fetch,
            websocket_socket_id: None,
            request_stage_chain: None,
        };
        assert!(
            conn.register_in_flight_subresource_fetch_request_for_session_owner(
                Some("SID-1"),
                Some("fetch-req-1".to_owned()),
                pending,
            ),
            "test setup should register owner-local in-flight subresource fetch"
        );
        let event =
            PendingSubresourceContinueEvent::ResponsePaused(PendingSubresourceResponseInfo {
                internal_id: 7,
                url: Url::parse("https://example.test/api").unwrap(),
                final_url: Url::parse("https://example.test/api").unwrap(),
                method: "GET".to_owned(),
                request_headers: Vec::new(),
                request_body: None,
                resource_type: SubresourceResourceType::Fetch,
                request_cookie_report: None,
                network_request_headers: None,
                response_status: 200,
                response_headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                response_body: SubresourceResponseBody::from_text("prepared".to_owned()),
                from_cache: false,
            });
        let action = crate::domains::activity::PreparedSubresourceContinueAction::capture_for_test(
            &mut conn,
            Some("SID-1"),
            event,
        )
        .expect("test target should capture the prepared continue action");
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::NetworkPreparedOutputSlot::from_outputs(
                NetworkPreparedOutputs::from_prepared_subresource_continue_actions_for_test(vec![
                    action,
                ]),
            ));
        let mut command_context = CommandDispatchContext::default();
        let mut context = ProtocolOutputProjectionContext {
            session_id: Some("SID-1"),
            command: &mut command_context,
            subresource_frame_id: None,
            subresource_document_url: None,
            subresource_timestamp: None,
            subresource_network_request_id: None,
        };

        super::NetworkOutputProjectionStep::PendingSubresourceContinueEvents
            .project_async(&mut conn, &mut context, Some(&mut prepared))
            .await;

        assert!(
            conn.runtime_session_owner_slot(Some("SID-1"))
                .expect("runtime owner slot should exist")
                .loaded_page()
                .is_none(),
            "prepared pending subresource continue emission must not require a loaded page"
        );
        let out = context
            .command
            .take_protocol_events()
            .into_iter()
            .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["method"], json!("Fetch.requestPaused"));
        assert_eq!(out[0]["sessionId"], json!("SID-1"));
        assert_eq!(out[0]["params"]["requestId"], json!("fetch-req-1"));
        assert_eq!(out[0]["params"]["networkId"], json!("network-req-1"));
        assert_eq!(out[0]["params"]["frameId"], json!("FRAME-1"));
        assert_eq!(out[0]["params"]["responseStatusCode"], json!(200));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepared_subresource_continue_rejects_replacement_id_and_handle_collision() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-collision".into());
        bc.set_active_target_id("TID-collision");
        bc.attach_active_session("SID-collision");
        conn.browser_context = Some(bc);

        let old_owner = conn
            .target_page_residence_identity_for_session(Some("SID-collision"))
            .expect("old Page residence should exist");
        let reused_handle = SubresourceNetworkRequestHandle::new(9);
        assert!(
            conn.register_in_flight_subresource_fetch_request_for_session_owner(
                Some("SID-collision"),
                Some("FETCH-old".to_owned()),
                pending_request_for_page(old_owner, 7, "NETWORK-old", reused_handle),
            )
        );
        let event =
            PendingSubresourceContinueEvent::ResponsePaused(PendingSubresourceResponseInfo {
                internal_id: 7,
                url: Url::parse("https://example.test/old").unwrap(),
                final_url: Url::parse("https://example.test/old").unwrap(),
                method: "GET".to_owned(),
                request_headers: Vec::new(),
                request_body: None,
                resource_type: SubresourceResourceType::Fetch,
                request_cookie_report: None,
                network_request_headers: None,
                response_status: 200,
                response_headers: Vec::new(),
                response_body: SubresourceResponseBody::from_text("old".to_owned()),
                from_cache: false,
            });
        let old_action =
            crate::domains::activity::PreparedSubresourceContinueAction::capture_for_test(
                &mut conn,
                Some("SID-collision"),
                event,
            )
            .expect("old continuation should capture its exact request state");

        conn.runtime_session_owner_slot_mut(Some("SID-collision"))
            .expect("runtime owner should remain addressable")
            .replace_page_attachment_id_for_test();
        let replacement_owner = conn
            .target_page_residence_identity_for_session(Some("SID-collision"))
            .expect("replacement Page residence should exist");
        assert!(
            conn.register_in_flight_subresource_fetch_request_for_session_owner(
                Some("SID-collision"),
                Some("FETCH-new".to_owned()),
                pending_request_for_page(replacement_owner, 7, "NETWORK-new", reused_handle),
            )
        );

        let mut out = Vec::new();
        crate::domains::activity::flush_prepared_subresource_continue_actions_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-collision"),
            vec![old_action],
        )
        .await;

        assert!(
            out.is_empty(),
            "stale continuation must not project an event for the replacement request"
        );
        assert_eq!(
            conn.in_flight_subresource_fetch_request_id_for_session_owner(
                Some("SID-collision"),
                7,
            )
            .as_deref(),
            Some("FETCH-new"),
            "stale apply must not claim the replacement request even when both renderer IDs collide"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn subresource_fetch_interception_drain_consumes_prepared_pause_without_page_readback() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
        assert!(bc.assign_auxiliary_session_to_target("TID-1", "FETCH-SID".to_owned()));
        conn.browser_context = Some(bc);
        assert!(conn.enable_network_listener_for_session_owner(Some("FETCH-SID")));
        let page_owner = conn
            .target_page_residence_identity_for_session(Some("SID-1"))
            .expect("test target should expose a Page residence identity");
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::NetworkPreparedOutputSlot::from_outputs(
                NetworkPreparedOutputs::from_subresource_fetch_interception_for_test(page_owner),
            ));
        let mut command_context = CommandDispatchContext::default();
        let mut context = ProtocolOutputProjectionContext {
            session_id: Some("SID-1"),
            command: &mut command_context,
            subresource_frame_id: None,
            subresource_document_url: None,
            subresource_timestamp: None,
            subresource_network_request_id: None,
        };

        super::NetworkOutputProjectionStep::SubresourceFetchInterception
            .project_async(&mut conn, &mut context, Some(&mut prepared))
            .await;

        assert!(
            conn.runtime_session_owner_slot(Some("SID-1"))
                .expect("runtime owner slot should exist")
                .loaded_page()
                .is_none(),
            "prepared subresource fetch interception emission must not require a loaded page"
        );
        let out = context
            .command
            .take_protocol_events()
            .into_iter()
            .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["method"], json!("Network.requestWillBeSent"));
        assert_eq!(out[0]["sessionId"], json!("FETCH-SID"));
        assert_eq!(out[0]["params"]["requestId"], json!("REQ-1"));
        assert_eq!(out[1]["method"], json!("Fetch.requestPaused"));
        assert_eq!(out[1]["sessionId"], json!("FETCH-SID"));
        assert_eq!(out[1]["params"]["requestId"], json!("FETCH-1"));
        assert_eq!(out[1]["params"]["networkId"], json!("REQ-1"));
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .active_target
                .fetch_owner
                .has_pending_subresource_fetch_for_test("FETCH-1"),
            "prepared emission should still register the paused request on the owner fetch state"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn subresource_fetch_interception_omits_network_events_without_listener() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
        assert!(bc.assign_auxiliary_session_to_target("TID-1", "FETCH-SID".to_owned()));
        conn.browser_context = Some(bc);
        let page_owner = conn
            .target_page_residence_identity_for_session(Some("SID-1"))
            .expect("test target should expose a Page residence identity");
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::NetworkPreparedOutputSlot::from_outputs(
                NetworkPreparedOutputs::from_subresource_fetch_interception_for_test(page_owner),
            ));
        let mut command_context = CommandDispatchContext::default();
        let mut context = ProtocolOutputProjectionContext {
            session_id: Some("SID-1"),
            command: &mut command_context,
            subresource_frame_id: None,
            subresource_document_url: None,
            subresource_timestamp: None,
            subresource_network_request_id: None,
        };

        super::NetworkOutputProjectionStep::SubresourceFetchInterception
            .project_async(&mut conn, &mut context, Some(&mut prepared))
            .await;

        let out = context
            .command
            .take_protocol_events()
            .into_iter()
            .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["method"], json!("Fetch.requestPaused"));
        assert_eq!(out[0]["sessionId"], json!("FETCH-SID"));
    }

    #[test]
    fn network_backlog_prepared_outputs_are_absent_without_loaded_observed_page() {
        let mut conn = crate::conn::CdpConnection::default();
        assert_eq!(
            super::network_backlog_prepared_outputs(&mut conn, None, None).outputs(),
            &[]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concrete_network_request_ids_are_connection_unique_across_target_streams() {
        async fn page() -> impl IntoResponse {
            (
                [("content-type", "text/html")],
                r#"<!doctype html><script src="/tracked.js"></script>"#,
            )
        }

        async fn script() -> impl IntoResponse {
            (
                [("content-type", "application/javascript")],
                "globalThis.__network_backlog_script = true;",
            )
        }

        fn script_request_id(out: &[serde_json::Value], session_id: &str) -> String {
            out.iter()
                .find(|message| {
                    message["sessionId"] == json!(session_id)
                        && message["method"] == json!("Network.requestWillBeSent")
                        && message["params"]["type"] == json!("Script")
                })
                .and_then(|message| message["params"]["requestId"].as_str())
                .expect("concrete script request should carry a request id")
                .to_owned()
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/page", get(page))
                    .route("/tracked.js", get(script)),
            )
            .await
            .unwrap();
        });

        let page_url = format!("http://{addr}/page");
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-active");
        bc.attach_active_session("SID-active");
        bc.background_targets.push(BackgroundTarget::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            page_url.clone(),
        ));
        ctx.conn.browser_context = Some(bc);

        for (id, session_id) in [(1, "SID-active"), (2, "SID-background")] {
            ctx.process_async(serde_json::json!({
                "id": id,
                "method": "Network.enable",
                "sessionId": session_id
            }))
            .await;
            ctx.expect_result(id, serde_json::json!({}), Some(session_id));
        }

        ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-active"))
            .await;
        ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-background"))
            .await;
        wait_until_messages(
            &mut ctx,
            Some("SID-active"),
            "concrete script requests for both target streams",
            |messages| {
                ["SID-active", "SID-background"]
                    .into_iter()
                    .all(|session_id| {
                        messages.iter().any(|message| {
                            message["sessionId"] == json!(session_id)
                                && message["method"] == json!("Network.requestWillBeSent")
                                && message["params"]["type"] == json!("Script")
                        })
                    })
            },
        )
        .await;

        let active_request_id = script_request_id(&ctx.sent, "SID-active");
        let background_request_id = script_request_id(&ctx.sent, "SID-background");

        assert_ne!(
            active_request_id, background_request_id,
            "Network request ids must be allocated at connection scope, not per target slot"
        );
        for session_id in ["SID-active", "SID-background"] {
            assert_eq!(
                ctx.sent
                    .iter()
                    .filter(|message| {
                        message["sessionId"] == json!(session_id)
                            && message["method"] == json!("Network.requestWillBeSent")
                            && message["params"]["type"] == json!("Script")
                    })
                    .count(),
                1,
                "each concrete script request must be projected once per target stream"
            );
        }

        server.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concrete_websocket_events_emit_once_in_handshake_before_frame_order() {
        async fn page() -> impl IntoResponse {
            (
                [("content-type", "text/html")],
                "<!doctype html><body>ws</body>",
            )
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/page", get(page))
                    .route("/socket", get(websocket_echo_handler)),
            )
            .await
            .unwrap();
        });

        let page_url = format!("http://{addr}/page");
        let socket_url = format!("ws://{addr}/socket");
        let socket_literal = serde_json::to_string(&socket_url).unwrap();
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
        ctx.conn.browser_context = Some(bc);
        ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-1"))
            .await;
        ctx.process_async(serde_json::json!({
            "id": 2,
            "method": "Network.enable",
            "sessionId": "SID-1"
        }))
        .await;
        ctx.expect_result(2, serde_json::json!({}), Some("SID-1"));
        ctx.sent.clear();

        ctx.process_async(serde_json::json!({
            "id": 3,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "awaitPromise": true,
                "expression": format!(r#"new Promise(resolve => {{
                    const socket = new WebSocket({socket_literal});
                    let echoed = 'error';
                    socket.addEventListener('open', () => socket.send('hello'));
                    socket.addEventListener('message', event => {{
                        echoed = event.data;
                        socket.close(1000, 'done');
                    }});
                    socket.addEventListener('close', () => resolve(echoed));
                    socket.addEventListener('error', () => resolve('error'));
                }})"#)
            }
        }))
        .await;
        wait_until_message(
            &mut ctx,
            "SID-1",
            "websocket Runtime.evaluate response",
            |message| message["id"] == json!(3),
        )
        .await;
        ctx.expect_result(
            3,
            serde_json::json!({ "result": { "type": "string", "value": "hello" }}),
            Some("SID-1"),
        );
        wait_until_messages(
            &mut ctx,
            Some("SID-1"),
            "complete concrete WebSocket Network sequence",
            |messages| {
                [
                    "Network.webSocketCreated",
                    "Network.webSocketWillSendHandshakeRequest",
                    "Network.webSocketHandshakeResponseReceived",
                    "Network.webSocketFrameSent",
                    "Network.webSocketFrameReceived",
                    "Network.webSocketClosed",
                ]
                .into_iter()
                .all(|method| {
                    messages
                        .iter()
                        .any(|message| message["method"] == json!(method))
                })
            },
        )
        .await;

        let network_messages = ctx
            .sent
            .iter()
            .filter(|message| message["method"].as_str().is_some())
            .collect::<Vec<_>>();
        let methods = network_messages
            .iter()
            .filter_map(|message| message["method"].as_str())
            .collect::<Vec<_>>();
        let created_index = methods
            .iter()
            .position(|method| *method == "Network.webSocketCreated")
            .expect("created event should be emitted");
        let request_index = methods
            .iter()
            .position(|method| *method == "Network.webSocketWillSendHandshakeRequest")
            .expect("handshake request event should be emitted");
        let response_index = methods
            .iter()
            .position(|method| *method == "Network.webSocketHandshakeResponseReceived")
            .expect("handshake response event should be emitted");
        let frame_index = methods
            .iter()
            .position(|method| {
                matches!(
                    *method,
                    "Network.webSocketFrameSent" | "Network.webSocketFrameReceived"
                )
            })
            .expect("frame event should be emitted");

        assert!(
            created_index < request_index
                && request_index < response_index
                && response_index < frame_index,
            "WebSocket Network events must preserve Chromium-style handshake-before-frame order: {methods:?}"
        );

        let request_id = network_messages[created_index]["params"]["requestId"]
            .as_str()
            .expect("created requestId")
            .to_owned();
        for index in [request_index, response_index, frame_index] {
            assert_eq!(
                network_messages[index]["params"]["requestId"], request_id,
                "handshake and frame events should share the WebSocket Network requestId"
            );
        }
        for method in [
            "Network.webSocketCreated",
            "Network.webSocketWillSendHandshakeRequest",
            "Network.webSocketHandshakeResponseReceived",
            "Network.webSocketClosed",
        ] {
            assert_eq!(
                ctx.sent
                    .iter()
                    .filter(|message| {
                        message["method"] == json!(method)
                            && message["params"]["requestId"] == json!(request_id)
                    })
                    .count(),
                1,
                "{method} must be projected exactly once from its concrete record"
            );
        }

        server.abort();
    }
}
