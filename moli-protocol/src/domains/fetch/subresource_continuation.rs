use anyhow::Result;
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
        let target = request.document_command_target()?.clone();
        let document = target.document();
        let binding = conn.target_root_document_output_identity_for_owner(owner, document)?;
        let intercept_response = !matches!(response, SubresourceResponseInterception::None);
        let internal_id = request.internal_id;
        let page_owner = request.page_owner()?.clone();
        // The renderer may publish a continuation before its command reply is
        // consumed. Freeze its original correlation before native dispatch.
        let correlation = PreparedSubresourceCorrelation::prepare_automatic(
            conn, owner, request_id, request, response,
        )?;
        let pending = conn.start_subresource_document_command(
            owner,
            &page_owner,
            Some(&target),
            moli_core::RendererPageCommand::ContinuePendingSubresourceFetch {
                internal_id,
                url: None,
                method: None,
                body: None,
                headers: None,
                intercept_response,
                handle_auth_requests,
            },
        );
        match pending {
            Ok(pending) => Some(Self {
                pending,
                correlation,
                binding,
            }),
            Err(error) => {
                correlation.rollback(conn);
                tracing::debug!(%error, internal_id, "automatic subresource continuation could not start");
                None
            }
        }
    }

    pub(crate) async fn run(self, driver: &mut ProtocolWorkflowDriver) -> Result<()> {
        let completed = self.pending.wait().await;
        let completion = SubresourceAutoContinueCompletion {
            correlation: self.correlation,
            binding: self.binding,
        };
        driver.with_state(move |state| {
            state.finish_subresource_auto_continue(completion, completed);
        })
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
    binding: TargetRootDocumentProtocolAttachmentIdentity,
}

impl SubresourceAutoContinueCompletion {
    pub(crate) fn finish(self, conn: &mut CdpConnection, completed: Result<CompletedPageCommand>) {
        if !conn.target_root_document_output_identity_is_current(&self.binding) {
            self.correlation.rollback(conn);
            return;
        }
        let result = completed.and_then(CompletedPageCommand::into_subresource_continue_outcome);
        if let Err(error) = result {
            self.correlation.rollback(conn);
            tracing::debug!(%error, "automatic subresource continuation failed");
        }
    }
}
