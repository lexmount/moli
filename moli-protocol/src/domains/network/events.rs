use crate::devtools_runtime::{
    AutomationEvent, DevToolsFetchRequestId, DevToolsFrameId, DevToolsLoaderId,
    DevToolsNetworkInterceptId, DevToolsNetworkResourceType, DevToolsRequestId, DevToolsTargetId,
    NetworkRedirectResponseEvent, NetworkRequestEvent,
};
use moli_cookie_jar::StoredCookieSetReport;
use moli_core::page::{
    SubresourceRequestInitiatorType, WebSocketFrameDirection, WebSocketFrameOpcode,
};
use moli_web_mime::response_header_value;
use serde_json::{Value, json};
use url::Url;

use super::*;
use crate::conn::{BackgroundProtocolEvent, build_event};

pub(crate) use moli_fetch::NET_ERR_ABORTED_ERROR_TEXT;

pub(crate) fn loading_failed_canceled(error_text: &str) -> bool {
    error_text == NET_ERR_ABORTED_ERROR_TEXT
}

pub(crate) trait CdpNetworkAutomationEventSink {
    fn push_protocol_event(&mut self, event: BackgroundProtocolEvent);

    fn push_request_will_be_sent_extra_info(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
        headers: serde_json::Map<String, Value>,
        cookie_access_report: Value,
        associated_cookies: Vec<Value>,
        request_time: f64,
    );

    fn push_response_received_extra_info(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
        headers: serde_json::Map<String, Value>,
        status_code: u16,
        cookie_reports: Vec<Value>,
        blocked_cookies: Vec<Value>,
    );

    fn push_websocket_created(&mut self, session_id: Option<&str>, request_id: &str, url: &Url);

    fn push_websocket_will_send_handshake_request(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
        timestamp: f64,
        headers: serde_json::Map<String, Value>,
    );

    fn push_websocket_handshake_response_received(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
        timestamp: f64,
        status: u16,
        status_text: &str,
        headers: serde_json::Map<String, Value>,
    );

    fn push_websocket_frame(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
        timestamp: f64,
        direction: WebSocketFrameDirection,
        opcode: WebSocketFrameOpcode,
        payload_length: usize,
    );

    fn push_network_automation_event(
        &mut self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
        event: AutomationEvent,
    );
}

impl CdpNetworkAutomationEventSink for Vec<BackgroundProtocolEvent> {
    fn push_protocol_event(&mut self, event: BackgroundProtocolEvent) {
        self.push(event);
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
        self.push(
            BackgroundProtocolEvent::network_request_will_be_sent_extra_info(
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
        self.push(
            BackgroundProtocolEvent::network_response_received_extra_info(
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
        self.push(BackgroundProtocolEvent::network_websocket_created(
            session_id,
            request_id,
            url.as_str(),
        ));
    }

    fn push_websocket_will_send_handshake_request(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
        timestamp: f64,
        headers: serde_json::Map<String, Value>,
    ) {
        self.push(
            BackgroundProtocolEvent::network_websocket_will_send_handshake_request(
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
        self.push(
            BackgroundProtocolEvent::network_websocket_handshake_response_received(
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
        self.push(BackgroundProtocolEvent::network_websocket_frame(
            session_id,
            request_id,
            timestamp,
            direction,
            opcode,
            payload_length,
        ));
    }

    fn push_network_automation_event(
        &mut self,
        method: &str,
        mut params: Value,
        session_id: Option<&str>,
        event: AutomationEvent,
    ) {
        apply_blocked_intercepts_marker(&mut params, &event);
        self.push(BackgroundProtocolEvent::immediate_automation_event(
            build_event(method, params, session_id),
            event,
        ));
    }
}

fn apply_blocked_intercepts_marker(params: &mut Value, event: &AutomationEvent) {
    let blocked_intercepts: &[DevToolsNetworkInterceptId] = match event {
        AutomationEvent::NetworkBeforeRequestSent(network_event)
        | AutomationEvent::NetworkResponseStarted(network_event)
        | AutomationEvent::NetworkAuthRequired(network_event)
        | AutomationEvent::RequestPaused(network_event)
        | AutomationEvent::NetworkResponseCompleted(network_event)
        | AutomationEvent::NetworkFetchError(network_event) => {
            network_event.blocked_intercepts.as_slice()
        }
        _ => &[],
    };
    if !blocked_intercepts.is_empty() {
        params["__moliBlockedInterceptors"] = Value::Array(
            blocked_intercepts
                .iter()
                .map(|intercept| Value::from(intercept.as_str()))
                .collect(),
        );
    }
}

fn bidi_request_initiator_type_override(
    initiator_type: SubresourceRequestInitiatorType,
) -> Option<&'static str> {
    match initiator_type.as_bidi_request_initiator_type() {
        Some("css") => Some("css"),
        _ => None,
    }
}

pub(crate) fn emit_request_will_be_sent_extra_info(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    request_headers: &[(String, String)],
    cookie_access_report: &moli_cookie_jar::StoredCookieQueryReport,
    request_time: f64,
) {
    out.push_request_will_be_sent_extra_info(
        session_id,
        request_id,
        request_headers_as_json_object(request_headers, Some(cookie_access_report)),
        cookie_query_report_to_json(cookie_access_report),
        associated_cookies_to_json(cookie_access_report),
        request_time,
    );
}

pub(crate) fn emit_request_will_be_sent(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
    document_url: &Url,
    request_url: &Url,
    method: &str,
    request_body: Option<&str>,
    request_headers: &[(String, String)],
    resource_type: DevToolsNetworkResourceType,
    request_initiator_type: SubresourceRequestInitiatorType,
    redirect_response: Option<(
        &Url,
        u16,
        &[(String, String)],
        bool,
        Option<moli_fetch::NegotiatedHttpVersion>,
    )>,
    redirect_has_extra_info: bool,
    cookie_access_report: Option<&moli_cookie_jar::StoredCookieQueryReport>,
    blocked_intercepts: &[DevToolsNetworkInterceptId],
) {
    if redirect_response.is_some_and(|(_, _, _, from_cache, _)| from_cache) {
        emit_request_served_from_cache(out, session_id, request_id);
    }
    emit_cdp_network_automation_event(
        out,
        AutomationEvent::NetworkBeforeRequestSent(NetworkRequestEvent {
            target_id: DevToolsTargetId::from(frame_id),
            frame_id: Some(DevToolsFrameId::from(frame_id)),
            request_id: DevToolsRequestId::from(request_id),
            loader_id: Some(DevToolsLoaderId::from(loader_id)),
            url: request_url.as_str().to_owned(),
            document_url: Some(document_url.as_str().to_owned()),
            method: Some(method.to_owned()),
            request_headers: request_headers.to_vec(),
            request_body: request_body.map(str::to_owned),
            request_initiator_type: Some(request_initiator_type.as_cdp_initiator_type().to_owned()),
            bidi_request_initiator_type: bidi_request_initiator_type_override(
                request_initiator_type,
            )
            .map(str::to_owned),
            redirect_response: redirect_response.map(
                |(
                    redirect_url,
                    redirect_status,
                    redirect_headers,
                    redirect_from_cache,
                    negotiated_http_version,
                )| {
                    NetworkRedirectResponseEvent {
                        url: redirect_url.as_str().to_owned(),
                        status: redirect_status,
                        status_text: None,
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
            resource_type: Some(resource_type),
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
            blocked_intercepts: blocked_intercepts.to_vec(),
            fetch_request_id: None,
            network_id: None,

            auth_challenge: None,
        }),
        session_id,
    );
    if let Some(cookie_access_report) = cookie_access_report {
        emit_request_will_be_sent_extra_info(
            out,
            session_id,
            request_id,
            request_headers,
            cookie_access_report,
            timestamp,
        );
    }
}

pub(crate) fn fetch_subresource_initial_request_network_events(
    session_id: Option<&str>,
    output: &TargetSubresourceFetchPauseNetworkOutput,
) -> Vec<BackgroundProtocolEvent> {
    let resource_type = output.resource_type().into();
    let event = NetworkRequestEvent {
        target_id: DevToolsTargetId::from(output.frame_id()),
        frame_id: Some(DevToolsFrameId::from(output.frame_id())),
        request_id: DevToolsRequestId::from(output.network_request_id()),
        loader_id: Some(DevToolsLoaderId::from(output.loader_id())),
        url: output.request_url().as_str().to_owned(),
        document_url: Some(output.document_url().as_str().to_owned()),
        method: Some(output.method().to_owned()),
        request_headers: output.request_headers().to_vec(),
        request_body: output.request_body().map(str::to_owned),
        request_initiator_type: Some(
            SubresourceRequestInitiatorType::Script
                .as_cdp_initiator_type()
                .to_owned(),
        ),
        bidi_request_initiator_type: bidi_request_initiator_type_override(
            SubresourceRequestInitiatorType::Script,
        )
        .map(str::to_owned),
        redirect_response: None,
        redirect_has_extra_info: false,
        request_cookie_report: output.request_cookie_report().cloned(),
        resource_type: Some(resource_type),
        timestamp: Some(output.timestamp()),
        wall_time: Some(output.timestamp()),
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
        blocked_intercepts: output.blocked_intercepts().to_vec(),
        fetch_request_id: output.fetch_request_id().map(DevToolsFetchRequestId::from),
        network_id: None,

        auth_challenge: None,
    };
    let mut events = Vec::new();
    // Chromium's pause-side requestWillBeSent omits the transport Cookie
    // header and does not synthesize requestWillBeSentExtraInfo from the cookie
    // lookup. The network stack owns the single ExtraInfo event for this hop.
    emit_network_before_request_sent(&mut events, event, session_id, false);
    events
}

pub(crate) fn emit_response_received_extra_info(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    response_headers: &[(String, String)],
    status: u16,
    cookie_set_reports: &[StoredCookieSetReport],
) {
    out.push_response_received_extra_info(
        session_id,
        request_id,
        headers_as_json_object(response_headers),
        status,
        cookie_set_reports
            .iter()
            .map(cookie_set_report_to_json)
            .collect(),
        Vec::new(),
    );
}

pub(crate) fn emit_redirect_response_received_extra_info(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    response_headers: &[(String, String)],
    status: u16,
    cookie_set_reports: &[StoredCookieSetReport],
) {
    if cookie_set_reports.is_empty() {
        return;
    }
    emit_response_received_extra_info(
        out,
        session_id,
        request_id,
        response_headers,
        status,
        cookie_set_reports,
    );
}

pub(crate) fn emit_response_received(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
    final_url: &Url,
    status: u16,
    status_text: Option<&str>,
    response_headers: &[(String, String)],
    cookie_set_reports: &[StoredCookieSetReport],
    encoded_data_length: usize,
    from_cache: bool,
    negotiated_http_version: Option<moli_fetch::NegotiatedHttpVersion>,
    network_extra_info_available: bool,
    resource_type: DevToolsNetworkResourceType,
    blocked_intercepts: &[DevToolsNetworkInterceptId],
    fetch_request_id: Option<&str>,
) {
    emit_response_received_with_extra_info_delivery(
        out,
        session_id,
        request_id,
        frame_id,
        loader_id,
        timestamp,
        final_url,
        status,
        status_text,
        response_headers,
        cookie_set_reports,
        encoded_data_length,
        from_cache,
        negotiated_http_version,
        network_extra_info_available,
        true,
        resource_type,
        blocked_intercepts,
        fetch_request_id,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_response_received_without_extra_info_event(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
    final_url: &Url,
    status: u16,
    status_text: Option<&str>,
    response_headers: &[(String, String)],
    encoded_data_length: usize,
    from_cache: bool,
    negotiated_http_version: Option<moli_fetch::NegotiatedHttpVersion>,
    network_extra_info_available: bool,
    resource_type: DevToolsNetworkResourceType,
) {
    emit_response_received_with_extra_info_delivery(
        out,
        session_id,
        request_id,
        frame_id,
        loader_id,
        timestamp,
        final_url,
        status,
        status_text,
        response_headers,
        &[],
        encoded_data_length,
        from_cache,
        negotiated_http_version,
        network_extra_info_available,
        false,
        resource_type,
        &[],
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_response_received_with_extra_info_delivery(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
    final_url: &Url,
    status: u16,
    status_text: Option<&str>,
    response_headers: &[(String, String)],
    cookie_set_reports: &[StoredCookieSetReport],
    encoded_data_length: usize,
    from_cache: bool,
    negotiated_http_version: Option<moli_fetch::NegotiatedHttpVersion>,
    network_extra_info_available: bool,
    emit_extra_info_event: bool,
    resource_type: DevToolsNetworkResourceType,
    blocked_intercepts: &[DevToolsNetworkInterceptId],
    fetch_request_id: Option<&str>,
) {
    if from_cache {
        emit_request_served_from_cache(out, session_id, request_id);
    }
    let has_extra_info = network_extra_info_available || !cookie_set_reports.is_empty();
    emit_cdp_network_automation_event(
        out,
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
            resource_type: Some(resource_type),
            timestamp: Some(timestamp),
            wall_time: None,
            status: Some(status),
            status_text: status_text.map(str::to_owned),
            response_headers: response_headers.to_vec(),
            response_mime_type: None,
            response_protocol: negotiated_http_version
                .map(|version| version.protocol_name().to_owned()),
            encoded_data_length: Some(encoded_data_length),
            from_cache,
            has_extra_info,
            error_text: None,
            loading_failed_canceled: false,
            blocked_intercepts: blocked_intercepts.to_vec(),
            fetch_request_id: fetch_request_id.map(DevToolsFetchRequestId::from),
            network_id: None,

            auth_challenge: None,
        }),
        session_id,
    );
    if has_extra_info && emit_extra_info_event {
        emit_response_received_extra_info(
            out,
            session_id,
            request_id,
            response_headers,
            status,
            cookie_set_reports,
        );
    }
}

pub(crate) fn emit_body_finished(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
    encoded_data_length: usize,
    resource_type: DevToolsNetworkResourceType,
) {
    emit_data_received(
        out,
        session_id,
        request_id,
        timestamp,
        encoded_data_length,
        encoded_data_length,
    );
    emit_loading_finished(
        out,
        session_id,
        request_id,
        frame_id,
        loader_id,
        timestamp,
        encoded_data_length,
        resource_type,
    );
}

pub(crate) fn emit_loading_finished(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
    encoded_data_length: usize,
    resource_type: DevToolsNetworkResourceType,
) {
    emit_cdp_network_automation_event(
        out,
        AutomationEvent::NetworkResponseCompleted(NetworkRequestEvent {
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
            resource_type: Some(resource_type),
            timestamp: Some(timestamp),
            wall_time: None,
            status: None,
            status_text: None,
            response_headers: Vec::new(),
            response_mime_type: None,
            response_protocol: None,
            encoded_data_length: Some(encoded_data_length),
            from_cache: false,
            has_extra_info: false,
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

pub(crate) fn emit_event_source_message_received(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    timestamp: f64,
    event_name: &str,
    event_id: &str,
    data: &str,
) {
    out.push_protocol_event(BackgroundProtocolEvent::immediate(build_event(
        "Network.eventSourceMessageReceived",
        json!({
            "requestId": request_id,
            "timestamp": timestamp,
            "eventName": event_name,
            "eventId": event_id,
            "data": data,
        }),
        session_id,
    )));
}

pub(crate) fn emit_request_served_from_cache(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
) {
    out.push_protocol_event(BackgroundProtocolEvent::immediate(build_event(
        "Network.requestServedFromCache",
        json!({ "requestId": request_id }),
        session_id,
    )));
}

pub(crate) fn emit_data_received(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    timestamp: f64,
    data_length: usize,
    encoded_data_length: usize,
) {
    if data_length == 0 && encoded_data_length == 0 {
        return;
    }
    out.push_protocol_event(BackgroundProtocolEvent::immediate(build_event(
        "Network.dataReceived",
        json!({
            "requestId": request_id,
            "timestamp": timestamp,
            "dataLength": data_length,
            "encodedDataLength": encoded_data_length,
        }),
        session_id,
    )));
}

pub(crate) fn emit_loading_failed(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
    error_text: &str,
    resource_type: DevToolsNetworkResourceType,
) {
    emit_cdp_network_automation_event(
        out,
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
            resource_type: Some(resource_type),
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

pub(crate) fn emit_cdp_network_automation_event(
    out: &mut impl CdpNetworkAutomationEventSink,
    event: AutomationEvent,
    session_id: Option<&str>,
) {
    match event {
        AutomationEvent::NetworkBeforeRequestSent(network_event) => {
            emit_network_before_request_sent(out, network_event, session_id, true);
        }
        AutomationEvent::NetworkResponseStarted(network_event) => {
            let params = {
                let response_mime_type = network_event
                    .response_mime_type
                    .clone()
                    .or_else(|| {
                        response_header_value(&network_event.response_headers, "content-type")
                    })
                    .unwrap_or_default();
                let response_protocol =
                    network_event.response_protocol.clone().unwrap_or_else(|| {
                        Url::parse(&network_event.url)
                            .ok()
                            .map(|url| response_protocol_for_url(&url).to_owned())
                            .unwrap_or_default()
                    });
                let mut params = json!({
                    "requestId": network_event.request_id.as_str(),
                    "loaderId": network_event.loader_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
                    "timestamp": network_event.timestamp.unwrap_or_default(),
                    "type": network_event
                        .resource_type
                        .map(DevToolsNetworkResourceType::as_cdp_type)
                        .unwrap_or_default(),
                    "frameId": network_event.frame_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
                    "response": {
                        "url": network_event.url.as_str(),
                        "status": network_event.status.unwrap_or_default(),
                        "statusText": network_event
                            .status_text
                            .as_deref()
                            .unwrap_or_else(|| http_status_text(network_event.status.unwrap_or_default())),
                        "headers": headers_as_json_object(&network_event.response_headers),
                        "mimeType": response_mime_type,
                        "connectionReused": false,
                        "connectionId": 0,
                        "encodedDataLength": network_event.encoded_data_length.unwrap_or_default(),
                        "fromDiskCache": network_event.from_cache,
                        "securityState": "secure",
                        "protocol": response_protocol,
                    },
                    "hasExtraInfo": network_event.has_extra_info,
                });
                if let Some(fetch_request_id) = network_event.fetch_request_id.as_ref() {
                    params["__moliFetchRequestId"] = json!(fetch_request_id.as_str());
                }
                params
            };
            out.push_network_automation_event(
                "Network.responseReceived",
                params,
                session_id,
                AutomationEvent::NetworkResponseStarted(network_event),
            );
        }
        AutomationEvent::NetworkResponseCompleted(network_event) => {
            let params = json!({
                "requestId": network_event.request_id.as_str(),
                "timestamp": network_event.timestamp.unwrap_or_default(),
                "encodedDataLength": network_event.encoded_data_length.unwrap_or_default(),
            });
            out.push_network_automation_event(
                "Network.loadingFinished",
                params,
                session_id,
                AutomationEvent::NetworkResponseCompleted(network_event),
            );
        }
        AutomationEvent::NetworkFetchError(network_event) => {
            let mut params = json!({
                "requestId": network_event.request_id.as_str(),
                "timestamp": network_event.timestamp.unwrap_or_default(),
                "type": network_event
                    .resource_type
                    .map(DevToolsNetworkResourceType::as_cdp_type)
                    .unwrap_or_default(),
                "errorText": network_event.error_text.as_deref().unwrap_or_default(),
                "canceled": network_event.loading_failed_canceled,
            });
            if network_event.error_text.as_deref() == Some("net::ERR_BLOCKED_BY_RESPONSE") {
                params["blockedReason"] = json!("CoopSandboxedIframeCannotNavigateToCoopPage");
            }
            out.push_network_automation_event(
                "Network.loadingFailed",
                params,
                session_id,
                AutomationEvent::NetworkFetchError(network_event),
            );
        }
        AutomationEvent::NetworkAuthRequired(network_event) => {
            let params = fetch_auth_required_params(&network_event);
            out.push_network_automation_event(
                "Fetch.authRequired",
                params,
                session_id,
                AutomationEvent::NetworkAuthRequired(network_event),
            );
        }
        AutomationEvent::RequestPaused(network_event) => {
            let params = fetch_request_paused_params(&network_event);
            out.push_network_automation_event(
                "Fetch.requestPaused",
                params,
                session_id,
                AutomationEvent::RequestPaused(network_event),
            );
        }
        _ => {}
    }
}

fn emit_network_before_request_sent(
    out: &mut impl CdpNetworkAutomationEventSink,
    network_event: NetworkRequestEvent,
    session_id: Option<&str>,
    include_cookie_header_from_access_report: bool,
) {
    let cookie_header_report = include_cookie_header_from_access_report
        .then_some(network_event.request_cookie_report.as_ref())
        .flatten();
    let mut params = json!({
        "requestId": network_event.request_id.as_str(),
        "loaderId": network_event.loader_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        "documentURL": network_event.document_url.as_deref().unwrap_or_default(),
        "request": {
            "url": network_event.url.as_str(),
            "method": network_event.method.as_deref().unwrap_or_default(),
            "headers": request_headers_as_json_object(
                &network_event.request_headers,
                cookie_header_report,
            ),
            "hasPostData": network_event.request_body.is_some(),
        },
        "timestamp": network_event.timestamp.unwrap_or_default(),
        "wallTime": network_event.wall_time.unwrap_or_default(),
        "initiator": {
            "type": network_event
                .request_initiator_type
                .as_deref()
                .unwrap_or("other")
        },
        "redirectHasExtraInfo": network_event.redirect_has_extra_info,
        "type": network_event
            .resource_type
            .map(DevToolsNetworkResourceType::as_cdp_type)
            .unwrap_or_default(),
        "frameId": network_event.frame_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        "hasUserGesture": false,
    });
    if let Some(request_body) = network_event.request_body.as_ref() {
        params["request"]["postData"] = json!(request_body);
    }
    if let Some(redirect_response) = network_event.redirect_response.as_ref() {
        params["redirectResponse"] = network_redirect_response_payload(redirect_response);
    }
    if let Some(cookie_access_report) = network_event.request_cookie_report.as_ref() {
        params["cookieAccessReport"] = cookie_query_report_to_json(cookie_access_report);
    }
    if let Some(bidi_request_initiator_type) = network_event.bidi_request_initiator_type.as_ref() {
        params["__moliRequestInitiatorType"] = json!(bidi_request_initiator_type);
    }
    if let Some(fetch_request_id) = network_event.fetch_request_id.as_ref() {
        params["__moliFetchRequestId"] = json!(fetch_request_id.as_str());
    }
    out.push_network_automation_event(
        "Network.requestWillBeSent",
        params,
        session_id,
        AutomationEvent::NetworkBeforeRequestSent(network_event),
    );
}

pub fn fetch_auth_required_params(network_event: &NetworkRequestEvent) -> Value {
    json!({
        "requestId": network_event.request_id.as_str(),
        "frameId": network_event.frame_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        "request": fetch_request_payload(network_event),
        "resourceType": network_event
            .resource_type
            .map(DevToolsNetworkResourceType::as_cdp_type)
            .unwrap_or_default(),
        "authChallenge": fetch_auth_challenge_payload(network_event),
    })
}

pub fn fetch_request_paused_params(network_event: &NetworkRequestEvent) -> Value {
    let mut params = json!({
        "requestId": network_event.request_id.as_str(),
        "frameId": network_event.frame_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        "request": fetch_request_payload(network_event),
        "resourceType": network_event
            .resource_type
            .map(DevToolsNetworkResourceType::as_cdp_type)
            .unwrap_or_default(),
    });
    if let Some(network_id) = network_event.network_id.as_ref() {
        params["networkId"] = json!(network_id.as_str());
    }
    if let Some(status) = network_event.status {
        params["responseStatusCode"] = json!(status);
        params["responseHeaders"] = Value::Array(
            network_event
                .response_headers
                .iter()
                .map(|(name, value)| json!({ "name": name, "value": value }))
                .collect(),
        );
        if let Some(status_text) = network_event.status_text.as_ref() {
            params["responseStatusText"] = json!(status_text);
        }
    }
    if !network_event.blocked_intercepts.is_empty() {
        params["__moliBlockedInterceptors"] = Value::Array(
            network_event
                .blocked_intercepts
                .iter()
                .map(|intercept| Value::from(intercept.as_str()))
                .collect(),
        );
    }
    params
}

fn fetch_request_payload(network_event: &NetworkRequestEvent) -> Value {
    let mut request = json!({
        "url": network_event.url.as_str(),
        "method": network_event.method.as_deref().unwrap_or_default(),
        "headers": request_headers_as_json_object(
            &network_event.request_headers,
            network_event.request_cookie_report.as_ref(),
        ),
        "hasPostData": network_event.request_body.is_some(),
    });
    if let Some(request_body) = network_event.request_body.as_ref() {
        request["postData"] = json!(request_body);
    }
    request
}

fn fetch_auth_challenge_payload(network_event: &NetworkRequestEvent) -> Value {
    if let Some(challenge) = network_event.auth_challenge.as_ref() {
        return json!({
            "origin": challenge.origin,
            "source": challenge.source,
            "scheme": challenge.scheme,
            "realm": challenge.realm,
        });
    }
    json!({
        "origin": "",
        "source": "Server",
        "scheme": "",
        "realm": "",
    })
}

pub(crate) fn emit_websocket_created(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    url: &Url,
) {
    out.push_websocket_created(session_id, request_id, url);
}

pub(crate) fn emit_websocket_will_send_handshake_request(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    timestamp: f64,
    request_headers: &[(String, String)],
) {
    out.push_websocket_will_send_handshake_request(
        session_id,
        request_id,
        timestamp,
        headers_as_json_object(request_headers),
    );
}

pub(crate) fn emit_websocket_handshake_response_received(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    timestamp: f64,
    status: u16,
    response_headers: &[(String, String)],
) {
    out.push_websocket_handshake_response_received(
        session_id,
        request_id,
        timestamp,
        status,
        http_status_text(status),
        headers_as_json_object(response_headers),
    );
}

pub(crate) fn emit_websocket_frame(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    timestamp: f64,
    direction: WebSocketFrameDirection,
    opcode: WebSocketFrameOpcode,
    payload_length: usize,
) {
    out.push_websocket_frame(
        session_id,
        request_id,
        timestamp,
        direction,
        opcode,
        payload_length,
    );
}

pub(crate) fn emit_websocket_frame_error(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    timestamp: f64,
    error_message: &str,
) {
    out.push_protocol_event(BackgroundProtocolEvent::immediate(build_event(
        "Network.webSocketFrameError",
        json!({
            "requestId": request_id,
            "timestamp": timestamp,
            "errorMessage": error_message,
        }),
        session_id,
    )));
}

pub(crate) fn emit_websocket_closed(
    out: &mut impl CdpNetworkAutomationEventSink,
    session_id: Option<&str>,
    request_id: &str,
    timestamp: f64,
) {
    out.push_protocol_event(BackgroundProtocolEvent::immediate(build_event(
        "Network.webSocketClosed",
        json!({
            "requestId": request_id,
            "timestamp": timestamp,
        }),
        session_id,
    )));
}

pub(crate) fn headers_as_json_object(
    headers: &[(String, String)],
) -> serde_json::Map<String, Value> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), json!(value)))
        .collect()
}

fn request_cookie_header_value(
    cookie_access_report: Option<&moli_cookie_jar::StoredCookieQueryReport>,
) -> Option<String> {
    let report = cookie_access_report?;
    if report.included_cookies.is_empty() {
        return None;
    }
    Some(
        report
            .included_cookies
            .iter()
            .map(|entry| format!("{}={}", entry.cookie.name, entry.cookie.value))
            .collect::<Vec<_>>()
            .join("; "),
    )
}

pub(crate) fn request_headers_as_json_object(
    headers: &[(String, String)],
    cookie_access_report: Option<&moli_cookie_jar::StoredCookieQueryReport>,
) -> serde_json::Map<String, Value> {
    let synthesized_cookie_header = request_cookie_header_value(cookie_access_report);
    let mut json_headers = serde_json::Map::new();
    for (name, value) in headers {
        if synthesized_cookie_header.is_some() && name.eq_ignore_ascii_case("cookie") {
            continue;
        }
        json_headers.insert(name.clone(), json!(value));
    }
    if let Some(cookie_header) = synthesized_cookie_header {
        json_headers.insert("Cookie".to_owned(), json!(cookie_header));
    }
    json_headers
}

#[cfg(test)]
pub(crate) fn build_response_payload(
    url: &Url,
    status: u16,
    response_headers: &[(String, String)],
    encoded_data_length: usize,
) -> Value {
    build_response_payload_with_status_text(
        url,
        status,
        None,
        response_headers,
        encoded_data_length,
        false,
    )
}

pub(crate) fn build_response_payload_with_status_text(
    url: &Url,
    status: u16,
    status_text: Option<&str>,
    response_headers: &[(String, String)],
    encoded_data_length: usize,
    from_cache: bool,
) -> Value {
    let mime_type = response_header_value(response_headers, "content-type").unwrap_or_default();
    let status_text = status_text.unwrap_or_else(|| http_status_text(status));
    json!({
        "url": url.as_str(),
        "status": status,
        "statusText": status_text,
        "headers": headers_as_json_object(response_headers),
        "mimeType": mime_type,
        "connectionReused": false,
        "connectionId": 0,
        "encodedDataLength": encoded_data_length,
        "fromDiskCache": from_cache,
        "securityState": "secure",
        "protocol": response_protocol_for_url(url),
    })
}

fn network_redirect_response_payload(response: &NetworkRedirectResponseEvent) -> Value {
    if let Ok(url) = Url::parse(&response.url) {
        let mut payload = build_response_payload_with_status_text(
            &url,
            response.status,
            response.status_text.as_deref(),
            &response.response_headers,
            response.encoded_data_length,
            response.from_cache,
        );
        if let Some(protocol) = response.response_protocol.as_ref() {
            payload["protocol"] = json!(protocol);
        }
        return payload;
    }
    let mime_type =
        response_header_value(&response.response_headers, "content-type").unwrap_or_default();
    let status_text = response
        .status_text
        .as_deref()
        .unwrap_or_else(|| http_status_text(response.status));
    json!({
        "url": response.url,
        "status": response.status,
        "statusText": status_text,
        "headers": headers_as_json_object(&response.response_headers),
        "mimeType": mime_type,
        "connectionReused": false,
        "connectionId": 0,
        "encodedDataLength": response.encoded_data_length,
        "fromDiskCache": response.from_cache,
        "securityState": "secure",
        "protocol": "",
    })
}

fn response_protocol_for_url(url: &Url) -> &str {
    match url.scheme() {
        "http" | "https" => "http/1.1",
        scheme => scheme,
    }
}

#[cfg(test)]
mod tests {
    use moli_core::page::{
        PendingSubresourceFetchInfo, SubresourceRequestInitiatorType, SubresourceResourceType,
    };
    use serde_json::{Value, json};
    use url::Url;

    use crate::devtools_runtime::{
        AutomationEvent, DevToolsFrameId, DevToolsLoaderId, DevToolsNetworkInterceptId,
        DevToolsNetworkResourceType, DevToolsRequestId, DevToolsTargetId, NetworkRequestEvent,
    };
    use crate::domains::network::TargetSubresourceFetchPauseNetworkOutput;

    use super::{
        NET_ERR_ABORTED_ERROR_TEXT, build_response_payload, emit_body_finished,
        emit_cdp_network_automation_event, emit_loading_failed, emit_request_will_be_sent,
        emit_response_received, fetch_subresource_initial_request_network_events,
    };

    fn protocol_messages_from_background_events(
        events: Vec<crate::conn::BackgroundProtocolEvent>,
    ) -> Vec<Value> {
        events
            .into_iter()
            .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
            .collect()
    }

    #[test]
    fn response_payload_mime_type_uses_http_header_name_matching() {
        let payload = build_response_payload(
            &Url::parse("https://example.test/").unwrap(),
            200,
            &[
                ("Content-Type".to_owned(), "text/html".to_owned()),
                ("Bad Header".to_owned(), "ignored".to_owned()),
            ],
            0,
        );

        assert_eq!(payload["mimeType"], "text/html");
    }

    #[test]
    fn response_payload_protocol_uses_http_version_for_http_urls() {
        for url in [
            Url::parse("http://example.test/").unwrap(),
            Url::parse("https://example.test/").unwrap(),
        ] {
            let payload = build_response_payload(&url, 200, &[], 0);
            assert_eq!(payload["protocol"], "http/1.1");
        }
    }

    #[test]
    fn response_payload_protocol_preserves_non_http_scheme() {
        let payload =
            build_response_payload(&Url::parse("data:text/plain,ok").unwrap(), 200, &[], 0);

        assert_eq!(payload["protocol"], "data");
    }

    #[test]
    fn fetch_pause_network_output_emits_initial_request_with_network_id() {
        let info = PendingSubresourceFetchInfo {
            internal_id: 1,
            network_request_handle: None,
            frame_id: Some("FRAME-1".to_owned()),
            document_url: Url::parse("https://example.test/page").unwrap(),
            url: Url::parse("https://example.test/api").unwrap(),
            websocket_socket_id: None,
            method: "POST".to_owned(),
            request_headers: vec![("x-test".to_owned(), "1".to_owned())],
            request_body: Some("body".to_owned()),
            request_body_bytes: Some(b"body".to_vec()),
            resource_type: SubresourceResourceType::Fetch,
            request_cookie_report: None,
        };
        let output = TargetSubresourceFetchPauseNetworkOutput::from_pending_fetch_info(
            "REQ-42".to_owned(),
            "FRAME-1".to_owned(),
            "LOADER-1".to_owned(),
            42.0,
            info.document_url.clone(),
            &info,
        );

        let events = fetch_subresource_initial_request_network_events(Some("SID-1"), &output);
        assert_eq!(events.len(), 1);
        let (message, sidecar) = events.into_iter().next().unwrap().into_parts();
        let out = [message];

        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["sessionId"], json!("SID-1"));
        assert_eq!(out[0]["method"], json!("Network.requestWillBeSent"));
        assert_eq!(out[0]["params"]["requestId"], json!("REQ-42"));
        assert_eq!(out[0]["params"]["frameId"], json!("FRAME-1"));
        assert_eq!(out[0]["params"]["loaderId"], json!("LOADER-1"));
        assert_eq!(
            out[0]["params"]["request"]["url"],
            json!("https://example.test/api")
        );
        assert_eq!(out[0]["params"]["request"]["method"], json!("POST"));
        assert_eq!(out[0]["params"]["request"]["postData"], json!("body"));
        assert_eq!(out[0]["params"]["type"], json!("Fetch"));
        assert!(matches!(
            sidecar.as_ref(),
            Some(AutomationEvent::NetworkBeforeRequestSent(event))
                if event.request_id.as_str() == "REQ-42"
                    && event.method.as_deref() == Some("POST")
                    && event.request_body.as_deref() == Some("body")
                    && event.resource_type == Some(DevToolsNetworkResourceType::Fetch)
                    && event.fetch_request_id.is_none()
        ));
    }

    #[test]
    fn response_received_serializes_from_automation_event_shape() {
        let mut events = Vec::new();

        emit_response_received(
            &mut events,
            Some("SID-1"),
            "REQ-42",
            "FRAME-1",
            "LOADER-1",
            43.0,
            &Url::parse("https://example.test/api").unwrap(),
            201,
            None,
            &[
                ("content-type".to_owned(), "application/json".to_owned()),
                ("x-test".to_owned(), "1".to_owned()),
            ],
            &[],
            128,
            false,
            Some(moli_fetch::NegotiatedHttpVersion::Http2),
            false,
            DevToolsNetworkResourceType::Fetch,
            &[],
            None,
        );
        let out = protocol_messages_from_background_events(events);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["sessionId"], json!("SID-1"));
        assert_eq!(out[0]["method"], json!("Network.responseReceived"));
        assert_eq!(out[0]["params"]["requestId"], json!("REQ-42"));
        assert_eq!(out[0]["params"]["loaderId"], json!("LOADER-1"));
        assert_eq!(out[0]["params"]["frameId"], json!("FRAME-1"));
        assert_eq!(out[0]["params"]["timestamp"], json!(43.0));
        assert_eq!(out[0]["params"]["type"], json!("Fetch"));
        assert_eq!(out[0]["params"]["hasExtraInfo"], json!(false));
        assert_eq!(
            out[0]["params"]["response"]["url"],
            json!("https://example.test/api")
        );
        assert_eq!(out[0]["params"]["response"]["status"], json!(201));
        assert_eq!(out[0]["params"]["response"]["statusText"], json!("Created"));
        assert_eq!(
            out[0]["params"]["response"]["headers"]["content-type"],
            json!("application/json")
        );
        assert_eq!(
            out[0]["params"]["response"]["encodedDataLength"],
            json!(128)
        );
        assert_eq!(out[0]["params"]["response"]["fromDiskCache"], json!(false));
        assert_eq!(out[0]["params"]["response"]["protocol"], json!("h2"));
    }

    #[test]
    fn cached_response_emits_served_from_cache_before_response() {
        let mut events = Vec::new();

        emit_response_received(
            &mut events,
            Some("SID-1"),
            "REQ-42",
            "FRAME-1",
            "LOADER-1",
            43.0,
            &Url::parse("https://example.test/cached.js").unwrap(),
            200,
            None,
            &[],
            &[],
            29,
            true,
            None,
            false,
            DevToolsNetworkResourceType::Script,
            &[],
            None,
        );

        assert_eq!(events.len(), 2);
        let (cached, cached_sidecar) = events.remove(0).into_parts();
        assert_eq!(
            cached,
            json!({
                "method": "Network.requestServedFromCache",
                "params": { "requestId": "REQ-42" },
                "sessionId": "SID-1"
            })
        );
        assert!(cached_sidecar.is_none());

        let (response, response_sidecar) = events.remove(0).into_parts();
        assert_eq!(response["method"], json!("Network.responseReceived"));
        assert_eq!(response["params"]["response"]["fromDiskCache"], json!(true));
        assert!(matches!(
            response_sidecar,
            Some(AutomationEvent::NetworkResponseStarted(event))
                if event.request_id.as_str() == "REQ-42" && event.from_cache
        ));
    }

    #[test]
    fn cached_redirect_emits_served_from_cache_before_next_request() {
        let mut events = Vec::new();
        let document_url = Url::parse("https://example.test/page").unwrap();
        let redirect_url = Url::parse("https://example.test/start").unwrap();
        let final_url = Url::parse("https://example.test/final").unwrap();

        emit_request_will_be_sent(
            &mut events,
            Some("SID-1"),
            "REQ-42",
            "FRAME-1",
            "LOADER-1",
            43.0,
            &document_url,
            &final_url,
            "GET",
            None,
            &[],
            DevToolsNetworkResourceType::Fetch,
            SubresourceRequestInitiatorType::Script,
            Some((
                &redirect_url,
                302,
                &[],
                true,
                Some(moli_fetch::NegotiatedHttpVersion::Http2),
            )),
            false,
            None,
            &[],
        );

        assert_eq!(events.len(), 2);
        let (cached, cached_sidecar) = events.remove(0).into_parts();
        assert_eq!(
            cached,
            json!({
                "method": "Network.requestServedFromCache",
                "params": { "requestId": "REQ-42" },
                "sessionId": "SID-1"
            })
        );
        assert!(cached_sidecar.is_none());

        let (request, request_sidecar) = events.remove(0).into_parts();
        assert_eq!(request["method"], json!("Network.requestWillBeSent"));
        assert_eq!(
            request["params"]["redirectResponse"]["fromDiskCache"],
            json!(true)
        );
        assert_eq!(
            request["params"]["redirectResponse"]["protocol"],
            json!("h2")
        );
        assert!(matches!(
            request_sidecar,
            Some(AutomationEvent::NetworkBeforeRequestSent(event))
                if event.request_id.as_str() == "REQ-42"
        ));
    }

    #[test]
    fn response_received_preserves_custom_status_text() {
        let mut events = Vec::new();

        emit_response_received(
            &mut events,
            None,
            "REQ-42",
            "FRAME-1",
            "LOADER-1",
            43.0,
            &Url::parse("https://example.test/service-worker-response").unwrap(),
            200,
            Some("OK from serviceworker"),
            &[],
            &[],
            0,
            false,
            None,
            false,
            DevToolsNetworkResourceType::Fetch,
            &[],
            None,
        );
        let out = protocol_messages_from_background_events(events);

        assert_eq!(
            out[0]["params"]["response"]["statusText"],
            json!("OK from serviceworker")
        );
    }

    #[test]
    fn blocked_response_received_serializes_fetch_request_marker() {
        let mut events = Vec::new();

        emit_response_received(
            &mut events,
            Some("SID-1"),
            "REQ-42",
            "FRAME-1",
            "LOADER-1",
            43.0,
            &Url::parse("https://example.test/api").unwrap(),
            200,
            None,
            &[],
            &[],
            0,
            false,
            None,
            false,
            DevToolsNetworkResourceType::Fetch,
            &[DevToolsNetworkInterceptId::from("intercept-response")],
            Some("FETCH-42"),
        );
        assert_eq!(events.len(), 1);
        let event = events.pop().expect("response event should be emitted");
        let internal_message = event
            .protocol_message()
            .expect("network event should carry protocol payload");

        assert_eq!(
            internal_message["method"],
            json!("Network.responseReceived")
        );
        assert_eq!(internal_message["params"]["requestId"], json!("REQ-42"));
        assert_eq!(
            internal_message["params"]["__moliBlockedInterceptors"],
            json!(["intercept-response"])
        );
        assert_eq!(
            internal_message["params"]["__moliFetchRequestId"],
            json!("FETCH-42")
        );
        assert!(matches!(
            event.clone().into_parts().1,
            Some(AutomationEvent::NetworkResponseStarted(network_event))
                if network_event.blocked_intercepts
                    == vec![DevToolsNetworkInterceptId::from("intercept-response")]
        ));

        let wire_message = event.into_protocol_message();
        assert_eq!(wire_message["method"], json!("Network.responseReceived"));
        assert_eq!(wire_message["params"]["requestId"], json!("REQ-42"));
        assert!(wire_message["params"]["__moliBlockedInterceptors"].is_null());
        assert!(wire_message["params"]["__moliFetchRequestId"].is_null());
    }

    #[test]
    fn body_finished_serializes_data_before_loading_finished() {
        let mut events = Vec::new();

        emit_body_finished(
            &mut events,
            Some("SID-1"),
            "REQ-42",
            "FRAME-1",
            "LOADER-1",
            44.0,
            256,
            DevToolsNetworkResourceType::Xhr,
        );
        assert!(events[0].clone().into_parts().1.is_none());
        assert!(matches!(
            events[1].clone().into_parts().1,
            Some(AutomationEvent::NetworkResponseCompleted(network_event))
                if network_event.request_id == DevToolsRequestId::from("REQ-42")
                    && network_event.encoded_data_length == Some(256)
                    && network_event.resource_type == Some(DevToolsNetworkResourceType::Xhr)
        ));
        let out = protocol_messages_from_background_events(events);

        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["sessionId"], json!("SID-1"));
        assert_eq!(out[0]["method"], json!("Network.dataReceived"));
        assert_eq!(
            out[0]["params"],
            json!({
                "requestId": "REQ-42",
                "timestamp": 44.0,
                "dataLength": 256,
                "encodedDataLength": 256,
            })
        );
        assert_eq!(out[1]["sessionId"], json!("SID-1"));
        assert_eq!(out[1]["method"], json!("Network.loadingFinished"));
        assert_eq!(
            out[1]["params"],
            json!({
                "requestId": "REQ-42",
                "timestamp": 44.0,
                "encodedDataLength": 256,
            })
        );
    }

    #[test]
    fn empty_body_finishes_without_fabricating_data_chunk() {
        let mut events = Vec::new();

        emit_body_finished(
            &mut events,
            Some("SID-1"),
            "REQ-42",
            "FRAME-1",
            "LOADER-1",
            44.0,
            0,
            DevToolsNetworkResourceType::Xhr,
        );
        let out = protocol_messages_from_background_events(events);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["method"], json!("Network.loadingFinished"));
        assert_eq!(out[0]["params"]["encodedDataLength"], json!(0));
    }

    #[test]
    fn loading_failed_serializes_from_automation_event_shape() {
        let mut events = Vec::new();

        emit_loading_failed(
            &mut events,
            Some("SID-1"),
            "REQ-42",
            "FRAME-1",
            "LOADER-1",
            45.0,
            "net::ERR_FAILED",
            DevToolsNetworkResourceType::Fetch,
        );
        assert!(matches!(
            events[0].clone().into_parts().1,
            Some(AutomationEvent::NetworkFetchError(network_event))
                if network_event.request_id == DevToolsRequestId::from("REQ-42")
                    && network_event.error_text.as_deref() == Some("net::ERR_FAILED")
        ));
        let out = protocol_messages_from_background_events(events);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["sessionId"], json!("SID-1"));
        assert_eq!(out[0]["method"], json!("Network.loadingFailed"));
        assert_eq!(
            out[0]["params"],
            json!({
                "requestId": "REQ-42",
                "timestamp": 45.0,
                "type": "Fetch",
                "errorText": "net::ERR_FAILED",
                "canceled": false,
            })
        );
    }

    #[test]
    fn aborted_loading_failed_serializes_canceled() {
        let mut events = Vec::new();

        emit_loading_failed(
            &mut events,
            Some("SID-1"),
            "REQ-42",
            "FRAME-1",
            "LOADER-1",
            45.0,
            NET_ERR_ABORTED_ERROR_TEXT,
            DevToolsNetworkResourceType::Fetch,
        );
        let out = protocol_messages_from_background_events(events);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["method"], json!("Network.loadingFailed"));
        assert_eq!(out[0]["params"]["errorText"], json!("net::ERR_ABORTED"));
        assert_eq!(out[0]["params"]["canceled"], json!(true));
    }

    #[test]
    fn aborted_loading_failed_fallback_serializes_canceled() {
        let mut events = Vec::new();

        emit_cdp_network_automation_event(
            &mut events,
            AutomationEvent::NetworkFetchError(NetworkRequestEvent {
                target_id: DevToolsTargetId::from("FRAME-1"),
                frame_id: Some(DevToolsFrameId::from("FRAME-1")),
                request_id: DevToolsRequestId::from("REQ-42"),
                loader_id: Some(DevToolsLoaderId::from("LOADER-1")),
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
                resource_type: Some(DevToolsNetworkResourceType::Fetch),
                timestamp: Some(45.0),
                wall_time: None,
                status: None,
                status_text: None,
                response_headers: Vec::new(),
                response_mime_type: None,
                response_protocol: None,
                encoded_data_length: None,
                from_cache: false,
                has_extra_info: false,
                error_text: Some(NET_ERR_ABORTED_ERROR_TEXT.to_owned()),
                loading_failed_canceled: true,
                blocked_intercepts: Vec::new(),
                fetch_request_id: None,
                network_id: None,

                auth_challenge: None,
            }),
            Some("SID-1"),
        );
        let out = protocol_messages_from_background_events(events);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["method"], json!("Network.loadingFailed"));
        assert_eq!(out[0]["params"]["errorText"], json!("net::ERR_ABORTED"));
        assert_eq!(out[0]["params"]["canceled"], json!(true));
    }
}
