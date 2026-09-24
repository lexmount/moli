use crate::conn::{CdpConnection, Cmd, DevToolsBrowserIdentityOverride};
use crate::domains::command_output::CommandOutputPlan;
#[allow(deprecated)]
use chromiumoxide_cdp::cdp::browser_protocol::network::{
    EmulateNetworkConditionsParams, SetBypassServiceWorkerParams, SetCacheDisabledParams,
    SetExtraHttpHeadersParams,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetBlockedUrlsParams {
    urls: Vec<String>,
}

pub(super) fn enabled_command_output_plan(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> CommandOutputPlan {
    let mut plan = CommandOutputPlan::success();
    if let Some(session_id) = session_id {
        plan.extend_background_events(
            super::super::target::dedicated_worker_main_script_network_replay_for_session(
                conn, session_id,
            ),
        );
    }
    plan
}

pub(super) fn clear_browser_cache_command_output_plan(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let response_streams = {
        let bc = match conn.browser_context_for_command_session_mut(cmd.session_id) {
            Ok(bc) => bc,
            Err((code, message)) => return CommandOutputPlan::error(code, message),
        };
        bc.clear_network_body_artifacts();
        let response_streams = bc
            .page_targets
            .iter_mut()
            .flat_map(|target| target.fetch_owner.drop_active_fetch_response_body_streams())
            .collect::<Vec<_>>();
        if let Err(message) = bc.clear_http_cache() {
            return CommandOutputPlan::error(-32000, message);
        }
        response_streams
    };
    for pending in response_streams {
        drop(conn.take_navigation_response(pending.permit));
    }
    CommandOutputPlan::success()
}

pub(super) fn cache_disabled_for_command(cmd: &Cmd<'_>) -> Result<bool, CommandOutputPlan> {
    let params: SetCacheDisabledParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(CommandOutputPlan::error(-32602, "InvalidParams")),
    };
    Ok(params.cache_disabled)
}

pub(super) fn bypass_service_worker_for_command(cmd: &Cmd<'_>) -> Result<bool, CommandOutputPlan> {
    let params: SetBypassServiceWorkerParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(CommandOutputPlan::error(-32602, "InvalidParams")),
    };
    Ok(params.bypass)
}

pub(super) fn blocked_urls_for_command(cmd: &Cmd<'_>) -> Result<Vec<String>, CommandOutputPlan> {
    let params: SetBlockedUrlsParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(CommandOutputPlan::error(-32602, "InvalidParams")),
    };
    Ok(params.urls)
}

#[allow(deprecated)]
pub(super) fn network_offline_for_emulation_command(
    cmd: &Cmd<'_>,
) -> Result<bool, CommandOutputPlan> {
    let params: EmulateNetworkConditionsParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(CommandOutputPlan::error(-32602, "InvalidParams")),
    };
    if !params.latency.is_finite()
        || !params.download_throughput.is_finite()
        || !params.upload_throughput.is_finite()
        || params.packet_loss.is_some_and(|value| !value.is_finite())
    {
        return Err(CommandOutputPlan::error(-32602, "InvalidParams"));
    }
    if params.latency > 0.0
        || params.download_throughput > 0.0
        || params.upload_throughput > 0.0
        || params.connection_type.is_some()
        || params.packet_loss.is_some_and(|value| value != 0.0)
        || params.packet_queue_length.is_some_and(|value| value != 0)
        || params.packet_reordering == Some(true)
    {
        return Err(CommandOutputPlan::error(
            -32000,
            "Network throttling and connection type overrides are not supported",
        ));
    }
    Ok(params.offline)
}

pub(super) fn extra_http_headers_for_command(
    cmd: &Cmd<'_>,
) -> Result<moli_fetch::RequestHeaders, CommandOutputPlan> {
    let params: SetExtraHttpHeadersParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(CommandOutputPlan::error(-32602, "InvalidParams")),
    };
    extra_http_headers_from_params(params)
        .ok_or_else(|| CommandOutputPlan::error(-32602, "InvalidParams"))
}

pub(crate) fn user_agent_override_for_command(
    cmd: &Cmd<'_>,
    base: &moli_browser_profile::BrowserIdentityProfile,
) -> Result<Option<DevToolsBrowserIdentityOverride>, CommandOutputPlan> {
    let params: moli_browser_profile::UserAgentOverride = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(CommandOutputPlan::error(-32602, "InvalidParams")),
    };
    params
        .validate()
        .map_err(|message| CommandOutputPlan::error(-32602, message))?;
    Ok(DevToolsBrowserIdentityOverride::from_command(
        base,
        params.user_agent,
        params.accept_language,
        params.platform,
        params.user_agent_metadata,
    ))
}

fn extra_http_headers_from_params(
    params: SetExtraHttpHeadersParams,
) -> Option<moli_fetch::RequestHeaders> {
    params.headers.inner().as_object().map(|headers| {
        headers
            .iter()
            .filter_map(|(name, value)| value.as_str().map(|v| (name.clone(), v.to_owned())))
            .collect::<Vec<_>>()
            .into()
    })
}
