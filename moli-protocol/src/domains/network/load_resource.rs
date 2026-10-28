use anyhow::Result;
use chromiumoxide_cdp::cdp::browser_protocol::network::LoadNetworkResourceParams;
use moli_url_policy::BrowserUrlScheme;
use serde_json::{Map, Value, json};
use url::Url;

use crate::{
    conn::{
        CapturedBody, CdpConnection, Cmd, CommandOwnerScope, CommandWorkflowContext,
        NoDocumentLoaded, ProtocolWorkflowDriver, RuntimePageScope,
    },
    domains::command_output::CommandOutputPlan,
};

use super::{
    CompletedNetworkCommandDispatch, CompletedNetworkCommandWork, NetworkCommandTaskStep,
    PendingNetworkCommandDispatch, PendingNetworkCommandKind, PendingNetworkCommandWork,
    headers_as_json_object,
};

const ERR_FAILED: i32 = -2;
const ERR_HTTP_RESPONSE_CODE_FAILURE: i32 = -379;

pub(super) struct ResourcePreparation<T> {
    scope: RuntimePageScope,
    command: T,
}

impl ResourcePreparation<moli_core::page::PendingPageCommand> {
    pub(super) async fn wait(
        self,
    ) -> ResourcePreparation<Result<moli_core::page::CompletedPageCommand>> {
        ResourcePreparation {
            scope: self.scope,
            command: self.command.wait().await,
        }
    }
}

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
    let scope = match RuntimePageScope::capture(conn, &owner_scope) {
        Ok(scope) => scope,
        Err(error) if error.is::<NoDocumentLoaded>() => {
            return NetworkCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "Frame not found",
            ));
        }
        Err(error) => {
            return NetworkCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error));
        }
    };
    let pending = match conn.loaded_page_mut_for_protocol_access_for_owner(scope.owner()) {
        Ok(page) => page.start_prepare_network_resource_load(
            frame_id,
            url,
            params.options.disable_cache,
            params.options.include_credentials,
        ),
        Err(error) if error.is::<crate::conn::NoDocumentLoaded>() => {
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
            kind: PendingNetworkCommandKind::LoadNetworkResource,
            pending: PendingNetworkCommandWork::Resource(Box::new(ResourcePreparation {
                scope,
                command: pending,
            })),
        }),
        Err(error) => {
            NetworkCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error.to_string()))
        }
    }
}

pub(crate) fn finish_resource_preparation(
    conn: &mut CdpConnection,
    scope: &RuntimePageScope,
    completed: Result<moli_core::page::CompletedPageCommand>,
) -> Result<Box<moli_core::page::RendererPreparedNetworkResourceLoad>, CommandOutputPlan> {
    let completion = completed.map_err(|error| CommandOutputPlan::error(-32000, error))?;
    let preparation = scope
        .page_mut(conn)
        .and_then(|page| page.finish_prepare_network_resource_load(completion))
        .map_err(|error| CommandOutputPlan::error(-32000, error))?;
    if !scope.can_start_operation(conn) {
        return Err(CommandOutputPlan::error(-32000, NoDocumentLoaded));
    }
    match preparation {
        moli_core::page::RendererNetworkResourceLoadPreparation::Ready(pending) => Ok(pending),
        moli_core::page::RendererNetworkResourceLoadPreparation::FrameNotFound => {
            Err(CommandOutputPlan::error(-32602, "Frame not found"))
        }
        moli_core::page::RendererNetworkResourceLoadPreparation::CspViolation => {
            Err(CommandOutputPlan::error(-32000, "CSP violation"))
        }
        moli_core::page::RendererNetworkResourceLoadPreparation::UnsupportedUrlScheme => {
            Err(CommandOutputPlan::error(-32602, "Unsupported URL scheme"))
        }
    }
}

pub(super) async fn complete_using(
    driver: &mut ProtocolWorkflowDriver,
    context: &mut CommandWorkflowContext,
    completed: CompletedNetworkCommandDispatch,
) -> Result<CommandOutputPlan> {
    let PendingNetworkCommandKind::LoadNetworkResource = completed.kind else {
        unreachable!("only resource loads use the network workflow");
    };
    let CompletedNetworkCommandWork::Resource(preparation) = completed.completed else {
        return Ok(invalid_completion_plan());
    };
    let ResourcePreparation {
        scope,
        command: completed,
    } = *preparation;
    if let Some(predecessor) = completed
        .as_ref()
        .ok()
        .and_then(|completed| completed.renderer_output_predecessor())
    {
        context
            .context_mut()
            .set_renderer_output_predecessor(predecessor);
    }
    let owner = scope.clone();
    let pending = context.with_state(driver, move |state, _| {
        state.finish_network_resource_preparation(owner, completed)
    })?;
    let pending = match pending {
        Ok(pending) => pending,
        Err(plan) => return Ok(plan),
    };
    match pending.execute().await {
        moli_core::page::RendererNetworkResourceLoadOutcome::FailedBeforeResponse(_) => Ok(
            failed_resource_result(ERR_FAILED, "net::ERR_FAILED", None, None),
        ),
        moli_core::page::RendererNetworkResourceLoadOutcome::Response(response) => {
            let response = *response;
            let headers = headers_as_json_object(&response.headers);
            if response.completion_error.is_some() {
                return Ok(failed_resource_result(
                    ERR_FAILED,
                    "net::ERR_FAILED",
                    Some(response.status),
                    Some(headers),
                ));
            }
            if !(200..300).contains(&response.status) {
                return Ok(failed_resource_result(
                    ERR_HTTP_RESPONSE_CODE_FAILURE,
                    "net::ERR_HTTP_RESPONSE_CODE_FAILURE",
                    Some(response.status),
                    Some(headers),
                ));
            }
            let body = crate::conn::run_body_storage_work(move || {
                Ok(CapturedBody::from_bytes_spooled(response.body))
            })
            .await?;
            let stream =
                driver.with_state(move |state| state.open_io_stream_for_page(scope, body))?;
            Ok(match stream {
                Ok(stream) => CommandOutputPlan::result(json!({
                    "resource": {
                        "success": true,
                        "httpStatusCode": response.status,
                        "headers": headers,
                        "stream": stream,
                    }
                })),
                Err(error) => CommandOutputPlan::error(-32000, error),
            })
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

fn invalid_completion_plan() -> CommandOutputPlan {
    CommandOutputPlan::error(-32000, "InvalidNetworkCommandCompletion")
}
