use std::{cell::RefCell, rc::Rc};

use anyhow::Context as _;
use anyhow::Result;

use super::*;
use crate::conn::{
    BackgroundPreparedNavigationCommit, CommandWorkflowContext, InstalledNavigationPage,
    NoDocumentLoaded, ProtocolWorkflowDriver, SessionDisposalWait,
};
use crate::domains::{
    command_output::CommandOutputPlan,
    target::{PreparedDedicatedWorkerRetirement, WorkerRetirementFailureCleanup},
};

#[derive(Debug, thiserror::Error)]
enum NavigationProtocolCommitError {
    #[error("prepared navigation has no renderer attachment transaction")]
    MissingAttachment,
    #[error("prepared navigation's renderer attachment is no longer current")]
    SupersededAttachment,
}

enum PreparedPageInstallation {
    Created(InstalledNavigationPage),
    Existing(LoadedNavigationPageCommit),
}

impl PreparedPageInstallation {
    async fn close_previous(self) -> LoadedNavigationPageCommit {
        match self {
            Self::Created(installed) => installed.close_previous().await,
            Self::Existing(commit) => commit,
        }
    }
}

struct PreparedLoadedNavigation {
    publication: LoadedNavigationPublication,
    installation: PreparedPageInstallation,
    main_commit: std::sync::Arc<moli_core::page::RendererMainDocumentCommit>,
    realms: Vec<RendererRuntimeRealmInfo>,
    renderer_attachment: moli_core::page::RendererAgentAttachmentId,
}

/// Recovery belongs to the same command entry. Its worker receipts are shared
/// with normal completion, so unwind cannot either lose or repeat a detach.
#[derive(Clone, Default)]
pub(crate) struct NavigationProtocolCleanup(Rc<RefCell<Vec<WorkerRetirementFailureCleanup>>>);

impl NavigationProtocolCleanup {
    pub(crate) fn register(&self, workers: &[PreparedDedicatedWorkerRetirement]) {
        self.0.borrow_mut().extend(
            workers
                .iter()
                .map(PreparedDedicatedWorkerRetirement::failure_cleanup),
        );
    }

    pub(crate) async fn recover(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) {
        let workers = std::mem::take(&mut *self.0.borrow_mut());
        for worker in workers {
            worker.run_into_context(driver, context).await;
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum NavigationCommandOutput {
    Background,
    Command,
    Composite,
}

impl NavigationCommandOutput {
    fn project(
        self,
        state: &NavigationDispatchState,
        plan: CommandOutputPlan,
    ) -> CommandOutputPlan {
        match self {
            Self::Background => {
                plan.into_background_event_plan(state.navigate_id, state.owner.session_id())
            }
            Self::Command => plan,
            Self::Composite if plan.command_status().is_some_and(|status| status.is_err()) => plan,
            Self::Composite => plan.into_composite_command_prefix(),
        }
    }

    pub(crate) fn retain_projected(
        self,
        context: &mut CommandWorkflowContext,
        plan: CommandOutputPlan,
    ) {
        if !matches!(self, Self::Background) && plan.command_status().is_some() {
            context.retain_terminal(plan);
        } else {
            context.retain_output(plan);
        }
    }

    pub(crate) fn retain(
        self,
        context: &mut CommandWorkflowContext,
        state: &NavigationDispatchState,
        plan: CommandOutputPlan,
    ) {
        self.retain_projected(context, self.project(state, plan));
    }
}

/// The native commit is already owned. This tail installs its exact Page,
/// retires the previous resources, and returns one finite publication for the
/// consuming actor turn. It never owns a connection across a wait.
pub(crate) struct PreparedNavigationProtocolCompletion {
    token: DocumentNavigationToken,
    state: NavigationDispatchState,
    binding: Option<SessionDisposalWait>,
    loaded: Option<PreparedLoadedNavigation>,
    failed_page: Option<moli_core::page::Page>,
    output_mode: NavigationCommandOutput,
    prefix_output: Option<CommandOutputBuffer>,
}

impl PreparedNavigationProtocolCompletion {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        completion: BackgroundPreparedNavigationCommit,
        context: &mut CommandWorkflowContext,
    ) -> Self {
        Self::prepare_with_output(
            conn,
            completion,
            context,
            NavigationCommandOutput::Background,
            Vec::new(),
        )
    }

    pub(crate) fn prepare_with_output(
        conn: &mut CdpConnection,
        completion: BackgroundPreparedNavigationCommit,
        context: &mut CommandWorkflowContext,
        output_mode: NavigationCommandOutput,
        prefix: Vec<crate::conn::BackgroundProtocolEvent>,
    ) -> Self {
        let BackgroundPreparedNavigationCommit {
            token,
            state,
            expected,
            transaction,
            navigation,
        } = completion;
        let binding = state
            .owner
            .session_id()
            .and_then(|id| conn.session_binding_lifetime(id));
        let mut output = CommandOutputBuffer::default();
        output.extend_background_events_after_messages(prefix);
        let mut result = Self {
            token,
            state,
            binding,
            loaded: None,
            failed_page: None,
            output_mode,
            prefix_output: None,
        };
        if !conn.accepts_pending_document_navigation_for_owner(&result.state.owner, &result.token) {
            super::super::push_superseded_navigation_result(&mut output, &result.state);
        } else {
            match navigation {
                Ok(navigation) => {
                    let materialized =
                        crate::domains::network::materialize_loaded_navigation_progress(
                            conn,
                            &result.state,
                            navigation,
                        );
                    match result.install(conn, materialized, transaction) {
                        Ok(loaded) => result.loaded = Some(loaded),
                        Err(error) => result.emit_failure(conn, &mut output, error),
                    }
                }
                Err(error) => {
                    if error.disposition() == moli_core::page::RendererPageReplacementFailureDisposition::DocumentUnavailable {
                        if let Some(expected) = expected.as_ref() {
                            result.failed_page = conn.take_loaded_page_after_failed_navigation_for_owner(
                                &result.state.owner, &result.token, Some(expected), &result.state.requested_url,
                            );
                        }
                    } else if let Some(transaction) = transaction
                        && let Err(error) = conn.rollback_committed_renderer_agent_candidate_for_owner(&result.state.owner, transaction) {
                        tracing::warn!(%error, "failed to roll back prepared navigation attachment");
                    }
                    result.emit_failure(conn, &mut output, error.into());
                }
            }
        }
        if result.loaded.is_some() {
            result.prefix_output = Some(output);
        } else {
            result.retain(context, output.into_plan());
        }
        result
    }

    fn install(
        &self,
        conn: &mut CdpConnection,
        navigation: MaterializedLoadedDocumentProgress,
        transaction: Option<CommittedRendererAgentAttachment>,
    ) -> Result<PreparedLoadedNavigation> {
        let transaction = transaction.ok_or(NavigationProtocolCommitError::MissingAttachment)?;
        anyhow::ensure!(
            transaction.navigation() == &self.token
                && conn.current_renderer_agent_attachment_id_for_owner(&self.state.owner)
                    == Some(transaction.current().id()),
            NavigationProtocolCommitError::SupersededAttachment
        );
        let renderer_attachment = transaction.current().id();
        let (publication, page, realms, main_commit) = LoadedNavigationPublication::split(
            self.state.clone(),
            Some(self.token.clone()),
            navigation,
        )?;
        let target_url = publication.target_url();
        let installation = match page {
            CommittedNavigationPage::Created(mut page) => {
                anyhow::ensure!(
                    page.renderer_devtools_agent_token() == transaction.current().agent_token(),
                    NavigationProtocolCommitError::SupersededAttachment
                );
                page.bind_renderer_agent_attachment(renderer_attachment);
                PreparedPageInstallation::Created(
                    conn.install_loaded_navigation_page_for_owner(
                        &self.state.owner,
                        *page,
                        LoadedNavigationRendererAttachmentCommit::AlreadyCommitted(transaction),
                        &target_url,
                    )
                    .context(NoDocumentLoaded)??,
                )
            }
            CommittedNavigationPage::Replaced(expected) => PreparedPageInstallation::Existing(
                conn.commit_existing_page_navigation_for_owner(
                    &self.state.owner,
                    &expected,
                    transaction,
                    &target_url,
                )
                .context(NoDocumentLoaded)??,
            ),
        };
        Ok(PreparedLoadedNavigation {
            publication,
            installation,
            main_commit,
            realms,
            renderer_attachment,
        })
    }

    fn emit_failure(
        &self,
        conn: &mut CdpConnection,
        output: &mut CommandOutputBuffer,
        error: anyhow::Error,
    ) {
        let error_text = format!("{error:#}");
        if let Some(invocation) = self.state.web_mcp_invocation.as_ref() {
            output.extend_background_events_after_messages(
                crate::domains::web_mcp::navigation_failed(
                    conn,
                    &self.state.owner,
                    invocation.invocation_id,
                ),
            );
        }
        let gate = crate::domains::network::failed_navigation_progress_gate(
            conn,
            &self.state,
            &error_text,
        );
        crate::domains::network::record_failed_main_document_response_body(
            conn,
            &self.state,
            error_text.clone(),
        );
        crate::domains::activity::MainDocumentFailedNavigationActivity::new(
            self.state.clone(),
            gate,
            crate::domains::network::FailedNavigationResponseMode::ProtocolError,
        )
        .emit_navigation_error_into_buffer(output, &error_text);
    }

    pub(crate) fn source_terminal_is_retained(&self) -> bool {
        self.loaded.is_none()
    }

    fn retain(&self, context: &mut CommandWorkflowContext, output: CommandOutputPlan) {
        self.output_mode.retain(context, &self.state, output);
    }

    pub(crate) async fn run(
        mut self,
        driver: &mut ProtocolWorkflowDriver,
        mut context: CommandWorkflowContext,
        cleanup: NavigationProtocolCleanup,
        after_publish: impl FnOnce() + 'static,
    ) -> crate::conn::CompletedCommandWorkflow {
        let result = self.run_inner(driver, &mut context, cleanup).await;
        match result {
            Ok(finalize) => context.complete_in_owner_turn(
                CommandOutputPlan::success(),
                move |state, context| {
                    let output =
                        state.publish_prepared_navigation_protocol_completion(finalize, context);
                    after_publish();
                    output
                },
            ),
            Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
        }
    }

    pub(crate) async fn run_into_context(
        mut self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        let output_mode = self.output_mode;
        let cleanup = NavigationProtocolCleanup::default();
        let completed = match self.run_inner(driver, context, cleanup.clone()).await {
            Ok(completed) => completed,
            Err(error) => {
                cleanup.recover(driver, context).await;
                return Err(error);
            }
        };
        context.with_state(driver, move |state, context| {
            let output = state
                .publish_prepared_navigation_protocol_completion(completed, context.context_mut());
            output_mode.retain_projected(context, output);
        })?;
        Ok(())
    }

    async fn run_inner(
        &mut self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
        cleanup: NavigationProtocolCleanup,
    ) -> Result<CompletedNavigationProtocolCompletion> {
        if let Some(page) = self.failed_page.take() {
            let _ = page.close_async().await;
        }
        let mut publication = None;
        let mut attachment = None;
        if let Some(loaded) = self.loaded.take() {
            let commit = loaded.installation.close_previous().await;
            let worker_cleanup = cleanup.clone();
            let workers = context.with_state(driver, move |state, _| {
                state
                    .prepare_replaced_navigation_workers(commit.replaced_page_owner, worker_cleanup)
            })?;
            for worker in workers {
                if let Err(error) = worker.run_into_context(driver, context).await {
                    tracing::warn!(%error, "navigation retired worker cleanup failed");
                }
            }
            let owner = self.state.owner.clone();
            let token = self.token.clone();
            let binding = self.binding.clone();
            let renderer_attachment = loaded.renderer_attachment;
            let main_commit = loaded.main_commit;
            let url = loaded.publication.target_url();
            let preloads = context.with_state(driver, move |state, context| {
                state.prepare_navigation_commit_preloads(
                    owner,
                    token,
                    binding,
                    renderer_attachment,
                    main_commit,
                    url,
                    loaded.realms,
                    commit.committed_document_post_response_continuation,
                    context.context_mut(),
                )
            })?;
            for preload in preloads {
                let mut events = Vec::new();
                preload.run(driver, &mut events).await;
                let mut output = CommandOutputPlan::default();
                output.extend_background_events(events);
                context.retain_output(output);
            }
            attachment = Some(renderer_attachment);
            publication = Some(loaded.publication);
        }
        let owner = self.state.owner.clone();
        let token = self.token.clone();
        let binding = self.binding.clone();
        let renderer = context.with_state(driver, move |state, _| {
            state.prepare_navigation_protocol_finish(owner, token, binding)
        })?;
        let events = match renderer {
            Some(renderer) => renderer.wait_using(driver).await?,
            None => Vec::new(),
        };
        Ok(CompletedNavigationProtocolCompletion {
            token: self.token.clone(),
            state: self.state.clone(),
            binding: self.binding.clone(),
            publication,
            attachment,
            finish_events: events,
            output_mode: self.output_mode,
            prefix_output: self.prefix_output.take(),
        })
    }
}

pub(crate) struct CompletedNavigationProtocolCompletion {
    token: DocumentNavigationToken,
    state: NavigationDispatchState,
    binding: Option<SessionDisposalWait>,
    publication: Option<LoadedNavigationPublication>,
    attachment: Option<moli_core::page::RendererAgentAttachmentId>,
    finish_events: Vec<crate::conn::BackgroundProtocolEvent>,
    output_mode: NavigationCommandOutput,
    prefix_output: Option<CommandOutputBuffer>,
}

impl CompletedNavigationProtocolCompletion {
    pub(crate) fn publish(
        self,
        conn: &mut CdpConnection,
        _context: &mut CommandDispatchContext,
    ) -> CommandOutputPlan {
        let mut output = self.prefix_output.unwrap_or_default();
        let current = self
            .binding
            .as_ref()
            .is_none_or(|binding| !binding.is_closed());
        if let Some(publication) = self.publication {
            if current
                && self.attachment
                    == conn.current_renderer_agent_attachment_id_for_owner(&self.state.owner)
                && conn
                    .runtime_session_owner_slot_for_owner(&self.state.owner)
                    .is_some_and(|slot| slot.accepts_document_body_completion_event(&self.token))
            {
                publication.publish(conn, &mut output);
            } else {
                publication.publish_superseded(&mut output);
            }
        }
        output.extend_background_events_after_messages(self.finish_events);
        if current {
            conn.abandon_unbound_prepared_document_navigation_for_owner(
                &self.state.owner,
                &self.token,
            );
            conn.clear_pending_document_navigation_for_owner_if_loader_matches(
                &self.state.owner,
                &self.state.loader_id,
            );
        }
        self.output_mode.project(&self.state, output.into_plan())
    }
}
