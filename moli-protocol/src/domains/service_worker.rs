use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use url::Url;

use crate::conn::{
    BackgroundProtocolEvent, BackgroundServiceWorkerErrorMessage,
    BackgroundServiceWorkerRegistration, BackgroundServiceWorkerVersion, BrowserContext,
    CdpConnection, CdpSessionRoute, Cmd, ServiceWorkerTargetState,
};
use crate::domains::actions::ServiceWorkerAction;
use crate::domains::command_output::CommandOutputPlan;

pub(crate) fn command_output_plan(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    match cmd.parse_action::<ServiceWorkerAction>() {
        Some(ServiceWorkerAction::Enable) => enable_command(conn, cmd),
        Some(ServiceWorkerAction::Disable) => disable_command(conn, cmd),
        Some(ServiceWorkerAction::SetForceUpdateOnPageLoad) => {
            set_force_update_on_page_load_command(conn, cmd)
        }
        Some(
            action @ (ServiceWorkerAction::DeliverPushMessage
            | ServiceWorkerAction::DispatchPeriodicSyncEvent
            | ServiceWorkerAction::DispatchSyncEvent
            | ServiceWorkerAction::SkipWaiting
            | ServiceWorkerAction::StartWorker
            | ServiceWorkerAction::StopAllWorkers
            | ServiceWorkerAction::StopWorker
            | ServiceWorkerAction::Unregister
            | ServiceWorkerAction::UpdateRegistration),
        ) => lifecycle_command(conn, cmd, action),
        None => CommandOutputPlan::error(-32601, "Unknown ServiceWorker method"),
    }
}

fn enable_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let Ok(browser_context_id) = browser_context_id_for_command(conn, cmd.session_id) else {
        return CommandOutputPlan::error(-32001, "Unknown sessionId");
    };
    let Some(context) = conn.browser_context_by_id_mut(&browser_context_id) else {
        return CommandOutputPlan::error(-32000, "No browser context");
    };
    context.set_service_worker_domain_enabled(cmd.session_id, true);

    let mut plan = CommandOutputPlan::success();
    for event in snapshot_events_for_context(context, cmd.session_id) {
        plan.push_background_event(event);
    }
    plan
}

fn disable_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let Ok(browser_context_id) = browser_context_id_for_command(conn, cmd.session_id) else {
        return CommandOutputPlan::error(-32001, "Unknown sessionId");
    };
    if let Some(context) = conn.browser_context_by_id_mut(&browser_context_id) {
        let was_enabled = context
            .service_worker_domain_enabled_sessions()
            .iter()
            .any(|session_id| session_id.as_deref() == cmd.session_id);
        context.set_service_worker_domain_enabled(cmd.session_id, false);
        if was_enabled {
            context
                .renderer_runtime()
                .set_service_worker_force_update_on_page_load_for_devtools(false);
        }
    }
    CommandOutputPlan::success()
}

fn set_force_update_on_page_load_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let Ok(browser_context_id) = browser_context_id_for_command(conn, cmd.session_id) else {
        return CommandOutputPlan::error(-32001, "Unknown sessionId");
    };
    let Some(context) = conn.browser_context_by_id(&browser_context_id) else {
        return CommandOutputPlan::error(-32000, "No browser context");
    };
    let Some(force_update) = force_update_on_page_load_param(cmd) else {
        return CommandOutputPlan::error(
            -32602,
            "Invalid ServiceWorker setForceUpdateOnPageLoad params",
        );
    };
    context
        .renderer_runtime()
        .set_service_worker_force_update_on_page_load_for_devtools(force_update);
    CommandOutputPlan::success()
}

fn lifecycle_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: ServiceWorkerAction,
) -> CommandOutputPlan {
    let Ok(browser_context_id) = browser_context_id_for_command(conn, cmd.session_id) else {
        return CommandOutputPlan::error(-32001, "Unknown sessionId");
    };
    let Some(context) = conn.browser_context_by_id(&browser_context_id) else {
        return CommandOutputPlan::error(-32000, "No browser context");
    };
    if !context
        .service_worker_domain_enabled_sessions()
        .iter()
        .any(|session_id| session_id.as_deref() == cmd.session_id)
    {
        return CommandOutputPlan::error(-32000, "ServiceWorker domain is not enabled");
    }
    let renderer_runtime = context.renderer_runtime();
    let result = match action {
        ServiceWorkerAction::Unregister => {
            let Some(scope_url) = scope_url_param(cmd) else {
                return CommandOutputPlan::error(-32602, "Invalid ServiceWorker scopeURL");
            };
            renderer_runtime
                .unregister_service_worker_scope_for_devtools(&scope_url)
                .map(|_| ())
        }
        ServiceWorkerAction::StartWorker => {
            let Some(scope_url) = scope_url_param(cmd) else {
                return CommandOutputPlan::error(-32602, "Invalid ServiceWorker scopeURL");
            };
            renderer_runtime
                .start_service_worker_for_devtools(&scope_url)
                .map(|_| ())
        }
        ServiceWorkerAction::StopWorker => {
            let Some(version_id) = version_id_param(cmd) else {
                return CommandOutputPlan::error(-32602, "Invalid ServiceWorker versionId");
            };
            renderer_runtime
                .stop_service_worker_for_devtools(version_id)
                .map(|_| ())
        }
        ServiceWorkerAction::StopAllWorkers => renderer_runtime
            .stop_all_service_workers_for_devtools()
            .map(|_| ()),
        ServiceWorkerAction::SkipWaiting => {
            let Some(scope_url) = scope_url_param(cmd) else {
                return CommandOutputPlan::error(-32602, "Invalid ServiceWorker scopeURL");
            };
            renderer_runtime
                .skip_waiting_service_worker_for_devtools(&scope_url)
                .map(|_| ())
        }
        ServiceWorkerAction::UpdateRegistration => {
            let Some(scope_url) = scope_url_param(cmd) else {
                return CommandOutputPlan::error(-32602, "Invalid ServiceWorker scopeURL");
            };
            renderer_runtime
                .update_service_worker_registration_for_devtools(&scope_url)
                .map(|_| ())
        }
        ServiceWorkerAction::DeliverPushMessage => {
            let Some(params) = deliver_push_message_params(cmd) else {
                return CommandOutputPlan::error(
                    -32602,
                    "Invalid ServiceWorker deliverPushMessage params",
                );
            };
            renderer_runtime
                .deliver_push_message_for_devtools(
                    &params.origin,
                    params.registration_id,
                    params.data,
                )
                .map(|_| ())
        }
        ServiceWorkerAction::DispatchSyncEvent => {
            let Some(params) = sync_event_params(cmd) else {
                return CommandOutputPlan::error(
                    -32602,
                    "Invalid ServiceWorker dispatchSyncEvent params",
                );
            };
            renderer_runtime
                .dispatch_sync_event_for_devtools(
                    &params.origin,
                    params.registration_id,
                    params.tag,
                    params.last_chance,
                )
                .map(|_| ())
        }
        ServiceWorkerAction::DispatchPeriodicSyncEvent => {
            let Some(params) = periodic_sync_event_params(cmd) else {
                return CommandOutputPlan::error(
                    -32602,
                    "Invalid ServiceWorker dispatchPeriodicSyncEvent params",
                );
            };
            renderer_runtime
                .dispatch_periodic_sync_event_for_devtools(
                    &params.origin,
                    params.registration_id,
                    params.tag,
                )
                .map(|_| ())
        }
        ServiceWorkerAction::Enable
        | ServiceWorkerAction::Disable
        | ServiceWorkerAction::SetForceUpdateOnPageLoad => {
            Err(format!("ServiceWorker.{} is not implemented", cmd.action))
        }
    };
    match result {
        Ok(()) => CommandOutputPlan::success(),
        Err(message) => CommandOutputPlan::error(-32000, message),
    }
}

#[derive(Deserialize)]
struct ScopeUrlParams {
    #[serde(rename = "scopeURL")]
    scope_url: String,
}

#[derive(Deserialize)]
struct VersionIdParams {
    #[serde(rename = "versionId")]
    version_id: String,
}

#[derive(Deserialize)]
struct DeliverPushMessageParams {
    origin: String,
    #[serde(rename = "registrationId")]
    registration_id: String,
    data: String,
}

#[derive(Deserialize)]
struct SyncEventParams {
    origin: String,
    #[serde(rename = "registrationId")]
    registration_id: String,
    tag: String,
    #[serde(rename = "lastChance")]
    last_chance: bool,
}

#[derive(Deserialize)]
struct PeriodicSyncEventParams {
    origin: String,
    #[serde(rename = "registrationId")]
    registration_id: String,
    tag: String,
}

#[derive(Deserialize)]
struct ForceUpdateOnPageLoadParams {
    #[serde(rename = "forceUpdateOnPageLoad")]
    force_update_on_page_load: bool,
}

struct ParsedDeliverPushMessageParams {
    origin: Url,
    registration_id: u64,
    data: Option<Vec<u8>>,
}

struct ParsedSyncEventParams {
    origin: Url,
    registration_id: u64,
    tag: String,
    last_chance: bool,
}

struct ParsedPeriodicSyncEventParams {
    origin: Url,
    registration_id: u64,
    tag: String,
}

fn scope_url_param(cmd: &Cmd<'_>) -> Option<Url> {
    let params = cmd.get_params::<ScopeUrlParams>().ok().flatten()?;
    Url::parse(&params.scope_url).ok()
}

fn version_id_param(cmd: &Cmd<'_>) -> Option<u64> {
    let params = cmd.get_params::<VersionIdParams>().ok().flatten()?;
    params.version_id.parse().ok()
}

fn deliver_push_message_params(cmd: &Cmd<'_>) -> Option<ParsedDeliverPushMessageParams> {
    let params = cmd
        .get_params::<DeliverPushMessageParams>()
        .ok()
        .flatten()?;
    Some(ParsedDeliverPushMessageParams {
        origin: Url::parse(&params.origin).ok()?,
        registration_id: params.registration_id.parse().ok()?,
        data: (!params.data.is_empty()).then(|| params.data.into_bytes()),
    })
}

fn sync_event_params(cmd: &Cmd<'_>) -> Option<ParsedSyncEventParams> {
    let params = cmd.get_params::<SyncEventParams>().ok().flatten()?;
    Some(ParsedSyncEventParams {
        origin: Url::parse(&params.origin).ok()?,
        registration_id: params.registration_id.parse().ok()?,
        tag: params.tag,
        last_chance: params.last_chance,
    })
}

fn periodic_sync_event_params(cmd: &Cmd<'_>) -> Option<ParsedPeriodicSyncEventParams> {
    let params = cmd.get_params::<PeriodicSyncEventParams>().ok().flatten()?;
    Some(ParsedPeriodicSyncEventParams {
        origin: Url::parse(&params.origin).ok()?,
        registration_id: params.registration_id.parse().ok()?,
        tag: params.tag,
    })
}

fn force_update_on_page_load_param(cmd: &Cmd<'_>) -> Option<bool> {
    cmd.get_params::<ForceUpdateOnPageLoadParams>()
        .ok()
        .flatten()
        .map(|params| params.force_update_on_page_load)
}

fn browser_context_id_for_command(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> Result<String, ()> {
    if let Some(session_id) = session_id {
        let Some(route) = conn.session_route(Some(session_id)) else {
            return Err(());
        };
        return browser_context_id_for_route(conn, route).ok_or(());
    }
    conn.browser_context
        .as_ref()
        .map(|context| context.id.clone())
        .ok_or(())
}

fn browser_context_id_for_route(conn: &CdpConnection, route: CdpSessionRoute) -> Option<String> {
    route.browser_context_id().map(str::to_owned).or_else(|| {
        conn.browser_context
            .as_ref()
            .map(|context| context.id.clone())
    })
}

pub(in crate::domains) fn enabled_sessions_for_browser_context(
    conn: &CdpConnection,
    browser_context_id: &str,
) -> Vec<Option<String>> {
    let Some(context) = conn.browser_context_by_id(browser_context_id) else {
        return Vec::new();
    };
    context
        .service_worker_domain_enabled_sessions()
        .into_iter()
        .filter(|session_id| {
            service_worker_domain_session_still_routes(conn, browser_context_id, session_id)
        })
        .collect()
}

fn service_worker_domain_session_still_routes(
    conn: &CdpConnection,
    browser_context_id: &str,
    session_id: &Option<String>,
) -> bool {
    let Some(session_id) = session_id.as_deref() else {
        return conn
            .browser_context
            .as_ref()
            .is_some_and(|context| context.id == browser_context_id);
    };
    conn.session_route(Some(session_id))
        .and_then(|route| route.browser_context_id().map(str::to_owned))
        .is_some_and(|route_browser_context_id| route_browser_context_id == browser_context_id)
}

pub(in crate::domains) fn snapshot_events_for_browser_context(
    conn: &CdpConnection,
    browser_context_id: &str,
    session_ids: &[Option<String>],
) -> Vec<BackgroundProtocolEvent> {
    let Some(context) = conn.browser_context_by_id(browser_context_id) else {
        return Vec::new();
    };
    session_ids
        .iter()
        .flat_map(|session_id| snapshot_events_for_context(context, session_id.as_deref()))
        .collect()
}

fn snapshot_events_for_context(
    context: &BrowserContext,
    session_id: Option<&str>,
) -> Vec<BackgroundProtocolEvent> {
    let registrations = registrations_for_context(context, false);
    let versions = versions_for_context(context, None);
    let mut events = Vec::new();
    if !registrations.is_empty() {
        events.push(worker_registration_updated_event(registrations, session_id));
    }
    if !versions.is_empty() {
        events.push(worker_version_updated_event(versions, session_id));
    }
    events
}

pub(in crate::domains) fn deleted_target_events(
    target: &ServiceWorkerTargetState,
    registration_deleted: bool,
    session_ids: &[Option<String>],
) -> Vec<BackgroundProtocolEvent> {
    session_ids
        .iter()
        .flat_map(|session_id| {
            let mut events = Vec::new();
            if registration_deleted {
                events.push(worker_registration_updated_event(
                    vec![registration_for_target(target, true)],
                    session_id.as_deref(),
                ));
            }
            events.push(worker_version_updated_event(
                vec![version_for_target_with_controlled_clients(
                    target,
                    Some("redundant"),
                    Vec::new(),
                )],
                session_id.as_deref(),
            ));
            events
        })
        .collect()
}

pub(in crate::domains) fn error_reported_events(
    target: &ServiceWorkerTargetState,
    message: &moli_core::page::RendererServiceWorkerExceptionMessage,
    session_ids: &[Option<String>],
) -> Vec<BackgroundProtocolEvent> {
    session_ids
        .iter()
        .map(|session_id| {
            BackgroundProtocolEvent::service_worker_error_reported(
                session_id.as_deref(),
                BackgroundServiceWorkerErrorMessage {
                    error_message: message.message.clone(),
                    registration_id: target.renderer_registration_id.to_string(),
                    version_id: target.renderer_version_id.to_string(),
                    source_url: message.filename.clone(),
                    line_number: message.lineno,
                    column_number: message.colno,
                },
            )
        })
        .collect()
}

fn registrations_for_context(
    context: &BrowserContext,
    is_deleted: bool,
) -> Vec<BackgroundServiceWorkerRegistration> {
    let mut registrations = BTreeMap::new();
    for target in context.service_worker_targets.values() {
        registrations
            .entry(target.renderer_registration_id)
            .or_insert_with(|| registration_for_target(target, is_deleted));
    }
    registrations.into_values().collect()
}

fn versions_for_context(
    context: &BrowserContext,
    status_override: Option<&'static str>,
) -> Vec<BackgroundServiceWorkerVersion> {
    context
        .service_worker_targets
        .values()
        .map(|target| version_for_target_in_context(context, target, status_override))
        .collect()
}

pub(in crate::domains) fn version_updated_events_for_target(
    context: &BrowserContext,
    target: &ServiceWorkerTargetState,
    session_ids: &[Option<String>],
) -> Vec<BackgroundProtocolEvent> {
    let version = version_for_target_in_context(context, target, None);
    session_ids
        .iter()
        .map(|session_id| {
            worker_version_updated_event(vec![version.clone()], session_id.as_deref())
        })
        .collect()
}

fn registration_for_target(
    target: &ServiceWorkerTargetState,
    is_deleted: bool,
) -> BackgroundServiceWorkerRegistration {
    BackgroundServiceWorkerRegistration {
        registration_id: target.renderer_registration_id.to_string(),
        scope_url: target.scope_url.clone(),
        is_deleted,
    }
}

fn version_for_target_in_context(
    context: &BrowserContext,
    target: &ServiceWorkerTargetState,
    status_override: Option<&'static str>,
) -> BackgroundServiceWorkerVersion {
    version_for_target_with_controlled_clients(
        target,
        status_override,
        controlled_client_target_ids_for_target(context, target),
    )
}

fn version_for_target_with_controlled_clients(
    target: &ServiceWorkerTargetState,
    status_override: Option<&'static str>,
    controlled_clients: Vec<String>,
) -> BackgroundServiceWorkerVersion {
    BackgroundServiceWorkerVersion {
        version_id: target.renderer_version_id.to_string(),
        registration_id: target.renderer_registration_id.to_string(),
        script_url: target.script_url.clone(),
        running_status: target.running_status_cdp_str().to_owned(),
        status: status_override
            .unwrap_or_else(|| target.version_status_cdp_str())
            .to_owned(),
        controlled_clients,
        target_id: target.target_id.clone(),
    }
}

fn controlled_client_target_ids_for_target(
    context: &BrowserContext,
    target: &ServiceWorkerTargetState,
) -> Vec<String> {
    let controlled_client_ids = context
        .renderer_runtime()
        .controlled_service_worker_window_client_ids_for_devtools(
            target.renderer_registration_id,
            target.renderer_version_id,
        );
    page_target_ids_for_controlled_client_ids(context, &controlled_client_ids)
}

fn page_target_ids_for_controlled_client_ids(
    context: &BrowserContext,
    controlled_client_ids: &[u64],
) -> Vec<String> {
    let controlled_client_ids = controlled_client_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if controlled_client_ids.is_empty() {
        return Vec::new();
    }

    let mut target_ids = BTreeSet::new();
    if let (Some(target_id), Some(page)) = (context.active_target_id(), context.loaded_page())
        && controlled_client_ids.contains(&page.service_worker_client_id())
    {
        target_ids.insert(target_id.to_owned());
    }
    for target in context.background_targets() {
        let Some(page) = target.loaded_page() else {
            continue;
        };
        if controlled_client_ids.contains(&page.service_worker_client_id()) {
            target_ids.insert(target.target_id().to_owned());
        }
    }
    target_ids.into_iter().collect()
}

fn worker_registration_updated_event(
    registrations: Vec<BackgroundServiceWorkerRegistration>,
    session_id: Option<&str>,
) -> BackgroundProtocolEvent {
    BackgroundProtocolEvent::service_worker_registration_updated(session_id, registrations)
}

fn worker_version_updated_event(
    versions: Vec<BackgroundServiceWorkerVersion>,
    session_id: Option<&str>,
) -> BackgroundProtocolEvent {
    BackgroundProtocolEvent::service_worker_version_updated(session_id, versions)
}

#[cfg(test)]
mod tests {
    use moli_core::page::RendererServiceWorkerVersionStatus;
    use serde_json::json;

    use crate::{
        conn::{BrowserContext, PageTargetHost, ServiceWorkerTargetState},
        testing::TestContext,
    };

    fn browser_context_with_service_worker_target() -> BrowserContext {
        let mut context = BrowserContext::new("BID-1".to_owned());
        let mut target = ServiceWorkerTargetState::new(
            41,
            7,
            "TID-service-worker".to_owned(),
            "https://example.test/service-worker.js".to_owned(),
            "https://example.test/app/".to_owned(),
            RendererServiceWorkerVersionStatus::Activated,
            None,
        );
        target.attach_session("SID-service-worker".to_owned());
        context.insert_service_worker_target(target);
        context
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn controlled_client_ids_map_to_exact_page_target_ids_even_for_same_url() {
        let mut ctx = TestContext::new();
        let active_page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<title>same-url</title>")
            .await
            .expect("active page should load");
        let background_page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<title>same-url</title>")
            .await
            .expect("background page should load");
        let active_client_id = active_page.service_worker_client_id();
        let background_client_id = background_page.service_worker_client_id();
        assert_ne!(active_client_id, background_client_id);

        let mut context = BrowserContext::new("BID-1".to_owned());
        context.set_active_target_id("TID-active".to_owned());
        context.set_target_url("data:text/html,<title>same-url</title>".to_owned());
        let _ = context.replace_loaded_page(Some(active_page));

        let mut background = PageTargetHost::with_url(
            "TID-background".to_owned(),
            None,
            "data:text/html,<title>same-url</title>".to_owned(),
        );
        let _ = background.replace_loaded_page(Some(background_page));
        context.insert_page_target_host(background);

        assert_eq!(
            super::page_target_ids_for_controlled_client_ids(&context, &[background_client_id]),
            vec!["TID-background".to_owned()]
        );
        assert_eq!(
            super::page_target_ids_for_controlled_client_ids(
                &context,
                &[
                    active_client_id,
                    background_client_id,
                    active_client_id,
                    999_999
                ],
            ),
            vec!["TID-active".to_owned(), "TID-background".to_owned()]
        );
    }

    #[tokio::test]
    async fn service_worker_enable_replays_registration_and_version_snapshot() {
        let mut ctx = TestContext::new();
        let mut context = browser_context_with_service_worker_target();
        context.set_active_target_id("TID-page".to_owned());
        context.attach_active_session("SID-page".to_owned());
        ctx.conn.install_browser_context_fixture_for_test(context);

        ctx.process_async(json!({
            "id": 1,
            "method": "ServiceWorker.enable",
            "sessionId": "SID-page"
        }))
        .await;
        ctx.expect_result(1, json!({}), Some("SID-page"));

        let registration = ctx.take_one();
        assert_eq!(
            registration["method"],
            "ServiceWorker.workerRegistrationUpdated"
        );
        assert_eq!(registration["sessionId"], "SID-page");
        assert_eq!(
            registration["params"]["registrations"][0],
            json!({
                "registrationId": "41",
                "scopeURL": "https://example.test/app/",
                "isDeleted": false
            })
        );

        let version = ctx.take_one();
        assert_eq!(version["method"], "ServiceWorker.workerVersionUpdated");
        assert_eq!(version["sessionId"], "SID-page");
        assert_eq!(version["params"]["versions"][0]["versionId"], "7");
        assert_eq!(version["params"]["versions"][0]["registrationId"], "41");
        assert_eq!(
            version["params"]["versions"][0]["scriptURL"],
            "https://example.test/service-worker.js"
        );
        assert_eq!(version["params"]["versions"][0]["runningStatus"], "stopped");
        assert_eq!(version["params"]["versions"][0]["status"], "activated");
        assert_eq!(
            version["params"]["versions"][0]["controlledClients"],
            json!([])
        );
        assert_eq!(
            version["params"]["versions"][0]["targetId"],
            "TID-service-worker"
        );
    }

    #[tokio::test]
    async fn service_worker_lifecycle_commands_require_enable_then_delegate_to_runtime() {
        let mut ctx = TestContext::new();
        let mut context = browser_context_with_service_worker_target();
        context.set_active_target_id("TID-page".to_owned());
        context.attach_active_session("SID-page".to_owned());
        ctx.conn.install_browser_context_fixture_for_test(context);

        ctx.process_async(json!({
            "id": 2,
            "method": "ServiceWorker.startWorker",
            "params": {"scopeURL": "https://example.test/app/"},
            "sessionId": "SID-page"
        }))
        .await;
        let error = ctx.take_one();
        assert_eq!(error["id"], 2);
        assert_eq!(error["sessionId"], "SID-page");
        assert_eq!(error["error"]["code"], -32000);

        ctx.process_async(json!({
            "id": 3,
            "method": "ServiceWorker.enable",
            "sessionId": "SID-page"
        }))
        .await;
        ctx.expect_result(3, json!({}), Some("SID-page"));
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 4,
            "method": "ServiceWorker.startWorker",
            "params": {"scopeURL": "https://example.test/app/"},
            "sessionId": "SID-page"
        }))
        .await;
        ctx.expect_result(4, json!({}), Some("SID-page"));

        ctx.process_async(json!({
            "id": 5,
            "method": "ServiceWorker.stopWorker",
            "params": {"versionId": "7"},
            "sessionId": "SID-page"
        }))
        .await;
        ctx.expect_result(5, json!({}), Some("SID-page"));

        ctx.process_async(json!({
            "id": 6,
            "method": "ServiceWorker.stopAllWorkers",
            "sessionId": "SID-page"
        }))
        .await;
        ctx.expect_result(6, json!({}), Some("SID-page"));

        ctx.process_async(json!({
            "id": 7,
            "method": "ServiceWorker.skipWaiting",
            "params": {"scopeURL": "https://example.test/app/"},
            "sessionId": "SID-page"
        }))
        .await;
        ctx.expect_result(7, json!({}), Some("SID-page"));

        ctx.process_async(json!({
            "id": 8,
            "method": "ServiceWorker.unregister",
            "params": {"scopeURL": "https://example.test/app/"},
            "sessionId": "SID-page"
        }))
        .await;
        ctx.expect_result(8, json!({}), Some("SID-page"));

        ctx.process_async(json!({
            "id": 9,
            "method": "ServiceWorker.updateRegistration",
            "params": {"scopeURL": "https://example.test/app/"},
            "sessionId": "SID-page"
        }))
        .await;
        ctx.expect_result(9, json!({}), Some("SID-page"));

        ctx.process_async(json!({
            "id": 10,
            "method": "ServiceWorker.deliverPushMessage",
            "params": {
                "origin": "https://example.test/",
                "registrationId": "41",
                "data": "payload"
            },
            "sessionId": "SID-page"
        }))
        .await;
        ctx.expect_result(10, json!({}), Some("SID-page"));

        ctx.process_async(json!({
            "id": 11,
            "method": "ServiceWorker.dispatchSyncEvent",
            "params": {
                "origin": "https://example.test/",
                "registrationId": "41",
                "tag": "sync-tag",
                "lastChance": true
            },
            "sessionId": "SID-page"
        }))
        .await;
        ctx.expect_result(11, json!({}), Some("SID-page"));

        ctx.process_async(json!({
            "id": 12,
            "method": "ServiceWorker.dispatchPeriodicSyncEvent",
            "params": {
                "origin": "https://example.test/",
                "registrationId": "41",
                "tag": "periodic-tag"
            },
            "sessionId": "SID-page"
        }))
        .await;
        ctx.expect_result(12, json!({}), Some("SID-page"));
    }

    #[tokio::test]
    async fn service_worker_force_update_on_page_load_does_not_require_enable_and_disable_resets() {
        let mut ctx = TestContext::new();
        let mut context = browser_context_with_service_worker_target();
        context.set_active_target_id("TID-page".to_owned());
        context.attach_active_session("SID-page".to_owned());
        ctx.conn.install_browser_context_fixture_for_test(context);

        ctx.process_async(json!({
            "id": 1,
            "method": "ServiceWorker.setForceUpdateOnPageLoad",
            "params": {"forceUpdateOnPageLoad": true},
            "sessionId": "SID-page"
        }))
        .await;
        ctx.expect_result(1, json!({}), Some("SID-page"));
        assert!(
            ctx.conn
                .browser_context
                .as_ref()
                .unwrap()
                .renderer_runtime()
                .service_worker_force_update_on_page_load_for_devtools()
        );

        ctx.process_async(json!({
            "id": 2,
            "method": "ServiceWorker.enable",
            "sessionId": "SID-page"
        }))
        .await;
        ctx.expect_result(2, json!({}), Some("SID-page"));
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 3,
            "method": "ServiceWorker.disable",
            "sessionId": "SID-page"
        }))
        .await;
        ctx.expect_result(3, json!({}), Some("SID-page"));
        assert!(
            !ctx.conn
                .browser_context
                .as_ref()
                .unwrap()
                .renderer_runtime()
                .service_worker_force_update_on_page_load_for_devtools()
        );
    }

    #[tokio::test]
    async fn service_worker_stop_worker_rejects_invalid_version_id() {
        let mut ctx = TestContext::new();
        let mut context = browser_context_with_service_worker_target();
        context.set_active_target_id("TID-page".to_owned());
        context.attach_active_session("SID-page".to_owned());
        ctx.conn.install_browser_context_fixture_for_test(context);

        ctx.process_async(json!({
            "id": 1,
            "method": "ServiceWorker.enable",
            "sessionId": "SID-page"
        }))
        .await;
        ctx.expect_result(1, json!({}), Some("SID-page"));
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 2,
            "method": "ServiceWorker.stopWorker",
            "params": {"versionId": "not-a-version"},
            "sessionId": "SID-page"
        }))
        .await;
        let error = ctx.take_one();
        assert_eq!(error["id"], 2);
        assert_eq!(error["sessionId"], "SID-page");
        assert_eq!(error["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn service_worker_functional_event_rejects_invalid_registration_id() {
        let mut ctx = TestContext::new();
        let mut context = browser_context_with_service_worker_target();
        context.set_active_target_id("TID-page".to_owned());
        context.attach_active_session("SID-page".to_owned());
        ctx.conn.install_browser_context_fixture_for_test(context);

        ctx.process_async(json!({
            "id": 1,
            "method": "ServiceWorker.enable",
            "sessionId": "SID-page"
        }))
        .await;
        ctx.expect_result(1, json!({}), Some("SID-page"));
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 2,
            "method": "ServiceWorker.deliverPushMessage",
            "params": {
                "origin": "https://example.test/",
                "registrationId": "not-a-registration",
                "data": "payload"
            },
            "sessionId": "SID-page"
        }))
        .await;
        let error = ctx.take_one();
        assert_eq!(error["id"], 2);
        assert_eq!(error["sessionId"], "SID-page");
        assert_eq!(error["error"]["code"], -32602);
    }
}
