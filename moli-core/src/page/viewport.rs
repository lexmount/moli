use anyhow::Result;
use moli_page_types::ViewportSurface;
use moli_renderer_v8::{RendererViewportFrame, RendererViewportSubscription};
use std::sync::Arc;
use tokio::sync::{Notify, watch};

use super::{
    CompletedPageCommand, Page, PendingPageCommand, RendererPageCommand, RendererPageReply,
};

/// A document's latest viewport paint, without queuing intermediate frames.
pub struct ViewportFrames {
    pub updates: watch::Receiver<Option<RendererViewportFrame>>,
    pub resource_wake: Arc<Notify>,
}

impl Page {
    pub async fn viewport_selected_text(
        &mut self,
        document: crate::RendererDocumentLifecycleIdentity,
    ) -> Result<Option<String>> {
        match self
            .dispatch_page_command_async(RendererPageCommand::ReadViewportSelection(document))
            .await?
        {
            RendererPageReply::OptionalString(text) => Ok(text),
            _ => anyhow::bail!("unexpected viewport selection reply"),
        }
    }

    pub async fn drop_viewport_files(
        &mut self,
        document: crate::RendererDocumentLifecycleIdentity,
        position: (f64, f64),
        files: Vec<super::RendererDraggedFile>,
        modifiers: u8,
    ) -> Result<bool> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::DropViewportFiles {
                document,
                x: position.0,
                y: position.1,
                files,
                modifiers,
            })
            .await?;
        Self::decode_bool_page_reply(reply, "drop viewport files")
    }

    pub async fn viewport_editor(&mut self) -> Result<Option<crate::RendererViewportEditor>> {
        match self
            .dispatch_page_command_async(RendererPageCommand::ReadViewportEditor)
            .await?
        {
            RendererPageReply::ViewportEditor(editor) => Ok(editor),
            _ => anyhow::bail!("unexpected viewport editor reply"),
        }
    }

    pub async fn edit_viewport(
        &mut self,
        edit: crate::RendererViewportEdit,
    ) -> Result<crate::RendererViewportEditReply> {
        match self
            .dispatch_page_command_async(RendererPageCommand::ApplyViewportEdit(edit))
            .await?
        {
            RendererPageReply::ViewportEdit(reply) => Ok(reply),
            _ => anyhow::bail!("unexpected viewport editing reply"),
        }
    }

    /// Navigate the existing browsing context. `None` reloads its current entry.
    pub fn start_viewport_navigation(&self, target: Option<String>) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::NavigateViewport(target))
    }

    pub fn finish_viewport_navigation(&mut self, completion: CompletedPageCommand) -> Result<bool> {
        match self.finish_page_command(completion) {
            RendererPageReply::Bool(navigated) => Ok(navigated),
            _ => anyhow::bail!("unexpected viewport navigation reply"),
        }
    }

    pub async fn traverse_viewport_history(&mut self, delta: i64) -> Result<bool> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::TraverseViewportHistory(delta))
            .await?;
        let traversed = Self::decode_bool_page_reply(reply, "traverse viewport history")?;
        self.wake_viewport_frames().await?;
        Ok(traversed)
    }

    pub async fn subscribe_viewport_frames(
        &mut self,
        surface: ViewportSurface,
        viewport_revision: u64,
    ) -> Result<ViewportFrames> {
        let (updates, receiver) = watch::channel(None);
        let resource_wake = Arc::new(Notify::new());
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SubscribeViewportFrames(RendererViewportSubscription {
                surface,
                viewport_revision,
                updates,
                resource_wake: Arc::clone(&resource_wake),
            }),
            "subscribe to viewport frames",
        )
        .await?;
        Ok(ViewportFrames {
            updates: receiver,
            resource_wake,
        })
    }

    pub async fn wake_viewport_frames(&mut self) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::WakeViewportFrames,
            "wake viewport frames",
        )
        .await
    }
}
