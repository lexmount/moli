use crate::conn::{CdpConnection, Cmd, CommandOwnerScope};
use crate::domains::command_output::CommandOutputPlan;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use serde_json::json;

use super::params::RequestIdParam;
use super::state::{
    mark_pending_subresource_response_body_taken_as_stream_for_action_session,
    pending_request_action_output_plan, pending_subresource_response_request_for_action_session,
};
use super::{
    CompletedFetchCommandOperation, FetchCommandTaskStep, PendingFetchCommandDispatch,
    PendingFetchCommandKind, PendingFetchCommandOperation,
};

const RESPONSE_BODY_NOT_AVAILABLE_AFTER_STREAM_TAKEN: &str =
    "Can only get response body on requests captured after headers received.";

pub(super) fn start_get_response_body_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> FetchCommandTaskStep {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let params: RequestIdParam = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return FetchCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    if let Some(transfer) = conn
        .take_pending_fetch_response_transfer_for_body_read_for_owner(&owner, &params.request_id)
    {
        return FetchCommandTaskStep::Pending(PendingFetchCommandDispatch::new_for_owner(
            cmd.id,
            owner,
            PendingFetchCommandKind::GetResponseBody,
            PendingFetchCommandOperation::MaterializeResponseBody {
                request_id: params.request_id,
                transfer: Box::new(transfer),
                limit: conn.response_body_materialize_limit(),
            },
        ));
    }
    FetchCommandTaskStep::Complete(get_response_body_without_transfer_command_output_plan(
        conn,
        &owner,
        &params.request_id,
    ))
}

fn get_response_body_without_transfer_command_output_plan(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    request_id: &str,
) -> CommandOutputPlan {
    if let Some(pending) = pending_subresource_response_request_for_action_session(
        conn,
        owner,
        owner.session_id(),
        request_id,
    ) {
        if pending.response_body_taken_as_stream {
            return CommandOutputPlan::error(
                -32000,
                RESPONSE_BODY_NOT_AVAILABLE_AFTER_STREAM_TAKEN,
            );
        }
        let body = pending.response_body;
        let limit = conn.response_body_materialize_limit();
        let bytes = match body.materialize_bytes_limited(limit) {
            Ok(bytes) => bytes,
            Err(error) => {
                return CommandOutputPlan::error(-32000, error.to_string());
            }
        };
        return response_body_command_output_plan(bytes);
    }
    pending_request_action_output_plan(conn, owner, request_id)
}

fn encode_response_body(bytes: Vec<u8>) -> (String, bool) {
    match String::from_utf8(bytes) {
        Ok(body) => (body, false),
        Err(error) => (BASE64_STANDARD.encode(error.into_bytes()), true),
    }
}

fn response_body_command_output_plan(bytes: Vec<u8>) -> CommandOutputPlan {
    let (body, base64_encoded) = encode_response_body(bytes);
    CommandOutputPlan::result(json!({
        "body": body,
        "base64Encoded": base64_encoded
    }))
}

pub(super) fn complete_get_response_body_from_transfer(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    completed: CompletedFetchCommandOperation,
    out: &mut super::FetchCommandOutput,
) {
    let CompletedFetchCommandOperation::MaterializeResponseBody { request_id, result } = completed
    else {
        out.extend_plan_as_command_response(CommandOutputPlan::error(
            -32000,
            "Missing response body completion",
        ));
        return;
    };
    let plan = match *result {
        Ok((Some(bytes), transfer)) => {
            conn.register_pending_fetch_response_transfer_for_owner(owner, request_id, transfer);
            response_body_command_output_plan(bytes)
        }
        Ok((None, transfer)) => {
            conn.register_pending_fetch_response_transfer_for_owner(
                owner,
                request_id.clone(),
                transfer,
            );
            get_response_body_without_transfer_command_output_plan(conn, owner, &request_id)
        }
        Err((message, transfer)) => {
            conn.register_pending_fetch_response_transfer_for_owner(owner, request_id, transfer);
            CommandOutputPlan::error(-32000, message)
        }
    };
    out.extend_plan_as_command_response(plan);
}

pub(super) fn take_response_body_as_stream_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let params: RequestIdParam = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };
    match open_pending_response_navigation_body_stream(conn, cmd.session_id, &params.request_id) {
        Ok(Some(handle)) => {
            return CommandOutputPlan::result(json!({ "stream": handle }));
        }
        Ok(None) => {}
        Err(message) => {
            return CommandOutputPlan::error(-32000, message);
        }
    }
    if let Some(pending) = pending_subresource_response_request_for_action_session(
        conn,
        &owner,
        cmd.session_id,
        &params.request_id,
    ) {
        if pending.response_body_taken_as_stream {
            return CommandOutputPlan::error(
                -32000,
                RESPONSE_BODY_NOT_AVAILABLE_AFTER_STREAM_TAKEN,
            );
        }
        match conn
            .open_io_stream_body_source_for_session_owner(cmd.session_id, pending.response_body)
        {
            Ok(handle) => {
                mark_pending_subresource_response_body_taken_as_stream_for_action_session(
                    conn,
                    &owner,
                    cmd.session_id,
                    &params.request_id,
                );
                return CommandOutputPlan::result(json!({ "stream": handle }));
            }
            Err(message) => {
                return CommandOutputPlan::error(-32000, message);
            }
        }
    }
    pending_request_action_output_plan(conn, &owner, &params.request_id)
}

fn open_pending_response_navigation_body_stream(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    request_id: &str,
) -> Result<Option<String>, String> {
    conn.open_pending_fetch_response_body_stream_for_session_owner(session_id, request_id)
}

#[cfg(test)]
mod tests {
    use super::encode_response_body;

    #[test]
    fn encode_response_body_preserves_utf8_and_base64_encodes_binary() {
        assert_eq!(
            encode_response_body(b"hello".to_vec()),
            ("hello".to_owned(), false)
        );
        assert_eq!(
            encode_response_body(vec![0, 255, b'a']),
            ("AP9h".to_owned(), true)
        );
    }
}
