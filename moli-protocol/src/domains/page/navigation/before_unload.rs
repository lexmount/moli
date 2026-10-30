use super::*;
use crate::conn::{CommandWorkflowContext, ProtocolWorkflowDriver, TargetPageResidenceIdentity};
use crate::domains::fetch::NavigationTermination;
use crate::domains::page::{NavigationCommandOutput, PreparedRendererNavigationResult};

pub(crate) struct PendingBeforeUnloadNavigationCommand {
    source: TargetPageResidenceIdentity,
    navigation: PreparedMainDocumentNavigation,
    pending: Result<PendingPageCommand>,
    pub(super) prefix_events: Vec<BackgroundProtocolEvent>,
    termination: Option<NavigationTermination>,
}

pub(crate) struct CompletedBeforeUnloadNavigationCommand {
    source: TargetPageResidenceIdentity,
    navigation: PreparedMainDocumentNavigation,
    completed: Result<CompletedPageCommand>,
    prefix_events: Vec<BackgroundProtocolEvent>,
    termination: Option<NavigationTermination>,
}

pub(crate) struct PreparedBeforeUnloadNavigation {
    source: TargetPageResidenceIdentity,
    navigation: PreparedMainDocumentNavigation,
    allowed: Result<bool>,
    prefix_events: Vec<BackgroundProtocolEvent>,
    projection: Option<tokio::sync::oneshot::Receiver<()>>,
    termination: Option<NavigationTermination>,
}

pub(crate) enum BeforeUnloadNavigationStep {
    Continue(PageCommandTaskStep),
    Stop(PreparedRendererNavigationResult),
}

impl PendingBeforeUnloadNavigationCommand {
    pub(super) fn new(
        conn: &CdpConnection,
        source: TargetPageResidenceIdentity,
        navigation: PreparedMainDocumentNavigation,
        pending: Result<PendingPageCommand>,
        prefix_events: Vec<BackgroundProtocolEvent>,
    ) -> Self {
        let termination = NavigationTermination::new(
            conn,
            &navigation.state.owner,
            Some(&navigation.token),
            &navigation.state,
        );
        Self {
            source,
            navigation,
            pending,
            prefix_events,
            termination,
        }
    }

    pub(in crate::domains::page) async fn wait(self) -> CompletedBeforeUnloadNavigationCommand {
        let completed = match self.pending {
            Ok(pending) => pending.wait().await,
            Err(error) => Err(error),
        };
        CompletedBeforeUnloadNavigationCommand {
            source: self.source,
            navigation: self.navigation,
            completed,
            prefix_events: self.prefix_events,
            termination: self.termination,
        }
    }
}

impl CompletedBeforeUnloadNavigationCommand {
    pub(crate) fn prepare(
        self,
        conn: &mut CdpConnection,
        context: &mut CommandWorkflowContext,
    ) -> PreparedBeforeUnloadNavigation {
        let mut projection = None;
        let allowed = self.completed.and_then(|completed| {
            let allowed = completed
                .bool_reply_value()
                .ok_or_else(|| anyhow::anyhow!("beforeunload check did not return a boolean"));
            // Settle the exact outgoing Page, including after a peer replaced
            // the target. Never refresh a replacement Page from this reply.
            let output = conn.settle_page_command_turn_for_owner(&self.source, completed);
            let (mut completion, predecessor) = output.into_completion_and_predecessor();
            if let Some(continuation) = completion.take_post_response_continuation() {
                continuation.release();
            }
            if let Some(predecessor) = predecessor {
                let target_id = conn
                    .target_owner_identity_for_owner(&self.navigation.state.owner)
                    .and_then(|(_, target_id)| target_id);
                projection =
                    Some(conn.renderer_output_projection_receipt(predecessor.clone(), target_id));
                context
                    .context_mut()
                    .set_renderer_output_predecessor(predecessor);
            }
            allowed
        });
        PreparedBeforeUnloadNavigation {
            source: self.source,
            navigation: self.navigation,
            allowed,
            prefix_events: self.prefix_events,
            projection,
            termination: self.termination,
        }
    }
}

impl PreparedBeforeUnloadNavigation {
    pub(crate) async fn run(
        mut self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
        output_mode: NavigationCommandOutput,
    ) -> Result<PageCommandTaskStep> {
        if let Some(projection) = self.projection.take() {
            projection
                .await
                .map_err(|_| anyhow::anyhow!("beforeunload output projection was retired"))?;
        }
        let mut termination = self.termination.take();
        let step = context.with_state(driver, move |state, context| {
            state.resume_before_unload_navigation(self, output_mode, context)
        })?;
        if let Some(termination) = termination.as_mut() {
            termination.disarm();
        }
        match step {
            BeforeUnloadNavigationStep::Continue(step) => Ok(step),
            BeforeUnloadNavigationStep::Stop(prepared) => {
                Box::pin(prepared.run(driver, context)).await?;
                Ok(PageCommandTaskStep::Complete(CommandOutputPlan::default()))
            }
        }
    }

    pub(crate) fn resume(
        self,
        conn: &mut CdpConnection,
        context: &mut CommandWorkflowContext,
        output_mode: NavigationCommandOutput,
    ) -> BeforeUnloadNavigationStep {
        let Self {
            source,
            navigation,
            allowed,
            prefix_events,
            ..
        } = self;
        let owner = navigation.state.owner.clone();
        if conn.accepts_pending_document_navigation_for_owner(&owner, &navigation.token)
            && conn.target_page_residence_identity_is_current(&source)
            && matches!(allowed, Ok(true))
        {
            let command_id = navigation.state.navigate_id;
            let start = start_prepared_main_document_navigation(conn, navigation, prefix_events);
            return BeforeUnloadNavigationStep::Continue(
                finish_started_navigation_command_for_parts(conn, command_id, owner, start, &[]),
            );
        }
        let prepared =
            if conn.accepts_pending_document_navigation_for_owner(&owner, &navigation.token) {
                let message = allowed.err().map_or_else(
                    || "Navigation aborted".to_owned(),
                    |error| error.to_string(),
                );
                let failure = network::materialize_navigation_failure_preserving_committed_document(
                    conn,
                    &navigation.state,
                    message,
                );
                PreparedRendererNavigationResult::prepare_materialized(
                    conn,
                    navigation.token,
                    navigation.state,
                    failure,
                    context,
                    output_mode,
                    prefix_events,
                )
            } else {
                // Stale cleanup belongs to this original request. It must not
                // announce a network request or mutate a replacement Document.
                PreparedRendererNavigationResult::prepare_with_output(
                    conn,
                    navigation.token,
                    navigation.state,
                    Err(moli_fetch::FetchCancelled.into()),
                    context,
                    output_mode,
                    prefix_events,
                )
            };
        BeforeUnloadNavigationStep::Stop(prepared)
    }
}
