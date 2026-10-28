use anyhow::Result;

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

pub(super) struct BufferedFetchBody {
    request_id: String,
    action_session_id: Option<String>,
    host: Option<crate::conn::ProtocolPageHostScope>,
    body: crate::conn::CapturedBody,
}

impl BufferedFetchBody {
    fn capture(
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        request_id: &str,
    ) -> Result<Option<Self>, CommandOutputPlan> {
        let Some(pending) = pending_subresource_response_request_for_action_session(
            conn,
            owner,
            owner.session_id(),
            request_id,
        ) else {
            return Ok(None);
        };
        if pending.response_body_taken_as_stream {
            return Err(CommandOutputPlan::error(
                -32000,
                FetchBodyReadError::StreamTaken,
            ));
        }
        Ok(Some(Self {
            request_id: request_id.to_owned(),
            action_session_id: owner.session_id().map(str::to_owned),
            host: crate::conn::ProtocolPageHostScope::capture(conn, owner).ok(),
            body: pending.response_body,
        }))
    }

    fn is_current(&self, conn: &mut CdpConnection, owner: &CommandOwnerScope) -> bool {
        self.host.as_ref().is_none_or(|host| host.is_current(conn))
            && pending_subresource_response_request_for_action_session(
                conn,
                owner,
                self.action_session_id.as_deref(),
                &self.request_id,
            )
            .is_some_and(|pending| {
                !pending.response_body_taken_as_stream
                    && pending.response_body.same_storage(&self.body)
            })
    }
}

pub(super) enum PreparedFetchBodyRead {
    Transfer {
        request_id: String,
        read: Box<crate::conn::PendingDocumentBodyRead>,
        fallback: Result<Option<BufferedFetchBody>, CommandOutputPlan>,
        limit: usize,
    },
    Buffered {
        body: BufferedFetchBody,
        limit: usize,
    },
}

pub(super) enum MaterializedFetchBody {
    Transfer(serde_json::Value),
    Buffered(serde_json::Value),
}

pub(super) enum CompletedFetchBodyRead {
    Transfer {
        request_id: String,
        completed: Box<crate::conn::CompletedDocumentBodyRead<Option<MaterializedFetchBody>>>,
        fallback: Result<Option<BufferedFetchBody>, CommandOutputPlan>,
    },
    Buffered {
        body: BufferedFetchBody,
        result: Result<serde_json::Value>,
    },
}

impl PreparedFetchBodyRead {
    pub(super) async fn run(self) -> CompletedFetchBodyRead {
        match self {
            Self::Transfer {
                request_id,
                read,
                fallback,
                limit,
            } => {
                let fallback_body = fallback
                    .as_ref()
                    .ok()
                    .and_then(|body| body.as_ref())
                    .map(|body| body.body.clone());
                let completed = read
                    .materialize(limit)
                    .await
                    .map_result_in_worker(move |bytes| {
                        Ok(match bytes {
                            Some(bytes) => {
                                Some(MaterializedFetchBody::Transfer(response_body_result(bytes)))
                            }
                            None => match fallback_body {
                                Some(body) => Some(MaterializedFetchBody::Buffered(
                                    response_body_result(body.materialize_bytes_limited(limit)?),
                                )),
                                None => None,
                            },
                        })
                    })
                    .await;
                CompletedFetchBodyRead::Transfer {
                    request_id,
                    completed: Box::new(completed),
                    fallback,
                }
            }
            Self::Buffered { body, limit } => {
                let result = body
                    .body
                    .clone()
                    .process_in_worker(move |body| {
                        Ok(response_body_result(body.materialize_bytes_limited(limit)?))
                    })
                    .await;
                CompletedFetchBodyRead::Buffered { body, result }
            }
        }
    }
}

impl CompletedFetchBodyRead {
    fn finish(self, conn: &mut CdpConnection, owner: &CommandOwnerScope) -> CommandOutputPlan {
        match self {
            Self::Transfer {
                request_id,
                completed,
                fallback,
            } => {
                let Some(result) =
                    conn.finish_pending_fetch_response_body_read_for_owner(owner, *completed)
                else {
                    return CommandOutputPlan::error(-32000, FetchBodyReadError::RequestNotFound);
                };
                match result {
                    Ok(Some(MaterializedFetchBody::Transfer(value))) => {
                        CommandOutputPlan::result(value)
                    }
                    Ok(Some(MaterializedFetchBody::Buffered(value))) => match fallback {
                        Ok(Some(body)) if body.is_current(conn, owner) => {
                            CommandOutputPlan::result(value)
                        }
                        _ => CommandOutputPlan::error(-32000, FetchBodyReadError::RequestNotFound),
                    },
                    Ok(None) => match fallback {
                        Err(plan) => plan,
                        Ok(None) => pending_request_action_output_plan(conn, owner, &request_id),
                        Ok(Some(_)) => {
                            CommandOutputPlan::error(-32000, FetchBodyReadError::RequestNotFound)
                        }
                    },
                    Err(error) => CommandOutputPlan::error(-32000, format!("{error:#}")),
                }
            }
            Self::Buffered { body, result } => {
                if !body.is_current(conn, owner) {
                    return CommandOutputPlan::error(-32000, FetchBodyReadError::RequestNotFound);
                }
                match result {
                    Ok(value) => CommandOutputPlan::result(value),
                    Err(error) => CommandOutputPlan::error(-32000, error),
                }
            }
        }
    }
}

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
                FetchBodyReadError::InvalidParams,
            ));
        }
    };
    let read = conn.start_pending_fetch_response_body_read_for_owner(&owner, &params.request_id);
    let fallback = BufferedFetchBody::capture(conn, &owner, &params.request_id);
    let limit = conn.response_body_materialize_limit();
    let prepared = match read {
        Some(read) => PreparedFetchBodyRead::Transfer {
            request_id: params.request_id.clone(),
            read: Box::new(read),
            fallback,
            limit,
        },
        None => match fallback {
            Ok(Some(body)) => PreparedFetchBodyRead::Buffered { body, limit },
            Ok(None) => {
                return FetchCommandTaskStep::Complete(pending_request_action_output_plan(
                    conn,
                    &owner,
                    &params.request_id,
                ));
            }
            Err(plan) => return FetchCommandTaskStep::Complete(plan),
        },
    };
    FetchCommandTaskStep::Pending(PendingFetchCommandDispatch::new_for_owner(
        cmd.id,
        owner,
        PendingFetchCommandKind::GetResponseBody,
        PendingFetchCommandOperation::MaterializeResponseBody(Box::new(prepared)),
    ))
}

fn response_body_result(bytes: Vec<u8>) -> serde_json::Value {
    // Chromium returns interception bytes as Base64 even for text responses.
    let (body, base64_encoded) = encode_response_body(bytes);
    json!({ "body": body, "base64Encoded": base64_encoded })
}

fn encode_response_body(bytes: Vec<u8>) -> (String, bool) {
    (BASE64_STANDARD.encode(bytes), true)
}

pub(super) fn complete_get_response_body_from_transfer(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    completed: CompletedFetchCommandOperation,
    out: &mut super::FetchCommandOutput,
) {
    let plan = match completed {
        CompletedFetchCommandOperation::MaterializeResponseBody(completed) => {
            completed.finish(conn, owner)
        }
        _ => CommandOutputPlan::error(-32000, FetchBodyReadError::MissingCompletion),
    };
    out.extend_plan_as_command_response(plan);
}

#[derive(Debug, thiserror::Error)]
enum FetchBodyReadError {
    #[error("Missing response body completion")]
    MissingCompletion,
    #[error("RequestNotFound")]
    RequestNotFound,
    #[error("InvalidParams")]
    InvalidParams,
    #[error("Can only get response body on requests captured after headers received.")]
    StreamTaken,
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
            return CommandOutputPlan::error(-32000, format!("{message:#}"));
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
                return CommandOutputPlan::error(-32000, format!("{message:#}"));
            }
        }
    }
    pending_request_action_output_plan(conn, &owner, &params.request_id)
}

fn open_pending_response_navigation_body_stream(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    request_id: &str,
) -> Result<Option<String>> {
    conn.open_pending_fetch_response_body_stream_for_session_owner(session_id, request_id)
}

#[cfg(test)]
mod tests {
    use super::encode_response_body;

    #[test]
    fn encode_response_body_uses_base64_for_text_binary_and_empty_responses() {
        assert_eq!(
            encode_response_body(b"hello".to_vec()),
            ("aGVsbG8=".to_owned(), true)
        );
        assert_eq!(
            encode_response_body(vec![0, 255, b'a']),
            ("AP9h".to_owned(), true)
        );
        assert_eq!(encode_response_body(Vec::new()), (String::new(), true));
    }
}
