use crate::page::CompletedPageCommand;
use crate::page::Page;
use crate::page::PendingPageCommand;
use crate::renderer::RendererCaptureScreencastFrameReply;
use crate::renderer::RendererCaptureScreencastFrameRequest;
use crate::renderer::RendererPageCommand;
use crate::renderer::RendererPageReply;
use crate::{RendererDevToolsPageCommand, RendererDevToolsPageReply};
use anyhow::Result;

impl Page {
    pub fn start_capture_screencast_frame(
        &self,
        request: RendererCaptureScreencastFrameRequest,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::CaptureScreencastFrame(request),
        ))
    }

    pub fn finish_capture_screencast_frame(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererCaptureScreencastFrameReply> {
        anyhow::ensure!(
            completion.renderer_agent_attachment_id() == self.renderer_agent_attachment_id(),
            "capture screencast frame completed for a stale renderer attachment"
        );
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "capture screencast frame page command",
            "a capture screencast frame reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::CaptureScreencastFrame(result)) => Ok(result),
        )
    }
}
