use chromiumoxide_cdp::cdp::browser_protocol::network::LoadNetworkResourceParams;
use moli_url_policy::BrowserUrlScheme;
use serde_json::{Map, Value, json};
use url::Url;

use crate::{
    conn::{CapturedBody, CdpConnection, Cmd, CommandOwnerScope},
    domains::command_output::CommandOutputPlan,
};

use super::{
    CompletedNetworkCommandDispatch, CompletedNetworkCommandWork, NetworkCommandTaskStep,
    PendingNetworkCommandDispatch, PendingNetworkCommandKind, PendingNetworkCommandWork,
    headers_as_json_object,
};

const ERR_FAILED: i32 = -2;
const ERR_HTTP_RESPONSE_CODE_FAILURE: i32 = -379;

pub(super) fn start_load_network_resource_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> NetworkCommandTaskStep {
    let params: LoadNetworkResourceParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return NetworkCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    let url = match Url::parse(&params.url) {
        Ok(url) => url,
        Err(_) => {
            return NetworkCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "The url must be valid",
            ));
        }
    };
    if BrowserUrlScheme::from_url(&url).is_local() {
        return NetworkCommandTaskStep::Complete(CommandOutputPlan::error(
            -32602,
            "Unsupported URL scheme",
        ));
    }
    let Some(frame_id) = params.frame_id.map(String::from) else {
        return NetworkCommandTaskStep::Complete(CommandOutputPlan::error(
            -32602,
            "Parameter frameId must be provided for frame targets",
        ));
    };
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let pending = match conn.loaded_page_mut_for_protocol_access(cmd.session_id) {
        Ok(page) => page.start_prepare_network_resource_load(
            frame_id,
            url,
            params.options.disable_cache,
            params.options.include_credentials,
        ),
        Err(message) if message == "NoDocumentLoaded" => {
            return NetworkCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "Frame not found",
            ));
        }
        Err(message) => {
            return NetworkCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
        }
    };
    match pending {
        Ok(pending) => NetworkCommandTaskStep::Pending(PendingNetworkCommandDispatch {
            command_id: cmd.id,
            owner_scope,
            kind: PendingNetworkCommandKind::PrepareNetworkResourceLoad,
            pending: PendingNetworkCommandWork::page(pending),
        }),
        Err(error) => {
            NetworkCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error.to_string()))
        }
    }
}

pub(super) fn complete_network_resource_preparation(
    conn: &mut CdpConnection,
    completed: CompletedNetworkCommandDispatch,
) -> NetworkCommandTaskStep {
    let owner_scope = completed.owner_scope.clone();
    let completion = match completed.completed {
        CompletedNetworkCommandWork::Page {
            completed: Ok(completion),
            ..
        } => *completion,
        CompletedNetworkCommandWork::Page {
            completed: Err(error),
            ..
        } => {
            return NetworkCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error));
        }
        CompletedNetworkCommandWork::Resource(_) => {
            return invalid_completion_step();
        }
    };
    let preparation = match conn
        .loaded_page_mut_for_protocol_access_for_route(
            owner_scope.session_id(),
            owner_scope.session_owner_route(),
        )
        .and_then(|page| {
            if page.renderer_agent_attachment_id() != completion.renderer_agent_attachment_id() {
                return Err("Document changed while preparing the network resource load".to_owned());
            }
            page.finish_prepare_network_resource_load(completion)
                .map_err(|error| error.to_string())
        }) {
        Ok(preparation) => preparation,
        Err(message) => {
            return NetworkCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
        }
    };
    match preparation {
        moli_core::page::RendererNetworkResourceLoadPreparation::Ready(pending) => {
            NetworkCommandTaskStep::Pending(PendingNetworkCommandDispatch {
                command_id: completed.command_id,
                owner_scope,
                kind: PendingNetworkCommandKind::FetchNetworkResource,
                pending: PendingNetworkCommandWork::Resource(pending),
            })
        }
        moli_core::page::RendererNetworkResourceLoadPreparation::FrameNotFound => {
            NetworkCommandTaskStep::Complete(CommandOutputPlan::error(-32602, "Frame not found"))
        }
        moli_core::page::RendererNetworkResourceLoadPreparation::CspViolation => {
            NetworkCommandTaskStep::Complete(CommandOutputPlan::error(-32000, "CSP violation"))
        }
        moli_core::page::RendererNetworkResourceLoadPreparation::UnsupportedUrlScheme => {
            NetworkCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "Unsupported URL scheme",
            ))
        }
    }
}

pub(super) fn complete_network_resource_fetch(
    conn: &mut CdpConnection,
    completed: CompletedNetworkCommandDispatch,
) -> CommandOutputPlan {
    let owner_scope = completed.owner_scope.clone();
    let outcome = match completed.completed {
        CompletedNetworkCommandWork::Resource(outcome) => outcome,
        CompletedNetworkCommandWork::Page { .. } => {
            return invalid_completion_plan();
        }
    };
    match outcome {
        moli_core::page::RendererNetworkResourceLoadOutcome::FailedBeforeResponse(_) => {
            failed_resource_result(ERR_FAILED, "net::ERR_FAILED", None, None)
        }
        moli_core::page::RendererNetworkResourceLoadOutcome::Response(response) => {
            let response = *response;
            let headers = headers_as_json_object(&response.headers);
            if response.completion_error.is_some() {
                return failed_resource_result(
                    ERR_FAILED,
                    "net::ERR_FAILED",
                    Some(response.status),
                    Some(headers),
                );
            }
            if !(200..300).contains(&response.status) {
                return failed_resource_result(
                    ERR_HTTP_RESPONSE_CODE_FAILURE,
                    "net::ERR_HTTP_RESPONSE_CODE_FAILURE",
                    Some(response.status),
                    Some(headers),
                );
            }
            let stream = match conn.open_io_stream_body_source_for_route(
                owner_scope.session_id(),
                owner_scope.session_owner_route(),
                CapturedBody::from_bytes_spooled(response.body),
            ) {
                Ok(stream) => stream,
                Err(message) => return CommandOutputPlan::error(-32000, message),
            };
            CommandOutputPlan::result(json!({
                "resource": {
                    "success": true,
                    "httpStatusCode": response.status,
                    "headers": headers,
                    "stream": stream,
                }
            }))
        }
    }
}

fn failed_resource_result(
    net_error: i32,
    net_error_name: &str,
    status: Option<u16>,
    headers: Option<Map<String, Value>>,
) -> CommandOutputPlan {
    let mut resource = Map::new();
    resource.insert("success".to_owned(), json!(false));
    resource.insert("netError".to_owned(), json!(net_error));
    resource.insert("netErrorName".to_owned(), json!(net_error_name));
    if let Some(status) = status {
        resource.insert("httpStatusCode".to_owned(), json!(status));
    }
    if let Some(headers) = headers {
        resource.insert("headers".to_owned(), Value::Object(headers));
    }
    CommandOutputPlan::result(json!({ "resource": resource }))
}

fn invalid_completion_step() -> NetworkCommandTaskStep {
    NetworkCommandTaskStep::Complete(invalid_completion_plan())
}

fn invalid_completion_plan() -> CommandOutputPlan {
    CommandOutputPlan::error(-32000, "InvalidNetworkCommandCompletion")
}
