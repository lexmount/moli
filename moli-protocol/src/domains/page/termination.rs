use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, Cmd, CommandOwnerScope, DEFAULT_LOADER_ID,
    DocumentNavigationToken, NavigationDispatchState, PendingFetchNavigation,
    PendingSubresourceFetchAuthRequest, PendingSubresourceFetchRequest,
    PendingSubresourceFetchResponseRequest, monotonic_timestamp_seconds,
};
use crate::domains::{activity, network};
use moli_core::RendererOutputFence;

use super::{PageCommandTaskStep, complete_materialized_navigation_into_buffer_async};
use crate::domains::command_output::{CommandOutputBuffer, CommandOutputPlan};

#[derive(Debug)]
pub(crate) struct PageTargetTerminationOwnerAction {
    owner_scope: CommandOwnerScope,
    target_id: String,
}

impl PageTargetTerminationOwnerAction {
    pub(crate) fn new(owner_scope: CommandOwnerScope, target_id: String) -> Self {
        Self {
            owner_scope,
            target_id,
        }
    }

    pub(crate) fn owner_scope(&self) -> &CommandOwnerScope {
        &self.owner_scope
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    fn into_parts(self) -> (CommandOwnerScope, String) {
        (self.owner_scope, self.target_id)
    }
}

fn complete_success_with_background_events(
    events: Vec<BackgroundProtocolEvent>,
) -> PageCommandTaskStep {
    let mut plan = CommandOutputPlan::success();
    for event in events {
        plan.push_background_event(event);
    }
    PageCommandTaskStep::Complete(plan)
}

async fn complete_tokened_materialized_navigation_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    token: Option<DocumentNavigationToken>,
    navigation_state: NavigationDispatchState,
    navigation: network::MaterializedNavigationLoadOutcome,
) -> Option<RendererOutputFence> {
    let command_id = navigation_state.navigate_id;
    let command_session_id = navigation_state.owner.session_id().map(str::to_owned);
    let Some(token) = token else {
        out.extend(
            CommandOutputPlan::error(-32000, "Navigation aborted")
                .into_background_events(command_id, command_session_id.as_deref()),
        );
        return None;
    };
    let mut output = CommandOutputBuffer::default();
    let mut command_context = crate::conn::CommandDispatchContext::default();
    complete_materialized_navigation_into_buffer_async(
        conn,
        &mut output,
        token,
        navigation_state,
        navigation,
        &mut command_context,
    )
    .await;
    let mut plan = output.into_plan();
    let predecessor = command_context
        .take_renderer_output_predecessor()
        .or_else(|| plan.take_renderer_output_predecessor());
    out.extend(plan.into_background_events(command_id, command_session_id.as_deref()));
    predecessor
}

fn merge_renderer_output_predecessor(
    current: &mut Option<RendererOutputFence>,
    next: Option<RendererOutputFence>,
) {
    if let Some(next) = next {
        next.merge_into_same_stream_tail(current);
    }
}

pub(crate) fn take_pending_fetch_state(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> (
    Vec<PendingFetchNavigation>,
    Vec<crate::conn::PendingFetchAuthNavigation>,
    Vec<crate::conn::PausedDocumentTransfer>,
    Vec<(String, PendingSubresourceFetchRequest)>,
    Vec<(String, crate::conn::PendingSubresourceFetchAuthRequest)>,
    Vec<(String, crate::conn::PendingSubresourceFetchResponseRequest)>,
) {
    let owner = CommandOwnerScope::capture(conn, session_id);
    take_pending_fetch_state_for_owner(conn, &owner)
}

fn take_pending_fetch_state_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
) -> (
    Vec<PendingFetchNavigation>,
    Vec<crate::conn::PendingFetchAuthNavigation>,
    Vec<crate::conn::PausedDocumentTransfer>,
    Vec<(String, PendingSubresourceFetchRequest)>,
    Vec<(String, crate::conn::PendingSubresourceFetchAuthRequest)>,
    Vec<(String, crate::conn::PendingSubresourceFetchResponseRequest)>,
) {
    conn.take_pending_fetch_state_for_route(owner.session_id(), owner.session_owner_route())
        .unwrap_or_else(|| {
            (
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
        })
}

pub(crate) async fn fail_pending_fetch_state_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    navigation_error_text: &str,
    subresource_error_text: &str,
    pending_navigations: Vec<PendingFetchNavigation>,
    pending_auth_navigations: Vec<crate::conn::PendingFetchAuthNavigation>,
    pending_response_navigations: Vec<crate::conn::PausedDocumentTransfer>,
    pending_subresource_fetches: Vec<(String, PendingSubresourceFetchRequest)>,
    pending_subresource_auths: Vec<(String, crate::conn::PendingSubresourceFetchAuthRequest)>,
    pending_subresource_responses: Vec<(
        String,
        crate::conn::PendingSubresourceFetchResponseRequest,
    )>,
) -> Option<RendererOutputFence> {
    let owner = CommandOwnerScope::capture(conn, session_id);
    fail_pending_fetch_state_for_owner_background_events_async(
        conn,
        out,
        &owner,
        navigation_error_text,
        subresource_error_text,
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    )
    .await
}

async fn fail_pending_fetch_state_for_owner_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    navigation_error_text: &str,
    subresource_error_text: &str,
    pending_navigations: Vec<PendingFetchNavigation>,
    pending_auth_navigations: Vec<crate::conn::PendingFetchAuthNavigation>,
    pending_response_navigations: Vec<crate::conn::PausedDocumentTransfer>,
    pending_subresource_fetches: Vec<(String, PendingSubresourceFetchRequest)>,
    pending_subresource_auths: Vec<(String, crate::conn::PendingSubresourceFetchAuthRequest)>,
    pending_subresource_responses: Vec<(
        String,
        crate::conn::PendingSubresourceFetchResponseRequest,
    )>,
) -> Option<RendererOutputFence> {
    let session_id = owner.session_id();
    // A protocol navigation waiter may expose an operation-specific failure,
    // while Network.loadingFailed must retain the underlying net error. For
    // Page.stopLoading Chromium reports ERR_ABORTED/canceled=true even though
    // Moli's pending navigation reply remains "Navigation stopped".
    let mut renderer_output_predecessor = None;
    for pending in pending_navigations {
        let token = pending.document_navigation_token;
        let navigation_state = pending.navigation;
        let navigation = network::materialize_navigation_failure_preserving_committed_document(
            conn,
            &navigation_state,
            navigation_error_text.to_owned(),
        );
        let predecessor = complete_tokened_materialized_navigation_background_events_async(
            conn,
            out,
            token,
            navigation_state,
            navigation,
        )
        .await;
        merge_renderer_output_predecessor(&mut renderer_output_predecessor, predecessor);
    }
    for pending in pending_auth_navigations {
        let token = pending.document_navigation_token;
        let navigation_state = pending.navigation;
        let navigation = network::materialize_navigation_failure_preserving_committed_document(
            conn,
            &navigation_state,
            navigation_error_text.to_owned(),
        );
        let predecessor = complete_tokened_materialized_navigation_background_events_async(
            conn,
            out,
            token,
            navigation_state,
            navigation,
        )
        .await;
        merge_renderer_output_predecessor(&mut renderer_output_predecessor, predecessor);
    }
    for pending in pending_response_navigations {
        let (token, navigation, _) = pending.fail(navigation_error_text.to_owned());
        let result = network::materialize_navigation_failure_preserving_committed_document(
            conn,
            &navigation,
            navigation_error_text.to_owned(),
        );
        let predecessor = complete_tokened_materialized_navigation_background_events_async(
            conn, out, token, navigation, result,
        )
        .await;
        merge_renderer_output_predecessor(&mut renderer_output_predecessor, predecessor);
    }
    for (_, pending) in pending_subresource_fetches {
        if !conn.pending_subresource_fetch_request_residence_is_current(session_id, &pending) {
            continue;
        }
        match conn
            .fail_pending_subresource_fetch_for_route_async(
                session_id,
                owner.session_owner_route(),
                pending.internal_id,
                subresource_error_text.to_owned(),
            )
            .await
        {
            Ok(predecessor) => {
                merge_renderer_output_predecessor(&mut renderer_output_predecessor, predecessor);
                activity::flush_post_subresource_fetch_request_activity_for_owner_background_events_async(
                    conn,
                    out,
                    owner,
                    &pending,
                )
                .await;
            }
            Err(message) if message == "NoDocumentLoaded" => {}
            Err(_) => {}
        }
    }
    for (_, pending) in pending_subresource_auths {
        if !conn
            .target_page_residence_identity_is_current_for_session(session_id, &pending.page_owner)
        {
            continue;
        }
        match conn
            .fail_pending_subresource_auth_for_route_async(
                session_id,
                owner.session_owner_route(),
                pending.internal_id,
                subresource_error_text.to_owned(),
            )
            .await
        {
            Ok(predecessor) => {
                merge_renderer_output_predecessor(&mut renderer_output_predecessor, predecessor);
                activity::flush_post_subresource_auth_activity_for_owner_background_events_async(
                    conn, out, owner, &pending,
                )
                .await;
            }
            Err(message) if message == "NoDocumentLoaded" => {}
            Err(_) => {}
        }
    }
    for (_, pending) in pending_subresource_responses {
        if !conn
            .target_page_residence_identity_is_current_for_session(session_id, &pending.page_owner)
        {
            continue;
        }
        match conn
            .fail_pending_subresource_response_for_route_async(
                session_id,
                owner.session_owner_route(),
                pending.internal_id,
                subresource_error_text.to_owned(),
            )
            .await
        {
            Ok(predecessor) => {
                merge_renderer_output_predecessor(&mut renderer_output_predecessor, predecessor);
                activity::flush_post_subresource_response_activity_for_owner_background_events_async(
                    conn,
                    out,
                    owner,
                    &pending,
                )
                .await;
            }
            Err(message) if message == "NoDocumentLoaded" => {}
            Err(_) => {}
        }
    }
    renderer_output_predecessor
}

/// Completes protocol-owned subresource pauses after the renderer has crashed.
///
/// A normal Fetch failure is first applied to the Page owner and then projected
/// from its network backlog. `Page.crash` cannot use that path: the Page owner
/// may be blocked in JavaScript, and the IO termination which unblocks it also
/// retires the Page residence. The pending Fetch residences were already
/// claimed by [`take_pending_fetch_state`], so emitting their terminal network
/// state here is both race-free and independent of the renderer owner.
fn fail_crashed_subresource_fetches_background_events(
    conn: &CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    error_text: &str,
    pending_subresource_fetches: Vec<(String, PendingSubresourceFetchRequest)>,
    pending_subresource_auths: Vec<(String, PendingSubresourceFetchAuthRequest)>,
    pending_subresource_responses: Vec<(String, PendingSubresourceFetchResponseRequest)>,
) {
    let loader_id = conn
        .current_document_loader_id_for_route(owner.session_id(), owner.session_owner_route())
        .unwrap_or_else(|| DEFAULT_LOADER_ID.to_owned());
    let event_session_ids =
        conn.network_event_session_ids_for_route(owner.session_id(), owner.session_owner_route());
    let timestamp = monotonic_timestamp_seconds();

    let mut emit_failure =
        |network_request_id: &str,
         frame_id: &str,
         resource_type: moli_core::page::SubresourceResourceType| {
            for event_session_id in &event_session_ids {
                network::emit_loading_failed(
                    out,
                    event_session_id.as_deref(),
                    network_request_id,
                    frame_id,
                    &loader_id,
                    timestamp,
                    error_text,
                    resource_type.into(),
                );
            }
        };

    for (_, pending) in pending_subresource_fetches {
        if let Some(continuation) = pending.detached_parser_script_fetch_continuation() {
            let _ = continuation.fail(error_text.to_owned());
        }
        emit_failure(
            &pending.network_request_id,
            &pending.frame_id,
            pending.resource_type,
        );
    }
    for (_, pending) in pending_subresource_auths {
        emit_failure(
            &pending.network_request_id,
            &pending.frame_id,
            pending.resource_type,
        );
    }
    for (_, pending) in pending_subresource_responses {
        emit_failure(
            &pending.network_request_id,
            &pending.frame_id,
            pending.resource_type,
        );
    }
}

pub(super) fn try_start_stop_loading_command_dispatch(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    PageCommandTaskStep::Pending(super::PendingPageCommandDispatch {
        command_id: cmd.id,
        owner_scope: crate::conn::CommandOwnerScope::capture(conn, cmd.session_id),
        kind: Box::new(super::PendingPageCommandKind::StopLoading),
    })
}

pub(super) async fn complete_stop_loading_command_dispatch(
    conn: &mut CdpConnection,
    _command_id: Option<u64>,
    owner: &CommandOwnerScope,
) -> PageCommandTaskStep {
    let mut out = Vec::new();
    let session_id = owner.session_id();
    if let Ok(slot) =
        conn.runtime_session_owner_slot_mut_for_route(session_id, owner.session_owner_route())
        && let Some(page) = slot.loaded_page_mut()
        && let Err(error) = page.stop_document_lifecycle_async().await
    {
        tracing::debug!(%error, "failed to stop renderer document lifecycle");
    }
    let (
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    ) = take_pending_fetch_state_for_owner(conn, owner);

    let renderer_output_predecessor = fail_pending_fetch_state_for_owner_background_events_async(
        conn,
        &mut out,
        owner,
        "Navigation stopped",
        moli_fetch::NET_ERR_ABORTED_ERROR_TEXT,
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    )
    .await;
    let mut plan = CommandOutputPlan::success();
    for event in out {
        plan.push_background_event(event);
    }
    if let Some(predecessor) = renderer_output_predecessor {
        plan.set_renderer_output_predecessor(predecessor);
    }
    PageCommandTaskStep::Complete(plan)
}

pub(super) fn try_start_crash_command_dispatch(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    match cmd.get_params::<serde_json::Value>() {
        Ok(Some(_)) | Ok(None) => {}
        Err(_) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    }
    PageCommandTaskStep::Pending(super::PendingPageCommandDispatch {
        command_id: cmd.id,
        owner_scope: crate::conn::CommandOwnerScope::capture(conn, cmd.session_id),
        kind: Box::new(super::PendingPageCommandKind::Crash),
    })
}

pub(super) async fn complete_crash_command_dispatch(
    conn: &mut CdpConnection,
    _command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> PageCommandTaskStep {
    let mut out = Vec::new();
    let session_id = owner.session_id();
    let Some((_, target_id)) =
        conn.target_owner_identity_for_route(session_id, owner.session_owner_route())
    else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error_without_session(
            -31998,
            super::missing_page_target_error_message(conn, session_id),
        ));
    };
    let Some(target_id) = target_id else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error_without_session(
            -31998,
            "TargetNotLoaded",
        ));
    };

    let (
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    ) = take_pending_fetch_state_for_owner(conn, owner);

    // Chromium handles Page.crash directly at the renderer IO-agent boundary;
    // it never enters a V8InspectorSession or the ordinary target IO task FIFO.
    // Seal both DevTools receivers and interrupt active V8 synchronously so
    // target retirement cannot wait behind earlier JavaScript or IO work.
    if let Ok(page) = conn.loaded_page_mut_for_interruptible_protocol_access_for_route(
        session_id,
        owner.session_owner_route(),
    ) {
        page.crash_devtools_target_from_io();
    }

    // Page.crash retires the target, not merely the DevTools session which
    // issued the command. Settle every attached session before dropping the
    // Page; otherwise a late completion from (for example) the primary
    // session can wait forever on a response sender owned by the retired
    // renderer while the crash was issued by an auxiliary session.
    let target_inspector_session_ids =
        conn.page_event_session_ids_for_route(session_id, owner.session_owner_route());
    let mut pending_await_events = Vec::new();
    for inspector_session_id in &target_inspector_session_ids {
        let inspector_owner = inspector_session_id
            .as_deref()
            .map(CommandOwnerScope::for_session)
            .unwrap_or_else(|| owner.clone());
        conn.fail_pending_inspector_awaits_for_owner_background_events_into(
            &mut pending_await_events,
            command_context.protocol_events_mut(),
            &inspector_owner,
            "Page crashed",
        );
    }
    out.extend(pending_await_events);

    let renderer_output_predecessor = fail_pending_fetch_state_for_owner_background_events_async(
        conn,
        &mut out,
        owner,
        "Page crashed",
        "Page crashed",
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .await;
    fail_crashed_subresource_fetches_background_events(
        conn,
        &mut out,
        owner,
        "Page crashed",
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    );
    if let Some(predecessor) = renderer_output_predecessor {
        command_context.set_renderer_output_predecessor(predecessor);
    }

    out.extend(mark_page_target_crashed_background_events_async(conn, owner, &target_id).await);
    complete_success_with_background_events(out)
}

async fn mark_page_target_crashed_background_events_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    target_id: &str,
) -> Vec<BackgroundProtocolEvent> {
    let inspector_session_ids =
        conn.page_event_session_ids_for_route(owner.session_id(), owner.session_owner_route());
    for inspector_session_id in &inspector_session_ids {
        let inspector_owner = inspector_session_id
            .as_deref()
            .map(CommandOwnerScope::for_session)
            .unwrap_or_else(|| owner.clone());
        let _ = conn.with_target_devtools_session_state_for_route_mut(
            inspector_owner.session_id(),
            inspector_owner.session_owner_route(),
            |state| {
                state
                    .runtime_session_state
                    .record_inspector_target_crashed();
            },
        );
    }
    let _ = conn
        .mark_target_crashed_for_route_async(owner.session_id(), owner.session_owner_route())
        .await;
    let mut out = inspector_session_ids
        .into_iter()
        .map(|inspector_session_id| {
            BackgroundProtocolEvent::inspector_target_crashed(inspector_session_id.as_deref())
        })
        .collect::<Vec<_>>();
    out.extend(conn.target_crashed_events_for_all_discovery_owners(target_id, "crashed", 1));
    out
}

pub(super) fn try_start_close_command_dispatch(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    PageCommandTaskStep::Pending(super::PendingPageCommandDispatch {
        command_id: cmd.id,
        owner_scope: crate::conn::CommandOwnerScope::capture(conn, cmd.session_id),
        kind: Box::new(super::PendingPageCommandKind::Close),
    })
}

pub(super) async fn complete_close_command_dispatch(
    conn: &mut CdpConnection,
    _command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> PageCommandTaskStep {
    let mut out = Vec::new();
    let session_id = owner.session_id();
    let Some((_, target_id)) =
        conn.target_owner_identity_for_route(session_id, owner.session_owner_route())
    else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error_without_session(
            -31998,
            super::missing_page_target_error_message(conn, session_id),
        ));
    };
    if target_id.is_none() {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error_without_session(
            -31998,
            "TargetNotLoaded",
        ));
    };
    let target_id = target_id.expect("validated Page target identity");

    let (
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    ) = take_pending_fetch_state_for_owner(conn, owner);

    let mut pending_await_events = Vec::new();
    conn.fail_pending_inspector_awaits_for_owner_background_events_into(
        &mut pending_await_events,
        command_context.protocol_events_mut(),
        owner,
        "Page closed",
    );
    out.extend(pending_await_events);

    let renderer_output_predecessor = fail_pending_fetch_state_for_owner_background_events_async(
        conn,
        &mut out,
        owner,
        "Page closed",
        "Page closed",
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    )
    .await;
    if let Some(predecessor) = renderer_output_predecessor {
        command_context.set_renderer_output_predecessor(predecessor);
    }

    // Closing the target here would retire its session route before the
    // separately transported final Page publication crosses protocol ingress.
    // Publish a concrete protocol-owner continuation instead. The command
    // fence first admits every renderer record produced above, then the
    // scheduler sends the Page.close response and runs this teardown action.
    conn.publish_page_target_termination_owner_action(PageTargetTerminationOwnerAction::new(
        owner.clone(),
        target_id,
    ));
    complete_success_with_background_events(out)
}

pub(crate) async fn complete_page_target_termination_owner_action_async(
    conn: &mut CdpConnection,
    action: PageTargetTerminationOwnerAction,
) -> crate::conn::CdpTurnOutcome {
    let (owner_scope, expected_target_id) = action.into_parts();
    let mut out = Vec::new();
    let current_target_id = conn
        .target_owner_identity_for_route(
            owner_scope.session_id(),
            owner_scope.session_owner_route(),
        )
        .and_then(|(_, target_id)| target_id);
    if current_target_id.as_deref() != Some(expected_target_id.as_str()) {
        return crate::conn::CdpTurnOutcome::new_with_protocol_events(
            out,
            conn.take_scheduler_events(),
        );
    }
    let target_host_closure = conn.prepare_target_host_closure(&expected_target_id);
    let is_active_target = conn
        .browser_context
        .as_ref()
        .is_some_and(|browser_context| browser_context.is_active_target(&expected_target_id));
    let closed = if is_active_target {
        conn.close_active_page_target_for_target_close_async(&mut out, "Target closed")
            .await
    } else {
        conn.close_background_page_target_for_target_close_async(
            &expected_target_id,
            &mut out,
            "Target closed",
        )
        .await
    };
    let Some(closed) = closed else {
        return crate::conn::CdpTurnOutcome::new_with_protocol_events(
            out,
            conn.take_scheduler_events(),
        );
    };
    let closed_target_id = closed.target_id.clone();
    let (target_detached_info_deltas, target_destroyed_deltas) = target_host_closure.into_parts();
    for sid in closed.inspector_detached_session_ids() {
        out.push(BackgroundProtocolEvent::inspector_detached(
            Some(sid),
            "Render process gone.",
        ));
    }
    out.extend(conn.prepared_target_host_deltas_event_plan(target_detached_info_deltas));
    out.extend(conn.detach_target_closure_cleanup_event_plan(
        closed.into_detach_cleanup_plan(Some("Render process gone.")),
        None,
    ));
    out.extend(conn.detach_closed_top_level_target_sessions_event_plan(
        &closed_target_id,
        Some("Render process gone."),
    ));
    out.extend(conn.prepared_target_host_deltas_event_plan(target_destroyed_deltas));
    conn.release_idle_navigation_engine_memory_after_target_close();
    crate::conn::CdpTurnOutcome::new_with_protocol_events(out, conn.take_scheduler_events())
}
