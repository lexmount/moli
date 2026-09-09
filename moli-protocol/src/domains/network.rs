use crate::conn::{CdpConnection, Cmd, CommandOwnerScope};
use crate::devtools_runtime::{
    DevToolsAddNetworkDataCollectorCommand, DevToolsBrowserContextId, DevToolsCommand,
    DevToolsCommandResult, DevToolsError, DevToolsErrorKind,
    DevToolsRemoveNetworkDataCollectorCommand, DevToolsSetCacheBehaviorCommand, DevToolsTargetId,
};

use super::storage::{
    CdpCookieParam, DeleteCookiesParams, associated_cookies_to_json, cookie_query_report_to_json,
    cookie_set_report_to_json,
};

pub fn http_status_text(status: u16) -> &'static str {
    if status == 203 {
        return "Non-Authoritative Information";
    }
    http::StatusCode::from_u16(status)
        .ok()
        .and_then(|status| status.canonical_reason())
        .unwrap_or("")
}

pub fn cdp_cookie_query_report(
    report: &moli_cookie_jar::StoredCookieQueryReport,
) -> serde_json::Value {
    cookie_query_report_to_json(report)
}

pub fn cdp_request_headers_object(
    headers: &[(String, String)],
    cookie_access_report: Option<&moli_cookie_jar::StoredCookieQueryReport>,
) -> serde_json::Map<String, serde_json::Value> {
    events::request_headers_as_json_object(headers, cookie_access_report)
}

mod activity;
mod agent;
mod backlog;
mod collectors;
mod cookie_context;
mod cookies;
mod events;
mod load_resource;
mod main_document_progress;
mod output;
mod output_queue;
mod response_body;
pub(crate) mod settings;
#[cfg(test)]
mod tests;

/// Removes one session's Network contributions, then applies the newly
/// aggregated target policy before the session disappears.
pub(in crate::domains) async fn dispose_session_policy_async(
    conn: &mut CdpConnection,
    session_id: &str,
) -> anyhow::Result<()> {
    if !conn.network_listener_enabled_for_session_owner(session_id) {
        return Ok(());
    }
    conn.clear_devtools_network_session_policy_async(session_id)
        .await
}

use super::actions::NetworkAction;
use super::command_output::CommandOutputPlan;
pub(crate) use activity::NetworkPreparedOutputSlot;
pub(crate) use activity::NetworkPreparedOutputs;
pub(in crate::domains) use activity::{
    network_backlog_prepared_outputs_for_owner, project_network_backlog_async,
    project_pending_subresource_continue_async, project_renderer_network_live_async,
    project_subresource_fetch_interception_async,
};
pub use agent::IoStreamState;
pub(crate) use agent::TargetIoStreamRead;
pub(crate) use agent::{
    CapturedRequestBody, CapturedResponseBody, CollectedNetworkDataArtifact,
    NetworkBacklogPreferredRequestId, RetiringTargetNetworkAgentState, TargetNetworkAgentState,
};
pub(crate) use backlog::{
    NetworkBacklogProjectionContext, emit_pending_network_backlog_activity_background_events,
    emit_prepared_renderer_network_live_background_events,
};
pub(crate) use collectors::NetworkDataCollectorStore;
pub(crate) use cookie_context::navigation_cookie_request_context;
use cookies::{
    start_clear_browser_cookies_command, start_delete_cookies_command,
    start_get_all_cookies_command, start_get_cookies_command, start_set_cookie_command,
    start_set_cookies_command,
};
#[cfg(test)]
pub(crate) use events::emit_cdp_network_automation_event;
pub(crate) use events::{
    emit_body_finished, emit_loading_failed, emit_loading_finished, emit_request_will_be_sent,
    emit_request_will_be_sent_extra_info, emit_response_received,
    emit_response_received_extra_info, emit_response_received_without_extra_info_event,
    fetch_subresource_initial_request_network_events, headers_as_json_object,
    request_headers_as_json_object,
};
pub use events::{fetch_auth_required_params, fetch_request_paused_params};
#[cfg(test)]
pub(crate) use main_document_progress::empty_main_document_progress_gate_for_test;
pub(crate) use main_document_progress::{
    CompletedDocumentProgressTransfer, CompletedDownloadProgressTransfer,
    CompletedMainDocumentNetworkEvents, FailedNavigationResponseMode,
    MainDocumentBodyNetworkProgress, MainDocumentBodyProgressSource,
    MainDocumentProgressBackgroundEventBarrier, MainDocumentProgressGate,
    MaterializedDownloadDocumentProgress, MaterializedFailedDocumentProgress,
    MaterializedLoadedDocumentProgress, MaterializedNavigationLoadOutcome,
    emit_child_document_navigation_network_background_events,
    emit_fetch_navigation_initial_request_for_pause_background_events,
    materialize_loaded_navigation_progress, materialize_navigation_load_result,
    native_error_document_finished_events, native_navigation_failure_events,
    native_navigation_response_events, record_completed_main_document_response_body,
    record_failed_main_document_response_body, record_main_document_request_body,
    response_stage_main_document_navigation_network_progress,
};
pub(crate) use output::{
    TargetSubresourceFetchPauseNetworkOutput, TargetSubresourceFetchPauseOutput,
};
pub(crate) use output_queue::{
    PendingNetworkBacklogDeliverySnapshot, PendingSubresourceNetworkActivity,
    PendingSubresourceNetworkActivitySession, PendingWebSocketNetworkActivity,
    PendingWebSocketNetworkActivitySession, TargetNetworkBacklogPreparedDelivery,
    TargetNetworkBacklogRequestIdResolver, TargetNetworkOutputQueue, TargetSubresourcePlanOutput,
};
use response_body::{
    get_network_data_result, get_request_post_data_command_output_plan,
    get_response_body_command_output_plan,
};
use settings::clear_browser_cache_command_output_plan;

pub(crate) struct PendingNetworkCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    kind: PendingNetworkCommandKind,
    pending: PendingNetworkCommandWork,
}

pub(crate) struct CompletedNetworkCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    kind: PendingNetworkCommandKind,
    completed: CompletedNetworkCommandWork,
}

enum PendingNetworkCommandWork {
    DocumentPolicy(crate::conn::PendingDocumentPolicyUpdate),
    ResourceRuntime(crate::conn::PendingDocumentResourceRuntimeUpdate),
    NetworkResourcePreparation(crate::conn::PendingNetworkResourceLoadPreparation),
    Resource(Box<moli_core::page::RendererPreparedNetworkResourceLoad>),
}

enum CompletedNetworkCommandWork {
    DocumentPolicy(Box<crate::conn::CompletedDocumentPolicyUpdate>),
    ResourceRuntime(Box<crate::conn::CompletedDocumentResourceRuntimeUpdate>),
    NetworkResourcePreparation(Box<crate::conn::CompletedNetworkResourceLoadPreparation>),
    Resource(moli_core::page::RendererNetworkResourceLoadOutcome),
}

pub(crate) enum NetworkCommandTaskStep {
    Pending(PendingNetworkCommandDispatch),
    Complete(CommandOutputPlan),
}

enum PendingNetworkCommandKind {
    Enable,
    Disable,
    SetCacheDisabled,
    SetExtraHttpHeaders,
    SetBlockedUrls,
    SetBypassServiceWorker,
    EmulateNetworkConditions,
    SetUserAgentOverride,
    PrepareNetworkResourceLoad,
    FetchNetworkResource,
}

impl PendingNetworkCommandDispatch {
    pub(crate) async fn wait(self) -> CompletedNetworkCommandDispatch {
        let completed = match self.pending {
            PendingNetworkCommandWork::DocumentPolicy(pending) => {
                CompletedNetworkCommandWork::DocumentPolicy(Box::new(pending.wait().await))
            }
            PendingNetworkCommandWork::ResourceRuntime(pending) => {
                CompletedNetworkCommandWork::ResourceRuntime(Box::new(pending.wait().await))
            }
            PendingNetworkCommandWork::NetworkResourcePreparation(pending) => {
                CompletedNetworkCommandWork::NetworkResourcePreparation(Box::new(
                    pending.wait().await,
                ))
            }
            PendingNetworkCommandWork::Resource(pending) => {
                CompletedNetworkCommandWork::Resource((*pending).execute().await)
            }
        };
        CompletedNetworkCommandDispatch {
            command_id: self.command_id,
            owner_scope: self.owner_scope,
            kind: self.kind,
            completed,
        }
    }
}

impl CompletedNetworkCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.owner_scope.session_id()
    }
}

pub(crate) enum NetworkDomainCommandTaskStep {
    Network(NetworkCommandTaskStep),
    Storage(crate::domains::storage::StorageCommandTaskStep),
    Complete(CommandOutputPlan),
}

pub(crate) fn start_network_domain_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> NetworkDomainCommandTaskStep {
    let Some(action) = cmd.parse_action::<NetworkAction>() else {
        return NetworkDomainCommandTaskStep::Complete(CommandOutputPlan::error(
            -32601,
            "UnknownMethod",
        ));
    };
    match action {
        NetworkAction::Enable => NetworkDomainCommandTaskStep::Network(
            start_set_network_domain_enabled_command(conn, cmd, true),
        ),
        NetworkAction::Disable => NetworkDomainCommandTaskStep::Network(
            start_set_network_domain_enabled_command(conn, cmd, false),
        ),
        NetworkAction::SetCacheDisabled => {
            NetworkDomainCommandTaskStep::Network(start_set_cache_disabled_command(conn, cmd))
        }
        NetworkAction::SetBypassServiceWorker => NetworkDomainCommandTaskStep::Network(
            start_set_bypass_service_worker_command(conn, cmd),
        ),
        NetworkAction::SetExtraHttpHeaders => {
            NetworkDomainCommandTaskStep::Network(start_set_extra_http_headers_command(conn, cmd))
        }
        NetworkAction::SetBlockedUrls => {
            NetworkDomainCommandTaskStep::Network(start_set_blocked_urls_command(conn, cmd))
        }
        NetworkAction::EmulateNetworkConditions => NetworkDomainCommandTaskStep::Network(
            start_emulate_network_conditions_command(conn, cmd),
        ),
        NetworkAction::SetUserAgentOverride => {
            NetworkDomainCommandTaskStep::Network(start_set_user_agent_override_command(conn, cmd))
        }
        NetworkAction::SetCookie => {
            NetworkDomainCommandTaskStep::Storage(start_set_cookie_command(conn, cmd))
        }
        NetworkAction::SetCookies => {
            NetworkDomainCommandTaskStep::Storage(start_set_cookies_command(conn, cmd))
        }
        NetworkAction::ClearBrowserCache => NetworkDomainCommandTaskStep::Complete(
            clear_browser_cache_command_output_plan(conn, cmd),
        ),
        NetworkAction::DeleteCookies => {
            NetworkDomainCommandTaskStep::Storage(start_delete_cookies_command(conn, cmd))
        }
        NetworkAction::ClearBrowserCookies => {
            NetworkDomainCommandTaskStep::Storage(start_clear_browser_cookies_command(conn, cmd))
        }
        NetworkAction::GetCookies => {
            NetworkDomainCommandTaskStep::Storage(start_get_cookies_command(conn, cmd))
        }
        NetworkAction::GetAllCookies => {
            NetworkDomainCommandTaskStep::Storage(start_get_all_cookies_command(conn, cmd))
        }
        NetworkAction::GetResponseBody => {
            NetworkDomainCommandTaskStep::Complete(get_response_body_command_output_plan(conn, cmd))
        }
        NetworkAction::GetRequestPostData => NetworkDomainCommandTaskStep::Complete(
            get_request_post_data_command_output_plan(conn, cmd),
        ),
        NetworkAction::LoadNetworkResource => NetworkDomainCommandTaskStep::Network(
            load_resource::start_load_network_resource_command(conn, cmd),
        ),
    }
}

pub(crate) fn execute_devtools_network_command(
    conn: &mut CdpConnection,
    command: DevToolsCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    match command {
        DevToolsCommand::AddNetworkDataCollector(command) => {
            add_network_data_collector_result(conn, command)
        }
        DevToolsCommand::RemoveNetworkDataCollector(command) => {
            remove_network_data_collector_result(conn, command)
        }
        DevToolsCommand::DisownNetworkData(command) => {
            response_body::disown_network_data_result(conn, command)
        }
        DevToolsCommand::GetNetworkData(command) => get_network_data_result(conn, command),
        DevToolsCommand::SetCacheBehavior(command) => set_cache_behavior_result(conn, command),
        _ => Err(DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "UnsupportedDevToolsNetworkCommand",
        )),
    }
}

fn add_network_data_collector_result(
    conn: &mut CdpConnection,
    mut command: DevToolsAddNetworkDataCollectorCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    command.target_ids = validate_network_data_collector_target_ids(conn, &command.target_ids)?;
    command.browser_context_ids =
        resolve_network_data_collector_browser_context_ids(conn, &command.browser_context_ids)?;
    conn.network_data_collectors.add_collector(command)
}

fn remove_network_data_collector_result(
    conn: &mut CdpConnection,
    command: DevToolsRemoveNetworkDataCollectorCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    conn.network_data_collectors
        .remove_collector(&command.collector_id)?;
    Ok(DevToolsCommandResult::Empty)
}

fn validate_network_data_collector_target_ids(
    conn: &CdpConnection,
    target_ids: &[DevToolsTargetId],
) -> Result<Vec<DevToolsTargetId>, DevToolsError> {
    let mut out = Vec::new();
    for target_id in target_ids {
        if conn
            .target_session_route_for_target_id(target_id.as_str())
            .is_some()
        {
            out.push(target_id.clone());
            continue;
        }
        if conn.has_attached_child_frame_id(target_id.as_str()) {
            return Err(DevToolsError::new(
                DevToolsErrorKind::InvalidArgument,
                "Data collectors are available only on top-level browsing contexts",
            ));
        }
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "NoSuchTarget",
        ));
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn resolve_network_data_collector_browser_context_ids(
    conn: &mut CdpConnection,
    browser_context_ids: &[DevToolsBrowserContextId],
) -> Result<Vec<DevToolsBrowserContextId>, DevToolsError> {
    let mut resolved = Vec::new();
    for browser_context_id in browser_context_ids {
        let browser_context_id = browser_context_id.as_str();
        if browser_context_id == "default" {
            let mut default_context_ids = conn
                .browser_contexts()
                .filter(|context| is_moli_internal_default_user_context(&context.id))
                .map(|context| context.id.clone())
                .collect::<Vec<_>>();
            if default_context_ids.is_empty() {
                let id = conn.default_browser_context_id().to_owned();
                conn.insert_browser_context(conn.new_browser_context(id.clone()));
                default_context_ids.push(id);
            }
            resolved.extend(
                default_context_ids
                    .into_iter()
                    .map(DevToolsBrowserContextId::from),
            );
            continue;
        }
        if !conn.has_browser_context_id(browser_context_id) {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "UnknownBrowserContextId",
            ));
        }
        resolved.push(DevToolsBrowserContextId::from(browser_context_id));
    }
    resolved.sort();
    resolved.dedup();
    Ok(resolved)
}

fn is_moli_internal_default_user_context(browser_context_id: &str) -> bool {
    browser_context_id == "BID-default"
        || browser_context_id
            .strip_prefix("BID-")
            .is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
            })
}

fn set_cache_behavior_result(
    conn: &mut CdpConnection,
    command: DevToolsSetCacheBehaviorCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let target_ids = if command.target_ids.is_empty() {
        if !conn.set_webdriver_cache_disabled(&command.context, command.cache_disabled) {
            conn.set_global_cache_disabled(command.cache_disabled);
        }
        top_level_target_ids(conn)
            .into_iter()
            .filter(|id| conn.webdriver_target_is_visible(&command.context, id))
            .collect()
    } else {
        validate_top_level_target_ids(conn, &command)?
    };
    for target_id in target_ids {
        if !conn.set_cache_disabled_for_target(&target_id, command.cache_disabled) {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "NoSuchTarget",
            ));
        }
    }
    Ok(DevToolsCommandResult::Empty)
}

fn top_level_target_ids(conn: &CdpConnection) -> Vec<String> {
    conn.browser_contexts()
        .flat_map(|browser_context| {
            browser_context
                .active_target_id()
                .map(str::to_owned)
                .into_iter()
                .chain(
                    browser_context
                        .background_targets()
                        .map(|target| target.target_id().to_owned()),
                )
        })
        .collect()
}

fn validate_top_level_target_ids(
    conn: &CdpConnection,
    command: &DevToolsSetCacheBehaviorCommand,
) -> Result<Vec<String>, DevToolsError> {
    let mut target_ids = Vec::new();
    for target_id in &command.target_ids {
        let target_id = target_id.as_str();
        if conn.target_session_route_for_target_id(target_id).is_none() {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "NoSuchTarget",
            ));
        }
        target_ids.push(target_id.to_owned());
    }
    target_ids.sort();
    target_ids.dedup();
    Ok(target_ids)
}

fn pending_network_resource_runtime_step(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    session_id: Option<&str>,
    kind: PendingNetworkCommandKind,
    start: impl FnOnce(
        &mut CdpConnection,
    ) -> Result<Option<crate::conn::PendingDocumentResourceRuntimeUpdate>, String>,
) -> NetworkCommandTaskStep {
    let owner_scope = CommandOwnerScope::capture(conn, session_id);
    let result = start(conn);
    match result {
        Ok(Some(pending)) => NetworkCommandTaskStep::Pending(PendingNetworkCommandDispatch {
            command_id,
            kind,
            pending: PendingNetworkCommandWork::ResourceRuntime(pending),
            owner_scope,
        }),
        Ok(None) => NetworkCommandTaskStep::Complete(CommandOutputPlan::success()),
        Err(message) if message == "BrowserContextNotLoaded" => NetworkCommandTaskStep::Complete(
            CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"),
        ),
        Err(message) => NetworkCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message)),
    }
}

fn pending_network_document_policy_step(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    session_id: Option<&str>,
    kind: PendingNetworkCommandKind,
    start: impl FnOnce(
        &mut CdpConnection,
    ) -> Result<Option<crate::conn::PendingDocumentPolicyUpdate>, String>,
) -> NetworkCommandTaskStep {
    let owner_scope = CommandOwnerScope::capture(conn, session_id);
    match start(conn) {
        Ok(Some(pending)) => NetworkCommandTaskStep::Pending(PendingNetworkCommandDispatch {
            command_id,
            kind,
            pending: PendingNetworkCommandWork::DocumentPolicy(pending),
            owner_scope,
        }),
        Ok(None) => NetworkCommandTaskStep::Complete(CommandOutputPlan::success()),
        Err(message) if message == "BrowserContextNotLoaded" => NetworkCommandTaskStep::Complete(
            CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"),
        ),
        Err(message) => NetworkCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message)),
    }
}

fn start_set_network_domain_enabled_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    enabled: bool,
) -> NetworkCommandTaskStep {
    let updated = if enabled {
        conn.enable_network_listener_for_session_owner(cmd.session_id)
    } else {
        conn.disable_network_listener_for_session_owner(cmd.session_id)
    };
    if !updated {
        return NetworkCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        ));
    }

    let kind = if enabled {
        PendingNetworkCommandKind::Enable
    } else {
        PendingNetworkCommandKind::Disable
    };
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    match conn.start_replay_effective_network_request_policy_for_session_owner(cmd.session_id) {
        Ok(Some(pending)) => NetworkCommandTaskStep::Pending(PendingNetworkCommandDispatch {
            command_id: cmd.id,
            kind,
            pending: PendingNetworkCommandWork::DocumentPolicy(pending),
            owner_scope,
        }),
        Ok(None) => NetworkCommandTaskStep::Complete(if enabled {
            settings::enabled_command_output_plan(conn, cmd.session_id)
        } else {
            CommandOutputPlan::success()
        }),
        Err(error) => NetworkCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error)),
    }
}

fn start_set_extra_http_headers_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> NetworkCommandTaskStep {
    let headers = match settings::extra_http_headers_for_command(cmd) {
        Ok(headers) => headers,
        Err(plan) => return NetworkCommandTaskStep::Complete(plan),
    };
    pending_network_document_policy_step(
        conn,
        cmd.id,
        cmd.session_id,
        PendingNetworkCommandKind::SetExtraHttpHeaders,
        |conn| conn.start_set_extra_http_headers_for_session_owner(cmd.session_id, headers),
    )
}

fn start_set_cache_disabled_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> NetworkCommandTaskStep {
    let cache_disabled = match settings::cache_disabled_for_command(cmd) {
        Ok(cache_disabled) => cache_disabled,
        Err(plan) => return NetworkCommandTaskStep::Complete(plan),
    };
    pending_network_document_policy_step(
        conn,
        cmd.id,
        cmd.session_id,
        PendingNetworkCommandKind::SetCacheDisabled,
        |conn| conn.start_set_cache_disabled_for_session_owner(cmd.session_id, cache_disabled),
    )
}

fn start_set_blocked_urls_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> NetworkCommandTaskStep {
    let patterns = match settings::blocked_urls_for_command(cmd) {
        Ok(patterns) => patterns,
        Err(plan) => return NetworkCommandTaskStep::Complete(plan),
    };
    pending_network_document_policy_step(
        conn,
        cmd.id,
        cmd.session_id,
        PendingNetworkCommandKind::SetBlockedUrls,
        |conn| conn.start_set_blocked_url_patterns_for_session_owner(cmd.session_id, patterns),
    )
}

fn start_set_bypass_service_worker_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> NetworkCommandTaskStep {
    let bypass = match settings::bypass_service_worker_for_command(cmd) {
        Ok(bypass) => bypass,
        Err(plan) => return NetworkCommandTaskStep::Complete(plan),
    };
    pending_network_document_policy_step(
        conn,
        cmd.id,
        cmd.session_id,
        PendingNetworkCommandKind::SetBypassServiceWorker,
        |conn| conn.start_set_bypass_service_worker_for_session_owner(cmd.session_id, bypass),
    )
}

fn start_emulate_network_conditions_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> NetworkCommandTaskStep {
    let offline = match settings::network_offline_for_command(cmd) {
        Ok(offline) => offline,
        Err(plan) => return NetworkCommandTaskStep::Complete(plan),
    };
    pending_network_document_policy_step(
        conn,
        cmd.id,
        cmd.session_id,
        PendingNetworkCommandKind::EmulateNetworkConditions,
        |conn| conn.start_set_network_offline_for_session_owner(cmd.session_id, offline),
    )
}

fn start_set_user_agent_override_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> NetworkCommandTaskStep {
    let base_identity = conn.base_browser_identity().clone();
    let browser_identity = match settings::user_agent_override_for_command(cmd, &base_identity) {
        Ok(browser_identity) => browser_identity,
        Err(plan) => return NetworkCommandTaskStep::Complete(plan),
    };
    pending_network_resource_runtime_step(
        conn,
        cmd.id,
        cmd.session_id,
        PendingNetworkCommandKind::SetUserAgentOverride,
        |conn| {
            conn.start_set_devtools_browser_identity_override_for_session_owner(
                cmd.session_id,
                browser_identity,
            )
        },
    )
}

pub(crate) fn complete_pending_network_command(
    conn: &mut CdpConnection,
    completed: CompletedNetworkCommandDispatch,
) -> NetworkCommandTaskStep {
    match completed.kind {
        PendingNetworkCommandKind::Enable => {
            NetworkCommandTaskStep::Complete(complete_network_policy_refresh(conn, completed, true))
        }
        PendingNetworkCommandKind::Disable => NetworkCommandTaskStep::Complete(
            complete_network_policy_refresh(conn, completed, false),
        ),
        PendingNetworkCommandKind::SetCacheDisabled => NetworkCommandTaskStep::Complete(
            complete_document_policy_network_command(conn, completed),
        ),
        PendingNetworkCommandKind::SetExtraHttpHeaders => NetworkCommandTaskStep::Complete(
            complete_document_policy_network_command(conn, completed),
        ),
        PendingNetworkCommandKind::SetBlockedUrls => NetworkCommandTaskStep::Complete(
            complete_document_policy_network_command(conn, completed),
        ),
        PendingNetworkCommandKind::SetBypassServiceWorker => NetworkCommandTaskStep::Complete(
            complete_document_policy_network_command(conn, completed),
        ),
        PendingNetworkCommandKind::EmulateNetworkConditions => NetworkCommandTaskStep::Complete(
            complete_document_policy_network_command(conn, completed),
        ),
        PendingNetworkCommandKind::SetUserAgentOverride => NetworkCommandTaskStep::Complete(
            complete_rebuild_loader_network_command(conn, completed),
        ),
        PendingNetworkCommandKind::PrepareNetworkResourceLoad => {
            load_resource::complete_network_resource_preparation(conn, completed)
        }
        PendingNetworkCommandKind::FetchNetworkResource => NetworkCommandTaskStep::Complete(
            load_resource::complete_network_resource_fetch(conn, completed),
        ),
    }
}

fn complete_document_policy_network_command(
    conn: &mut CdpConnection,
    completed: CompletedNetworkCommandDispatch,
) -> CommandOutputPlan {
    match finish_document_policy_network_command(conn, completed) {
        Ok(()) => CommandOutputPlan::success(),
        Err(error) => CommandOutputPlan::error(-32000, error),
    }
}

fn finish_document_policy_network_command(
    conn: &mut CdpConnection,
    completed: CompletedNetworkCommandDispatch,
) -> Result<(), String> {
    let owner_scope = completed.owner_scope;
    let CompletedNetworkCommandWork::DocumentPolicy(completed) = completed.completed else {
        return Err("InvalidNetworkCommandCompletion".to_owned());
    };
    let document = completed.document();
    match conn.finish_document_policy_update(*completed) {
        Ok(()) => Ok(()),
        Err(error)
            if error == "Document changed"
                && conn
                    .runtime_session_owner_slot_for_owner(&owner_scope)
                    .is_ok()
                && conn.loaded_browser_document_for_owner(&owner_scope).ok() != Some(document) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn complete_network_policy_refresh(
    conn: &mut CdpConnection,
    completed: CompletedNetworkCommandDispatch,
    enabled: bool,
) -> CommandOutputPlan {
    let session_id = completed.owner_scope.session_id().map(str::to_owned);
    if let Err(error) = finish_document_policy_network_command(conn, completed) {
        return CommandOutputPlan::error(-32000, error);
    }
    if enabled {
        settings::enabled_command_output_plan(conn, session_id.as_deref())
    } else {
        CommandOutputPlan::success()
    }
}

fn complete_rebuild_loader_network_command(
    conn: &mut CdpConnection,
    completed: CompletedNetworkCommandDispatch,
) -> CommandOutputPlan {
    let owner_scope = completed.owner_scope.clone();
    let completed = match completed.completed {
        CompletedNetworkCommandWork::ResourceRuntime(completed) => *completed,
        CompletedNetworkCommandWork::DocumentPolicy(_)
        | CompletedNetworkCommandWork::NetworkResourcePreparation(_)
        | CompletedNetworkCommandWork::Resource(_) => {
            return CommandOutputPlan::error(-32000, "InvalidNetworkCommandCompletion");
        }
    };
    let document = completed.document();
    match conn.finish_document_resource_runtime_update(completed) {
        Ok(()) => CommandOutputPlan::success(),
        Err(_)
            if network_page_configuration_will_be_replayed(conn, &owner_scope, document.id()) =>
        {
            CommandOutputPlan::success()
        }
        Err(error) => CommandOutputPlan::error(-32000, error),
    }
}

fn network_page_configuration_will_be_replayed(
    conn: &CdpConnection,
    owner_scope: &CommandOwnerScope,
    dispatched_document: moli_core::browser::DocumentId,
) -> bool {
    conn.runtime_session_owner_slot_for_owner(owner_scope)
        .is_ok_and(|_| conn.current_document_id_for_owner(owner_scope) != Some(dispatched_document))
}

#[cfg(test)]
mod status_text_tests {
    use super::http_status_text;

    #[test]
    fn status_text_uses_hyphenated_standard_phrase_for_203() {
        assert_eq!(http_status_text(203), "Non-Authoritative Information");
    }
}
