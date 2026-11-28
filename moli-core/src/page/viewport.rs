use anyhow::Result;
use moli_page_types::ViewportSurface;
use moli_renderer_v8::{
    RendererDevToolsPageReply, RendererPageNavigationState, RendererViewportCursor,
    RendererViewportFrame, RendererViewportPointerInput, RendererViewportSubscription,
    RendererViewportWheelInput,
};
use std::sync::Arc;
use tokio::sync::{Notify, watch};

use super::{
    CompletedPageCommand, Page, PendingPageCommand, RendererPageCommand, RendererPageReply,
};

/// A document's latest viewport paint, without queuing intermediate frames.
pub struct ViewportFrames {
    pub updates: watch::Receiver<Option<RendererViewportFrame>>,
    pub cursor: watch::Receiver<Option<RendererViewportCursor>>,
    /// The viewport revision awaiting display frames, while rendering work is pending.
    pub display_tick_requests: watch::Receiver<Option<u64>>,
    pub resource_wake: Arc<Notify>,
}

impl Page {
    /// Dispatches pointer input for the displayed viewport when its geometry is available.
    pub async fn dispatch_viewport_pointer(
        &mut self,
        input: RendererViewportPointerInput,
    ) -> Result<Option<super::RendererInputDispatchOutcome>> {
        match self
            .dispatch_page_command_async(RendererPageCommand::DispatchViewportPointer(input))
            .await?
        {
            RendererPageReply::DevTools(RendererDevToolsPageReply::InputDispatchOutcome(
                outcome,
            )) => Ok(Some(outcome)),
            RendererPageReply::Unit => Ok(None),
            reply => Self::unexpected_page_reply(
                "dispatch viewport pointer",
                "an input outcome or unit",
                reply,
            ),
        }
    }

    /// Cancels a pressed pointer for the given viewport, even without usable geometry.
    pub async fn cancel_viewport_pointer(
        &mut self,
        document: crate::RendererDocumentLifecycleIdentity,
        viewport_revision: u64,
    ) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::CancelViewportPointer {
                document,
                viewport_revision,
            },
            "cancel viewport pointer",
        )
        .await
    }

    /// Queues desktop wheel input on the renderer owner without waiting for DOM dispatch.
    pub fn enqueue_viewport_wheel(&self, input: RendererViewportWheelInput) -> Result<()> {
        self.handle.enqueue_viewport_wheel(input)
    }

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

    /// Dispatches a native copy action and returns the clipboard's plain text.
    pub async fn copy_viewport_selection(
        &mut self,
        document: crate::RendererDocumentLifecycleIdentity,
    ) -> Result<Option<String>> {
        self.write_viewport_clipboard(RendererPageCommand::CopyViewportSelection(document))
            .await
    }

    /// Dispatches a native cut action and returns the clipboard's plain text.
    pub async fn cut_viewport_selection(
        &mut self,
        document: crate::RendererDocumentLifecycleIdentity,
    ) -> Result<Option<String>> {
        self.cut_viewport_selection_with_sequence(document, None)
            .await
    }

    /// Dispatches a native cut and completes the optional platform editor sequence.
    pub async fn cut_viewport_selection_with_sequence(
        &mut self,
        document: crate::RendererDocumentLifecycleIdentity,
        sequence: Option<u64>,
    ) -> Result<Option<String>> {
        self.write_viewport_clipboard(RendererPageCommand::CutViewportSelection {
            document,
            sequence,
        })
        .await
    }

    async fn write_viewport_clipboard(
        &mut self,
        command: RendererPageCommand,
    ) -> Result<Option<String>> {
        match self.dispatch_page_command_async(command).await? {
            RendererPageReply::OptionalString(text) => Ok(text),
            _ => anyhow::bail!("unexpected viewport clipboard reply"),
        }
    }

    /// Dispatches a native paste action with the plain text supplied by the browser.
    /// The optional sequence acknowledges the platform editor's predicted edit.
    pub async fn paste_viewport_text(
        &mut self,
        document: crate::RendererDocumentLifecycleIdentity,
        text: Option<String>,
        sequence: Option<u64>,
    ) -> Result<bool> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::PasteViewportText {
                document,
                text,
                sequence,
            })
            .await?;
        Self::decode_bool_page_reply(reply, "paste viewport text")
    }

    /// Completes the editor sequence when keydown cancels a clipboard shortcut.
    pub async fn acknowledge_canceled_viewport_clipboard_edit(
        &mut self,
        document: crate::RendererDocumentLifecycleIdentity,
        sequence: u64,
    ) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::CompleteCanceledClipboardEdit { document, sequence },
            "complete canceled clipboard edit",
        )
        .await
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
        self.viewport_editor_with_frame_sequence()
            .await
            .map(|(editor, _)| editor)
    }

    /// Reads the editor with the last published frame sequence on this subscription.
    pub async fn viewport_editor_with_frame_sequence(
        &mut self,
    ) -> Result<(Option<crate::RendererViewportEditor>, u64)> {
        match self
            .dispatch_page_command_async(RendererPageCommand::ReadViewportEditor)
            .await?
        {
            RendererPageReply::ViewportEditor {
                editor,
                last_frame_sequence,
            } => Ok((editor, last_frame_sequence)),
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
            RendererPageReply::ViewportNavigation(navigated) => Ok(navigated),
            _ => anyhow::bail!("unexpected viewport navigation reply"),
        }
    }

    pub async fn traverse_viewport_history(&mut self, delta: i64) -> Result<bool> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::TraverseViewportHistory(delta))
            .await?;
        let RendererPageReply::ViewportNavigation(traversed) = reply else {
            anyhow::bail!("unexpected viewport history traversal reply");
        };
        Ok(traversed)
    }

    /// Watch committed navigation state across Document replacements and visibility changes.
    ///
    /// The subscription closes with the Page and does not require a paint surface.
    pub async fn subscribe_navigation_state(
        &self,
    ) -> Result<watch::Receiver<Option<RendererPageNavigationState>>> {
        self.handle.subscribe_navigation_state().await
    }

    pub async fn subscribe_viewport_frames(
        &mut self,
        surface: ViewportSurface,
        viewport_revision: u64,
    ) -> Result<ViewportFrames> {
        self.subscribe_viewport_frames_with_pointer(surface, viewport_revision, None)
            .await
    }

    pub(super) async fn subscribe_viewport_frames_with_pointer(
        &mut self,
        surface: ViewportSurface,
        viewport_revision: u64,
        initial_pointer_position: Option<(f64, f64)>,
    ) -> Result<ViewportFrames> {
        let (updates, receiver) = watch::channel(None);
        let (cursor, cursor_receiver) = watch::channel(None);
        let (display_tick_requests, display_tick_receiver) = watch::channel(None);
        let resource_wake = Arc::new(Notify::new());
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SubscribeViewportFrames(RendererViewportSubscription {
                surface,
                viewport_revision,
                initial_pointer_position,
                updates,
                cursor,
                display_tick_requests,
                resource_wake: Arc::clone(&resource_wake),
            }),
            "subscribe to viewport frames",
        )
        .await?;
        Ok(ViewportFrames {
            updates: receiver,
            cursor: cursor_receiver,
            display_tick_requests: display_tick_receiver,
            resource_wake,
        })
    }

    pub async fn set_viewport_pointer_position(
        &mut self,
        viewport_revision: u64,
        position: Option<(f64, f64)>,
    ) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SetViewportPointerPosition {
                viewport_revision,
                position,
            },
            "set viewport pointer position",
        )
        .await
    }

    /// Signals that the native window has reached another display frame.
    pub async fn viewport_display_tick(
        &self,
        revision: u64,
        time: std::time::Instant,
    ) -> Result<()> {
        self.handle.viewport_display_tick(revision, time).await
    }

    pub async fn wake_viewport_frames(&mut self) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::WakeViewportFrames,
            "wake viewport frames",
        )
        .await
    }
}
