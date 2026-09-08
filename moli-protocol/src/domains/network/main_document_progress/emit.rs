use crate::devtools_runtime::{
    AutomationEvent, DevToolsFrameId, DevToolsLoaderId, DevToolsNetworkResourceType,
    DevToolsRequestId, DevToolsTargetId, NetworkRedirectResponseEvent, NetworkRequestEvent,
};
use moli_cookie_jar::{StoredCookieQueryReport, StoredCookieSetReport};
use moli_core::page::{
    SubresourceRequestInitiatorType, WebSocketFrameDirection, WebSocketFrameOpcode,
};
use serde_json::Value;
use url::Url;

use super::super::events::{
    CdpNetworkAutomationEventSink, emit_cdp_network_automation_event,
    emit_request_served_from_cache, loading_failed_canceled,
};
use super::MainDocumentProgressOutputTarget;

impl CdpNetworkAutomationEventSink for MainDocumentProgressOutputTarget<'_> {
    fn push_protocol_event(&mut self, event: crate::conn::BackgroundProtocolEvent) {
        self.push_background_event(event);
    }

    fn push_request_will_be_sent_extra_info(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
        headers: serde_json::Map<String, Value>,
        cookie_access_report: Value,
        associated_cookies: Vec<Value>,
        request_time: f64,
    ) {
        self.push_background_event(
            crate::conn::BackgroundProtocolEvent::network_request_will_be_sent_extra_info(
                session_id,
                request_id,
                headers,
                cookie_access_report,
                associated_cookies,
                request_time,
            ),
        );
    }

    fn push_response_received_extra_info(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
        headers: serde_json::Map<String, Value>,
        status_code: u16,
        cookie_reports: Vec<Value>,
        blocked_cookies: Vec<Value>,
    ) {
        self.push_background_event(
            crate::conn::BackgroundProtocolEvent::network_response_received_extra_info(
                session_id,
                request_id,
                headers,
                status_code,
                cookie_reports,
                blocked_cookies,
            ),
        );
    }

    fn push_websocket_created(&mut self, session_id: Option<&str>, request_id: &str, url: &Url) {
        self.push_background_event(
            crate::conn::BackgroundProtocolEvent::network_websocket_created(
                session_id,
                request_id,
                url.as_str(),
            ),
        );
    }

    fn push_websocket_will_send_handshake_request(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
        timestamp: f64,
        headers: serde_json::Map<String, Value>,
    ) {
        self.push_background_event(
            crate::conn::BackgroundProtocolEvent::network_websocket_will_send_handshake_request(
                session_id, request_id, timestamp, headers,
            ),
        );
    }

    fn push_websocket_handshake_response_received(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
        timestamp: f64,
        status: u16,
        status_text: &str,
        headers: serde_json::Map<String, Value>,
    ) {
        self.push_background_event(
            crate::conn::BackgroundProtocolEvent::network_websocket_handshake_response_received(
                session_id,
                request_id,
                timestamp,
                status,
                status_text,
                headers,
            ),
        );
    }

    fn push_websocket_frame(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
        timestamp: f64,
        direction: WebSocketFrameDirection,
        opcode: WebSocketFrameOpcode,
        payload_length: usize,
    ) {
        self.push_background_event(
            crate::conn::BackgroundProtocolEvent::network_websocket_frame(
                session_id,
                request_id,
                timestamp,
                direction,
                opcode,
                payload_length,
            ),
        );
    }

    fn push_network_automation_event(
        &mut self,
        method: &str,
        params: serde_json::Value,
        session_id: Option<&str>,
        event: AutomationEvent,
    ) {
        self.push_automation_event(method, params, session_id, event);
    }
}

pub(super) fn emit_main_document_request_will_be_sent(
    output: &mut MainDocumentProgressOutputTarget<'_>,
    session_id: Option<&str>,
    request_id: &str,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
    url: &Url,
    method: &str,
    request_body: Option<&str>,
    request_headers: &[(String, String)],
    request_initiator_type: SubresourceRequestInitiatorType,
    redirect_response: Option<(
        &Url,
        u16,
        Option<&str>,
        &[(String, String)],
        bool,
        Option<moli_fetch::NegotiatedHttpVersion>,
    )>,
    redirect_has_extra_info: bool,
    cookie_access_report: Option<&StoredCookieQueryReport>,
    request_pause: Option<(&str, &[crate::devtools_runtime::DevToolsNetworkInterceptId])>,
) {
    if redirect_response.is_some_and(|(_, _, _, _, from_cache, _)| from_cache) {
        emit_request_served_from_cache(output, session_id, request_id);
    }
    emit_cdp_network_automation_event(
        output,
        AutomationEvent::NetworkBeforeRequestSent(NetworkRequestEvent {
            target_id: DevToolsTargetId::from(frame_id),
            frame_id: Some(DevToolsFrameId::from(frame_id)),
            request_id: DevToolsRequestId::from(request_id),
            loader_id: Some(DevToolsLoaderId::from(loader_id)),
            url: url.as_str().to_owned(),
            document_url: Some(url.as_str().to_owned()),
            method: Some(method.to_owned()),
            request_headers: request_headers.to_vec(),
            request_body: request_body.map(str::to_owned),
            request_initiator_type: Some(request_initiator_type.as_cdp_initiator_type().to_owned()),
            bidi_request_initiator_type: match request_initiator_type
                .as_bidi_request_initiator_type()
            {
                Some("css") => Some("css".to_owned()),
                _ => None,
            },
            redirect_response: redirect_response.map(
                |(
                    redirect_url,
                    redirect_status,
                    redirect_status_text,
                    redirect_headers,
                    redirect_from_cache,
                    negotiated_http_version,
                )| {
                    NetworkRedirectResponseEvent {
                        url: redirect_url.as_str().to_owned(),
                        status: redirect_status,
                        status_text: redirect_status_text.map(str::to_owned),
                        response_headers: redirect_headers.to_vec(),
                        encoded_data_length: 0,
                        from_cache: redirect_from_cache,
                        response_protocol: negotiated_http_version
                            .map(|version| version.protocol_name().to_owned()),
                    }
                },
            ),
            redirect_has_extra_info,
            request_cookie_report: cookie_access_report.cloned(),
            resource_type: Some(DevToolsNetworkResourceType::Document),
            timestamp: Some(timestamp),
            wall_time: Some(timestamp),
            status: None,
            status_text: None,
            response_headers: Vec::new(),
            response_mime_type: None,
            response_protocol: None,
            encoded_data_length: None,
            from_cache: false,
            has_extra_info: false,
            error_text: None,
            loading_failed_canceled: false,
            blocked_intercepts: request_pause
                .map(|(_, intercepts)| intercepts.to_vec())
                .unwrap_or_default(),
            fetch_request_id: request_pause.map(|(request_id, _)| {
                crate::devtools_runtime::DevToolsFetchRequestId::from(request_id)
            }),
            network_id: None,

            auth_challenge: None,
        }),
        session_id,
    );
}

pub(super) fn emit_request_will_be_sent_extra_info(
    output: &mut MainDocumentProgressOutputTarget<'_>,
    session_id: Option<&str>,
    request_id: &str,
    request_headers: &[(String, String)],
    cookie_access_report: &StoredCookieQueryReport,
    request_time: f64,
) {
    output.push_request_will_be_sent_extra_info(
        session_id,
        request_id,
        super::super::request_headers_as_json_object(request_headers, Some(cookie_access_report)),
        super::super::cookie_query_report_to_json(cookie_access_report),
        super::super::associated_cookies_to_json(cookie_access_report),
        request_time,
    );
}

pub(super) fn emit_response_received_extra_info(
    output: &mut MainDocumentProgressOutputTarget<'_>,
    session_id: Option<&str>,
    request_id: &str,
    response_headers: &[(String, String)],
    status: u16,
    cookie_set_reports: &[StoredCookieSetReport],
) {
    output.push_response_received_extra_info(
        session_id,
        request_id,
        super::super::headers_as_json_object(response_headers),
        status,
        cookie_set_reports
            .iter()
            .map(super::super::cookie_set_report_to_json)
            .collect(),
        Vec::new(),
    );
}

pub(super) fn emit_main_document_response_received(
    output: &mut MainDocumentProgressOutputTarget<'_>,
    session_id: Option<&str>,
    request_id: &str,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
    final_url: &Url,
    status: u16,
    response_headers: &[(String, String)],
    cookie_set_reports: &[StoredCookieSetReport],
    extra_info_status: u16,
    extra_info_headers: &[(String, String)],
    _network_extra_info_available: bool,
    emit_extra_info: bool,
    encoded_data_length: usize,
    from_cache: bool,
    negotiated_http_version: Option<moli_fetch::NegotiatedHttpVersion>,
    has_extra_info: bool,
) {
    if from_cache {
        emit_request_served_from_cache(output, session_id, request_id);
    }
    if emit_extra_info {
        emit_response_received_extra_info(
            output,
            session_id,
            request_id,
            extra_info_headers,
            extra_info_status,
            cookie_set_reports,
        );
    }
    emit_cdp_network_automation_event(
        output,
        AutomationEvent::NetworkResponseStarted(NetworkRequestEvent {
            target_id: DevToolsTargetId::from(frame_id),
            frame_id: Some(DevToolsFrameId::from(frame_id)),
            request_id: DevToolsRequestId::from(request_id),
            loader_id: Some(DevToolsLoaderId::from(loader_id)),
            url: final_url.as_str().to_owned(),
            document_url: None,
            method: None,
            request_headers: Vec::new(),
            request_body: None,
            request_initiator_type: None,
            bidi_request_initiator_type: None,
            redirect_response: None,
            redirect_has_extra_info: false,
            request_cookie_report: None,
            resource_type: Some(DevToolsNetworkResourceType::Document),
            timestamp: Some(timestamp),
            wall_time: None,
            status: Some(status),
            status_text: None,
            response_headers: response_headers.to_vec(),
            response_mime_type: None,
            response_protocol: negotiated_http_version
                .map(|version| version.protocol_name().to_owned()),
            encoded_data_length: Some(encoded_data_length),
            from_cache,
            has_extra_info,
            error_text: None,
            loading_failed_canceled: false,
            blocked_intercepts: Vec::new(),
            fetch_request_id: None,
            network_id: None,

            auth_challenge: None,
        }),
        session_id,
    );
}

pub(super) fn emit_body_finished(
    output: &mut MainDocumentProgressOutputTarget<'_>,
    session_id: Option<&str>,
    request_id: &str,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
    encoded_data_length: usize,
) {
    super::super::events::emit_body_finished(
        output,
        session_id,
        request_id,
        frame_id,
        loader_id,
        timestamp,
        encoded_data_length,
        DevToolsNetworkResourceType::Document,
    );
}

pub(super) fn emit_loading_failed(
    output: &mut MainDocumentProgressOutputTarget<'_>,
    session_id: Option<&str>,
    request_id: &str,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
    error_text: &str,
) {
    emit_cdp_network_automation_event(
        output,
        AutomationEvent::NetworkFetchError(NetworkRequestEvent {
            target_id: DevToolsTargetId::from(frame_id),
            frame_id: Some(DevToolsFrameId::from(frame_id)),
            request_id: DevToolsRequestId::from(request_id),
            loader_id: Some(DevToolsLoaderId::from(loader_id)),
            url: String::new(),
            document_url: None,
            method: None,
            request_headers: Vec::new(),
            request_body: None,
            request_initiator_type: None,
            bidi_request_initiator_type: None,
            redirect_response: None,
            redirect_has_extra_info: false,
            request_cookie_report: None,
            resource_type: Some(DevToolsNetworkResourceType::Document),
            timestamp: Some(timestamp),
            wall_time: None,
            status: None,
            status_text: None,
            response_headers: Vec::new(),
            response_mime_type: None,
            response_protocol: None,
            encoded_data_length: None,
            from_cache: false,
            has_extra_info: false,
            error_text: Some(error_text.to_owned()),
            loading_failed_canceled: loading_failed_canceled(error_text),
            blocked_intercepts: Vec::new(),
            fetch_request_id: None,
            network_id: None,

            auth_challenge: None,
        }),
        session_id,
    );
}
