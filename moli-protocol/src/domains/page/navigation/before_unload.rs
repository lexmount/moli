use super::*;

pub(in crate::domains::page) struct PendingBeforeUnloadNavigationCommand {
    pub(super) pending: PendingNavigationUnload,
    pub(super) prefix_events: Vec<BackgroundProtocolEvent>,
}

pub(in crate::domains::page) struct CompletedBeforeUnloadNavigationCommand {
    check: NavigationAfterBeforeUnload,
    prefix_events: Vec<BackgroundProtocolEvent>,
}

impl PendingBeforeUnloadNavigationCommand {
    pub(in crate::domains::page) async fn wait(self) -> CompletedBeforeUnloadNavigationCommand {
        let BackgroundNavigationCompletion::BeforeLoad(check) = self.pending.wait().await else {
            unreachable!("a beforeunload command must complete the before-load phase")
        };
        CompletedBeforeUnloadNavigationCommand {
            check: *check,
            prefix_events: self.prefix_events,
        }
    }
}

pub(in crate::domains::page) async fn complete_pending_beforeunload_navigation_command(
    conn: &mut CdpConnection,
    completed: CompletedBeforeUnloadNavigationCommand,
    command_context: &mut CommandDispatchContext,
) -> PageCommandTaskStep {
    let command_id = completed.check.check.navigation.state.navigate_id;
    let owner = completed.check.check.navigation.state.owner.clone();
    let start = completed
        .check
        .finish_check(conn, command_context, completed.prefix_events, false)
        .await;
    finish_started_navigation_command_for_parts(conn, command_id, owner, start, &[])
}

pub struct NavigationAfterBeforeUnload {
    pub(super) check: NavigationBeforeLoad,
    pub(super) completed: anyhow::Result<CompletedPageCommand>,
}

impl NavigationAfterBeforeUnload {
    async fn finish_check(
        self,
        conn: &mut CdpConnection,
        command_context: &mut CommandDispatchContext,
        prefix_events: Vec<BackgroundProtocolEvent>,
        output_already_projected: bool,
    ) -> NavigateCommandStart {
        let NavigationBeforeLoad { source, navigation } = self.check;
        let owner = navigation.state.owner.clone();
        let check = self.completed.and_then(|completed| {
            let allowed = completed
                .bool_reply_value()
                .ok_or_else(|| anyhow::anyhow!("beforeunload check did not return a boolean"));
            // Foreground WebDriver callers can own an explicit target route
            // without a CDP session. Refresh that exact Page, never the active
            // target selected by a missing session id.
            let output = if conn.target_page_residence_identity_is_current(&source)
                && let Ok(slot) = conn.runtime_session_owner_slot_mut_for_owner(&owner)
                && let Some(page) = slot.loaded_page_mut()
            {
                page.finish_page_command_turn(completed)
            } else {
                completed.into_output()
            };
            let (mut completion, predecessor) = output.into_completion_and_predecessor();
            if output_already_projected {
                assert!(
                    predecessor.as_ref().is_none_or(|fence| {
                        conn.renderer_output_cursor_is_projected(fence.cursor())
                    }),
                    "beforeunload output must cross ingress before the navigation starts loading"
                );
            } else if let Some(predecessor) = predecessor {
                command_context.set_renderer_output_predecessor(predecessor);
            }
            if let Some(continuation) = completion.take_post_response_continuation() {
                continuation.release();
            }
            allowed
        });
        let current = conn.accepts_pending_document_navigation_for_owner(&owner, &navigation.token);
        let plan = if !current {
            let mut output = CommandOutputBuffer::default();
            output.extend_background_events_after_messages(prefix_events);
            push_superseded_navigation_result(&mut output, &navigation.state);
            finish_document_navigation(
                conn,
                &mut output,
                &owner,
                &navigation.token,
                &navigation.state.loader_id,
            )
            .await;
            output.into_plan()
        } else if !conn.target_page_residence_identity_is_current(&source)
            || !matches!(check, Ok(true))
        {
            let error = check.err().map_or_else(
                || "Navigation aborted".to_owned(),
                |error| error.to_string(),
            );
            let failure = network::materialize_navigation_failure_preserving_committed_document(
                conn,
                &navigation.state,
                error,
            );
            let mut output = CommandOutputBuffer::default();
            output.extend_background_events_after_messages(prefix_events);
            complete_materialized_navigation_after_unload_into_buffer_async(
                conn,
                &mut output,
                navigation.token,
                navigation.state,
                failure,
                command_context,
            )
            .await;
            output.into_plan()
        } else {
            return start_prepared_main_document_navigation(conn, navigation, prefix_events);
        };
        NavigateCommandStart::CompletePlan(plan)
    }

    pub(crate) async fn finish(
        self,
        conn: &mut CdpConnection,
        command_context: &mut CommandDispatchContext,
    ) {
        let command_id = self.check.navigation.state.navigate_id;
        let owner = self.check.navigation.state.owner.clone();
        // The scheduler has projected the source turn, including actions that
        // invalidate this navigation token, before allowing the load to start.
        let start = self
            .finish_check(conn, command_context, Vec::new(), true)
            .await;
        let plan = match start {
            NavigateCommandStart::CompletePlan(plan)
            | NavigateCommandStart::CompleteImmediate(plan) => plan,
            NavigateCommandStart::PendingLoad(pending) => {
                let completed = pending.wait().await;
                let PageCommandTaskStep::Complete(plan) =
                    complete_pending_navigate_load_command(conn, completed, command_context).await
                else {
                    unreachable!("a completed document load must produce its commit plan")
                };
                plan
            }
            NavigateCommandStart::PendingContinueWithoutRequestPause(pending) => {
                let PageCommandTaskStep::Complete(plan) =
                    complete_pending_continue_navigation_without_request_pause_command(
                        conn,
                        pending.wait().await,
                    )
                    .await
                else {
                    unreachable!("a continued document request must produce its output plan")
                };
                plan
            }
            NavigateCommandStart::PendingBeforeUnload(_)
            | NavigateCommandStart::PendingChildFrame(_)
            | NavigateCommandStart::PendingSameDocument(_) => {
                unreachable!("prepared main-document loads cannot change navigation kind")
            }
        };
        let (prefix, boundary, suffix, post_response) = plan
            .into_renderer_fenced_background_and_post_response_events(
                command_id,
                owner.session_id(),
            );
        command_context.append_renderer_fenced_protocol_events(prefix, boundary, suffix);
        command_context.extend_post_response_events(post_response);
    }
}
