use crate::conn::{CdpConnection, Cmd, DevToolsBrowserIdentityOverride};
use crate::domains::command_output::CommandOutputPlan;
#[allow(deprecated)]
use chromiumoxide_cdp::cdp::browser_protocol::{
    emulation::SetUserAgentOverrideParams,
    network::{
        EmulateNetworkConditionsParams, SetBypassServiceWorkerParams, SetCacheDisabledParams,
        SetExtraHttpHeadersParams,
    },
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetBlockedUrlsParams {
    urls: Vec<String>,
}

pub(super) struct EmulatedNetworkConditionsForCommand {
    pub(super) offline: bool,
    pub(super) latency: f64,
    pub(super) download_throughput: f64,
    pub(super) upload_throughput: f64,
    pub(super) connection_type: Option<String>,
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
    let bc = match conn.browser_context_for_command_session_mut(cmd.session_id) {
        Ok(bc) => bc,
        Err((code, message)) => return CommandOutputPlan::error(code, message),
    };
    bc.clear_network_body_artifacts();
    for target in bc.page_targets.iter_mut() {
        target.fetch_owner.drop_active_fetch_response_body_streams();
    }
    if let Err(message) = bc.clear_http_cache() {
        return CommandOutputPlan::error(-32000, message);
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
pub(super) fn emulated_network_conditions_for_command(
    cmd: &Cmd<'_>,
) -> Result<EmulatedNetworkConditionsForCommand, CommandOutputPlan> {
    let params: EmulateNetworkConditionsParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(CommandOutputPlan::error(-32602, "InvalidParams")),
    };
    let connection_type = params
        .connection_type
        .as_ref()
        .and_then(cdp_connection_type_string);
    Ok(EmulatedNetworkConditionsForCommand {
        offline: params.offline,
        latency: params.latency,
        download_throughput: params.download_throughput,
        upload_throughput: params.upload_throughput,
        connection_type,
    })
}

pub(super) fn extra_http_headers_for_command(
    cmd: &Cmd<'_>,
) -> Result<Vec<(String, String)>, CommandOutputPlan> {
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
    let params: SetUserAgentOverrideParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(CommandOutputPlan::error(-32602, "InvalidParams")),
    };
    if !params.user_agent.is_empty() && !is_valid_chromium_header_value(&params.user_agent) {
        return Err(CommandOutputPlan::error(
            -32602,
            "Invalid characters found in userAgent",
        ));
    }
    if params
        .accept_language
        .as_deref()
        .is_some_and(|value| !value.is_empty() && !is_valid_chromium_header_value(value))
    {
        return Err(CommandOutputPlan::error(
            -32602,
            "Invalid characters found in acceptLanguage",
        ));
    }
    if params.user_agent.is_empty() && params.user_agent_metadata.is_some() {
        return Err(CommandOutputPlan::error(
            -32602,
            "Empty userAgent invalid with userAgentMetadata provided",
        ));
    }
    let full_version = match cmd
        .params
        .and_then(|params| params.get("userAgentMetadata"))
        .and_then(|metadata| metadata.get("fullVersion"))
    {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(value)) => Some(value.clone()),
        Some(_) => return Err(CommandOutputPlan::error(-32602, "InvalidParams")),
    };
    if let Some(metadata) = params.user_agent_metadata.as_ref() {
        validate_client_hint_brand_list(metadata.brands.as_deref())?;
        validate_client_hint_brand_list(metadata.full_version_list.as_deref())?;
        validate_client_hint_field(full_version.as_deref(), "Invalid full version string")?;
        validate_client_hint_field(Some(&metadata.platform), "Invalid platform string")?;
        validate_client_hint_field(
            Some(&metadata.platform_version),
            "Invalid platform version string",
        )?;
        validate_client_hint_field(Some(&metadata.architecture), "Invalid architecture string")?;
        validate_client_hint_field(Some(&metadata.model), "Invalid model string")?;
        validate_client_hint_field(metadata.bitness.as_deref(), "Invalid bitness string")?;
        if let Some(form_factors) = metadata.form_factors.as_deref() {
            for form_factor in form_factors {
                validate_client_hint_field(Some(form_factor), "Invalid form factor string")?;
            }
        }
    }
    let metadata = params.user_agent_metadata.map(|metadata| {
        moli_browser_profile::BrowserUserAgentMetadataOverride {
            brands: metadata.brands.map(|brands| {
                brands
                    .into_iter()
                    .map(|entry| moli_browser_profile::BrowserBrandVersion {
                        brand: entry.brand,
                        version: entry.version,
                    })
                    .collect()
            }),
            full_version_list: metadata.full_version_list.map(|brands| {
                brands
                    .into_iter()
                    .map(|entry| moli_browser_profile::BrowserBrandVersion {
                        brand: entry.brand,
                        version: entry.version,
                    })
                    .collect()
            }),
            full_version,
            platform: metadata.platform,
            platform_version: metadata.platform_version,
            architecture: metadata.architecture,
            model: metadata.model,
            mobile: metadata.mobile,
            bitness: metadata.bitness,
            wow64: metadata.wow64,
            form_factors: metadata.form_factors,
        }
    });
    Ok(DevToolsBrowserIdentityOverride::from_command(
        base,
        params.user_agent,
        params.accept_language,
        params.platform,
        metadata,
    ))
}

fn is_valid_chromium_header_value(value: &str) -> bool {
    !value.bytes().any(|byte| matches!(byte, 0 | b'\r' | b'\n'))
}

fn is_ascii_printable(value: &str) -> bool {
    value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
}

fn validate_client_hint_field(
    value: Option<&str>,
    error_message: &'static str,
) -> Result<(), CommandOutputPlan> {
    if value.is_some_and(|value| !is_ascii_printable(value)) {
        return Err(CommandOutputPlan::error(-32602, error_message));
    }
    Ok(())
}

fn validate_client_hint_brand_list(
    brands: Option<&[chromiumoxide_cdp::cdp::browser_protocol::emulation::UserAgentBrandVersion]>,
) -> Result<(), CommandOutputPlan> {
    let Some(brands) = brands else {
        return Ok(());
    };
    for brand in brands {
        validate_client_hint_field(Some(&brand.brand), "Invalid brand string")?;
        validate_client_hint_field(Some(&brand.version), "Invalid brand version string")?;
    }
    Ok(())
}

fn extra_http_headers_from_params(
    params: SetExtraHttpHeadersParams,
) -> Option<Vec<(String, String)>> {
    params.headers.inner().as_object().map(|headers| {
        headers
            .iter()
            .filter_map(|(name, value)| value.as_str().map(|v| (name.clone(), v.to_owned())))
            .collect::<Vec<_>>()
    })
}

fn cdp_connection_type_string(
    connection_type: &chromiumoxide_cdp::cdp::browser_protocol::network::ConnectionType,
) -> Option<String> {
    serde_json::to_value(connection_type)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
}
