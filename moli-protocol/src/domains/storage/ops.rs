use crate::conn::{
    BrowserContext, BrowserContextCookieManagerSurfaceSnapshot, CdpConnection, Cmd,
    CommandOwnerScope, SiteDataClearOptions,
};
use crate::devtools_runtime::{
    DevToolsBrowserContextId, DevToolsCommand, DevToolsCommandResult, DevToolsCookieParam,
    DevToolsDeleteCookiesCommand, DevToolsDeleteCookiesResult, DevToolsError, DevToolsErrorKind,
    DevToolsGetCookiesCommand, DevToolsGetCookiesResult, DevToolsProtocol,
    DevToolsSetCookiesCommand, DevToolsSetCookiesResult, DevToolsTargetId,
};
use crate::domains::actions::StorageAction;
use crate::domains::command_output::CommandOutputPlan;
use moli_cookie_jar::StoredCookieSetReport;
use moli_core::page::{CompletedPageCommand, PendingPageCommand};
use serde_json::{Value, json};
use url::Url;

use super::normalize::{NormalizedCdpCookieParam, normalize_cookie_param, normalize_partition_key};
use super::params::{
    BrowserContextParam, CdpCookieParam, ClearDataForOriginParams, ClearDataForStorageKeyParams,
    DeleteCookiesParams, GetStorageKeyForFrameParams, GetUsageAndQuotaParams,
    OverrideQuotaForOriginParams, SetCookiesParams,
};
use super::reports::{
    cookie_set_report_to_json, storage_cookie_matches_url, storage_cookie_to_json,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString)]
#[strum(serialize_all = "snake_case", ascii_case_insensitive)]
enum ClearDataStorageType {
    Cookies,
    LocalStorage,
    #[strum(serialize = "indexeddb")]
    IndexedDb,
    StorageBuckets,
    CacheStorage,
    All,
}

pub(crate) struct PendingStorageCommandDispatch {
    command_id: Option<u64>,
    session_id: Option<String>,
    kind: PendingStorageCommandKind,
    pending: PendingPageCommand,
}

pub(crate) struct CompletedStorageCommandDispatch {
    command_id: Option<u64>,
    session_id: Option<String>,
    kind: PendingStorageCommandKind,
    completed: Result<CompletedPageCommand, String>,
}

pub(crate) enum StorageCommandTaskStep {
    Pending(PendingStorageCommandDispatch),
    Complete(CommandOutputPlan),
}

enum DevToolsSetCookiesTaskStep {
    Pending(PendingStorageCommandDispatch),
    Complete(Result<DevToolsSetCookiesResult, DevToolsError>),
}

enum PendingStorageCommandKind {
    GetStorageKeyForTopFrame {
        owner_scope: CommandOwnerScope,
    },
    GetStorageKeyForFrame {
        owner_scope: CommandOwnerScope,
        frame_id: String,
    },
    SetCookies {
        browser_context_id: String,
        cookies: Vec<CdpCookieParam>,
    },
}

impl PendingStorageCommandDispatch {
    pub(crate) async fn wait(self) -> CompletedStorageCommandDispatch {
        CompletedStorageCommandDispatch {
            command_id: self.command_id,
            session_id: self.session_id,
            kind: self.kind,
            completed: self.pending.wait().await.map_err(|error| error.to_string()),
        }
    }
}

impl CompletedStorageCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
}

pub(crate) fn try_start_storage_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> StorageCommandTaskStep {
    match cmd.parse_action::<StorageAction>() {
        Some(StorageAction::ClearDataForOrigin) => {
            StorageCommandTaskStep::Complete(clear_data_for_origin_command_output_plan(conn, cmd))
        }
        Some(StorageAction::ClearDataForStorageKey) => StorageCommandTaskStep::Complete(
            clear_data_for_storage_key_command_output_plan(conn, cmd),
        ),
        Some(StorageAction::GetUsageAndQuota) => {
            StorageCommandTaskStep::Complete(get_usage_and_quota_command_output_plan(conn, cmd))
        }
        Some(StorageAction::OverrideQuotaForOrigin) => StorageCommandTaskStep::Complete(
            override_quota_for_origin_command_output_plan(conn, cmd),
        ),
        Some(StorageAction::GetStorageKeyForFrame) => {
            start_get_storage_key_for_frame_command(conn, cmd)
        }
        Some(StorageAction::RunBounceTrackingMitigations) => {
            StorageCommandTaskStep::Complete(CommandOutputPlan::result(json!({})))
        }
        Some(StorageAction::ClearCookies) => start_storage_clear_cookies_command(conn, cmd),
        Some(StorageAction::GetCookies) => start_storage_get_cookies_command(conn, cmd),
        Some(StorageAction::SetCookies) => start_storage_set_cookies_command(conn, cmd),
        Some(StorageAction::DeleteCookies) => start_storage_delete_cookies_command(conn, cmd),
        None => StorageCommandTaskStep::Complete(CommandOutputPlan::error(-32601, "UnknownMethod")),
    }
}

pub(crate) fn start_devtools_storage_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command: DevToolsCommand,
) -> StorageCommandTaskStep {
    match command {
        DevToolsCommand::GetCookies(command) => StorageCommandTaskStep::Complete(
            devtools_get_cookies_output_plan(complete_devtools_get_cookies_result(conn, command)),
        ),
        DevToolsCommand::DeleteCookies(command) => {
            StorageCommandTaskStep::Complete(devtools_delete_cookies_output_plan(
                complete_devtools_delete_cookies_result(conn, command),
            ))
        }
        DevToolsCommand::SetCookies(command) => {
            start_devtools_set_cookies_command(conn, command_id, command)
        }
        _ => StorageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "UnsupportedDevToolsCommand",
        )),
    }
}

pub(crate) async fn execute_devtools_storage_command_async(
    conn: &mut CdpConnection,
    command: DevToolsCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    match command {
        DevToolsCommand::GetCookies(command) => complete_devtools_get_cookies_result(conn, command)
            .map(DevToolsCommandResult::GetCookies),
        DevToolsCommand::DeleteCookies(command) => {
            complete_devtools_delete_cookies_result(conn, command)
                .map(DevToolsCommandResult::DeleteCookies)
        }
        DevToolsCommand::SetCookies(command) => {
            let mut step = start_devtools_set_cookies_result(conn, None, command);
            loop {
                match step {
                    DevToolsSetCookiesTaskStep::Complete(result) => {
                        return result.map(DevToolsCommandResult::SetCookies);
                    }
                    DevToolsSetCookiesTaskStep::Pending(pending) => {
                        step = DevToolsSetCookiesTaskStep::Complete(
                            complete_pending_set_cookies_result(conn, pending.wait().await),
                        );
                    }
                }
            }
        }
        _ => Err(DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "UnsupportedDevToolsCommand",
        )),
    }
}

fn start_storage_get_cookies_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> StorageCommandTaskStep {
    let command = match build_cdp_storage_get_cookies_command(cmd) {
        Ok(command) => command,
        Err(plan) => return StorageCommandTaskStep::Complete(plan),
    };
    start_devtools_storage_command(conn, cmd.id, DevToolsCommand::GetCookies(command))
}

fn start_storage_clear_cookies_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> StorageCommandTaskStep {
    let command = match build_cdp_storage_clear_cookies_command(cmd) {
        Ok(command) => command,
        Err(plan) => return StorageCommandTaskStep::Complete(plan),
    };
    start_devtools_storage_command(conn, cmd.id, DevToolsCommand::DeleteCookies(command))
}

pub(crate) fn build_cdp_storage_clear_cookies_command(
    cmd: &Cmd<'_>,
) -> Result<DevToolsDeleteCookiesCommand, CommandOutputPlan> {
    let browser_context_id = browser_context_id_from_optional_params(cmd)
        .map_err(|message| CommandOutputPlan::error(-32602, message))?
        .map(DevToolsBrowserContextId::from);
    Ok(DevToolsDeleteCookiesCommand {
        context: cmd
            .devtools_command_context(Option::<DevToolsTargetId>::None, browser_context_id.clone()),
        browser_context_id,
        name: None,
        url: None,
        domain: None,
        path: None,
        partition_key: None,
        filter: None,
    })
}

pub(crate) fn build_cdp_storage_get_cookies_command(
    cmd: &Cmd<'_>,
) -> Result<DevToolsGetCookiesCommand, CommandOutputPlan> {
    let browser_context_id = browser_context_id_from_optional_params(cmd)
        .map_err(|message| CommandOutputPlan::error(-32602, message))?
        .map(DevToolsBrowserContextId::from);
    Ok(DevToolsGetCookiesCommand {
        context: cmd
            .devtools_command_context(Option::<DevToolsTargetId>::None, browser_context_id.clone()),
        browser_context_id,
        urls: None,
        filter: None,
    })
}

pub(crate) fn start_storage_delete_cookies_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> StorageCommandTaskStep {
    let command = match build_cdp_storage_delete_cookies_command(cmd) {
        Ok(command) => command,
        Err(plan) => return StorageCommandTaskStep::Complete(plan),
    };
    start_devtools_storage_command(conn, cmd.id, DevToolsCommand::DeleteCookies(command))
}

pub(crate) fn build_cdp_storage_delete_cookies_command(
    cmd: &Cmd<'_>,
) -> Result<DevToolsDeleteCookiesCommand, CommandOutputPlan> {
    let params: DeleteCookiesParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => DeleteCookiesParams {
            name: None,
            url: None,
            domain: None,
            path: None,
            partition_key: None,
            browser_context_id: None,
        },
        Err(_) => return Err(CommandOutputPlan::error(-32602, "InvalidParams")),
    };
    let partition_key = normalize_partition_key(params.partition_key.as_ref(), false)
        .map_err(|_| CommandOutputPlan::error(-32602, "InvalidParams"))?;
    let browser_context_id = params
        .browser_context_id
        .map(DevToolsBrowserContextId::from);
    Ok(DevToolsDeleteCookiesCommand {
        context: cmd
            .devtools_command_context(Option::<DevToolsTargetId>::None, browser_context_id.clone()),
        browser_context_id,
        name: params.name,
        url: params.url,
        domain: params.domain,
        path: params.path,
        partition_key,
        filter: None,
    })
}

fn clear_data_for_origin_command_output_plan(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let params: ClearDataForOriginParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };

    let origin = match Url::parse(params.origin.as_str()) {
        Ok(origin) => origin,
        Err(_) => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };

    let site_data_options = site_data_clear_options_for_storage_types(&params.storage_types);
    if site_data_options != SiteDataClearOptions::default() {
        match browser_context_for_storage_clear(
            conn,
            params.browser_context_id.as_deref(),
            cmd.session_id,
        ) {
            Ok(browser_context) => {
                if let Err(message) =
                    browser_context.clear_site_data_for_origin(&origin, site_data_options)
                {
                    return CommandOutputPlan::error(-32000, message);
                }
            }
            Err((code, message)) => {
                return CommandOutputPlan::error(code, message);
            }
        }
    }

    CommandOutputPlan::success()
}

fn clear_data_for_storage_key_command_output_plan(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let params: ClearDataForStorageKeyParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };

    let storage_key =
        match moli_storage_key::deserialize_serialized_storage_key(&params.storage_key) {
            Some(storage_key) => storage_key,
            None => {
                return CommandOutputPlan::error(-32602, "UnableToDeserializeStorageKey");
            }
        };
    if moli_storage_key::serialized_storage_key_has_opaque_origin(
        &storage_key.serialized_storage_key(),
    ) {
        return CommandOutputPlan::error(-32602, "UnsupportedStorageKey");
    }

    let site_data_options = site_data_clear_options_for_storage_types(&params.storage_types);
    if site_data_options != SiteDataClearOptions::default() {
        match browser_context_for_storage_clear(conn, None, cmd.session_id) {
            Ok(browser_context) => {
                if let Err(message) =
                    browser_context.clear_site_data_for_storage_key(&storage_key, site_data_options)
                {
                    return CommandOutputPlan::error(-32000, message);
                }
            }
            Err((code, message)) => {
                return CommandOutputPlan::error(code, message);
            }
        }
    }

    CommandOutputPlan::success()
}

fn get_usage_and_quota_command_output_plan(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let params: GetUsageAndQuotaParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };

    let serialized_origin = match serialized_security_origin(&params.origin) {
        Ok(serialized_origin) => serialized_origin,
        Err(plan) => return plan,
    };

    let (usage, quota, override_active) =
        match conn.browser_context_for_command_session_mut(cmd.session_id) {
            Ok(browser_context) => {
                let (quota, override_active) =
                    browser_context.storage_quota_for_origin(&serialized_origin);
                let usage = match browser_context.storage_usage_for_origin(&serialized_origin) {
                    Ok(usage) => usage,
                    Err(message) => return CommandOutputPlan::error(-32000, message),
                };
                (usage, quota, override_active)
            }
            Err((code, message)) => return CommandOutputPlan::error(code, message),
        };

    CommandOutputPlan::result(json!({
        "usage": usage.total_usage as f64,
        "quota": quota,
        "overrideActive": override_active,
        "usageBreakdown": [
            {
                "storageType": "local_storage",
                "usage": usage.local_storage_usage as f64,
            },
            {
                "storageType": "indexeddb",
                "usage": usage.indexed_db_usage as f64,
            },
            {
                "storageType": "storage_buckets",
                "usage": usage.storage_buckets_usage as f64,
            }
        ]
    }))
}

fn override_quota_for_origin_command_output_plan(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let params: OverrideQuotaForOriginParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };

    let serialized_origin = match serialized_security_origin(&params.origin) {
        Ok(serialized_origin) => serialized_origin,
        Err(plan) => return plan,
    };

    let browser_context = match conn.browser_context_for_command_session_mut(cmd.session_id) {
        Ok(browser_context) => browser_context,
        Err((code, message)) => return CommandOutputPlan::error(code, message),
    };

    match params.quota_size {
        Some(quota) if quota.is_finite() && quota >= 0.0 => {
            browser_context.set_storage_quota_override(serialized_origin, quota);
        }
        Some(_) => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
        None => {
            browser_context.clear_storage_quota_override(&serialized_origin);
        }
    }

    CommandOutputPlan::success()
}

fn serialized_security_origin(origin: &str) -> Result<String, CommandOutputPlan> {
    let url = Url::parse(origin).map_err(|_| CommandOutputPlan::error(-32602, "InvalidParams"))?;
    let serialized_origin = url.origin().ascii_serialization();
    if serialized_origin == "null" {
        return Err(CommandOutputPlan::error(-32602, "InvalidParams"));
    }
    Ok(serialized_origin)
}

fn start_get_storage_key_for_frame_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> StorageCommandTaskStep {
    let params: GetStorageKeyForFrameParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return StorageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };

    let Some((target_id, target_url, _, _)) =
        conn.target_session_owner_frame_tree_identity(cmd.session_id)
    else {
        let plan = if conn.browser_context.is_none()
            && conn
                .target_owner_identity_for_session(cmd.session_id)
                .is_none()
        {
            CommandOutputPlan::error(-31998, "BrowserContextNotLoaded")
        } else {
            CommandOutputPlan::error(-31998, "TargetNotLoaded")
        };
        return StorageCommandTaskStep::Complete(plan);
    };

    if params.frame_id == target_id {
        let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
        if let Some(page) =
            loaded_page_mut_for_session(conn, cmd.session_id, owner_scope.session_owner_route())
        {
            return match page.start_document_storage_key_snapshot() {
                Ok(pending) => StorageCommandTaskStep::Pending(PendingStorageCommandDispatch {
                    command_id: cmd.id,
                    session_id: cmd.session_id.map(str::to_owned),
                    kind: PendingStorageCommandKind::GetStorageKeyForTopFrame { owner_scope },
                    pending,
                }),
                Err(error) => StorageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    error.to_string(),
                )),
            };
        }
        let storage_key = match top_frame_storage_key_for_url(&target_url) {
            Ok(storage_key) => storage_key,
            Err(plan) => return StorageCommandTaskStep::Complete(plan),
        };
        return StorageCommandTaskStep::Complete(CommandOutputPlan::result(
            json!({ "storageKey": storage_key }),
        ));
    }

    if let Err(message) = conn.ensure_document_accessible_for_session_owner(cmd.session_id) {
        return StorageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
    }
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let Some(page) =
        loaded_page_mut_for_session(conn, cmd.session_id, owner_scope.session_owner_route())
    else {
        return StorageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "NoFrameForGivenId",
        ));
    };
    match page.start_child_frame_tree_snapshot() {
        Ok(pending) => StorageCommandTaskStep::Pending(PendingStorageCommandDispatch {
            command_id: cmd.id,
            session_id: cmd.session_id.map(str::to_owned),
            kind: PendingStorageCommandKind::GetStorageKeyForFrame {
                owner_scope,
                frame_id: params.frame_id,
            },
            pending,
        }),
        Err(error) => {
            StorageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error.to_string()))
        }
    }
}

fn start_storage_set_cookies_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> StorageCommandTaskStep {
    let command = match build_cdp_storage_set_cookies_command(cmd) {
        Ok(command) => command,
        Err(plan) => return StorageCommandTaskStep::Complete(plan),
    };
    start_devtools_storage_command(conn, cmd.id, DevToolsCommand::SetCookies(command))
}

pub(crate) fn build_cdp_storage_set_cookies_command(
    cmd: &Cmd<'_>,
) -> Result<DevToolsSetCookiesCommand, CommandOutputPlan> {
    let params: SetCookiesParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return Err(CommandOutputPlan::error(-32602, "InvalidParams"));
        }
    };
    let browser_context_id = params
        .browser_context_id
        .map(DevToolsBrowserContextId::from);
    Ok(DevToolsSetCookiesCommand {
        context: cmd
            .devtools_command_context(Option::<DevToolsTargetId>::None, browser_context_id.clone()),
        browser_context_id,
        cookies: params.cookies.into_iter().map(Into::into).collect(),
    })
}

impl From<CdpCookieParam> for DevToolsCookieParam {
    fn from(param: CdpCookieParam) -> Self {
        Self {
            name: param.name,
            value: param.value,
            url: param.url,
            domain: param.domain,
            path: param.path,
            secure: param.secure,
            http_only: param.http_only,
            same_site: param.same_site,
            priority: param.priority,
            source_scheme: param.source_scheme,
            source_port: param.source_port,
            partition_key: param.partition_key,
            partition_key_opaque: param.partition_key_opaque,
            expires: param.expires,
        }
    }
}

impl From<DevToolsCookieParam> for CdpCookieParam {
    fn from(param: DevToolsCookieParam) -> Self {
        Self {
            name: param.name,
            value: param.value,
            url: param.url,
            domain: param.domain,
            path: param.path,
            secure: param.secure,
            http_only: param.http_only,
            same_site: param.same_site,
            priority: param.priority,
            source_scheme: param.source_scheme,
            source_port: param.source_port,
            partition_key: param.partition_key,
            partition_key_opaque: param.partition_key_opaque,
            expires: param.expires,
        }
    }
}

fn start_devtools_set_cookies_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command: DevToolsSetCookiesCommand,
) -> StorageCommandTaskStep {
    match start_devtools_set_cookies_result(conn, command_id, command) {
        DevToolsSetCookiesTaskStep::Pending(pending) => StorageCommandTaskStep::Pending(pending),
        DevToolsSetCookiesTaskStep::Complete(result) => {
            StorageCommandTaskStep::Complete(devtools_set_cookies_output_plan(result))
        }
    }
}

fn start_devtools_set_cookies_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command: DevToolsSetCookiesCommand,
) -> DevToolsSetCookiesTaskStep {
    let session_id = (command.context.protocol == DevToolsProtocol::Cdp)
        .then(|| command.context.session_id.as_ref().map(|id| id.as_str()))
        .flatten();
    let browser_context_id = command
        .browser_context_id
        .or(command.context.browser_context_id)
        .map(DevToolsBrowserContextId::into_string);
    let target_id = command.context.target_id.as_ref().map(|id| id.as_str());
    let cookies = command.cookies.into_iter().map(Into::into).collect();
    let browser_context_id =
        match effective_storage_browser_context_id(conn, session_id, target_id, browser_context_id)
        {
            Ok(browser_context_id) => browser_context_id,
            Err((code, message)) => {
                return DevToolsSetCookiesTaskStep::Complete(Err(
                    devtools_error_from_code_message(code, message),
                ));
            }
        };
    let Some(browser_context) = conn.browser_context_by_id_mut(&browser_context_id) else {
        return DevToolsSetCookiesTaskStep::Complete(Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "UnknownBrowserContextId",
        )));
    };
    let Some(page) = browser_context
        .page_targets
        .active_mut()
        .and_then(|target| target.runtime_slot.loaded_page_mut())
    else {
        let manager_surface = browser_context.cookie_manager_surface_snapshot_without_live_page();
        let reports = set_cookies_with_manager_surface(browser_context, &manager_surface, cookies);
        return DevToolsSetCookiesTaskStep::Complete(Ok(set_cookie_reports_result(&reports)));
    };
    match page.start_document_cookie_owner_snapshot() {
        Ok(pending) => DevToolsSetCookiesTaskStep::Pending(PendingStorageCommandDispatch {
            command_id,
            session_id: session_id.map(str::to_owned),
            kind: PendingStorageCommandKind::SetCookies {
                browser_context_id,
                cookies,
            },
            pending,
        }),
        Err(error) => DevToolsSetCookiesTaskStep::Complete(Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            error.to_string(),
        ))),
    }
}

fn effective_storage_browser_context_id(
    conn: &CdpConnection,
    session_id: Option<&str>,
    target_id: Option<&str>,
    browser_context_id: Option<String>,
) -> Result<String, (i32, &'static str)> {
    if let Some(browser_context_id) = browser_context_id {
        if conn.browser_context_by_id(&browser_context_id).is_some() {
            return Ok(browser_context_id);
        }
        return Err((-31998, "UnknownBrowserContextId"));
    }
    if let Some(target_id) = target_id {
        return conn
            .browser_context_id_for_target(target_id)
            .map(str::to_owned)
            .ok_or((-31998, "NoSuchTarget"));
    }
    if let Some(session_id) = session_id {
        return match conn.session_route(Some(session_id)) {
            Some(crate::conn::CdpSessionRoute::PageTarget {
                browser_context_id, ..
            })
            | Some(crate::conn::CdpSessionRoute::BrowserContext { browser_context_id }) => {
                Ok(browser_context_id)
            }
            Some(crate::conn::CdpSessionRoute::Browser) => conn
                .browser_context
                .as_ref()
                .map(|browser_context| browser_context.id.clone())
                .ok_or((-31998, "BrowserContextNotLoaded")),
            Some(
                crate::conn::CdpSessionRoute::TabTarget { .. }
                | crate::conn::CdpSessionRoute::SharedWorkerTarget { .. }
                | crate::conn::CdpSessionRoute::DedicatedWorkerTarget { .. }
                | crate::conn::CdpSessionRoute::ServiceWorkerTarget { .. },
            ) => Err((-31998, "DirectSessionRouteRequired")),
            None => Err((-32001, "Unknown sessionId")),
        };
    }
    conn.browser_context
        .as_ref()
        .map(|browser_context| browser_context.id.clone())
        .ok_or((-31998, "BrowserContextNotLoaded"))
}

pub(crate) fn complete_pending_storage_command(
    conn: &mut CdpConnection,
    completed: CompletedStorageCommandDispatch,
) -> CommandOutputPlan {
    match completed.kind {
        PendingStorageCommandKind::GetStorageKeyForTopFrame { owner_scope } => {
            complete_get_storage_key_for_top_frame_command(conn, &owner_scope, completed.completed)
        }
        PendingStorageCommandKind::GetStorageKeyForFrame {
            owner_scope,
            frame_id,
        } => complete_get_storage_key_for_frame_command(
            conn,
            &owner_scope,
            completed.completed,
            &frame_id,
        ),
        PendingStorageCommandKind::SetCookies {
            browser_context_id,
            cookies,
        } => devtools_set_cookies_output_plan(complete_set_cookies_result(
            conn,
            &browser_context_id,
            completed.completed,
            cookies,
        )),
    }
}

fn complete_get_storage_key_for_top_frame_command(
    conn: &mut CdpConnection,
    owner_scope: &CommandOwnerScope,
    completed: Result<CompletedPageCommand, String>,
) -> CommandOutputPlan {
    let completion = match completed {
        Ok(completion) => completion,
        Err(error) => return CommandOutputPlan::error(-32000, error),
    };
    let Some(page) = loaded_page_mut_for_session(
        conn,
        owner_scope.session_id(),
        owner_scope.session_owner_route(),
    ) else {
        return CommandOutputPlan::error(-32000, "NoFrameForGivenId");
    };
    match page.finish_document_storage_key_snapshot(completion) {
        Ok(storage_key) => storage_key_result_plan(&storage_key),
        Err(error) => CommandOutputPlan::error(-32000, error.to_string()),
    }
}

fn complete_get_storage_key_for_frame_command(
    conn: &mut CdpConnection,
    owner_scope: &CommandOwnerScope,
    completed: Result<CompletedPageCommand, String>,
    frame_id: &str,
) -> CommandOutputPlan {
    let completion = match completed {
        Ok(completion) => completion,
        Err(error) => return CommandOutputPlan::error(-32000, error),
    };
    let Some(page) = loaded_page_mut_for_session(
        conn,
        owner_scope.session_id(),
        owner_scope.session_owner_route(),
    ) else {
        return CommandOutputPlan::error(-32000, "NoFrameForGivenId");
    };
    let child_frames = match page.finish_child_frame_tree_snapshot(completion) {
        Ok(child_frames) => child_frames,
        Err(error) => {
            return CommandOutputPlan::error(
                -32000,
                format!("Failed to snapshot child frame tree: {error}"),
            );
        }
    };

    let Some(frame) = find_child_frame(&child_frames, frame_id) else {
        return CommandOutputPlan::error(-32000, "NoFrameForGivenId");
    };
    storage_key_result_plan(&frame.storage_key)
}

fn top_frame_storage_key_for_url(target_url: &str) -> Result<String, CommandOutputPlan> {
    let url =
        Url::parse(target_url).map_err(|_| CommandOutputPlan::error(-32602, "InvalidParams"))?;
    let storage_key = moli_storage_key::MoliStorageKey::first_party_from_url(
        &url,
        moli_storage_key::url_needs_opaque_nonce(&url)
            .then_some(moli_storage_key::OpaqueOriginNonce::new(0)),
    )
    .serialized_storage_key();
    storage_key_for_cdp_result(&storage_key)
}

fn storage_key_for_cdp_result(storage_key: &str) -> Result<String, CommandOutputPlan> {
    if moli_storage_key::serialized_storage_key_has_opaque_origin(storage_key) {
        return Err(opaque_storage_key_error());
    }
    Ok(storage_key.to_owned())
}

fn storage_key_result_plan(storage_key: &str) -> CommandOutputPlan {
    match storage_key_for_cdp_result(storage_key) {
        Ok(storage_key) => CommandOutputPlan::result(json!({ "storageKey": storage_key })),
        Err(plan) => plan,
    }
}

fn opaque_storage_key_error() -> CommandOutputPlan {
    CommandOutputPlan::error(
        -32000,
        "Frame corresponds to an opaque origin and its storage key cannot be serialized",
    )
}

fn complete_pending_set_cookies_result(
    conn: &mut CdpConnection,
    completed: CompletedStorageCommandDispatch,
) -> Result<DevToolsSetCookiesResult, DevToolsError> {
    let PendingStorageCommandKind::SetCookies {
        browser_context_id,
        cookies,
    } = completed.kind
    else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "UnexpectedStorageCommandKind",
        ));
    };
    complete_set_cookies_result(conn, &browser_context_id, completed.completed, cookies)
}

fn complete_set_cookies_result(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    completed: Result<CompletedPageCommand, String>,
    cookies: Vec<CdpCookieParam>,
) -> Result<DevToolsSetCookiesResult, DevToolsError> {
    let completion = match completed {
        Ok(completion) => completion,
        Err(error) => return Err(DevToolsError::new(DevToolsErrorKind::Internal, error)),
    };
    let Some(browser_context) = conn.browser_context_by_id_mut(browser_context_id) else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "UnknownBrowserContextId",
        ));
    };
    let owner = {
        let Some(page) = browser_context
            .page_targets
            .active_mut()
            .and_then(|target| target.runtime_slot.loaded_page_mut())
        else {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Internal,
                "NoDocumentLoaded",
            ));
        };
        match page.finish_document_cookie_owner_snapshot(completion) {
            Ok(owner) => owner,
            Err(error) => {
                return Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    error.to_string(),
                ));
            }
        }
    };
    let manager_surface = browser_context.cookie_manager_surface_snapshot_with_owner(&owner);
    let reports = set_cookies_with_manager_surface(browser_context, &manager_surface, cookies);
    Ok(set_cookie_reports_result(&reports))
}

fn loaded_page_mut_for_session<'a>(
    conn: &'a mut CdpConnection,
    session_id: Option<&str>,
    owner_route: Option<&crate::conn::CdpSessionRoute>,
) -> Option<&'a mut moli_core::page::Page> {
    conn.loaded_page_mut_for_protocol_access_for_route(session_id, owner_route)
        .ok()
}

fn find_child_frame<'a>(
    frames: &'a [moli_core::page::ChildFrameTreeSnapshot],
    frame_id: &str,
) -> Option<&'a moli_core::page::ChildFrameTreeSnapshot> {
    for frame in frames {
        if frame.frame_id == frame_id {
            return Some(frame);
        }
        if let Some(frame) = find_child_frame(&frame.child_frames, frame_id) {
            return Some(frame);
        }
    }
    None
}

fn storage_types_include_cookies(storage_types: &str) -> bool {
    storage_types_include(storage_types, ClearDataStorageType::Cookies)
}

fn storage_types_include_local_storage(storage_types: &str) -> bool {
    storage_types_include(storage_types, ClearDataStorageType::LocalStorage)
}

fn storage_types_include_indexed_db(storage_types: &str) -> bool {
    storage_types_include(storage_types, ClearDataStorageType::IndexedDb)
}

fn storage_types_include_storage_buckets(storage_types: &str) -> bool {
    storage_types_include(storage_types, ClearDataStorageType::StorageBuckets)
}

fn storage_types_include_cache_storage(storage_types: &str) -> bool {
    storage_types_include(storage_types, ClearDataStorageType::CacheStorage)
}

fn site_data_clear_options_for_storage_types(storage_types: &str) -> SiteDataClearOptions {
    SiteDataClearOptions {
        cookies: storage_types_include_cookies(storage_types),
        local_storage: storage_types_include_local_storage(storage_types),
        indexed_db: storage_types_include_indexed_db(storage_types),
        storage_buckets: storage_types_include_storage_buckets(storage_types),
        http_cache: storage_types_include_cache_storage(storage_types),
    }
}

fn storage_types_include(storage_types: &str, wanted: ClearDataStorageType) -> bool {
    storage_types.split(',').any(|token| {
        token
            .trim()
            .parse::<ClearDataStorageType>()
            .is_ok_and(|parsed| parsed == wanted || parsed == ClearDataStorageType::All)
    })
}

fn browser_context_for_storage_clear<'a>(
    conn: &'a mut CdpConnection,
    browser_context_id: Option<&str>,
    session_id: Option<&str>,
) -> Result<&'a mut BrowserContext, (i32, &'static str)> {
    if let Some(wanted_id) = browser_context_id {
        return conn
            .browser_context_by_id_mut(wanted_id)
            .ok_or((-31998, "UnknownBrowserContextId"));
    }

    conn.browser_context_for_command_session_mut(session_id)
}

pub(crate) fn complete_devtools_get_cookies_result(
    conn: &mut CdpConnection,
    command: DevToolsGetCookiesCommand,
) -> Result<DevToolsGetCookiesResult, DevToolsError> {
    let browser_context_id = command
        .browser_context_id
        .or(command.context.browser_context_id)
        .map(DevToolsBrowserContextId::into_string);
    let session_id = (command.context.protocol == DevToolsProtocol::Cdp)
        .then(|| command.context.session_id.as_ref().map(|id| id.as_str()))
        .flatten();
    let target_id = command.context.target_id.as_ref().map(|id| id.as_str());
    let wanted_id =
        match effective_storage_browser_context_id(conn, session_id, target_id, browser_context_id)
        {
            Ok(browser_context_id) => Some(browser_context_id),
            Err((code, message)) => return Err(devtools_error_from_code_message(code, message)),
        };
    match get_cookies_for_browser_context(conn, Ok(wanted_id)) {
        Ok(cookies) => match filter_cookies_by_urls(cookies, command.urls.as_deref())
            .map(|cookies| filter_cookies_by_devtools_filter(cookies, command.filter.as_ref()))
        {
            Ok(cookies) => Ok(DevToolsGetCookiesResult { cookies }),
            Err(message) => Err(DevToolsError::new(
                DevToolsErrorKind::InvalidArgument,
                message,
            )),
        },
        Err((code, message)) => Err(devtools_error_from_code_message(code, message)),
    }
}

fn filter_cookies_by_urls(
    cookies: Vec<Value>,
    urls: Option<&[String]>,
) -> Result<Vec<Value>, &'static str> {
    let Some(urls) = urls else {
        return Ok(cookies);
    };
    let urls = urls
        .iter()
        .map(|url| Url::parse(url))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "InvalidParams")?;
    Ok(cookies
        .into_iter()
        .filter(|cookie| cookie_matches_any_url(cookie, &urls))
        .collect::<Vec<_>>())
}

fn cookie_matches_any_url(cookie: &Value, urls: &[Url]) -> bool {
    let host_only = !cookie["domain"]
        .as_str()
        .is_some_and(|domain| domain.starts_with('.'));
    let storage_cookie = moli_cookie_jar::StoredCookie {
        name: cookie["name"].as_str().unwrap_or_default().to_owned(),
        value: cookie["value"].as_str().unwrap_or_default().to_owned(),
        domain: cookie["domain"]
            .as_str()
            .unwrap_or_default()
            .trim_start_matches('.')
            .to_owned(),
        host_only,
        path: cookie["path"].as_str().unwrap_or("/").to_owned(),
        secure: cookie["secure"].as_bool().unwrap_or(false),
        http_only: cookie["httpOnly"].as_bool().unwrap_or(false),
        expires: cookie["expires"].as_f64().and_then(|value| {
            if !value.is_finite() || value < 0.0 {
                return None;
            }
            let seconds = value.trunc() as i64;
            let nanos = ((value.fract()) * 1_000_000_000.0).round() as i64;
            time::OffsetDateTime::from_unix_timestamp(seconds)
                .ok()
                .and_then(|dt| dt.checked_add(time::Duration::nanoseconds(nanos)))
        }),
        same_site: super::normalize::stored_cookie_same_site_from_cdp(cookie["sameSite"].as_str()),
        priority: cookie["priority"]
            .as_str()
            .and_then(moli_cookie_jar::CookiePriority::parse),
        partition_key: None,
        source_scheme: super::normalize::stored_cookie_source_scheme_from_cdp(
            cookie["sourceScheme"].as_str(),
        ),
        source_port: cookie["sourcePort"]
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(-1),
        creation_index: 0,
        last_access_index: 0,
    };
    urls.iter()
        .any(|url| storage_cookie_matches_url(&storage_cookie, url))
}

pub(crate) fn complete_devtools_delete_cookies_result(
    conn: &mut CdpConnection,
    command: DevToolsDeleteCookiesCommand,
) -> Result<DevToolsDeleteCookiesResult, DevToolsError> {
    let browser_context_id = command
        .browser_context_id
        .or(command.context.browser_context_id)
        .map(DevToolsBrowserContextId::into_string);
    let session_id = (command.context.protocol == DevToolsProtocol::Cdp)
        .then(|| command.context.session_id.as_ref().map(|id| id.as_str()))
        .flatten();
    let target_id = command.context.target_id.as_ref().map(|id| id.as_str());
    let browser_context_id =
        match effective_storage_browser_context_id(conn, session_id, target_id, browser_context_id)
        {
            Ok(browser_context_id) => Some(browser_context_id),
            Err((code, message)) => return Err(devtools_error_from_code_message(code, message)),
        };
    let result = if command.filter.is_some() {
        delete_cookies_for_browser_context_by_filter(
            conn,
            browser_context_id,
            command.name,
            command.domain,
            command.path,
            command.filter.as_ref(),
        )
    } else {
        delete_cookies_for_browser_context_with_normalized_partition_key(
            conn,
            DeleteCookiesParams {
                name: command.name,
                url: command.url,
                domain: command.domain,
                path: command.path,
                partition_key: None,
                browser_context_id,
            },
            command.partition_key,
        )
    };
    match result {
        Ok(()) => Ok(DevToolsDeleteCookiesResult {
            partition_key: json!({}),
        }),
        Err((code, message)) => Err(devtools_error_from_code_message(code, message)),
    }
}

fn filter_cookies_by_devtools_filter(
    cookies: Vec<Value>,
    filter: Option<&crate::devtools_runtime::DevToolsCookieFilter>,
) -> Vec<Value> {
    let Some(filter) = filter else {
        return cookies;
    };
    cookies
        .into_iter()
        .filter(|cookie| cookie_matches_devtools_filter(cookie, filter))
        .collect()
}

fn cookie_matches_devtools_filter(
    cookie: &Value,
    filter: &crate::devtools_runtime::DevToolsCookieFilter,
) -> bool {
    if let Some(name) = filter.name.as_deref()
        && cookie.get("name").and_then(Value::as_str) != Some(name)
    {
        return false;
    }
    if let Some(value) = filter.value.as_deref()
        && cookie.get("value").and_then(Value::as_str) != Some(value)
    {
        return false;
    }
    if let Some(domain) = filter.domain.as_deref() {
        let actual = cookie
            .get("domain")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim_start_matches('.')
            .to_ascii_lowercase();
        if actual != domain.trim_start_matches('.').to_ascii_lowercase() {
            return false;
        }
    }
    if let Some(path) = filter.path.as_deref()
        && cookie.get("path").and_then(Value::as_str) != Some(path)
    {
        return false;
    }
    if let Some(secure) = filter.secure
        && cookie.get("secure").and_then(Value::as_bool) != Some(secure)
    {
        return false;
    }
    if let Some(http_only) = filter.http_only
        && cookie.get("httpOnly").and_then(Value::as_bool) != Some(http_only)
    {
        return false;
    }
    if let Some(same_site) = filter.same_site.as_deref()
        && cdp_cookie_same_site_for_filter(cookie) != same_site
    {
        return false;
    }
    if let Some(size) = filter.size
        && cookie.get("size").and_then(Value::as_u64) != Some(size)
    {
        return false;
    }
    if let Some(expires) = filter.expires {
        let actual = cookie
            .get("expires")
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| value.trunc() as i64);
        if actual != Some(expires) {
            return false;
        }
    }
    true
}

fn cdp_cookie_same_site_for_filter(cookie: &Value) -> &str {
    match cookie.get("sameSite").and_then(Value::as_str) {
        Some("None") => "none",
        Some("Lax") => "lax",
        Some("Strict") => "strict",
        _ => "default",
    }
}

fn delete_cookies_for_browser_context_by_filter(
    conn: &mut CdpConnection,
    browser_context_id: Option<String>,
    name: Option<String>,
    domain: Option<String>,
    path: Option<String>,
    filter: Option<&crate::devtools_runtime::DevToolsCookieFilter>,
) -> Result<(), (i32, &'static str)> {
    let cookies = get_cookies_for_browser_context(conn, Ok(browser_context_id.clone()))?;
    let cookies = filter_cookies_by_devtools_filter(cookies, filter);
    for cookie in cookies {
        delete_cookies_for_browser_context(
            conn,
            DeleteCookiesParams {
                name: name.clone().or_else(|| {
                    cookie
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                }),
                url: None,
                domain: domain.clone().or_else(|| {
                    cookie
                        .get("domain")
                        .and_then(Value::as_str)
                        .map(|domain| domain.trim_start_matches('.').to_owned())
                }),
                path: path.clone().or_else(|| {
                    cookie
                        .get("path")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                }),
                partition_key: None,
                browser_context_id: browser_context_id.clone(),
            },
        )?;
    }
    Ok(())
}

pub(crate) fn browser_context_id_from_optional_params(
    cmd: &Cmd<'_>,
) -> Result<Option<String>, &'static str> {
    match cmd.get_params::<BrowserContextParam>() {
        Ok(Some(params)) => Ok(params.browser_context_id),
        Ok(None) => Ok(None),
        Err(message) => Err(message),
    }
}

pub(crate) fn get_cookies_for_browser_context(
    conn: &mut CdpConnection,
    wanted_id: Result<Option<String>, &'static str>,
) -> Result<Vec<Value>, (i32, &'static str)> {
    let wanted_id = wanted_id.map_err(|message| (-32602, message))?;

    let bc = match wanted_id.as_deref() {
        Some(wanted_id) => conn
            .browser_context_by_id(wanted_id)
            .ok_or((-31998, "UnknownBrowserContextId"))?,
        None => conn
            .browser_context
            .as_ref()
            .ok_or((-31998, "BrowserContextNotLoaded"))?,
    };

    Ok(bc
        .snapshot_cookies()
        .iter()
        .map(storage_cookie_to_json)
        .collect::<Vec<_>>())
}

#[cfg(test)]
pub(crate) fn set_cookies_for_browser_context(
    conn: &mut CdpConnection,
    browser_context_id: Option<String>,
    cookies: Vec<CdpCookieParam>,
) -> Result<Vec<StoredCookieSetReport>, (i32, &'static str)> {
    let bc = match browser_context_id.as_deref() {
        Some(wanted_id) => conn
            .browser_context_by_id_mut(wanted_id)
            .ok_or((-31998, "UnknownBrowserContextId"))?,
        None => conn
            .browser_context
            .as_mut()
            .ok_or((-31998, "BrowserContextNotLoaded"))?,
    };
    // A single CDP `setCookies` call should observe one manager-owned browser
    // boundary snapshot for default scoped URL, readiness, and normalized
    // facade reject semantics instead of recomputing those per cookie.
    let manager_surface = bc.cookie_manager_surface_snapshot();
    Ok(set_cookies_with_manager_surface(
        bc,
        &manager_surface,
        cookies,
    ))
}

#[cfg(test)]
pub(crate) async fn set_cookies_for_browser_context_async(
    conn: &mut CdpConnection,
    browser_context_id: Option<String>,
    cookies: Vec<CdpCookieParam>,
) -> Result<Vec<StoredCookieSetReport>, (i32, &'static str)> {
    let bc = match browser_context_id.as_deref() {
        Some(wanted_id) => conn
            .browser_context_by_id_mut(wanted_id)
            .ok_or((-31998, "UnknownBrowserContextId"))?,
        None => conn
            .browser_context
            .as_mut()
            .ok_or((-31998, "BrowserContextNotLoaded"))?,
    };
    // A single CDP `setCookies` call should observe one manager-owned browser
    // boundary snapshot for default scoped URL, readiness, and normalized
    // facade reject semantics instead of recomputing those per cookie.
    let manager_surface = bc.cookie_manager_surface_snapshot_async().await;
    Ok(set_cookies_with_manager_surface(
        bc,
        &manager_surface,
        cookies,
    ))
}

fn set_cookies_with_manager_surface(
    browser_context: &BrowserContext,
    manager_surface: &BrowserContextCookieManagerSurfaceSnapshot,
    cookies: Vec<CdpCookieParam>,
) -> Vec<StoredCookieSetReport> {
    let mut reports = Vec::new();
    for cookie in cookies {
        match normalize_cookie_param(
            cookie,
            manager_surface
                .structured_write
                .default_cookie_write_url
                .as_ref(),
        ) {
            NormalizedCdpCookieParam::Ready(cookie, request_url) => reports.push(
                browser_context.execute_structured_cookie_write_with_manager_surface(
                    manager_surface,
                    *cookie,
                    request_url,
                ),
            ),
            NormalizedCdpCookieParam::Rejected(report) => reports.push(report),
        }
    }

    reports
}

fn set_cookie_reports_result(reports: &[StoredCookieSetReport]) -> DevToolsSetCookiesResult {
    DevToolsSetCookiesResult {
        success: reports.iter().all(StoredCookieSetReport::is_accepted),
        cookie_reports: reports
            .iter()
            .map(cookie_set_report_to_json)
            .collect::<Vec<_>>(),
        partition_key: json!({}),
    }
}

fn devtools_get_cookies_output_plan(
    result: Result<DevToolsGetCookiesResult, DevToolsError>,
) -> CommandOutputPlan {
    match result {
        Ok(result) => {
            CommandOutputPlan::from_devtools_result(DevToolsCommandResult::GetCookies(result))
        }
        Err(error) => CommandOutputPlan::from_devtools_error(error),
    }
}

fn devtools_delete_cookies_output_plan(
    result: Result<DevToolsDeleteCookiesResult, DevToolsError>,
) -> CommandOutputPlan {
    match result {
        Ok(result) => {
            CommandOutputPlan::from_devtools_result(DevToolsCommandResult::DeleteCookies(result))
        }
        Err(error) => CommandOutputPlan::from_devtools_error(error),
    }
}

fn devtools_set_cookies_output_plan(
    result: Result<DevToolsSetCookiesResult, DevToolsError>,
) -> CommandOutputPlan {
    match result {
        Ok(result) => {
            CommandOutputPlan::from_devtools_result(DevToolsCommandResult::SetCookies(result))
        }
        Err(error) => CommandOutputPlan::from_devtools_error(error),
    }
}

fn devtools_error_from_code_message(code: i32, message: impl Into<String>) -> DevToolsError {
    let message = message.into();
    let kind = match code {
        -32602 => DevToolsErrorKind::InvalidArgument,
        -32001 => DevToolsErrorKind::NoSuchSession,
        -31998 => DevToolsErrorKind::NoSuchTarget,
        -32000 if message == "UnsupportedDevToolsCommand" => DevToolsErrorKind::Unsupported,
        _ => DevToolsErrorKind::Internal,
    };
    DevToolsError::new(kind, message)
}

pub(crate) fn delete_cookies_for_browser_context(
    conn: &mut CdpConnection,
    params: DeleteCookiesParams,
) -> Result<(), (i32, &'static str)> {
    let partition_key = normalize_partition_key(params.partition_key.as_ref(), false)
        .map_err(|_| (-32602, "InvalidParams"))?;
    delete_cookies_for_browser_context_with_normalized_partition_key(conn, params, partition_key)
}

fn delete_cookies_for_browser_context_with_normalized_partition_key(
    conn: &mut CdpConnection,
    params: DeleteCookiesParams,
    partition_key: Option<moli_cookie_jar::StoredCookiePartitionKey>,
) -> Result<(), (i32, &'static str)> {
    let bc = match params.browser_context_id.as_deref() {
        Some(wanted_id) => conn
            .browser_context_by_id_mut(wanted_id)
            .ok_or((-31998, "UnknownBrowserContextId"))?,
        None => conn
            .browser_context
            .as_mut()
            .ok_or((-31998, "BrowserContextNotLoaded"))?,
    };

    let normalized_domain = params
        .domain
        .as_deref()
        .map(|domain| domain.trim().trim_start_matches('.').to_ascii_lowercase());
    let normalized_url_host = params.url.as_deref().and_then(|url| {
        Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
    });

    bc.delete_cookies_with_partition_key(
        params.name.as_deref(),
        normalized_domain.as_deref(),
        params.path.as_deref(),
        normalized_url_host.as_deref(),
        partition_key.as_ref(),
    );

    Ok(())
}

#[cfg(test)]
mod protocol_neutral_tests {
    use crate::devtools_runtime::{DevToolsCommand, DevToolsProtocol};
    use serde_json::{Value, json};

    use crate::conn::{CdpConnection, Cmd};

    use super::{
        StorageCommandTaskStep, build_cdp_storage_clear_cookies_command,
        build_cdp_storage_delete_cookies_command, build_cdp_storage_get_cookies_command,
        build_cdp_storage_set_cookies_command, start_devtools_storage_command,
    };

    #[test]
    fn cdp_storage_get_cookies_builds_protocol_neutral_command() {
        let params = json!({ "browserContextId": "BID-cookies" });
        let cmd = Cmd::for_test(
            Some(140),
            "Storage.getCookies",
            &params,
            Some("SID-storage"),
            r#"{"id":140,"method":"Storage.getCookies"}"#,
        );

        let command = build_cdp_storage_get_cookies_command(&cmd);
        let Ok(command) = command else {
            panic!("valid Storage.getCookies command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-storage")
        );
        assert_eq!(
            command.browser_context_id.as_ref().map(|id| id.as_str()),
            Some("BID-cookies")
        );
        assert_eq!(
            command
                .context
                .browser_context_id
                .as_ref()
                .map(|id| id.as_str()),
            Some("BID-cookies")
        );
        assert_eq!(command.urls, None);
    }

    #[test]
    fn devtools_storage_entry_routes_get_cookies_command_to_cookie_owner() {
        let mut conn = CdpConnection::new();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(141),
            "Storage.getCookies",
            &params,
            None,
            r#"{"id":141,"method":"Storage.getCookies"}"#,
        );
        let command = build_cdp_storage_get_cookies_command(&cmd);
        let Ok(command) = command else {
            panic!("default Storage.getCookies command should build");
        };

        let step =
            start_devtools_storage_command(&mut conn, cmd.id, DevToolsCommand::GetCookies(command));

        let StorageCommandTaskStep::Complete(plan) = step else {
            panic!("getCookies should complete through the shared storage entry");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(141));
        assert_eq!(out[0]["error"]["code"], json!(-31998));
        assert_eq!(out[0]["error"]["message"], json!("BrowserContextNotLoaded"));
    }

    #[test]
    fn cdp_storage_delete_cookies_builds_protocol_neutral_command() {
        let params = json!({
            "browserContextId": "BID-cookies",
            "name": "sid",
            "url": "https://example.com/",
            "domain": "example.com",
            "path": "/",
            "partitionKey": {
                "topLevelSite": "https://top.example",
                "hasCrossSiteAncestor": false
            }
        });
        let cmd = Cmd::for_test(
            Some(145),
            "Storage.deleteCookies",
            &params,
            Some("SID-storage"),
            r#"{"id":145,"method":"Storage.deleteCookies"}"#,
        );

        let command = build_cdp_storage_delete_cookies_command(&cmd);
        let Ok(command) = command else {
            panic!("valid Storage.deleteCookies command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-storage")
        );
        assert_eq!(
            command.browser_context_id.as_ref().map(|id| id.as_str()),
            Some("BID-cookies")
        );
        assert_eq!(command.name.as_deref(), Some("sid"));
        assert_eq!(command.url.as_deref(), Some("https://example.com/"));
        assert_eq!(command.domain.as_deref(), Some("example.com"));
        assert_eq!(command.path.as_deref(), Some("/"));
        assert_eq!(
            command.partition_key,
            Some(moli_cookie_jar::StoredCookiePartitionKey::site(
                "https://top.example".to_owned(),
                false,
            ))
        );
    }

    #[test]
    fn devtools_storage_entry_routes_delete_cookies_command_to_cookie_owner() {
        let mut conn = CdpConnection::new();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(146),
            "Storage.deleteCookies",
            &params,
            None,
            r#"{"id":146,"method":"Storage.deleteCookies"}"#,
        );
        let command = build_cdp_storage_delete_cookies_command(&cmd);
        let Ok(command) = command else {
            panic!("default Storage.deleteCookies command should build");
        };

        let step = start_devtools_storage_command(
            &mut conn,
            cmd.id,
            DevToolsCommand::DeleteCookies(command),
        );

        let StorageCommandTaskStep::Complete(plan) = step else {
            panic!("deleteCookies should complete through the shared storage entry");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(146));
        assert_eq!(out[0]["error"]["code"], json!(-31998));
        assert_eq!(out[0]["error"]["message"], json!("BrowserContextNotLoaded"));
    }

    #[test]
    fn cdp_storage_clear_cookies_builds_protocol_neutral_delete_command() {
        let params = json!({ "browserContextId": "BID-cookies" });
        let cmd = Cmd::for_test(
            Some(152),
            "Storage.clearCookies",
            &params,
            Some("SID-storage"),
            r#"{"id":152,"method":"Storage.clearCookies"}"#,
        );

        let command = build_cdp_storage_clear_cookies_command(&cmd);
        let Ok(command) = command else {
            panic!("valid Storage.clearCookies command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-storage")
        );
        assert_eq!(
            command.browser_context_id.as_ref().map(|id| id.as_str()),
            Some("BID-cookies")
        );
        assert_eq!(command.name, None);
        assert_eq!(command.url, None);
        assert_eq!(command.domain, None);
        assert_eq!(command.path, None);
    }

    #[test]
    fn devtools_storage_entry_routes_clear_cookies_command_to_cookie_owner() {
        let mut conn = CdpConnection::new();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(153),
            "Storage.clearCookies",
            &params,
            None,
            r#"{"id":153,"method":"Storage.clearCookies"}"#,
        );
        let command = build_cdp_storage_clear_cookies_command(&cmd);
        let Ok(command) = command else {
            panic!("default Storage.clearCookies command should build");
        };

        let step = start_devtools_storage_command(
            &mut conn,
            cmd.id,
            DevToolsCommand::DeleteCookies(command),
        );

        let StorageCommandTaskStep::Complete(plan) = step else {
            panic!("clearCookies should complete through the shared storage entry");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(153));
        assert_eq!(out[0]["error"]["code"], json!(-31998));
        assert_eq!(out[0]["error"]["message"], json!("BrowserContextNotLoaded"));
    }

    #[test]
    fn cdp_storage_set_cookies_builds_protocol_neutral_command() {
        let params = json!({
            "browserContextId": "BID-cookies",
            "cookies": [{
                "name": "sid",
                "value": "1",
                "url": "https://example.com/",
                "httpOnly": true,
                "sameSite": "Lax"
            }]
        });
        let cmd = Cmd::for_test(
            Some(149),
            "Storage.setCookies",
            &params,
            Some("SID-storage"),
            r#"{"id":149,"method":"Storage.setCookies"}"#,
        );

        let command = build_cdp_storage_set_cookies_command(&cmd);
        let Ok(command) = command else {
            panic!("valid Storage.setCookies command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.browser_context_id.as_ref().map(|id| id.as_str()),
            Some("BID-cookies")
        );
        assert_eq!(command.cookies.len(), 1);
        let cookie = &command.cookies[0];
        assert_eq!(cookie.name, "sid");
        assert_eq!(cookie.value, "1");
        assert_eq!(cookie.url.as_deref(), Some("https://example.com/"));
        assert!(cookie.http_only);
        assert_eq!(cookie.same_site.as_deref(), Some("Lax"));
    }

    #[test]
    fn devtools_storage_entry_routes_set_cookies_command_to_cookie_owner() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "cookies": [{
                "name": "sid",
                "value": "1",
                "url": "https://example.com/"
            }]
        });
        let cmd = Cmd::for_test(
            Some(150),
            "Storage.setCookies",
            &params,
            None,
            r#"{"id":150,"method":"Storage.setCookies"}"#,
        );
        let command = build_cdp_storage_set_cookies_command(&cmd);
        let Ok(command) = command else {
            panic!("valid Storage.setCookies command");
        };

        let step =
            start_devtools_storage_command(&mut conn, cmd.id, DevToolsCommand::SetCookies(command));

        let StorageCommandTaskStep::Complete(plan) = step else {
            panic!("missing browser context should fail synchronously");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(150));
        assert_eq!(out[0]["error"]["code"], json!(-31998));
        assert_eq!(out[0]["error"]["message"], json!("BrowserContextNotLoaded"));
    }
}

#[cfg(test)]
mod storage_type_tests {
    use super::{
        ClearDataStorageType, storage_types_include_cache_storage, storage_types_include_cookies,
        storage_types_include_indexed_db, storage_types_include_local_storage,
        storage_types_include_storage_buckets,
    };

    #[test]
    fn clear_data_storage_type_tokens_are_derived_and_case_insensitive() {
        assert_eq!(
            "cookies".parse::<ClearDataStorageType>(),
            Ok(ClearDataStorageType::Cookies)
        );
        assert_eq!(
            "LOCAL_STORAGE".parse::<ClearDataStorageType>(),
            Ok(ClearDataStorageType::LocalStorage)
        );
        assert_eq!(
            "IndexedDB".parse::<ClearDataStorageType>(),
            Ok(ClearDataStorageType::IndexedDb)
        );
        assert_eq!(
            "all".parse::<ClearDataStorageType>(),
            Ok(ClearDataStorageType::All)
        );
        assert_eq!(
            "cache_storage".parse::<ClearDataStorageType>(),
            Ok(ClearDataStorageType::CacheStorage)
        );
        assert_eq!(
            "storage_buckets".parse::<ClearDataStorageType>(),
            Ok(ClearDataStorageType::StorageBuckets)
        );
    }

    #[test]
    fn storage_type_helpers_match_selected_tokens_and_all() {
        assert!(storage_types_include_cookies(" cookies , local_storage "));
        assert!(storage_types_include_local_storage("cookies,LOCAL_STORAGE"));
        assert!(storage_types_include_indexed_db("all"));
        assert!(storage_types_include_storage_buckets("all"));
        assert!(storage_types_include_storage_buckets("storage_buckets"));
        assert!(storage_types_include_cache_storage("cache_storage"));
        assert!(!storage_types_include_cookies("local_storage,indexeddb"));
        assert!(!storage_types_include_indexed_db("cookies,unknown"));
        assert!(!storage_types_include_storage_buckets("cookies,unknown"));
    }
}
