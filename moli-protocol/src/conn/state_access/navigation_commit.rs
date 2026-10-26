use std::{cell::RefCell, future::Future, pin::Pin, rc::Rc};

use moli_core::{
    page::RendererPageReplacementError,
    runtime::{
        BuiltDocumentPage, CompletedDocumentPageReplacement, PendingDocumentPageReplacement,
    },
};

use super::{ProtocolStateAccess, ProtocolWorkflowState};
use crate::conn::{
    CdpConnection, CommandOwnerScope, CommittedNavigationPage, CommittedRendererAgentAttachment,
    DocumentNavigationToken, LoadedNavigation, NavigationDispatchState, ResponseCommitReady,
    StablePageNavigationCommitTarget, spawn_owned_protocol_wait,
};
use crate::domains::page::BackgroundNavigationCompletion;

type AttachmentTransaction = Rc<RefCell<Option<CommittedRendererAgentAttachment>>>;

enum NativeDocumentCommit {
    New(Pin<Box<dyn Future<Output = anyhow::Result<BuiltDocumentPage>>>>),
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
                anyhow::anyhow!("NavigationRequestNotCurrent"),
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
            .map_err(|error| {
                RendererPageReplacementError::document_preserved(anyhow::anyhow!(error))
            })?;
        let transaction = match expected.as_ref() {
            Some(expected) => {
                conn.commit_renderer_agent_candidate_for_existing_page(owner, candidate, expected)
            }
            None => conn.commit_renderer_agent_candidate_for_owner(owner, candidate, renderer_page),
        }
        .map_err(|error| {
            RendererPageReplacementError::document_preserved(anyhow::anyhow!(error))
        })?;
        let attachment = transaction.current().id();
        // Register the transaction before returning even an immediately-ready
        // native reply. The supervisor owns rollback if delivery is cancelled.
        *transaction_slot.borrow_mut() = Some(transaction);
        let fact = navigation.main_document_commit().ok_or_else(|| {
            RendererPageReplacementError::document_preserved(anyhow::anyhow!(
                "prepared Document is missing its frozen main Document commit identity"
            ))
        })?;
        if !conn
            .runtime_session_owner_slot_mut_for_owner(owner)
            .is_some_and(|slot| slot.register_renderer_document_commit(token, attachment, fact))
        {
            return Err(RendererPageReplacementError::document_preserved(
                anyhow::anyhow!("NavigationRequestNotCurrent"),
            ));
        }
        let permit = navigation.issue_commit_permit();
        let native = match expected {
            Some(_) => {
                let page = conn.loaded_page_mut_for_owner(owner).ok_or_else(|| {
                    RendererPageReplacementError::document_preserved(anyhow::anyhow!(
                        "navigation's Page was removed"
                    ))
                })?;
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
        let transaction = transaction.as_ref().ok_or_else(|| {
            RendererPageReplacementError::document_unavailable(anyhow::anyhow!(
                "navigation commit lost its attachment transaction"
            ))
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
                anyhow::anyhow!("navigation commit was superseded before adoption"),
            ));
        }
        let page = conn.loaded_page_mut_for_owner(owner).ok_or_else(|| {
            RendererPageReplacementError::document_unavailable(anyhow::anyhow!(
                "navigation's Page was removed"
            ))
        })?;
        completed.adopt(page)
    }
}

async fn commit_prepared_document(
    access: ProtocolStateAccess,
    owner: CommandOwnerScope,
    token: DocumentNavigationToken,
    navigation: ResponseCommitReady,
    web_mcp_invocation: Option<moli_page_types::RendererWebMcpNavigation>,
    transaction: AttachmentTransaction,
) -> Result<LoadedNavigation<CommittedNavigationPage>, RendererPageReplacementError> {
    let admission_owner = owner.clone();
    let admission_token = token.clone();
    let admission_transaction = transaction.clone();
    let (navigation, native) = access
        .with_state(move |state| {
            state.start_prepared_document_commit(
                &admission_owner,
                &admission_token,
                navigation,
                web_mcp_invocation,
                &admission_transaction,
            )
        })
        .await
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
            let committed = access
                .with_state(move |state| {
                    state.adopt_prepared_document_commit(
                        &owner,
                        &token,
                        &adoption_target,
                        &transaction,
                        completed,
                    )
                })
                .await
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
        let access = self
            .protocol_state_access()
            .expect("scheduler owns the state request receiver");
        let sender = self
            .background_navigation_completion_sender_for_owner(&state.owner)
            .expect("background navigation retains its completion channel");
        let web_mcp_invocation = state.web_mcp_invocation.clone();
        let expected = navigation.stable_page_target().cloned();
        let transaction = AttachmentTransaction::default();
        let wait_transaction = transaction.clone();
        let wait_owner = state.owner.clone();
        let wait_token = token.clone();
        let task_token = token.clone();
        let task_owner = state.owner.clone();
        let task = spawn_owned_protocol_wait(
            async move {
                commit_prepared_document(
                    access,
                    wait_owner,
                    wait_token,
                    navigation,
                    web_mcp_invocation,
                    wait_transaction,
                )
                .await
            },
            move |result| {
                let transaction = transaction.borrow_mut().take();
                let navigation = result.unwrap_or_else(|failure| {
                    let error = anyhow::anyhow!(failure.message());
                    Err(if transaction.is_some() {
                        RendererPageReplacementError::document_unavailable(error)
                    } else {
                        RendererPageReplacementError::document_preserved(error)
                    })
                });
                let _ = sender.send(BackgroundNavigationCompletion::PreparedDocument(Box::new(
                    BackgroundPreparedNavigationCommit {
                        token,
                        state,
                        expected,
                        transaction,
                        navigation,
                    },
                )));
            },
        );
        if let Some(slot) = self.runtime_session_owner_slot_mut_for_owner(&task_owner) {
            slot.supervise_background_navigation(&task_token, task);
        }
    }
}
