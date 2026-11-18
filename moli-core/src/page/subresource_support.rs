use super::Page;
use super::PendingPageCommand;
use super::RendererPendingJavaScriptDialog;
use crate::renderer::RendererPageCommand;
use crate::renderer::RendererPageReply;
use anyhow::Result;

impl Page {
    pub fn start_stop_document_lifecycle(&self) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::StopDocumentLifecycle)
    }

    pub fn take_pending_modal_javascript_dialogs(&self) -> Vec<RendererPendingJavaScriptDialog> {
        self.handle.take_pending_modal_javascript_dialogs()
    }

    pub async fn stop_document_lifecycle_async(&mut self) -> Result<()> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::StopDocumentLifecycle)
            .await?;
        expect_page_reply!(
            reply,
            "stop document lifecycle page command",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }
}
