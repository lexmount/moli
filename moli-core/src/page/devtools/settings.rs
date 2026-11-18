use crate::RendererDevToolsPageCommand;
use crate::page::CompletedPageCommand;
use crate::page::Page;
use crate::page::PendingDevToolsIoCommandDispatch;
use crate::page::PendingPageCommand;
use crate::page::document_cookie_diagnostics::DocumentCookieFacadeTelemetrySnapshot;
use crate::page::document_cookie_diagnostics::DocumentCookieOwnerSnapshot;
use crate::page::protocol_support::SubresourceInterceptionClasses;
use crate::renderer::RendererPageCommand;
use crate::renderer::RendererPageCookieFacadeSnapshotReply;
use crate::renderer::RendererPageReply;
use crate::renderer::RendererRuntimeInspectorResponseSender;
use anyhow::Result;

impl Page {
    pub async fn set_fetch_subresource_interception_async(
        &mut self,
        classes: SubresourceInterceptionClasses,
    ) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::DevTools(
                RendererDevToolsPageCommand::SetFetchSubresourceInterception { classes },
            ),
            "set fetch subresource interception",
        )
        .await
    }

    pub fn start_set_fetch_subresource_interception(
        &self,
        classes: SubresourceInterceptionClasses,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::SetFetchSubresourceInterception { classes },
        ))
    }

    pub fn finish_set_fetch_subresource_interception(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set fetch subresource interception",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    /// Admits the DevTools IO-agent setting through the target's shared IO
    /// task FIFO without entering the renderer owner's Main command queue.
    pub fn start_set_script_execution_disabled_from_io(
        &self,
        disabled: bool,
    ) -> PendingDevToolsIoCommandDispatch {
        let route = self
            .handle
            .enqueue_set_script_execution_disabled_io_command(
                self.renderer_agent_attachment_id,
                self.renderer_devtools_command_session_id.clone(),
                disabled,
            );
        Self::pending_devtools_io_command_dispatch(route)
    }

    /// Publishes the terminal Emulation response through the concrete
    /// renderer DevTools session that owns this Page attachment.
    pub fn start_set_script_execution_disabled_from_io_with_response(
        &self,
        inspector_session_id: Option<String>,
        disabled: bool,
        response: RendererRuntimeInspectorResponseSender,
    ) -> Result<PendingDevToolsIoCommandDispatch> {
        let attachment = self.renderer_agent_attachment_id.ok_or_else(|| {
            anyhow::anyhow!("Emulation IO response requires a renderer attachment")
        })?;
        let route = self
            .handle
            .enqueue_set_script_execution_disabled_io_command_with_response(
                attachment,
                inspector_session_id,
                disabled,
                response,
            );
        Ok(Self::pending_devtools_io_command_dispatch(route))
    }

    pub async fn document_cookie_telemetry_snapshot_async(
        &mut self,
    ) -> Result<DocumentCookieFacadeTelemetrySnapshot> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::DocumentCookieTelemetrySnapshot)
            .await?;
        match reply {
            RendererPageReply::CookieFacadeSnapshot(snapshot) => match *snapshot {
                RendererPageCookieFacadeSnapshotReply::Telemetry(snapshot) => Ok(snapshot.into()),
                other => Page::unexpected_page_reply(
                    "document cookie telemetry page command",
                    "a cookie facade telemetry snapshot reply",
                    RendererPageReply::CookieFacadeSnapshot(Box::new(other)),
                ),
            },
            other => Page::unexpected_page_reply(
                "document cookie telemetry page command",
                "a cookie facade telemetry snapshot reply",
                other,
            ),
        }
    }

    pub async fn document_cookie_owner_snapshot_async(
        &mut self,
    ) -> Result<DocumentCookieOwnerSnapshot> {
        let pending = self.start_document_cookie_owner_snapshot()?;
        self.finish_document_cookie_owner_snapshot(pending.wait().await?)
    }

    pub fn finish_document_cookie_owner_snapshot(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<DocumentCookieOwnerSnapshot> {
        let reply = self.finish_page_command(completion);
        match reply {
            RendererPageReply::CookieFacadeSnapshot(snapshot) => match *snapshot {
                RendererPageCookieFacadeSnapshotReply::Owner(snapshot) => Ok((*snapshot).into()),
                other => Page::unexpected_page_reply(
                    "document cookie owner page command",
                    "a cookie facade owner snapshot reply",
                    RendererPageReply::CookieFacadeSnapshot(Box::new(other)),
                ),
            },
            other => Page::unexpected_page_reply(
                "document cookie owner page command",
                "a cookie facade owner snapshot reply",
                other,
            ),
        }
    }
}
