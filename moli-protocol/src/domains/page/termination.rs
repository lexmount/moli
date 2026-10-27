use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, Cmd, CommandOwnerScope, DEFAULT_LOADER_ID,
    DocumentNavigationToken, NavigationDispatchState, PendingFetchNavigation,
    PendingSubresourceFetchAuthRequest, PendingSubresourceFetchRequest,
    PendingSubresourceFetchResponseRequest, monotonic_timestamp_seconds,
};
use crate::domains::{activity, network};
use anyhow::Result;
use moli_core::RendererOutputFence;

use super::{PageCommandTaskStep, complete_materialized_navigation_into_buffer_async};
use crate::domains::command_output::{CommandOutputBuffer, CommandOutputPlan};

mod target;
pub(crate) use target::{
    CompletedTargetTermination, TargetTermination, TargetTerminationSelection,
};

#[derive(Clone, Debug)]
pub(crate) struct PageTargetTerminationOwnerAction {
    owner_scope: CommandOwnerScope,
    target_id: String,
    host: Option<crate::conn::PageTargetHostIdentity>,
}

impl PageTargetTerminationOwnerAction {
    pub(crate) fn new(
        conn: &CdpConnection,
        owner_scope: CommandOwnerScope,
        target_id: String,
    ) -> Self {
        let host = conn.page_target_host_identity_for_owner(&owner_scope);
        // An admitted close belongs to the target, including after its
        // initiating frontend session detaches.
        let owner_scope = conn
            .target_session_route_for_target_id(&target_id)
            .map(CommandOwnerScope::for_route)
            .unwrap_or(owner_scope);
        Self {
            owner_scope,
            target_id,
            host,
        }
    }

    pub(crate) fn owner_scope(&self) -> &CommandOwnerScope {
        &self.owner_scope
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn is_current_host(&self, conn: &CdpConnection) -> bool {
        self.host.as_ref().is_some_and(|host| {
            conn.page_target_host_identity_for_owner(&self.owner_scope)
                .is_some_and(|current| host.matches(&current))
        })
    }

    fn into_parts(
        self,
    ) -> (
        CommandOwnerScope,
        String,
        Option<crate::conn::PageTargetHostIdentity>,
    ) {
        (self.owner_scope, self.target_id, self.host)
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
    conn.take_pending_fetch_state_for_owner(owner)
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
        let (token, navigation, _) =
            pending.fail(anyhow::Error::msg(navigation_error_text.to_owned()));
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
        if !conn.pending_subresource_fetch_request_residence_is_current(&pending) {
            continue;
        }
        if let Ok(predecessor) = conn
            .fail_pending_subresource_fetch_for_owner_async(
                owner,
                pending.internal_id,
                subresource_error_text.to_owned(),
            )
            .await
        {
            merge_renderer_output_predecessor(&mut renderer_output_predecessor, predecessor);
            activity::flush_post_subresource_fetch_request_activity_for_owner_background_events(
                conn, out, owner, &pending,
            );
        }
    }
    for (_, pending) in pending_subresource_auths {
        if !conn.target_page_residence_identity_is_current(&pending.page_owner) {
            continue;
        }
        if let Ok(predecessor) = conn
            .fail_pending_subresource_auth_for_owner_async(
                owner,
                pending.internal_id,
                subresource_error_text.to_owned(),
            )
            .await
        {
            merge_renderer_output_predecessor(&mut renderer_output_predecessor, predecessor);
            activity::flush_post_subresource_auth_activity_for_owner_background_events(
                conn, out, owner, &pending,
            );
        }
    }
    for (_, pending) in pending_subresource_responses {
        if !conn.target_page_residence_identity_is_current(&pending.page_owner) {
            continue;
        }
        if let Ok(predecessor) = conn
            .fail_pending_subresource_response_for_owner_async(
                owner,
                pending.internal_id,
                subresource_error_text.to_owned(),
            )
            .await
        {
            merge_renderer_output_predecessor(&mut renderer_output_predecessor, predecessor);
            activity::flush_post_subresource_response_activity_for_owner_background_events(
                conn, out, owner, &pending,
            );
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
        .current_document_loader_id_for_owner(owner)
        .unwrap_or_else(|| DEFAULT_LOADER_ID.to_owned());
    let event_session_ids = conn.network_event_session_ids_for_owner(owner);
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
    if let Some(page) = conn.loaded_page_mut_for_owner(owner)
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
    let Some((_, target_id)) = conn.target_owner_identity_for_owner(owner) else {
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
    if let Some(page) = conn.loaded_page_mut_for_owner(owner) {
        page.crash_devtools_target_from_io();
    }

    // Page.crash retires the target, not merely the DevTools session which
    // issued the command. Settle every attached session before dropping the
    // Page; otherwise a late completion from (for example) the primary
    // session can wait forever on a response sender owned by the retired
    // renderer while the crash was issued by an attached session.
    let target_inspector_session_ids = conn.page_event_session_ids_for_owner(owner);
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
    let inspector_session_ids = conn.page_event_session_ids_for_owner(owner);
    for inspector_session_id in &inspector_session_ids {
        let inspector_owner = inspector_session_id
            .as_deref()
            .map(CommandOwnerScope::for_session)
            .unwrap_or_else(|| owner.clone());
        let _ = conn.with_target_devtools_session_state_for_owner_mut(&inspector_owner, |state| {
            state
                .runtime_session_state
                .record_inspector_target_crashed();
        });
    }
    let _ = conn.mark_target_crashed_for_owner_async(owner).await;
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
    let session_id = owner.session_id();
    let Some((_, target_id)) = conn.target_owner_identity_for_owner(owner) else {
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
    let out = prepare_page_target_termination_async(conn, owner, target_id, command_context).await;
    complete_success_with_background_events(out)
}

pub(crate) struct PreparedPageTargetTermination {
    cleanup: crate::domains::fetch::PreparedFetchCleanup,
    action: PageTargetTerminationOwnerAction,
}

impl PreparedPageTargetTermination {
    pub(super) fn prepare(
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        target_id: String,
        command_context: &mut crate::conn::CommandDispatchContext,
    ) -> Self {
        let action = PageTargetTerminationOwnerAction::new(conn, owner.clone(), target_id);
        conn.mark_page_target_closing(owner);
        let pending = take_pending_fetch_state_for_owner(conn, owner);
        let mut events = Vec::new();
        conn.fail_pending_inspector_awaits_for_owner_background_events_into(
            &mut events,
            command_context.protocol_events_mut(),
            owner,
            "Page closed",
        );
        command_context.protocol_events_mut().extend(events);
        let cleanup = crate::domains::fetch::prepare_page_close(conn, owner, pending);
        Self { cleanup, action }
    }

    pub(crate) async fn run(
        self,
        driver: &mut crate::conn::ProtocolWorkflowDriver<'_>,
        context: &mut crate::conn::CommandWorkflowContext,
    ) -> Result<()> {
        let result = self
            .cleanup
            .run_using(driver)
            .await
            .into_disposal_output(context.context_mut().protocol_events_mut());
        if let Some(predecessor) = result.predecessor {
            context
                .context_mut()
                .set_renderer_output_predecessor(predecessor);
        }
        if let Some(error) = result.first_error {
            tracing::warn!(%error, "Page close cleanup failed; retaining target retirement");
        }
        driver
            .with_state(move |state| state.publish_page_output_close(self.action))
            .await
    }
}

pub(super) async fn prepare_page_target_termination_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    target_id: String,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> Vec<BackgroundProtocolEvent> {
    let prepared = PreparedPageTargetTermination::prepare(conn, owner, target_id, command_context);
    let mut local =
        crate::conn::CommandDispatchContext::new(command_context.response_flush().clone());
    let (mut context, _scope) =
        crate::conn::CommandWorkflowContext::take_from(&mut local, None, None);
    prepared
        .run(
            &mut crate::conn::ProtocolWorkflowDriver::inline(conn),
            &mut context,
        )
        .await
        .expect("inline Page close retains its protocol state");
    context.finish_projection_inline(conn, command_context);
    Vec::new()
}

pub(crate) async fn complete_page_target_termination_owner_action_async(
    conn: &mut CdpConnection,
    action: PageTargetTerminationOwnerAction,
) -> crate::conn::CdpTurnOutcome {
    let Some(termination) = TargetTermination::prepare(conn, action) else {
        return crate::conn::CdpTurnOutcome::new_with_protocol_events(
            Vec::new(),
            conn.take_scheduler_events(),
        );
    };
    termination
        .run(&mut crate::conn::ProtocolWorkflowDriver::inline(conn))
        .await
        .finish(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::BrowserContext;

    #[tokio::test]
    async fn admitted_target_termination_survives_its_initiating_session() {
        let mut conn = CdpConnection::new();
        let mut context = BrowserContext::new("BID-close-initiator".into());
        context.set_active_target_id("TID-close-initiator");
        context.attach_active_session("SID-primary");
        assert!(
            context
                .assign_attached_session_to_target("TID-close-initiator", "SID-initiator".into())
        );
        conn.install_browser_context_fixture_for_test(context);
        let action = PageTargetTerminationOwnerAction::new(
            &conn,
            CommandOwnerScope::for_session("SID-initiator"),
            "TID-close-initiator".into(),
        );
        let detached = conn.process_message_with_turn_outcome_async(
            r#"{"id":2,"method":"Target.detachFromTarget","params":{"sessionId":"SID-initiator"}}"#,
        ).await;
        assert!(
            detached
                .into_parts()
                .0
                .iter()
                .any(|message| message["id"] == 2 && message.get("result").is_some())
        );
        assert!(!conn.has_attached_devtools_session("SID-initiator"));

        complete_page_target_termination_owner_action_async(&mut conn, action).await;
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .page_target("TID-close-initiator")
                .is_none()
        );
        assert!(!conn.has_attached_devtools_session("SID-primary"));
    }

    #[tokio::test]
    async fn delayed_target_termination_rejects_a_recreated_host_with_the_same_ids() {
        let mut conn = CdpConnection::new();
        let install = |conn: &mut CdpConnection| {
            let mut context = BrowserContext::new("BID-recreated".into());
            context.set_active_target_id("TID-recreated");
            conn.install_browser_context_fixture_for_test(context);
        };
        install(&mut conn);
        let owner = CommandOwnerScope::capture(&conn, None);
        let action =
            PageTargetTerminationOwnerAction::new(&conn, owner.clone(), "TID-recreated".into());
        let original = conn.page_target_host_identity_for_owner(&owner).unwrap();
        install(&mut conn);
        let replacement = conn.page_target_host_identity_for_owner(&owner).unwrap();
        assert!(!original.matches(&replacement));

        let outcome = complete_page_target_termination_owner_action_async(&mut conn, action).await;
        assert!(outcome.into_parts().0.is_empty());
        assert!(replacement.matches(&conn.page_target_host_identity_for_owner(&owner).unwrap()));
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .page_target("TID-recreated")
                .is_some()
        );
    }
}
