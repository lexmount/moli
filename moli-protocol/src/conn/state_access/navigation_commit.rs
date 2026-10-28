use anyhow::Context as _;
use anyhow::Result;

use std::{cell::RefCell, future::Future, pin::Pin, rc::Rc};

use moli_core::{
    page::RendererPageReplacementError,
    runtime::{
        BuiltDocumentPage, CompletedDocumentPageReplacement, PendingDocumentPageReplacement,
    },
};

use super::{ProtocolStateHandle, ProtocolWorkflowDriver, ProtocolWorkflowState};
use crate::conn::{
    CdpConnection, CommandOwnerScope, CommittedNavigationPage, CommittedRendererAgentAttachment,
    DocumentNavigationToken, LoadedNavigation, NavigationDispatchState, NoDocumentLoaded,
    ResponseCommitReady, StablePageNavigationCommitTarget, spawn_owned_protocol_wait,
};
use crate::domains::page::BackgroundNavigationCompletion;

#[derive(Debug, thiserror::Error)]
enum PreparedDocumentCommitError {
    #[error("NavigationRequestNotCurrent")]
    RequestNotCurrent,
    #[error("prepared Document is missing its frozen main Document commit identity")]
    MissingCommitIdentity,
    #[error("navigation commit lost its attachment transaction")]
    MissingAttachmentTransaction,
    #[error("navigation commit was superseded before adoption")]
    SupersededBeforeAdoption,
}

#[derive(Default)]
struct NavigationAttachmentTransaction {
    expected: Option<StablePageNavigationCommitTarget>,
    attachment: Option<CommittedRendererAgentAttachment>,
}

type AttachmentTransaction = Rc<RefCell<NavigationAttachmentTransaction>>;

/// Rollback authority stays with the original command even after its wait has
/// returned but before the actor consumes the result. It owns no connection.
#[derive(Clone)]
pub(crate) struct PreparedDocumentCommitState {
    transaction: AttachmentTransaction,
}

impl PreparedDocumentCommitState {
    pub(crate) fn new() -> Self {
        Self {
            transaction: Rc::default(),
        }
    }

    pub(crate) async fn commit(
        &self,
        access: ProtocolStateHandle,
        token: DocumentNavigationToken,
        state: &NavigationDispatchState,
        navigation: ResponseCommitReady,
    ) -> Result<LoadedNavigation<CommittedNavigationPage>, RendererPageReplacementError> {
        self.commit_using(
            &mut ProtocolWorkflowDriver::new(access),
            token,
            state,
            navigation,
        )
        .await
    }

    pub(crate) async fn commit_using(
        &self,
        driver: &mut ProtocolWorkflowDriver,
        token: DocumentNavigationToken,
        state: &NavigationDispatchState,
        navigation: ResponseCommitReady,
    ) -> Result<LoadedNavigation<CommittedNavigationPage>, RendererPageReplacementError> {
        commit_prepared_document(
            driver,
            state.owner.clone(),
            token,
            navigation,
            state.web_mcp_invocation.clone(),
            self.transaction.clone(),
        )
        .await
    }

    pub(crate) fn complete(
        &self,
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
        navigation: Result<LoadedNavigation<CommittedNavigationPage>, RendererPageReplacementError>,
    ) -> BackgroundPreparedNavigationCommit {
        let mut transaction = self.transaction.borrow_mut();
        BackgroundPreparedNavigationCommit {
            token,
            state,
            expected: transaction.expected.clone(),
            transaction: transaction.attachment.take(),
            navigation,
        }
    }

    pub(crate) fn cancel(
        &self,
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
    ) -> BackgroundPreparedNavigationCommit {
        self.fail(token, state, anyhow::Error::new(moli_fetch::FetchCancelled))
    }

    fn fail(
        &self,
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
        error: anyhow::Error,
    ) -> BackgroundPreparedNavigationCommit {
        let error = if self.transaction.borrow().attachment.is_some() {
            RendererPageReplacementError::document_unavailable(error)
        } else {
            RendererPageReplacementError::document_preserved(error)
        };
        self.complete(token, state, Err(error))
    }
}

enum NativeDocumentCommit {
    New(Pin<Box<dyn Future<Output = Result<BuiltDocumentPage>>>>),
    Replacement(PendingDocumentPageReplacement),
}

/// The result of the prepared Document transaction. The existing navigation
/// owner retains the task until this result is consumed or that owner retires.
pub struct BackgroundPreparedNavigationCommit {
    pub(crate) token: DocumentNavigationToken,
    pub(crate) state: NavigationDispatchState,
    pub(crate) expected: Option<StablePageNavigationCommitTarget>,
    pub(crate) transaction: Option<CommittedRendererAgentAttachment>,
    pub(crate) navigation:
        Result<LoadedNavigation<CommittedNavigationPage>, RendererPageReplacementError>,
}

impl BackgroundPreparedNavigationCommit {
    pub(crate) fn requested_url(&self) -> &str {
        self.state.requested_url.as_str()
    }
}

impl ProtocolWorkflowState<'_> {
    fn start_prepared_document_commit(
        self,
        owner: &CommandOwnerScope,
        token: &DocumentNavigationToken,
        mut navigation: ResponseCommitReady,
        web_mcp_invocation: Option<moli_page_types::RendererWebMcpNavigation>,
        transaction_slot: &AttachmentTransaction,
    ) -> Result<(ResponseCommitReady, NativeDocumentCommit), RendererPageReplacementError> {
        let conn = self.connection;
        if !conn.accepts_pending_document_navigation_for_owner(owner, token) {
            return Err(RendererPageReplacementError::document_preserved(
                PreparedDocumentCommitError::RequestNotCurrent.into(),
            ));
        }
        // Configuration and native admission share this synchronous state
        // operation. An update accepted before admission belongs to this
        // Document; there is no configuration-ACK window to replay a stale
        // snapshot over another session's later policy.
        let mut configuration = conn
            .prepared_document_commit_configuration_for_owner(owner, navigation.final_url())
            .map_err(RendererPageReplacementError::document_preserved)?;
        configuration.web_mcp_invocation = web_mcp_invocation;
        let expected = navigation.stable_page_target().cloned();
        let renderer_page = navigation.renderer_page_residence_identity();
        let candidate = conn
            .prepare_renderer_agent_candidate_token_for_owner(
                owner,
                token,
                navigation.renderer_devtools_agent_token(),
            )
            .map_err(RendererPageReplacementError::document_preserved)?;
        let transaction = match expected.as_ref() {
            Some(expected) => {
                conn.commit_renderer_agent_candidate_for_existing_page(owner, candidate, expected)
            }
            None => conn.commit_renderer_agent_candidate_for_owner(owner, candidate, renderer_page),
        }
        .map_err(RendererPageReplacementError::document_preserved)?;
        let attachment = transaction.current().id();
        // Register the transaction before returning even an immediately-ready
        // native reply. The supervisor owns rollback if delivery is cancelled.
        *transaction_slot.borrow_mut() = NavigationAttachmentTransaction {
            expected: expected.clone(),
            attachment: Some(transaction),
        };
        let fact = navigation.main_document_commit().ok_or_else(|| {
            RendererPageReplacementError::document_preserved(
                PreparedDocumentCommitError::MissingCommitIdentity.into(),
            )
        })?;
        if !conn
            .runtime_session_owner_slot_mut_for_owner(owner)
            .is_some_and(|slot| slot.register_renderer_document_commit(token, attachment, fact))
        {
            return Err(RendererPageReplacementError::document_preserved(
                PreparedDocumentCommitError::RequestNotCurrent.into(),
            ));
        }
        let permit = navigation.issue_commit_permit();
        let native = match expected {
            Some(_) => {
                let page = conn
                    .loaded_page_mut_for_owner(owner)
                    .context(NoDocumentLoaded)
                    .map_err(RendererPageReplacementError::document_preserved)?;
                NativeDocumentCommit::Replacement(navigation.start_page_replacement(
                    permit,
                    page,
                    configuration,
                )?)
            }
            None => NativeDocumentCommit::New(Box::pin(
                navigation
                    .start_new_document_commit(permit, configuration)
                    .map_err(RendererPageReplacementError::document_preserved)?,
            )),
        };
        // Both variants synchronously enqueue their exact renderer commit.
        // A later task poll cannot move this admission behind another command.
        Ok((navigation, native))
    }

    fn adopt_prepared_document_commit(
        self,
        owner: &CommandOwnerScope,
        token: &DocumentNavigationToken,
        expected: &StablePageNavigationCommitTarget,
        transaction_slot: &AttachmentTransaction,
        completed: CompletedDocumentPageReplacement,
    ) -> Result<moli_core::runtime::CommittedDocumentPageReplacement, RendererPageReplacementError>
    {
        let conn = self.connection;
        let transaction = transaction_slot.borrow();
        let transaction = transaction.attachment.as_ref().ok_or_else(|| {
            RendererPageReplacementError::document_unavailable(
                PreparedDocumentCommitError::MissingAttachmentTransaction.into(),
            )
        })?;
        if !conn.accepts_pending_document_navigation_for_owner(owner, token)
            || conn
                .target_page_residence_identity_for_owner(owner)
                .as_ref()
                != Some(&expected.target_page)
            || conn.renderer_page_residence_identity_for_owner(owner)
                != Some(expected.renderer_page)
            || conn.current_renderer_agent_attachment_id_for_owner(owner)
                != Some(transaction.current().id())
        {
            return Err(RendererPageReplacementError::document_unavailable(
                PreparedDocumentCommitError::SupersededBeforeAdoption.into(),
            ));
        }
        let page = conn
            .loaded_page_mut_for_owner(owner)
            .context(NoDocumentLoaded)
            .map_err(RendererPageReplacementError::document_unavailable)?;
        completed.adopt(page)
    }
}

async fn commit_prepared_document(
    driver: &mut ProtocolWorkflowDriver,
    owner: CommandOwnerScope,
    token: DocumentNavigationToken,
    navigation: ResponseCommitReady,
    web_mcp_invocation: Option<moli_page_types::RendererWebMcpNavigation>,
    transaction: AttachmentTransaction,
) -> Result<LoadedNavigation<CommittedNavigationPage>, RendererPageReplacementError> {
    let admission_owner = owner.clone();
    let admission_token = token.clone();
    let admission_transaction = transaction.clone();
    let (navigation, native) = driver
        .with_state(move |state| {
            state.start_prepared_document_commit(
                &admission_owner,
                &admission_token,
                navigation,
                web_mcp_invocation,
                &admission_transaction,
            )
        })
        .map_err(RendererPageReplacementError::document_preserved)??;
    match native {
        NativeDocumentCommit::New(pending) => {
            let built = pending
                .await
                .map_err(RendererPageReplacementError::document_preserved)?;
            navigation
                .finish_committed_document(
                    CommittedNavigationPage::Created(Box::new(built.page)),
                    built.pending_download,
                    built.page_creation_diagnostics,
                    built.page_creation_artifacts,
                )
                .await
                .map_err(RendererPageReplacementError::document_unavailable)
        }
        NativeDocumentCommit::Replacement(pending) => {
            let completed = pending.wait().await?;
            let expected = navigation
                .stable_page_target()
                .expect("replacement commit retains its exact Page")
                .clone();
            let adoption_target = expected.clone();
            let committed = driver
                .with_state(move |state| {
                    state.adopt_prepared_document_commit(
                        &owner,
                        &token,
                        &adoption_target,
                        &transaction,
                        completed,
                    )
                })
                .map_err(RendererPageReplacementError::document_unavailable)??;
            navigation
                .finish_committed_document(
                    CommittedNavigationPage::Replaced(expected),
                    committed.pending_download,
                    committed.page_creation_diagnostics,
                    committed.page_creation_artifacts,
                )
                .await
                .map_err(RendererPageReplacementError::document_unavailable)
        }
    }
}

impl CdpConnection {
    pub(crate) fn start_background_prepared_navigation_commit(
        &mut self,
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
        navigation: ResponseCommitReady,
    ) {
        if self
            .runtime_session_owner_slot_for_owner(&state.owner)
            .is_some_and(|slot| slot.has_supervised_background_navigation())
        {
            // The original transaction owns the sole terminal result. A
            // duplicate prepared response must not replace or cancel its task.
            return;
        }
        let access = self.protocol_state_access();
        let sender = self
            .background_navigation_completion_sender_for_owner(&state.owner)
            .expect("background navigation retains its completion channel");
        let commit = PreparedDocumentCommitState::new();
        let wait_commit = commit.clone();
        let wait_state = state.clone();
        let wait_token = token.clone();
        let task_token = token.clone();
        let task_owner = state.owner.clone();
        let task = spawn_owned_protocol_wait(
            async move {
                wait_commit
                    .commit(access, wait_token, &wait_state, navigation)
                    .await
            },
            move |result| {
                let completion = match result {
                    Ok(navigation) => commit.complete(token, state, navigation),
                    Err(failure) => commit.fail(token, state, anyhow::Error::new(failure)),
                };
                let _ = sender.send(BackgroundNavigationCompletion::PreparedDocument(Box::new(
                    completion,
                )));
            },
        );
        if let Some(slot) = self.runtime_session_owner_slot_mut_for_owner(&task_owner) {
            slot.supervise_background_navigation(&task_token, task);
        }
    }
}
