use super::Page;
use super::RendererAgentAttachmentId;
use super::RendererCommandTurnOutput;
use super::RendererPageCommand;
use super::RendererPageCommandPending;
use super::RendererPageReply;
use super::RendererRuntimeInspectorMessage;
use crate::RendererOutputFence;
use anyhow::Result;
use anyhow::bail;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageDocumentCommandTarget {
    pub(super) target: moli_renderer_v8::RendererDocumentCommandTarget,
    pub(super) network_pause: Option<moli_renderer_v8::network::ResourceInterceptionPause>,
    pub(super) response_body: Option<moli_renderer_v8::network::ResourceInterceptionBody>,
}

impl PageDocumentCommandTarget {
    pub(crate) fn new(target: moli_renderer_v8::RendererDocumentCommandTarget) -> Self {
        Self {
            target,
            network_pause: None,
            response_body: None,
        }
    }
    pub fn document(&self) -> crate::RendererDocumentLifecycleIdentity {
        self.target.document()
    }
    pub fn start(&self, command: RendererPageCommand) -> Result<PendingPageCommand> {
        self.target
            .enqueue(command)
            .map(PendingPageCommand::from_renderer)
    }
}

pub struct PendingPageCommand {
    pub(in crate::page) pending: RendererPageCommandPending,
    pub(in crate::page) renderer_agent_attachment_id: Option<RendererAgentAttachmentId>,
}

impl PendingPageCommand {
    pub(crate) fn from_renderer(pending: RendererPageCommandPending) -> Self {
        Self {
            pending,
            renderer_agent_attachment_id: None,
        }
    }

    pub fn renderer_agent_attachment_id(&self) -> Option<RendererAgentAttachmentId> {
        self.renderer_agent_attachment_id
    }
}

pub struct CompletedPageCommand {
    pub(in crate::page) output: RendererCommandTurnOutput,
    pub(in crate::page) renderer_agent_attachment_id: Option<RendererAgentAttachmentId>,
}

#[derive(Debug, thiserror::Error)]
#[error("Document command returned an unexpected renderer reply")]
pub(super) struct UnexpectedDocumentCommandReply;

impl CompletedPageCommand {
    /// Fetch settlement owns its exact renderer output; Page adoption is not
    /// required to acknowledge the already-executed native operation.
    pub fn into_unit_reply(self) -> Result<()> {
        self.into_unit_page_command_turn().map(drop)
    }

    pub fn into_subresource_continue_outcome(
        self,
    ) -> Result<crate::page::PendingSubresourceContinueOutcome> {
        match self.output.into_reply_and_state().0 {
            RendererPageReply::PendingSubresourceContinueOutcome(outcome) => Ok(outcome),
            _ => Err(UnexpectedDocumentCommandReply.into()),
        }
    }

    /// Returns the exact renderer output position produced by this command.
    ///
    /// Protocol dispatch must capture this before a command-specific decoder
    /// consumes the completion. The concrete publication travels separately
    /// from the renderer reply, so dropping this cursor would allow the reply
    /// to overtake owner actions or observations produced by the same turn.
    pub fn renderer_output_predecessor(&self) -> Option<RendererOutputFence> {
        self.output.renderer_output_predecessor()
    }

    pub fn renderer_agent_attachment_id(&self) -> Option<RendererAgentAttachmentId> {
        self.renderer_agent_attachment_id
    }

    pub fn bool_reply_value(&self) -> Option<bool> {
        match self.output.completion().reply() {
            RendererPageReply::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn into_output(self) -> RendererCommandTurnOutput {
        self.output
    }

    pub(crate) fn output(&self) -> &RendererCommandTurnOutput {
        &self.output
    }

    /// Consumes a command that was already settled by the renderer owner.
    ///
    /// This does not require the originating [`Page`] handle to remain the
    /// target's current attachment. A Runtime command can synchronously start
    /// a navigation after V8 has produced its response; replacing the
    /// attachment must not invalidate that already-frozen response or its
    /// concrete output cursor.
    pub fn into_runtime_protocol_message_command_turn(self) -> Result<RendererCommandTurnOutput> {
        if !matches!(
            self.output.completion().reply(),
            RendererPageReply::RuntimeInspectorProtocolMessages(_)
        ) {
            return Err(anyhow::anyhow!(
                "runtime protocol page command returned an unexpected renderer reply"
            ));
        }
        Ok(self.output)
    }

    /// Consumes a unit-reply command that settled on an attachment which is
    /// no longer current.
    ///
    /// The caller deliberately does not install the frozen Page state on the
    /// replacement attachment. Target-scoped browser state must instead be
    /// replayed while that replacement Page is installed.
    pub fn into_unit_page_command_turn(self) -> Result<RendererCommandTurnOutput> {
        if !matches!(self.output.completion().reply(), RendererPageReply::Unit) {
            return Err(UnexpectedDocumentCommandReply.into());
        }
        Ok(self.output)
    }
}

impl Page {
    pub fn document_command_target(
        &self,
        document: crate::RendererDocumentLifecycleIdentity,
    ) -> Result<PageDocumentCommandTarget> {
        self.handle
            .document_command_target(document)
            .map(PageDocumentCommandTarget::new)
    }

    pub(crate) fn start_page_command(
        &self,
        command: RendererPageCommand,
    ) -> Result<PendingPageCommand> {
        #[cfg(feature = "devtools")]
        let mut command = command;
        #[cfg(feature = "devtools")]
        if let Some(attachment_id) = self.renderer_agent_attachment_id {
            command.bind_inspector_attachment(attachment_id);
        }
        #[cfg(feature = "devtools")]
        let pending = if self.renderer_agent_attachment_id.is_some() {
            self.handle.enqueue_protocol_command_in_inspector_session(
                command,
                self.renderer_devtools_command_session_id.clone(),
            )?
        } else {
            // CLI, embedding and other renderer-owner callers reuse the thin
            // protocol-turn capture policy, but they are not a
            // DevToolsSession receiver and must not acquire a Main ingress
            // lane that is scoped to a renderer attachment.
            self.handle.enqueue_protocol_command(command)?
        };
        #[cfg(not(feature = "devtools"))]
        let pending = self.handle.enqueue_protocol_command(command)?;
        Ok(PendingPageCommand {
            pending,
            renderer_agent_attachment_id: self.renderer_agent_attachment_id,
        })
    }

    pub fn start_runtime_realm_inventory(&self) -> Result<PendingPageCommand> {
        self.start_full_page_command(RendererPageCommand::RuntimeRealmInventory)
    }

    pub(super) fn start_full_page_command(
        &self,
        command: RendererPageCommand,
    ) -> Result<PendingPageCommand> {
        #[cfg(feature = "devtools")]
        let mut command = command;
        #[cfg(feature = "devtools")]
        if let Some(attachment_id) = self.renderer_agent_attachment_id {
            command.bind_inspector_attachment(attachment_id);
        }
        Ok(PendingPageCommand {
            pending: self.handle.enqueue_async_command(command)?,
            renderer_agent_attachment_id: self.renderer_agent_attachment_id,
        })
    }

    pub fn finish_page_command_turn(
        &mut self,
        completion: CompletedPageCommand,
    ) -> RendererCommandTurnOutput {
        self.replace_page_state(completion.output.completion().page_state().clone());
        completion.output
    }

    pub(crate) fn finish_page_command(
        &mut self,
        completion: CompletedPageCommand,
    ) -> RendererPageReply {
        let output = self.finish_page_command_turn(completion);
        let (completion, _renderer_output_predecessor) = output.into_completion_and_predecessor();
        let (reply, _, _) = completion.into_parts();
        reply
    }

    pub(super) async fn dispatch_page_command_async(
        &mut self,
        command: RendererPageCommand,
    ) -> Result<RendererPageReply> {
        // Async page commands follow the same page-state refresh rule: keep one
        // refresh boundary so the renderer owner only has a single async
        // dispatch boundary to replace.
        let pending = self.start_full_page_command(command)?;
        let completion = pending.wait().await?;
        Ok(self.finish_page_command(completion))
    }

    pub(super) async fn dispatch_unit_page_command_async(
        &mut self,
        command: RendererPageCommand,
        operation: &str,
    ) -> Result<()> {
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            operation,
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub(super) fn decode_bool_page_reply(
        reply: RendererPageReply,
        operation: &str,
    ) -> Result<bool> {
        expect_page_reply!(
            reply,
            operation,
            "a bool reply",
            RendererPageReply::Bool(value) => Ok(value),
        )
    }

    pub(super) fn decode_runtime_inspector_protocol_messages_page_reply(
        reply: RendererPageReply,
        operation: &str,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        expect_page_reply!(
            reply,
            operation,
            "runtime inspector protocol messages",
            RendererPageReply::RuntimeInspectorProtocolMessages(messages) => {
                Ok(messages.into_messages())
            },
        )
    }

    pub(super) fn page_reply_kind(reply: &RendererPageReply) -> &'static str {
        match reply {
            #[cfg(feature = "devtools")]
            RendererPageReply::DevTools(reply) => super::devtools::devtools_page_reply_kind(reply),
            RendererPageReply::RuntimeEvaluationResult(_) => "a runtime evaluation result reply",
            #[cfg(feature = "devtools")]
            RendererPageReply::RuntimeHeapUsage(_) => "runtime heap usage",
            RendererPageReply::ElementClickPreparation(_) => "an element click preparation reply",
            RendererPageReply::Bool(_) => "a bool reply",
            #[cfg(feature = "ace-protocol")]
            RendererPageReply::AcePageTask(_) => "AcePageTask",
            RendererPageReply::RuntimeInspectorProtocolMessages(_) => {
                "runtime inspector protocol messages"
            }
            RendererPageReply::RuntimeConsoleMessageSnapshots(_) => "runtime console snapshots",
            RendererPageReply::RuntimeRealmInventory(_) => "runtime realm inventory",
            RendererPageReply::ExecutionContextIds(_) => "execution context replies",
            RendererPageReply::OptionalExecutionContextId(_) => {
                "an optional execution context reply"
            }
            RendererPageReply::DocumentNodeRequested { .. } => "a requested document node reply",
            RendererPageReply::OptionalDocumentNodeObjectSnapshot(_) => {
                "an optional document node object snapshot"
            }
            RendererPageReply::OptionalDocumentChildNodeSnapshots(_) => {
                "optional document child node snapshots"
            }
            RendererPageReply::OptionalDocumentChildNodeSnapshotEvents(_) => {
                "optional document child node snapshot events"
            }
            RendererPageReply::OptionalAccessibilityNodes(_) => "optional accessibility nodes",
            RendererPageReply::DocumentQuerySelectorNode(_) => "a document query selector node",
            RendererPageReply::RuntimeRemoteObjectResolution(_) => {
                "a runtime remote object resolution reply"
            }
            RendererPageReply::BlobUuid(_) => "a Blob UUID reply",
            RendererPageReply::OptionalBlobBytes(_) => "optional Blob bytes",
            RendererPageReply::OptionalDocumentNodeReference(_) => {
                "an optional document node reference reply"
            }
            RendererPageReply::OptionalClientRect(_) => "a client rect reply",
            RendererPageReply::DocumentStorageKey(_) => "a document storage key reply",
            RendererPageReply::AppManifestLoadPreparation(_) => "an app manifest load preparation",
            RendererPageReply::ChildFrameTreeSnapshots(_) => "child frame tree snapshots",
            RendererPageReply::OptionalString(_) => "an optional string reply",
            RendererPageReply::SerializedDocument { .. } => "a serialized document reply",
            RendererPageReply::OptionalU64(_) => "an optional u64 reply",
            RendererPageReply::Usize(_) => "a usize reply",
            RendererPageReply::SetDocumentContentResult(_) => "a set-document-content result reply",
            RendererPageReply::DocumentStartScriptResult(_) => "a document-start result reply",
            RendererPageReply::PendingSubresourceContinueOutcome(_) => "a continue outcome reply",
            RendererPageReply::PageDiagnosticsSnapshot(_) => "a page diagnostics snapshot reply",
            RendererPageReply::CookieFacadeSnapshot(_) => "a cookie facade snapshot reply",
            RendererPageReply::LayoutMetrics(_) => "a layout metrics reply",
            RendererPageReply::PrintToPdf(_) => "a print to PDF reply",
            RendererPageReply::CaptureScreenshot(_) => "a capture screenshot reply",
            RendererPageReply::Unit => "a unit reply",
            // Backend capabilities may be enabled by another dependency while
            // the corresponding facade feature remains disabled.
            _ => "an unknown renderer reply",
        }
    }

    pub(super) fn unexpected_page_reply<T>(
        operation: &str,
        expected: &str,
        reply: RendererPageReply,
    ) -> Result<T> {
        bail!(
            "{operation} expected {expected}, got {}",
            Self::page_reply_kind(&reply)
        );
    }
}

impl PendingPageCommand {
    pub async fn wait(self) -> Result<CompletedPageCommand> {
        let mut output = self.pending.wait().await?;
        if let Some(attachment_id) = self.renderer_agent_attachment_id {
            output.bind_renderer_agent_attachment(attachment_id);
        }
        Ok(CompletedPageCommand {
            output,
            renderer_agent_attachment_id: self.renderer_agent_attachment_id,
        })
    }
}
