use crate::conn::{BackgroundProtocolEvent, CdpConnection, build_event};
use crate::conn::{
    FetchAuthChallenge, NavigationDispatchState, PendingFetchAuthNavigation,
    PendingFetchNavigation, PendingSubresourceFetchAuthRequest,
    PendingSubresourceFetchResponseRequest,
};
use crate::devtools_runtime::{
    AutomationEvent, DevToolsFrameId, DevToolsLoaderId, DevToolsNetworkInterceptId,
    DevToolsNetworkResourceType, DevToolsRequestId, DevToolsTargetId, NetworkAuthChallengeEvent,
    NetworkRequestEvent,
};
use crate::domains::network;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use http::header::{HeaderName, HeaderValue};
use moli_cookie_jar::StoredCookieQueryReport;
use moli_core::page::{
    SubresourceAuthChallenge, SubresourceAuthCredentials, extract_subresource_auth_challenge,
    subresource_auth_credentials_for_challenge,
};
use serde_json::Value;
use url::Url;

use super::params::HeaderEntry;

pub(crate) fn decode_base64_to_string(body: &str) -> Result<String, ()> {
    let decoded = decode_base64_bytes(body)?;
    String::from_utf8(decoded).map_err(|_| ())
}

pub(super) fn response_headers_from_params(
    response_headers: Option<Vec<HeaderEntry>>,
    binary_response_headers: Option<impl AsRef<str>>,
) -> Result<Vec<(String, String)>, ()> {
    if let Some(binary_response_headers) = binary_response_headers {
        parse_binary_response_headers(binary_response_headers.as_ref())
    } else {
        Ok(response_headers
            .unwrap_or_default()
            .into_iter()
            .map(|header| (header.name, header.value))
            .collect())
    }
}

pub(super) fn response_headers_with_presence_from_params(
    response_headers: Option<Vec<HeaderEntry>>,
    binary_response_headers: Option<impl AsRef<str>>,
) -> Result<Option<Vec<(String, String)>>, ()> {
    if let Some(binary_response_headers) = binary_response_headers {
        parse_binary_response_headers(binary_response_headers.as_ref()).map(Some)
    } else {
        Ok(response_headers.map(|headers| {
            headers
                .into_iter()
                .map(|header| (header.name, header.value))
                .collect()
        }))
    }
}

pub(crate) fn parse_binary_response_headers(encoded: &str) -> Result<Vec<(String, String)>, ()> {
    let decoded = decode_base64_bytes(encoded)?;
    let mut headers = Vec::new();
    for entry in decoded.split(|byte| *byte == b'\0') {
        if entry.is_empty() {
            continue;
        }
        let Some(separator) = entry.iter().position(|byte| *byte == b':') else {
            return Err(());
        };
        let (name, value) = entry.split_at(separator);
        let name = trim_ascii(name);
        let value = trim_ascii_start(&value[1..]);
        if HeaderName::from_bytes(name).is_err() || HeaderValue::from_bytes(value).is_err() {
            return Err(());
        }
        let name = String::from_utf8_lossy(name).trim().to_owned();
        if name.is_empty() {
            return Err(());
        }
        let value = String::from_utf8_lossy(value).to_string();
        headers.push((name, value));
    }
    Ok(headers)
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    trim_ascii_start(trim_ascii_end(bytes))
}

fn trim_ascii_start(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    &bytes[start..]
}

fn trim_ascii_end(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map(|index| index + 1)
        .unwrap_or(0);
    &bytes[..end]
}

pub(crate) fn decode_base64_bytes(input: &str) -> Result<Vec<u8>, ()> {
    let mut compact = input
        .bytes()
        .filter(|byte| !matches!(byte, b'=' | b' ' | b'\n' | b'\r' | b'\t'))
        .collect::<Vec<_>>();
    if compact.len() % 4 == 1 {
        return Err(());
    }
    while compact.len() % 4 != 0 {
        compact.push(b'=');
    }
    BASE64_STANDARD.decode(compact).map_err(|_| ())
}

#[cfg(test)]
pub(crate) fn emit_auth_required(
    out: &mut Vec<Value>,
    session_id: Option<&str>,
    pending: &PendingFetchNavigation,
    challenge: &FetchAuthChallenge,
    request_cookie_report: Option<&StoredCookieQueryReport>,
    blocked_intercepts: &[DevToolsNetworkInterceptId],
) {
    emit_navigation_auth_required(
        out,
        session_id,
        &pending.fetch_request_id,
        &pending.navigation,
        challenge,
        request_cookie_report,
        blocked_intercepts,
    );
}

pub(crate) fn pending_fetch_auth_navigation_required_event(
    session_id: Option<&str>,
    pending: &PendingFetchAuthNavigation,
    blocked_intercepts: &[DevToolsNetworkInterceptId],
) -> BackgroundProtocolEvent {
    let (payload, event) =
        pending_fetch_auth_navigation_required_parts(pending, blocked_intercepts);
    BackgroundProtocolEvent::immediate_automation_event(
        build_event("Fetch.authRequired", payload, session_id),
        event,
    )
}

#[cfg(test)]
fn emit_navigation_auth_required(
    out: &mut Vec<Value>,
    session_id: Option<&str>,
    fetch_request_id: &str,
    navigation: &NavigationDispatchState,
    challenge: &FetchAuthChallenge,
    request_cookie_report: Option<&StoredCookieQueryReport>,
    blocked_intercepts: &[DevToolsNetworkInterceptId],
) {
    let (_, event) = navigation_auth_required_parts(
        fetch_request_id,
        navigation,
        challenge,
        request_cookie_report,
        blocked_intercepts,
    );
    let mut events = Vec::new();
    network::emit_cdp_network_automation_event(&mut events, event, session_id);
    out.extend(
        events
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message),
    );
}

fn pending_fetch_auth_navigation_required_parts(
    pending: &PendingFetchAuthNavigation,
    blocked_intercepts: &[DevToolsNetworkInterceptId],
) -> (Value, AutomationEvent) {
    navigation_auth_required_parts(
        &pending.fetch_request_id,
        &pending.navigation,
        &pending.challenge,
        pending.request_cookie_report.as_ref(),
        blocked_intercepts,
    )
}

fn navigation_auth_required_parts(
    fetch_request_id: &str,
    navigation: &NavigationDispatchState,
    challenge: &FetchAuthChallenge,
    request_cookie_report: Option<&StoredCookieQueryReport>,
    blocked_intercepts: &[DevToolsNetworkInterceptId],
) -> (Value, AutomationEvent) {
    let network_event = NetworkRequestEvent {
        target_id: DevToolsTargetId::from(navigation.frame_id.as_str()),
        frame_id: Some(DevToolsFrameId::from(navigation.frame_id.as_str())),
        request_id: DevToolsRequestId::from(fetch_request_id),
        loader_id: None,
        url: navigation.requested_url.as_str().to_owned(),
        document_url: Some(navigation.requested_url.as_str().to_owned()),
        method: Some(navigation.request_method.clone()),
        request_headers: navigation.request_headers.clone(),
        request_body: navigation.request_body.clone(),
        request_initiator_type: None,
        bidi_request_initiator_type: None,
        redirect_response: None,
        redirect_has_extra_info: false,
        request_cookie_report: request_cookie_report.cloned(),
        resource_type: Some(DevToolsNetworkResourceType::Document),
        timestamp: Some(navigation.timestamp),
        wall_time: Some(navigation.timestamp),
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
        network_id: navigation
            .request_id
            .as_deref()
            .map(DevToolsRequestId::from),
        auth_challenge: Some(NetworkAuthChallengeEvent {
            origin: challenge.origin.clone(),
            source: challenge.source.clone(),
            scheme: challenge.scheme.clone(),
            realm: challenge.realm.clone(),
        }),
    };
    let payload = network::fetch_auth_required_params(&network_event);
    (payload, AutomationEvent::NetworkAuthRequired(network_event))
}

pub(crate) fn pending_subresource_auth_required_event(
    session_id: Option<&str>,
    request_id: &str,
    pending: &PendingSubresourceFetchAuthRequest,
    blocked_intercepts: &[DevToolsNetworkInterceptId],
) -> BackgroundProtocolEvent {
    let (payload, event) =
        pending_subresource_auth_required_parts(request_id, pending, blocked_intercepts);
    BackgroundProtocolEvent::immediate_automation_event(
        build_event("Fetch.authRequired", payload, session_id),
        event,
    )
}

fn pending_subresource_auth_required_parts(
    request_id: &str,
    pending: &PendingSubresourceFetchAuthRequest,
    blocked_intercepts: &[DevToolsNetworkInterceptId],
) -> (Value, AutomationEvent) {
    let network_event = NetworkRequestEvent {
        target_id: DevToolsTargetId::from(pending.frame_id.as_str()),
        frame_id: Some(DevToolsFrameId::from(pending.frame_id.as_str())),
        request_id: DevToolsRequestId::from(request_id),
        loader_id: None,
        url: pending.url.as_str().to_owned(),
        document_url: Some(pending.document_url.as_str().to_owned()),
        method: Some(pending.method.clone()),
        request_headers: pending.request_headers.clone(),
        request_body: pending.request_body.clone(),
        request_initiator_type: None,
        bidi_request_initiator_type: None,
        redirect_response: None,
        redirect_has_extra_info: false,
        request_cookie_report: pending.request_cookie_report.clone(),
        resource_type: Some(DevToolsNetworkResourceType::from_fetch_interception_type(
            pending.resource_type,
        )),
        timestamp: None,
        wall_time: None,
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
        network_id: Some(DevToolsRequestId::from(pending.network_request_id.as_str())),
        auth_challenge: Some(NetworkAuthChallengeEvent {
            origin: pending.challenge.origin.clone(),
            source: pending.challenge.source.clone(),
            scheme: pending.challenge.scheme.clone(),
            realm: pending.challenge.realm.clone(),
        }),
    };
    let payload = network::fetch_auth_required_params(&network_event);
    (payload, AutomationEvent::NetworkAuthRequired(network_event))
}

pub(crate) fn navigation_response_stage_request_paused_event(
    conn: &CdpConnection,
    session_id: Option<&str>,
    fetch_request_id: &str,
    navigation: &NavigationDispatchState,
    final_url: &url::Url,
    request_cookie_report: Option<&StoredCookieQueryReport>,
    response_status: u16,
    response_headers: &[(String, String)],
) -> BackgroundProtocolEvent {
    let (payload, event) = navigation_response_stage_request_paused_parts(
        fetch_request_id,
        navigation,
        final_url,
        request_cookie_report,
        response_status,
        response_headers,
        navigation_blocked_intercepts(
            conn,
            navigation,
            crate::conn::FetchRequestStage::Response,
            final_url,
        ),
    );
    BackgroundProtocolEvent::immediate_automation_event(
        build_event("Fetch.requestPaused", payload, session_id),
        event,
    )
}

fn navigation_response_stage_request_paused_parts(
    fetch_request_id: &str,
    navigation: &NavigationDispatchState,
    final_url: &url::Url,
    request_cookie_report: Option<&StoredCookieQueryReport>,
    response_status: u16,
    response_headers: &[(String, String)],
    blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
) -> (Value, AutomationEvent) {
    let network_event = NetworkRequestEvent {
        target_id: DevToolsTargetId::from(navigation.frame_id.as_str()),
        frame_id: Some(DevToolsFrameId::from(navigation.frame_id.as_str())),
        request_id: DevToolsRequestId::from(fetch_request_id),
        loader_id: Some(DevToolsLoaderId::from(navigation.loader_id.as_str())),
        url: final_url.as_str().to_owned(),
        document_url: None,
        method: Some(navigation.request_method.clone()),
        request_headers: navigation.request_headers.clone(),
        request_body: navigation.request_body.clone(),
        request_initiator_type: None,
        bidi_request_initiator_type: None,
        redirect_response: None,
        redirect_has_extra_info: false,
        request_cookie_report: request_cookie_report.cloned(),
        resource_type: Some(DevToolsNetworkResourceType::Document),
        timestamp: None,
        wall_time: None,
        status: Some(response_status),
        status_text: None,
        response_headers: response_headers.to_vec(),
        response_mime_type: None,
        response_protocol: None,
        encoded_data_length: None,
        from_cache: false,
        has_extra_info: false,
        error_text: None,
        loading_failed_canceled: false,
        blocked_intercepts,
        fetch_request_id: None,
        network_id: navigation
            .request_id
            .as_deref()
            .map(DevToolsRequestId::from),
        auth_challenge: None,
    };
    let payload = network::fetch_request_paused_params(&network_event);
    (payload, AutomationEvent::RequestPaused(network_event))
}

pub(crate) fn pending_subresource_response_stage_request_paused_event(
    session_id: Option<&str>,
    request_id: &str,
    pending: &PendingSubresourceFetchResponseRequest,
    blocked_intercepts: &[DevToolsNetworkInterceptId],
) -> BackgroundProtocolEvent {
    let (payload, event) = pending_subresource_response_stage_request_paused_parts(
        request_id,
        pending,
        blocked_intercepts,
    );
    BackgroundProtocolEvent::immediate_automation_event(
        build_event("Fetch.requestPaused", payload, session_id),
        event,
    )
}

fn pending_subresource_response_stage_request_paused_parts(
    request_id: &str,
    pending: &PendingSubresourceFetchResponseRequest,
    blocked_intercepts: &[DevToolsNetworkInterceptId],
) -> (Value, AutomationEvent) {
    let network_event = NetworkRequestEvent {
        target_id: DevToolsTargetId::from(pending.frame_id.as_str()),
        frame_id: Some(DevToolsFrameId::from(pending.frame_id.as_str())),
        request_id: DevToolsRequestId::from(request_id),
        loader_id: None,
        url: pending.url.as_str().to_owned(),
        document_url: Some(pending.document_url.as_str().to_owned()),
        method: Some(pending.method.clone()),
        request_headers: pending.request_headers.clone(),
        request_body: pending.request_body.clone(),
        request_initiator_type: None,
        bidi_request_initiator_type: None,
        redirect_response: None,
        redirect_has_extra_info: false,
        request_cookie_report: pending.request_cookie_report.clone(),
        resource_type: Some(DevToolsNetworkResourceType::from_fetch_interception_type(
            pending.resource_type,
        )),
        timestamp: None,
        wall_time: None,
        status: Some(pending.response_status),
        status_text: None,
        response_headers: pending.response_headers.clone(),
        response_mime_type: None,
        response_protocol: None,
        encoded_data_length: None,
        from_cache: false,
        has_extra_info: false,
        error_text: None,
        loading_failed_canceled: false,
        blocked_intercepts: blocked_intercepts.to_vec(),
        fetch_request_id: None,
        network_id: Some(DevToolsRequestId::from(pending.network_request_id.as_str())),
        auth_challenge: None,
    };
    let payload = network::fetch_request_paused_params(&network_event);
    (payload, AutomationEvent::RequestPaused(network_event))
}

pub(crate) fn extract_auth_challenge(headers: &[(String, String)]) -> Option<FetchAuthChallenge> {
    extract_subresource_auth_challenge(headers).map(|challenge| FetchAuthChallenge {
        origin: String::new(),
        source: challenge.source,
        scheme: challenge.scheme,
        realm: challenge.realm,
    })
}

pub(crate) fn populate_auth_challenge_origin(
    conn: &CdpConnection,
    session_id: Option<&str>,
    request_url: &Url,
    challenge: &mut FetchAuthChallenge,
) {
    let origin_url = if challenge.source == "Proxy" {
        conn.http_proxy_for_session_owner_owned(session_id)
            .and_then(|proxy| Url::parse(&proxy).ok())
            .or_else(|| Some(request_url.clone()))
    } else {
        Some(request_url.clone())
    };
    challenge.origin = origin_url
        .map(|url| url.origin().ascii_serialization())
        .filter(|origin| origin != "null")
        .unwrap_or_default();
}

pub(crate) fn request_auth_for_challenge(
    challenge: &FetchAuthChallenge,
    username: &str,
    password: &str,
) -> Option<SubresourceAuthCredentials> {
    subresource_auth_credentials_for_challenge(
        &SubresourceAuthChallenge {
            source: challenge.source.clone(),
            scheme: challenge.scheme.clone(),
            realm: challenge.realm.clone(),
        },
        username,
        password,
    )
}

#[cfg(test)]
pub(crate) fn encode_basic_auth(username: &str, password: &str) -> String {
    let input = format!("{username}:{password}");
    BASE64_STANDARD.encode(input)
}

pub(crate) fn request_paused_background_event(
    conn: &CdpConnection,
    session_id: Option<&str>,
    pending: &PendingFetchNavigation,
) -> BackgroundProtocolEvent {
    let network_event = NetworkRequestEvent {
        target_id: DevToolsTargetId::from(pending.navigation.frame_id.as_str()),
        frame_id: Some(DevToolsFrameId::from(pending.navigation.frame_id.as_str())),
        request_id: DevToolsRequestId::from(pending.fetch_request_id.as_str()),
        loader_id: Some(DevToolsLoaderId::from(
            pending.navigation.loader_id.as_str(),
        )),
        url: pending.navigation.requested_url.as_str().to_owned(),
        document_url: Some(pending.navigation.requested_url.as_str().to_owned()),
        method: Some(pending.navigation.request_method.clone()),
        request_headers: pending.navigation.request_headers.clone(),
        request_body: pending.navigation.request_body.clone(),
        request_initiator_type: None,
        bidi_request_initiator_type: None,
        redirect_response: None,
        redirect_has_extra_info: false,
        request_cookie_report: pending.request_cookie_report.clone(),
        resource_type: Some(DevToolsNetworkResourceType::Document),
        timestamp: Some(pending.navigation.timestamp),
        wall_time: Some(pending.navigation.timestamp),
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
        blocked_intercepts: navigation_blocked_intercepts(
            conn,
            &pending.navigation,
            crate::conn::FetchRequestStage::Request,
            &pending.navigation.requested_url,
        ),
        fetch_request_id: None,
        network_id: pending
            .navigation
            .request_id
            .as_deref()
            .map(DevToolsRequestId::from),
        auth_challenge: None,
    };
    let payload = network::fetch_request_paused_params(&network_event);
    let message = build_event("Fetch.requestPaused", payload, session_id);
    BackgroundProtocolEvent::immediate_automation_event(
        message,
        AutomationEvent::RequestPaused(network_event),
    )
}

fn navigation_blocked_intercepts(
    conn: &CdpConnection,
    navigation: &NavigationDispatchState,
    stage: crate::conn::FetchRequestStage,
    url: &Url,
) -> Vec<DevToolsNetworkInterceptId> {
    conn.target_fetch_subresource_interception_snapshot_for_owner(&navigation.owner)
        .map(|snapshot| {
            snapshot.matching_network_intercepts(stage, DevToolsNetworkResourceType::Document, url)
        })
        .unwrap_or_default()
}
