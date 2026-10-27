use anyhow::{Context as _, Result};
use moli_core::page::{CompletedPageCommand, PendingPageCommand};

use super::state::PreparedSubresourceCorrelation;
use crate::automation::DevToolsNetworkInterceptId;
use crate::conn::{
    CdpConnection, CommandOwnerScope, PendingSubresourceFetchRequest, ProtocolWorkflowDriver,
    TargetRootDocumentProtocolAttachmentIdentity,
};

pub(super) enum SubresourceResponseInterception {
    None,
    Matched(Vec<DevToolsNetworkInterceptId>),
    Candidate,
}

pub(crate) struct PreparedSubresourceAutoContinue {
    pending: PendingPageCommand,
    correlation: PreparedSubresourceCorrelation,
    owner: CommandOwnerScope,
    binding: TargetRootDocumentProtocolAttachmentIdentity,
}

impl PreparedSubresourceAutoContinue {
    pub(super) fn prepare(
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        request_id: Option<String>,
        request: PendingSubresourceFetchRequest,
        handle_auth_requests: bool,
        response: SubresourceResponseInterception,
    ) -> Option<Self> {
        let document = conn.target_root_document_lifecycle_identity_for_owner(owner)?;
        let binding =
            conn.target_root_document_protocol_attachment_identity_for_owner(owner, document)?;
        let intercept_response = !matches!(response, SubresourceResponseInterception::None);
        let internal_id = request.internal_id;
        // The renderer may publish a continuation before its command reply is
        // consumed. Freeze its original correlation before native dispatch.
        let correlation = PreparedSubresourceCorrelation::prepare_automatic(
            conn, owner, request_id, request, response,
        )?;
        let pending = conn
            .loaded_page_mut_for_owner(owner)
            .context(crate::conn::NoDocumentLoaded)
            .and_then(|page| {
                page.start_continue_pending_subresource_fetch(
                    internal_id,
                    None,
                    None,
                    None,
                    None,
                    intercept_response,
                    handle_auth_requests,
                )
                .context("subresource fetch continue failed")
            });
        match pending {
            Ok(pending) => Some(Self {
                pending,
                correlation,
                owner: owner.clone(),
                binding,
            }),
            Err(error) => {
                correlation.rollback(conn);
                tracing::debug!(%error, internal_id, "automatic subresource continuation could not start");
                None
            }
        }
    }

    pub(crate) async fn run(self, driver: &mut ProtocolWorkflowDriver<'_>) -> Result<()> {
        let completed = self.pending.wait().await;
        let completion = SubresourceAutoContinueCompletion {
            correlation: self.correlation,
            owner: self.owner,
            binding: self.binding,
        };
        driver
            .with_state(move |state| {
                state.finish_subresource_auto_continue(completion, completed);
            })
            .await
    }
}

impl std::fmt::Debug for PreparedSubresourceAutoContinue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedSubresourceAutoContinue")
            .field("correlation", &self.correlation)
            .finish_non_exhaustive()
    }
}

pub(crate) struct SubresourceAutoContinueCompletion {
    correlation: PreparedSubresourceCorrelation,
    owner: CommandOwnerScope,
    binding: TargetRootDocumentProtocolAttachmentIdentity,
}

impl SubresourceAutoContinueCompletion {
    pub(crate) fn finish(self, conn: &mut CdpConnection, completed: Result<CompletedPageCommand>) {
        if !conn.target_root_document_protocol_attachment_identity_is_current(&self.binding) {
            self.correlation.rollback(conn);
            return;
        }
        let result = completed.and_then(|completed| {
            conn.loaded_page_mut_for_owner(&self.owner)
                .context(crate::conn::NoDocumentLoaded)?
                .finish_continue_pending_subresource_fetch(completed)
                .context("subresource fetch continue failed")
        });
        if let Err(error) = result {
            self.correlation.rollback(conn);
            tracing::debug!(%error, "automatic subresource continuation failed");
        }
    }
}
