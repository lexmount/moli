use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use serde_json::{Value, json};

use super::{CapturedRequestBody, CapturedResponseBody, collectors::CollectedNetworkDataBody};
use crate::conn::{CdpConnection, Cmd, CommandOwnerScope};
use crate::devtools_runtime::{
    DevToolsCommandResult, DevToolsDisownNetworkDataCommand, DevToolsError, DevToolsErrorKind,
    DevToolsGetNetworkDataCommand, DevToolsNetworkDataBytesType, DevToolsNetworkDataCollectorId,
    DevToolsNetworkDataResult, DevToolsNetworkDataType, DevToolsRequestId,
};
use crate::domains::command_output::CommandOutputPlan;

fn network_events_enabled_for_session(
    slot: &super::TargetNetworkAgentState,
    session_id: Option<&str>,
    primary_session_id: Option<&str>,
) -> bool {
    slot.event_session_ids(session_id, primary_session_id)
        .iter()
        .any(|event_session_id| event_session_id.as_deref() == session_id)
}

pub(super) fn get_response_body_command_output_plan(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let request_id = match cmd
        .params
        .and_then(|params| params.get("requestId"))
        .and_then(Value::as_str)
    {
        Some(request_id) => request_id,
        None => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    if conn.network_owner_identity_for_owner(&owner).is_none() {
        return CommandOutputPlan::error(-31998, "BrowserContextNotLoaded");
    }
    let response_body_materialize_limit = conn.response_body_materialize_limit();
    let primary_session_id = conn.runtime_session_owner_primary_session_id(cmd.session_id);
    let Some(slot) = conn.network_agent_for_owner(&owner) else {
        return CommandOutputPlan::error(-32000, "No resource with given identifier found");
    };
    if !network_events_enabled_for_session(slot, cmd.session_id, primary_session_id.as_deref()) {
        return CommandOutputPlan::error(-32000, "No resource with given identifier found");
    };
    let Some(body) = slot.captured_response_body(request_id) else {
        return CommandOutputPlan::error(-32000, "No resource with given identifier found");
    };
    if !body.is_visible_to_session(cmd.session_id) {
        return CommandOutputPlan::error(-32000, "No resource with given identifier found");
    };
    let body = match body.body_bytes_limited(response_body_materialize_limit) {
        Ok(body) => body,
        Err(error) => {
            return CommandOutputPlan::error(-32000, error.to_string());
        }
    };
    let (body, base64_encoded) = encode_cdp_body(body);
    CommandOutputPlan::result(json!({ "body": body, "base64Encoded": base64_encoded }))
}

pub(super) fn get_request_post_data_command_output_plan(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let request_id = match cmd
        .params
        .and_then(|params| params.get("requestId"))
        .and_then(Value::as_str)
    {
        Some(request_id) => request_id,
        None => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    if conn.network_owner_identity_for_owner(&owner).is_none() {
        return CommandOutputPlan::error(-31998, "BrowserContextNotLoaded");
    }
    let body_materialize_limit = conn.response_body_materialize_limit();
    let primary_session_id = conn.runtime_session_owner_primary_session_id(cmd.session_id);
    let Some(slot) = conn.network_agent_for_owner(&owner) else {
        return CommandOutputPlan::error(-32000, "No resource with given id was found");
    };
    if !network_events_enabled_for_session(slot, cmd.session_id, primary_session_id.as_deref()) {
        return CommandOutputPlan::error(-32000, "No resource with given id was found");
    }
    let Some(body) = slot.captured_request_body(request_id) else {
        let request_is_known = slot
            .captured_response_body(request_id)
            .is_some_and(|body| body.is_visible_to_session(cmd.session_id));
        let message = if request_is_known {
            "No post data available for the request"
        } else {
            "No resource with given id was found"
        };
        return CommandOutputPlan::error(-32000, message);
    };
    if !body.is_visible_to_session(cmd.session_id) {
        return CommandOutputPlan::error(-32000, "No resource with given id was found");
    }
    let body = match body.body_bytes_limited(body_materialize_limit) {
        Ok(body) if !body.is_empty() => body,
        Ok(_) => {
            return CommandOutputPlan::error(-32000, "No post data available for the request");
        }
        Err(error) => {
            return CommandOutputPlan::error(-32000, error.to_string());
        }
    };
    let (post_data, base64_encoded) = encode_cdp_body(body);
    CommandOutputPlan::result(json!({ "postData": post_data, "base64Encoded": base64_encoded }))
}

pub(crate) fn get_network_data_result(
    conn: &mut CdpConnection,
    command: DevToolsGetNetworkDataCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    if command.disown {
        let Some(collector) = command.collector.as_ref() else {
            return Err(DevToolsError::new(
                DevToolsErrorKind::InvalidArgument,
                "network.getData disown requires collector",
            ));
        };
        conn.network_data_collectors
            .ensure_collector_exists(collector)?;
    }
    let collector = command.collector.clone();
    if let Some(collector) = collector.as_ref() {
        conn.network_data_collectors
            .ensure_collector_exists(collector)?;
    }
    let (body, body_was_collected_by_collector, body_collector_ids) = {
        let body = network_data_body(
            conn,
            &command.request_id,
            command.data_type,
            command.context.session_id.as_ref().map(|id| id.as_str()),
            collector.as_ref(),
        )?;
        let body_was_collected_by_collector = collector
            .as_ref()
            .is_some_and(|collector| body.was_collected_by(collector.as_str()));
        let body_collector_ids = body.collector_ids().clone();
        let body = body
            .body_bytes_limited(conn.response_body_materialize_limit())
            .map_err(|_| no_such_network_data())?;
        (body, body_was_collected_by_collector, body_collector_ids)
    };
    if let Some(collector) = collector.as_ref() {
        let collected = conn.network_data_collectors.body_is_collected(
            command.request_id.as_str(),
            command.data_type,
            collector,
            body_was_collected_by_collector,
        )?;
        if !collected {
            return Err(no_such_network_data());
        }
        if command.disown {
            conn.network_data_collectors.disown_data(
                command.request_id.as_str(),
                command.data_type,
                collector,
            )?;
        }
    } else if !conn.network_data_collectors.body_has_owned_collector(
        command.request_id.as_str(),
        command.data_type,
        &body_collector_ids,
    ) {
        return Err(no_such_network_data());
    }
    Ok(DevToolsCommandResult::NetworkData(network_data_result(
        body,
    )))
}

pub(crate) fn disown_network_data_result(
    conn: &mut CdpConnection,
    command: DevToolsDisownNetworkDataCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    conn.network_data_collectors
        .ensure_collector_exists(&command.collector_id)?;
    let body_was_collected_by_collector = {
        let body = network_data_body(
            conn,
            &command.request_id,
            command.data_type,
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some(&command.collector_id),
        )?;
        let body_was_collected_by_collector = body.was_collected_by(command.collector_id.as_str());
        body.body_bytes_limited(conn.response_body_materialize_limit())
            .map_err(|_| no_such_network_data())?;
        body_was_collected_by_collector
    };
    let collected = conn.network_data_collectors.body_is_collected(
        command.request_id.as_str(),
        command.data_type,
        &command.collector_id,
        body_was_collected_by_collector,
    )?;
    if !collected {
        return Err(no_such_network_data());
    }
    conn.network_data_collectors.disown_data(
        command.request_id.as_str(),
        command.data_type,
        &command.collector_id,
    )?;
    Ok(DevToolsCommandResult::Empty)
}

enum NetworkDataBody<'a> {
    Request(&'a CapturedRequestBody),
    Response(&'a CapturedResponseBody),
    Collected(&'a CollectedNetworkDataBody),
}

impl NetworkDataBody<'_> {
    fn body_bytes_limited(&self, limit: usize) -> anyhow::Result<Vec<u8>> {
        match self {
            Self::Request(body) => body.body_bytes_limited(limit),
            Self::Response(body) => body.body_bytes_limited(limit),
            Self::Collected(body) => body.body_bytes_limited(limit),
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
    DevToolsError::new(DevToolsErrorKind::NoSuchNetworkData, "no such network data")
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
