use std::collections::HashSet;

use crate::conn::{CdpConnection, CommandDispatchContext, PreparedTargetHostClosure};
use crate::devtools_runtime::{DevToolsError, DevToolsErrorKind, DevToolsTargetKind};
use moli_core::browser::BrowserContextId;

use super::{events, worker_target};

const DISPOSE_REASON: &str = "Browser context disposed";
const INSPECTOR_DETACHED_REASON: &str = "Render process gone.";

struct PageTargetDisposal {
    target_id: String,
    web_contents: moli_core::browser::WebContentsHandle,
    fetch_owner_session_id: Option<Option<String>>,
    host_closure: PreparedTargetHostClosure,
}

pub(super) struct BrowserContextDisposal {
    browser_context_id: String,
    context: BrowserContextId,
    page_targets: Vec<PageTargetDisposal>,
    inspector_session_ids: Vec<String>,
    pending_inspector_session_owners: Vec<Option<String>>,
}

impl BrowserContextDisposal {
    pub(super) fn prepare(
        conn: &CdpConnection,
        browser_context_id: &str,
    ) -> Result<Self, DevToolsError> {
        let Some(browser_context) = conn.browser_context_by_id(browser_context_id) else {
            return Err(browser_context_not_found(browser_context_id));
        };

        let active_page_target_id = browser_context.active_target_id().map(str::to_owned);
        let mut page_target_ids = browser_context
            .background_targets()
            .rev()
            .map(|target| target.target_id().to_owned())
            .collect::<Vec<_>>();
        if let Some(active_page_target_id) = active_page_target_id {
            page_target_ids.push(active_page_target_id);
        }

        let target_ids = browser_context
            .devtools_target_infos()
            .into_iter()
            .filter_map(|target_info| {
                target_info
                    .target_id
                    .map(|target_id| (target_info.kind, target_id.into_string()))
            })
            .collect::<Vec<_>>();

        let page_targets = page_target_ids
            .into_iter()
            .map(|target_id| {
                let session_ids = target_session_ids(conn, browser_context, &target_id);
                let fetch_owner_session_id =
                    page_fetch_owner_session_id(browser_context, &target_id, &session_ids);
                PageTargetDisposal {
                    fetch_owner_session_id,
                    host_closure: conn.prepare_target_host_closure(&target_id),
                    web_contents: browser_context
                        .web_contents_handle_for_target(&target_id)
                        .expect("prepared Page target must retain its exact WebContents"),
                    target_id,
                }
            })
            .collect();

        let mut seen_sessions = HashSet::new();
        let inspector_session_ids = target_ids
            .into_iter()
            .filter(|(kind, _)| {
                matches!(
                    kind,
                    DevToolsTargetKind::Page
                        | DevToolsTargetKind::SharedWorker
                        | DevToolsTargetKind::ServiceWorker
                )
            })
            .flat_map(|(_, target_id)| target_session_ids(conn, browser_context, &target_id))
            .filter(|session_id| seen_sessions.insert(session_id.clone()))
            .collect::<Vec<_>>();

        let mut pending_inspector_session_owners = browser_context
            .active_target_id()
            .is_some()
            .then_some(None)
            .into_iter()
            .collect::<Vec<_>>();
        pending_inspector_session_owners.extend(inspector_session_ids.iter().cloned().map(Some));

        Ok(Self {
            browser_context_id: browser_context_id.to_owned(),
            context: browser_context.browser_context_id(),
            page_targets,
            inspector_session_ids,
            pending_inspector_session_owners,
        })
    }
}

fn target_session_ids(
    conn: &CdpConnection,
    browser_context: &crate::conn::BrowserContext,
    target_id: &str,
) -> Vec<String> {
    let mut session_ids = browser_context.devtools_session_ids_for_target(target_id);
    session_ids.extend(conn.attached_sessions_for_target(target_id));
    session_ids.sort();
    session_ids.dedup();
    session_ids
}

fn page_fetch_owner_session_id(
    browser_context: &crate::conn::BrowserContext,
    target_id: &str,
    session_ids: &[String],
) -> Option<Option<String>> {
    if browser_context.is_active_target(target_id) {
        return Some(browser_context.active_session_id_owned());
    }
    browser_context
        .background_target(target_id)
        .and_then(|target| target.session_id().map(str::to_owned))
        .or_else(|| session_ids.first().cloned())
        .map(Some)
}

pub(super) async fn execute_browser_context_disposal_async(
    conn: &mut CdpConnection,
    disposal: BrowserContextDisposal,
    out: &mut events::TargetProtocolSideEffects,
    command_context: &mut CommandDispatchContext,
) -> Result<(), DevToolsError> {
    let restore_browser_context = conn.active_browser_context_id();
    if !conn.activate_browser_context_by_browser_id(disposal.context) {
        return Err(browser_context_not_found(&disposal.browser_context_id));
    }

    fail_target_pending_work(conn, &disposal, out, command_context).await;
    out.extend_background_events(command_context.take_protocol_events());

    for session_id in &disposal.inspector_session_ids {
        out.background_events_mut()
            .push(events::inspector_detached_event(
                session_id,
                INSPECTOR_DETACHED_REASON,
            ));
    }

    out.extend_background_events(
        worker_target::close_browser_context_worker_targets_for_dispose_async(
            conn,
            &disposal.browser_context_id,
            disposal.context,
            DISPOSE_REASON,
        )
        .await,
    );

    for page_target in disposal.page_targets {
        close_page_target(conn, out, page_target).await;
    }

    let removed =
        conn.remove_browser_context_restoring_active(disposal.context, restore_browser_context);
    let Some(mut removed) = removed else {
        return Err(browser_context_not_found(&disposal.browser_context_id));
    };
    removed.close_all_pages_async().await;
    match removed.remove_from_browser() {
        Ok(true) => {}
        Ok(false) => return Err(browser_context_not_found(&disposal.browser_context_id)),
        Err(error) => {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Internal,
                format!("failed to remove BrowserContext: {error}"),
            ));
        }
    }
    Ok(())
}

async fn fail_target_pending_work(
    conn: &mut CdpConnection,
    disposal: &BrowserContextDisposal,
    out: &mut events::TargetProtocolSideEffects,
    command_context: &mut CommandDispatchContext,
) {
    for session_id in &disposal.pending_inspector_session_owners {
        conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
            out.background_events_mut(),
            command_context.protocol_events_mut(),
            session_id.as_deref(),
            DISPOSE_REASON,
        );
    }
    for page_target in &disposal.page_targets {
        if let Some(session_id) = &page_target.fetch_owner_session_id {
            fail_pending_navigations_for_disposed_target_async(
                conn,
                out.background_events_mut(),
                session_id.as_deref(),
            )
            .await;
        }
    }
}

async fn fail_pending_navigations_for_disposed_target_async(
    conn: &mut CdpConnection,
    out: &mut Vec<crate::conn::BackgroundProtocolEvent>,
    session_id: Option<&str>,
) {
    let (
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        _pending_subresource_fetches,
        _pending_subresource_auths,
        _pending_subresource_responses,
    ) = crate::domains::page::take_pending_fetch_state(conn, session_id);

    // Chromium closes an incognito BrowserContext as a renderer-lifetime
    // boundary. Pending main-resource navigations still receive their
    // protocol terminal before the target detaches, but fetch/XHR promises
    // disappear with the renderer realm and do not synthesize
    // Network.loadingFailed. In particular, disposal must not join concrete
    // output cursors from every Page stream merely to report subresource
    // cancellation.
    let _ = crate::domains::page::fail_pending_fetch_state_background_events_async(
        conn,
        out,
        session_id,
        DISPOSE_REASON,
        DISPOSE_REASON,
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .await;
}

async fn close_page_target(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    page_target: PageTargetDisposal,
) {
    let target_id = page_target.target_id;
    let closed = conn
        .close_web_contents_for_target_close_async(
            &target_id,
            page_target.web_contents,
            out.background_events_mut(),
            DISPOSE_REASON,
        )
        .await;
    let Some(closed) = closed else {
        tracing::warn!(
            target_id,
            "browser context disposal could not close a prepared page target"
        );
        return;
    };

    let (target_detached_info_deltas, target_destroyed_deltas) =
        page_target.host_closure.into_parts();
    out.extend_background_events(
        conn.prepared_target_host_deltas_event_plan(target_detached_info_deltas),
    );
    out.extend_background_events(
        conn.dispose_target_closure_sessions_event_plan_async(
            closed.into_detach_cleanup_plan(Some(INSPECTOR_DETACHED_REASON)),
            None,
        )
        .await,
    );
    if let Some(tab_cleanup) = conn.take_closed_top_level_target_sessions_cleanup_plan(
        &target_id,
        Some(INSPECTOR_DETACHED_REASON),
    ) {
        out.extend_background_events(
            conn.dispose_target_closure_sessions_event_plan_async(tab_cleanup, None)
                .await,
        );
    }
    out.extend_background_events(
        conn.prepared_target_host_deltas_event_plan(target_destroyed_deltas),
    );
}

fn browser_context_not_found(browser_context_id: &str) -> DevToolsError {
    DevToolsError::new(
        DevToolsErrorKind::Internal,
        format!("Failed to find context with id {browser_context_id}"),
    )
}
