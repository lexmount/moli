use anyhow::Result;

use super::{
    CompletedPageCommand, Page, PendingPageCommand, RendererCommandTurnOutput, RendererPageCommand,
    RendererPageReply, RendererSetDocumentContentResult,
};

impl Page {
    /// Captures this Page without queuing renderer work or resuming a paused
    /// script. Preparation activates the replacement only after a response is
    /// accepted as a Document.
    pub fn document_replacement_target(
        &self,
        cancellation: moli_fetch::FetchCancelHandle,
    ) -> crate::runtime::RendererPageReplacementTarget {
        self.handle.document_replacement_target(cancellation)
    }

    pub fn start_set_document_content(
        &self,
        frame_id: String,
        html: String,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SetDocumentContent { frame_id, html })
    }

    pub fn finish_set_document_content(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererSetDocumentContentResult> {
        let (result, _) = self.finish_set_document_content_command_turn(completion)?;
        Ok(result)
    }

    pub fn finish_set_document_content_command_turn(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<(RendererSetDocumentContentResult, RendererCommandTurnOutput)> {
        let output = self.finish_page_command_turn(completion);
        let RendererPageReply::SetDocumentContentResult(result) = output.completion().reply()
        else {
            return Err(anyhow::anyhow!(
                "set document content page command returned an unexpected renderer reply"
            ));
        };
        Ok((*result, output))
    }
}
