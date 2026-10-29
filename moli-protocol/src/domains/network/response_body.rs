use anyhow::Result;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use serde_json::{Value, json};

use super::{CapturedRequestBody, CapturedResponseBody, collectors::CollectedNetworkDataBody};
use crate::automation::{
    AutomationResult, DevToolsError, DevToolsErrorKind, DevToolsNetworkDataBytesType,
    DevToolsNetworkDataCollectorId, DevToolsNetworkDataResult, DevToolsNetworkDataType,
    DevToolsRequestId,
};
use crate::conn::{CapturedBody, CdpConnection, Cmd, TargetRuntimeSlot};
use crate::domains::command_output::CommandOutputPlan;

fn network_events_enabled_for_session(
    slot: &TargetRuntimeSlot,
    session_id: Option<&str>,
    primary_session_id: Option<&str>,
) -> bool {
    slot.network_event_session_ids(session_id, primary_session_id)
        .iter()
        .any(|event_session_id| event_session_id.as_deref() == session_id)
}

enum NetworkDataBody<'a> {
    Request(&'a CapturedRequestBody),
    Response(&'a CapturedResponseBody),
    Collected(&'a CollectedNetworkDataBody),
}

impl NetworkDataBody<'_> {
    fn captured_body(&self) -> Result<&CapturedBody> {
        match self {
            Self::Request(body) => Ok(body.captured_body()),
            Self::Response(body) => body.captured_body(),
            Self::Collected(body) => Ok(body.captured_body()),
        }
    }

    fn was_collected_by(&self, collector_id: &str) -> bool {
        match self {
            Self::Request(body) => body.was_collected_by(collector_id),
            Self::Response(body) => body.was_collected_by(collector_id),
            Self::Collected(body) => body.was_collected_by(collector_id),
        }
    }

    fn collector_ids(&self) -> &std::collections::HashSet<String> {
        match self {
            Self::Request(body) => body.collector_ids(),
            Self::Response(body) => body.collector_ids(),
            Self::Collected(body) => body.collector_ids(),
        }
    }
}

fn network_data_body<'a>(
    conn: &'a CdpConnection,
    request_id: &DevToolsRequestId,
    data_type: DevToolsNetworkDataType,
    session_id: Option<&str>,
    collector: Option<&DevToolsNetworkDataCollectorId>,
) -> Result<NetworkDataBody<'a>, DevToolsError> {
    let collected_body = || {
        conn.network_data_collectors
            .collected_body(request_id.as_str(), data_type)
            .filter(|body| {
                collector.is_none_or(|collector| body.was_collected_by(collector.as_str()))
            })
            .map(NetworkDataBody::Collected)
    };
    if let Some(body) = collected_body() {
        return Ok(body);
    }
    let body = match data_type {
        DevToolsNetworkDataType::Request => conn
            .captured_request_body_for_bidi_network_data(request_id.as_str(), session_id)
            .map(NetworkDataBody::Request),
        DevToolsNetworkDataType::Response => conn
            .captured_response_body_for_bidi_network_data(request_id.as_str(), session_id)
            .map(NetworkDataBody::Response),
    };
    body.or_else(collected_body)
        .ok_or_else(no_such_network_data)
}

fn no_such_network_data() -> DevToolsError {
    DevToolsError::new(
        DevToolsErrorKind::NoSuchNetworkData,
        automation::NetworkDataAccessError::NoSuchData,
    )
}

fn encode_cdp_body(bytes: Vec<u8>) -> (String, bool) {
    match String::from_utf8(bytes) {
        Ok(body) => (body, false),
        Err(error) => (BASE64_STANDARD.encode(error.into_bytes()), true),
    }
}

fn network_data_result(bytes: Vec<u8>) -> DevToolsNetworkDataResult {
    match String::from_utf8(bytes) {
        Ok(value) => DevToolsNetworkDataResult {
            bytes_type: DevToolsNetworkDataBytesType::String,
            value,
        },
        Err(error) => DevToolsNetworkDataResult {
            bytes_type: DevToolsNetworkDataBytesType::Base64,
            value: BASE64_STANDARD.encode(error.into_bytes()),
        },
    }
}

mod automation;
pub(crate) use automation::PreparedNetworkDataAccess;

mod cdp;
pub(super) use cdp::{
    CompletedCdpBodyRead, PreparedCdpBodyRead, start_request_post_data, start_response_body,
};
