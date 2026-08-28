use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, Cmd, CommandOwnerScope, DEFAULT_LOADER_ID,
    DocumentNavigationToken, NavigationDispatchState, PendingFetchNavigation,
    PendingSubresourceFetchAuthRequest, PendingSubresourceFetchRequest,
    PendingSubresourceFetchResponseRequest, TargetPageResidenceIdentity,
    monotonic_timestamp_seconds,
};
use crate::domains::{activity, network};
use moli_core::RendererOutputFence;
use std::time::Duration;

use super::{PageCommandTaskStep, complete_materialized_navigation_into_buffer_async};
use crate::domains::command_output::{CommandOutputBuffer, CommandOutputPlan};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageTargetTerminationKind {
    Crash,
    PageClose,
    TargetClose,
    WindowClose,
}

const PAGE_CLOSE_UNLOAD_ACK_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PageTargetTerminationPhase {
    AwaitRendererUnloadAck,
    Finalize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PageTargetCloseRequestPhase {
    Accepted,
    NetworkDrained,
    UnloadAcknowledged,
}

/// Browser-owner preflight for one renderer `window.close()` record.
///
/// The exact Page residence prevents a delayed close from retiring a target
/// that has already installed a replacement Page. The final target teardown is
/// still delegated to `PageTargetTerminationOwnerAction` after pending
/// renderer terminals have acquired their output fence.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PageTargetCloseRequestOwnerAction {
    owner_scope: CommandOwnerScope,
    page_owner: TargetPageResidenceIdentity,
    target_id: String,
    kind: PageTargetTerminationKind,
    phase: PageTargetCloseRequestPhase,
}

impl PageTargetCloseRequestOwnerAction {
    pub(crate) fn new(
        owner_scope: CommandOwnerScope,
        page_owner: TargetPageResidenceIdentity,
        target_id: String,
        kind: PageTargetTerminationKind,
    ) -> Self {
        assert!(
            matches!(
                kind,
                PageTargetTerminationKind::PageClose | PageTargetTerminationKind::WindowClose
            ),
            "a renderer close request must retain its Page or Window source"
        );
        Self {
            owner_scope,
            page_owner,
            target_id,
            kind,
            phase: PageTargetCloseRequestPhase::Accepted,
        }
    }

    pub(crate) fn new_unload_ack(
        owner_scope: CommandOwnerScope,
        page_owner: TargetPageResidenceIdentity,
        target_id: String,
        kind: PageTargetTerminationKind,
    ) -> Self {
        assert_ne!(
            kind,
            PageTargetTerminationKind::Crash,
            "a renderer unload ACK cannot represent a crash"
        );
        Self {
            owner_scope,
            page_owner,
            target_id,
            kind,
            phase: PageTargetCloseRequestPhase::UnloadAcknowledged,
        }
    }

    pub(crate) fn new_network_drained(
        owner_scope: CommandOwnerScope,
        page_owner: TargetPageResidenceIdentity,
        target_id: String,
        kind: PageTargetTerminationKind,
    ) -> Self {
        assert_ne!(
            kind,
            PageTargetTerminationKind::Crash,
            "a renderer network-drained ACK cannot represent a crash"
        );
        Self {
            owner_scope,
            page_owner,
            target_id,
            kind,
            phase: PageTargetCloseRequestPhase::NetworkDrained,
        }
    }

    pub(crate) fn owner_scope(&self) -> &CommandOwnerScope {
        &self.owner_scope
    }

    pub(crate) fn page_owner(&self) -> &TargetPageResidenceIdentity {
        &self.page_owner
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    fn into_parts(
        self,
    ) -> (
        CommandOwnerScope,
        TargetPageResidenceIdentity,
        String,
        PageTargetTerminationKind,
        PageTargetCloseRequestPhase,
    ) {
        (
            self.owner_scope,
            self.page_owner,
            self.target_id,
            self.kind,
            self.phase,
        )
    }
}

#[derive(Debug)]
pub(crate) struct PageTargetTerminationOwnerAction {
    owner_scope: CommandOwnerScope,
    target_id: String,
    expected_page_owner: Option<TargetPageResidenceIdentity>,
    kind: PageTargetTerminationKind,
    phase: PageTargetTerminationPhase,
}

impl PageTargetTerminationOwnerAction {
    pub(crate) fn new(
        owner_scope: CommandOwnerScope,
        target_id: String,
        kind: PageTargetTerminationKind,
    ) -> Self {
        Self {
            owner_scope,
            target_id,
            expected_page_owner: None,
            kind,
            phase: if kind == PageTargetTerminationKind::Crash {
                PageTargetTerminationPhase::Finalize
            } else {
                PageTargetTerminationPhase::AwaitRendererUnloadAck
            },
        }
    }

    pub(crate) fn new_for_window_close(
        owner_scope: CommandOwnerScope,
        target_id: String,
        expected_page_owner: TargetPageResidenceIdentity,
    ) -> Self {
        Self {
            owner_scope,
            target_id,
            expected_page_owner: Some(expected_page_owner),
            kind: PageTargetTerminationKind::WindowClose,
            phase: PageTargetTerminationPhase::AwaitRendererUnloadAck,
        }
    }

    pub(crate) fn owner_scope(&self) -> &CommandOwnerScope {
        &self.owner_scope
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn kind(&self) -> PageTargetTerminationKind {
        self.kind
    }

    fn into_parts(
        self,
    ) -> (
        CommandOwnerScope,
        String,
        Option<TargetPageResidenceIdentity>,
        PageTargetTerminationKind,
        PageTargetTerminationPhase,
    ) {
        (
            self.owner_scope,
            self.target_id,
            self.expected_page_owner,
            self.kind,
            self.phase,
        )
    }

    fn finalize(
        owner_scope: CommandOwnerScope,
        target_id: String,
        expected_page_owner: Option<TargetPageResidenceIdentity>,
        kind: PageTargetTerminationKind,
    ) -> Self {
        Self {
            owner_scope,
            target_id,
            expected_page_owner,
            kind,
            phase: PageTargetTerminationPhase::Finalize,
        }
    }
}

async fn finalize_page_target_termination_for_current_owner_async(
    conn: &mut CdpConnection,
    owner_scope: CommandOwnerScope,
    expected_target_id: String,
    kind: PageTargetTerminationKind,
) -> crate::conn::CdpTurnOutcome {
    let mut out = Vec::new();
    let target_host_closure = conn.prepare_target_host_closure(&expected_target_id);
    let closed = match kind {
        PageTargetTerminationKind::Crash => {
            unreachable!("crash termination returns before the close branch")
        }
        PageTargetTerminationKind::PageClose | PageTargetTerminationKind::WindowClose => {
            conn.close_page_target_for_session_owner_async(owner_scope.session_id())
                .await
        }
        PageTargetTerminationKind::TargetClose => {
            conn.close_page_target_for_target_close_session_owner_async(
                owner_scope.session_id(),
                &mut out,
                "Target closed",
            )
            .await
        }
    };
    let Some(closed) = closed else {
        return crate::conn::CdpTurnOutcome::new_with_protocol_events(
            out,
            conn.take_scheduler_events(),
        );
    };
    let closed_target_id = closed.target_id.clone();
    conn.discard_command_bound_page_close_records_for_target(&closed_target_id);
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

fn complete_success_with_background_events(
    events: Vec<BackgroundProtocolEvent>,
) -> PageCommandTaskStep {
    let mut plan = CommandOutputPlan::success();
    for event in events {
        plan.push_background_event(event);
    }
    PageCommandTaskStep::Complete(plan)
}

fn renderer_close_source(
    kind: PageTargetTerminationKind,
) -> moli_core::RendererTopLevelCloseSource {
    match kind {
        PageTargetTerminationKind::PageClose => moli_core::RendererTopLevelCloseSource::Page,
        PageTargetTerminationKind::TargetClose => moli_core::RendererTopLevelCloseSource::Target,
        PageTargetTerminationKind::WindowClose => moli_core::RendererTopLevelCloseSource::Window,
        PageTargetTerminationKind::Crash => {
            unreachable!("a renderer close barrier cannot represent a crash")
        }
    }
}

async fn complete_tokened_materialized_navigation_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    token: Option<DocumentNavigationToken>,
    navigation_state: NavigationDispatchState,
    navigation: network::MaterializedNavigationLoadOutcome,
) -> Option<RendererOutputFence> {
    let command_id = navigation_state.navigate_id;
    let command_session_id = navigation_state.navigate_session_id.clone();
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
        None,
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
    conn.take_pending_fetch_state_for_session_owner(session_id)
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
    error_text: &str,
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
    let mut renderer_output_predecessor = None;
    for pending in pending_navigations {
        let token = pending.document_navigation_token;
        let navigation_state = pending.navigation;
        let navigation = network::materialize_navigation_failure_preserving_committed_document(
            conn,
            &navigation_state,
            error_text.to_owned(),
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
            error_text.to_owned(),
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
        let (token, navigation, _) = pending.fail(error_text.to_owned());
        let result = network::materialize_navigation_failure_preserving_committed_document(
            conn,
            &navigation,
            error_text.to_owned(),
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
            .fail_pending_subresource_fetch_for_session_owner_async(
                session_id,
                pending.internal_id,
                error_text.to_owned(),
            )
            .await
        {
            Ok(predecessor) => {
                merge_renderer_output_predecessor(&mut renderer_output_predecessor, predecessor);
                activity::flush_post_subresource_fetch_request_activity_background_events_async(
                    conn, out, session_id, &pending,
                )
                .await
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
            .fail_pending_subresource_auth_for_session_owner_async(
                session_id,
                pending.internal_id,
                error_text.to_owned(),
            )
            .await
        {
            Ok(predecessor) => {
                merge_renderer_output_predecessor(&mut renderer_output_predecessor, predecessor);
                activity::flush_post_subresource_auth_activity_background_events_async(
                    conn, out, session_id, &pending,
                )
                .await
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
            .fail_pending_subresource_response_for_session_owner_async(
                session_id,
                pending.internal_id,
                error_text.to_owned(),
            )
            .await
        {
            Ok(predecessor) => {
                merge_renderer_output_predecessor(&mut renderer_output_predecessor, predecessor);
                activity::flush_post_subresource_response_activity_background_events_async(
                    conn, out, session_id, &pending,
                )
                .await
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
    session_id: Option<&str>,
    error_text: &str,
    pending_subresource_fetches: Vec<(String, PendingSubresourceFetchRequest)>,
    pending_subresource_auths: Vec<(String, PendingSubresourceFetchAuthRequest)>,
    pending_subresource_responses: Vec<(String, PendingSubresourceFetchResponseRequest)>,
) {
    let loader_id = conn
        .current_document_loader_id_for_session_owner(session_id)
        .unwrap_or_else(|| DEFAULT_LOADER_ID.to_owned());
    let event_session_ids = conn.network_event_session_ids_for_session_owner(session_id);
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
    session_id: Option<&str>,
) -> PageCommandTaskStep {
    let mut out = Vec::new();
    let restore_browser_context_id = conn.browser_context.as_ref().map(|bc| bc.id.clone());
    if let Some(session_id) = session_id
        && !conn
            .activate_browser_context_for_session_async(session_id)
            .await
    {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error_without_session(
            -32001,
            "Unknown sessionId",
        ));
    }
    if let Ok(slot) = conn.runtime_session_owner_slot_mut(session_id)
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
    ) = take_pending_fetch_state(conn, session_id);

    let renderer_output_predecessor = fail_pending_fetch_state_background_events_async(
        conn,
        &mut out,
        session_id,
        "Navigation stopped",
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    )
    .await;
    restore_stop_loading_browser_context_async(conn, restore_browser_context_id.as_deref()).await;
    let mut plan = CommandOutputPlan::success();
    for event in out {
        plan.push_background_event(event);
    }
    if let Some(predecessor) = renderer_output_predecessor {
        plan.set_renderer_output_predecessor(predecessor);
    }
    PageCommandTaskStep::Complete(plan)
}

async fn restore_stop_loading_browser_context_async(
    conn: &mut CdpConnection,
    browser_context_id: Option<&str>,
) {
    if let Some(browser_context_id) = browser_context_id
        && conn.has_browser_context_id(browser_context_id)
        && conn
            .browser_context
            .as_ref()
            .is_none_or(|bc| bc.id != browser_context_id)
    {
        let _ = conn
            .activate_browser_context_by_id_async(browser_context_id)
            .await;
    }
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
    session_id: Option<&str>,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> PageCommandTaskStep {
    let mut out = Vec::new();
    let Some((_, target_id)) = conn.target_owner_identity_for_session(session_id) else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error_without_session(
            -31998,
            "BrowserContextNotLoaded",
        ));
    };
    let Some(target_id) = target_id else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error_without_session(
            -31998,
            "TargetNotLoaded",
        ));
    };

    let primary_session_id = conn.runtime_session_owner_primary_session_id(session_id);
    let fail_session_id = session_id.or(primary_session_id.as_deref());
    let (
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    ) = take_pending_fetch_state(conn, session_id);

    // Chromium handles Page.crash directly at the renderer IO-agent boundary;
    // it never enters a V8InspectorSession or the ordinary target IO task FIFO.
    // Seal both DevTools receivers and interrupt active V8 synchronously so
    // target retirement cannot wait behind earlier JavaScript or IO work.
    if let Ok(page) = conn.loaded_page_mut_for_interruptible_protocol_access(session_id) {
        page.crash_devtools_target_from_io();
    }

    // Page.crash retires the target, not merely the DevTools session which
    // issued the command. Settle every attached session before dropping the
    // Page; otherwise a late completion from (for example) the primary
    // session can wait forever on a response sender owned by the retired
    // renderer while the crash was issued by an auxiliary session.
    let target_inspector_session_ids = conn.page_event_session_ids_for_session_owner(session_id);
    let mut pending_await_events = Vec::new();
    for inspector_session_id in &target_inspector_session_ids {
        conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
            &mut pending_await_events,
            command_context.protocol_events_mut(),
            inspector_session_id.as_deref(),
            "Page crashed",
        );
    }
    out.extend(pending_await_events);

    let renderer_output_predecessor = fail_pending_fetch_state_background_events_async(
        conn,
        &mut out,
        fail_session_id,
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
        fail_session_id,
        "Page crashed",
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    );
    if let Some(predecessor) = renderer_output_predecessor {
        command_context.set_renderer_output_predecessor(predecessor);
    }

    out.extend(
        mark_page_target_crashed_background_events_async(conn, session_id, &target_id).await,
    );
    complete_success_with_background_events(out)
}

async fn mark_page_target_crashed_background_events_async(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    target_id: &str,
) -> Vec<BackgroundProtocolEvent> {
    let inspector_session_ids = conn.page_event_session_ids_for_session_owner(session_id);
    for inspector_session_id in &inspector_session_ids {
        let _ = conn.with_target_devtools_session_state_for_session_mut(
            inspector_session_id.as_deref(),
            |state| {
                state
                    .runtime_session_state
                    .record_inspector_target_crashed();
            },
        );
    }
    let _ = conn
        .mark_target_crashed_for_session_owner_async(session_id)
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
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let pending = conn
        .loaded_page_mut_for_interruptible_protocol_access(cmd.session_id)
        .ok()
        .map(|page| page.start_browser_page_close_request())
        .transpose();
    let pending = match pending {
        Ok(pending) => pending,
        Err(error) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32000,
                format!("failed to start Page.close preflight: {error}"),
            ));
        }
    };
    PageCommandTaskStep::Pending(super::PendingPageCommandDispatch {
        command_id: cmd.id,
        owner_scope: crate::conn::CommandOwnerScope::capture(conn, cmd.session_id),
        kind: Box::new(super::PendingPageCommandKind::Close { pending }),
    })
}

pub(super) async fn complete_close_command_dispatch(
    conn: &mut CdpConnection,
    _command_id: Option<u64>,
    session_id: Option<&str>,
    completed: Option<super::CompletedPageCloseRequest>,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> PageCommandTaskStep {
    if let Some(completed) = completed {
        let completion = match completed {
            super::CompletedPageCloseRequest::Completed(completion) => *completion,
            super::CompletedPageCloseRequest::Interrupted(_predecessor) => {
                // The renderer has published the beforeunload dialog prefix
                // and retains the authoritative close continuation. The CDP
                // command is intentionally no longer the transaction owner.
                return PageCommandTaskStep::Complete(CommandOutputPlan::success());
            }
            super::CompletedPageCloseRequest::Failed(message) => {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    format!("Page.close preflight failed: {message}"),
                ));
            }
        };
        let Some(page) = conn
            .runtime_session_owner_slot_mut(session_id)
            .ok()
            .and_then(|slot| slot.loaded_page_mut())
        else {
            // The renderer command already settled against the previous Page.
            // Its typed Page-close owner action, if accepted, remains durable
            // and is target-relative across a replacement.
            return PageCommandTaskStep::Complete(CommandOutputPlan::success());
        };
        let (accepted, output) = match page.finish_browser_page_close_request(completion) {
            Ok(result) => result,
            Err(error) => {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    format!("Page.close preflight failed: {error}"),
                ));
            }
        };
        command_context.consume_renderer_command_turn_output(output);
        // Accepted requests already published their typed owner action. A
        // rejected beforeunload keeps the target alive but Page.close itself
        // still resolves, matching Chromium's command contract.
        if !accepted {
            return PageCommandTaskStep::Complete(CommandOutputPlan::success());
        }

        let Some(command_bound_page_owner) =
            conn.target_page_residence_identity_for_session(session_id)
        else {
            return PageCommandTaskStep::Complete(CommandOutputPlan::success());
        };
        conn.register_command_bound_page_close_record(command_bound_page_owner);

        // Page.close is a browser-originated close command. Keep its response
        // behind the whole close transaction instead of making callers race a
        // later renderer publication. Cancelling pending network work first
        // puts every terminal record before pagehide/unload in the same Page
        // stream; the unload ACK then becomes the command's latest exact
        // predecessor and performs final target teardown at ingress.
        let mut out = Vec::new();
        let primary_session_id = conn.runtime_session_owner_primary_session_id(session_id);
        let fail_session_id = session_id.or(primary_session_id.as_deref());
        let (
            pending_navigations,
            pending_auth_navigations,
            pending_response_navigations,
            pending_subresource_fetches,
            pending_subresource_auths,
            pending_subresource_responses,
        ) = take_pending_fetch_state(conn, session_id);
        let mut pending_await_events = Vec::new();
        conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
            &mut pending_await_events,
            command_context.protocol_events_mut(),
            session_id,
            "Page closed",
        );
        out.extend(pending_await_events);
        if let Some(predecessor) = fail_pending_fetch_state_background_events_async(
            conn,
            &mut out,
            fail_session_id,
            "Page closed",
            pending_navigations,
            pending_auth_navigations,
            pending_response_navigations,
            pending_subresource_fetches,
            pending_subresource_auths,
            pending_subresource_responses,
        )
        .await
        {
            command_context.set_renderer_output_predecessor(predecessor);
        }

        let owner_scope = CommandOwnerScope::capture(conn, session_id);
        if !dispatch_browser_owned_close_unload_before_command_response(
            conn,
            &owner_scope,
            moli_core::RendererTopLevelCloseSource::Page,
            command_context,
        )
        .await
        {
            // A missing Page, failed command, or the bounded unload timeout is
            // the browser-owned force-close fallback. The accepted preflight
            // cursor still orders this continuation after renderer output.
            conn.publish_page_target_termination_owner_action(
                PageTargetTerminationOwnerAction::finalize(
                    owner_scope,
                    conn.target_owner_identity_for_session(session_id)
                        .and_then(|(_, target_id)| target_id)
                        .expect("accepted Page.close must retain its target"),
                    None,
                    PageTargetTerminationKind::PageClose,
                ),
            );
        }
        return complete_success_with_background_events(out);
    }

    let mut out = Vec::new();
    let Some((_, target_id)) = conn.target_owner_identity_for_session(session_id) else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error_without_session(
            -31998,
            "BrowserContextNotLoaded",
        ));
    };
    if target_id.is_none() {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error_without_session(
            -31998,
            "TargetNotLoaded",
        ));
    };
    let target_id = target_id.expect("validated Page target identity");
    let primary_session_id = conn.runtime_session_owner_primary_session_id(session_id);
    let fail_session_id = session_id.or(primary_session_id.as_deref());

    let (
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    ) = take_pending_fetch_state(conn, session_id);

    let mut pending_await_events = Vec::new();
    conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
        &mut pending_await_events,
        command_context.protocol_events_mut(),
        session_id,
        "Page closed",
    );
    out.extend(pending_await_events);

    let renderer_output_predecessor = fail_pending_fetch_state_background_events_async(
        conn,
        &mut out,
        fail_session_id,
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
        CommandOwnerScope::capture(conn, session_id),
        target_id,
        PageTargetTerminationKind::PageClose,
    ));
    complete_success_with_background_events(out)
}

/// Dispatches pagehide/unload for a browser-originated close while retaining
/// the exact renderer output cursor on that command response.
///
/// The typed unload ACK is projected from the Page stream and performs final
/// teardown at ingress. This keeps `Page.close` and `Target.closeTarget`
/// synchronous from the DevTools client's perspective without retiring the
/// route before unload-produced console/network output has crossed protocol.
pub(crate) async fn dispatch_browser_owned_close_unload_before_command_response(
    conn: &mut CdpConnection,
    owner_scope: &CommandOwnerScope,
    source: moli_core::RendererTopLevelCloseSource,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> bool {
    let mut route_scope = owner_scope.enter(conn);
    let conn = route_scope.conn_mut();
    let unload_result = if let Ok(slot) =
        conn.runtime_session_owner_slot_mut(owner_scope.session_id())
        && let Some(page) = slot.loaded_page_mut()
    {
        Some(
            tokio::time::timeout(
                PAGE_CLOSE_UNLOAD_ACK_TIMEOUT,
                page.dispatch_browser_page_close_unload_async(source),
            )
            .await,
        )
    } else {
        None
    };
    match unload_result {
        Some(Ok(Ok((true, output)))) => {
            command_context.consume_renderer_command_turn_output(output);
            true
        }
        Some(Ok(Ok((false, _)))) | None => false,
        Some(Ok(Err(error))) => {
            tracing::debug!(%error, "renderer browser-owned close unload dispatch failed");
            false
        }
        Some(Err(_elapsed)) => {
            tracing::debug!(
                timeout_ms = PAGE_CLOSE_UNLOAD_ACK_TIMEOUT.as_millis(),
                "renderer browser-owned close unload ACK timed out"
            );
            false
        }
    }
}

pub(crate) async fn complete_page_target_termination_owner_action_async(
    conn: &mut CdpConnection,
    action: PageTargetTerminationOwnerAction,
) -> crate::conn::CdpTurnOutcome {
    let (owner_scope, expected_target_id, expected_page_owner, kind, phase) = action.into_parts();
    let mut route_scope = owner_scope.enter(conn);
    let conn = route_scope.conn_mut();
    let mut out = Vec::new();
    let current_target_id = conn
        .target_owner_identity_for_session(owner_scope.session_id())
        .and_then(|(_, target_id)| target_id);
    if current_target_id.as_deref() != Some(expected_target_id.as_str())
        || expected_page_owner.as_ref().is_some_and(|page_owner| {
            !conn.target_page_residence_identity_is_current_for_session(
                owner_scope.session_id(),
                page_owner,
            )
        })
    {
        return crate::conn::CdpTurnOutcome::new_with_protocol_events(
            out,
            conn.take_scheduler_events(),
        );
    }
    if kind == PageTargetTerminationKind::Crash {
        out.extend(
            mark_page_target_crashed_background_events_async(
                conn,
                owner_scope.session_id(),
                &expected_target_id,
            )
            .await,
        );
        return crate::conn::CdpTurnOutcome::new_with_protocol_events(
            out,
            conn.take_scheduler_events(),
        );
    }

    if phase == PageTargetTerminationPhase::AwaitRendererUnloadAck {
        let unload_source = renderer_close_source(kind);
        let unload_result = if let Ok(slot) =
            conn.runtime_session_owner_slot_mut(owner_scope.session_id())
            && let Some(page) = slot.loaded_page_mut()
        {
            Some(
                tokio::time::timeout(
                    PAGE_CLOSE_UNLOAD_ACK_TIMEOUT,
                    page.dispatch_browser_page_close_unload_async(unload_source),
                )
                .await,
            )
        } else {
            None
        };
        match unload_result {
            Some(Ok(Ok((true, _output)))) => {
                // The renderer appended a typed unload ACK after all unload
                // output in its own stream. That record will publish the
                // finalize action after ingress; this owner turn therefore
                // has no recursive renderer fence.
                return crate::conn::CdpTurnOutcome::new_with_protocol_events(
                    out,
                    conn.take_scheduler_events(),
                );
            }
            Some(Ok(Ok((false, _output)))) => {}
            Some(Ok(Err(error))) => {
                tracing::debug!(%error, "renderer Page close unload dispatch failed");
            }
            Some(Err(_elapsed)) => {
                tracing::debug!(
                    timeout_ms = PAGE_CLOSE_UNLOAD_ACK_TIMEOUT.as_millis(),
                    "renderer Page close unload ACK timed out"
                );
            }
            None => {}
        }
    }

    finalize_page_target_termination_for_current_owner_async(
        conn,
        owner_scope,
        expected_target_id,
        kind,
    )
    .await
}

pub(crate) async fn complete_page_target_close_request_owner_action_async(
    conn: &mut CdpConnection,
    action: PageTargetCloseRequestOwnerAction,
) -> crate::conn::CdpTurnOutcome {
    let (owner_scope, page_owner, expected_target_id, kind, phase) = action.into_parts();
    let mut route_scope = owner_scope.enter(conn);
    let conn = route_scope.conn_mut();
    let session_id = owner_scope.session_id();
    let target_is_current = conn
        .target_owner_identity_for_session(session_id)
        .and_then(|(_, target_id)| target_id)
        .as_deref()
        == Some(expected_target_id.as_str());
    if !target_is_current
        || (kind == PageTargetTerminationKind::WindowClose
            && !conn.target_page_residence_identity_is_current_for_session(session_id, &page_owner))
    {
        return crate::conn::CdpTurnOutcome::new_with_protocol_events(
            Vec::new(),
            conn.take_scheduler_events(),
        );
    }

    if phase == PageTargetCloseRequestPhase::UnloadAcknowledged {
        return finalize_page_target_termination_for_current_owner_async(
            conn,
            owner_scope,
            expected_target_id,
            kind,
        )
        .await;
    }
    if phase == PageTargetCloseRequestPhase::NetworkDrained {
        conn.publish_page_target_termination_owner_action(match kind {
            PageTargetTerminationKind::WindowClose => {
                PageTargetTerminationOwnerAction::new_for_window_close(
                    owner_scope,
                    expected_target_id,
                    page_owner,
                )
            }
            PageTargetTerminationKind::PageClose | PageTargetTerminationKind::TargetClose => {
                PageTargetTerminationOwnerAction::new(owner_scope, expected_target_id, kind)
            }
            PageTargetTerminationKind::Crash => {
                unreachable!("renderer close barrier cannot represent a crash")
            }
        });
        return crate::conn::CdpTurnOutcome::new_with_protocol_events(
            Vec::new(),
            conn.take_scheduler_events(),
        );
    }

    let error_text = match kind {
        PageTargetTerminationKind::PageClose => "Page closed",
        PageTargetTerminationKind::TargetClose => "Target closed",
        PageTargetTerminationKind::WindowClose => "Window closed",
        PageTargetTerminationKind::Crash => {
            unreachable!("renderer close request cannot represent a crash")
        }
    };
    let primary_session_id = conn.runtime_session_owner_primary_session_id(session_id);
    let fail_session_id = session_id.or(primary_session_id.as_deref());
    let (
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    ) = take_pending_fetch_state(conn, session_id);
    let mut out = Vec::new();
    let mut claimed_await_events = Vec::new();
    conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
        &mut out,
        &mut claimed_await_events,
        session_id,
        error_text,
    );
    out.extend(claimed_await_events);
    let renderer_output_predecessor = fail_pending_fetch_state_background_events_async(
        conn,
        &mut out,
        fail_session_id,
        error_text,
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    )
    .await;

    if renderer_output_predecessor.is_some() {
        let source = renderer_close_source(kind);
        let network_ack = if let Ok(slot) = conn.runtime_session_owner_slot_mut(session_id)
            && let Some(page) = slot.loaded_page_mut()
        {
            Some(
                tokio::time::timeout(
                    PAGE_CLOSE_UNLOAD_ACK_TIMEOUT,
                    page.acknowledge_browser_page_close_network_drained_async(source),
                )
                .await,
            )
        } else {
            None
        };
        match network_ack {
            Some(Ok(Ok((true, _output)))) => {
                return crate::conn::CdpTurnOutcome::new_with_protocol_events(
                    out,
                    conn.take_scheduler_events(),
                );
            }
            Some(Ok(Ok((false, _)))) | None => {}
            Some(Ok(Err(error))) => {
                tracing::debug!(%error, "renderer Page close network-drained ACK failed");
            }
            Some(Err(_elapsed)) => {
                tracing::debug!(
                    timeout_ms = PAGE_CLOSE_UNLOAD_ACK_TIMEOUT.as_millis(),
                    "renderer Page close network-drained ACK timed out"
                );
            }
        }
    }

    conn.publish_page_target_termination_owner_action(match kind {
        PageTargetTerminationKind::WindowClose => {
            PageTargetTerminationOwnerAction::new_for_window_close(
                owner_scope,
                expected_target_id,
                page_owner,
            )
        }
        PageTargetTerminationKind::PageClose => PageTargetTerminationOwnerAction::new(
            owner_scope,
            expected_target_id,
            PageTargetTerminationKind::PageClose,
        ),
        PageTargetTerminationKind::TargetClose => PageTargetTerminationOwnerAction::new(
            owner_scope,
            expected_target_id,
            PageTargetTerminationKind::TargetClose,
        ),
        PageTargetTerminationKind::Crash => {
            unreachable!("renderer close request cannot represent a crash")
        }
    });
    crate::conn::CdpTurnOutcome::new_with_protocol_events(out, conn.take_scheduler_events())
}
