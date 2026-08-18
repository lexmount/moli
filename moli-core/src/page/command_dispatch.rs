use super::{
    Page, RendererAgentAttachmentId, RendererCommandTurnOutput, RendererPageCommand,
    RendererPageCommandPending, RendererPageReply, RendererRuntimeInspectorIoCommandClaim,
    RendererRuntimeInspectorIoCommandRoute, RendererRuntimeInspectorMainCommandCompletion,
    RendererRuntimeInspectorMainCommandRoute, RendererRuntimeInspectorMessage,
};
use crate::RendererOutputFence;
use anyhow::{Result, bail};

pub struct PendingPageCommand {
    pending: RendererPageCommandPending,
    renderer_agent_attachment_id: Option<RendererAgentAttachmentId>,
}

pub struct PendingRuntimeInspectorCommandDispatch {
    kind: PendingRuntimeInspectorCommandDispatchKind,
}

/// One non-V8 renderer agent command admitted through the Page IO
/// `DevToolsSession` receiver.
pub struct PendingDevToolsIoCommandDispatch {
    route: RendererRuntimeInspectorIoCommandRoute,
}

enum PendingRuntimeInspectorCommandDispatchKind {
    MainIngress(Box<RendererRuntimeInspectorMainCommandRoute>),
    Io(PendingDevToolsIoCommandDispatch),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletedDevToolsIoCommandDispatch {
    Dispatched,
    Canceled,
}

/// Identifies which execution owner consumed one routable Inspector command.
///
/// `Inspector` claims complete either in V8's interrupt callback or its nested
/// message loop while the Page actor is blocked. Callers must not synchronously
/// re-enter that Page before exposing the Inspector response.
pub enum CompletedRuntimeInspectorCommandDispatch {
    Owner(Box<CompletedPageCommand>),
    Inspector,
    Canceled,
}

pub struct CompletedPageCommand {
    output: RendererCommandTurnOutput,
    renderer_agent_attachment_id: Option<RendererAgentAttachmentId>,
}

impl CompletedPageCommand {
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
}

impl Page {
    pub(crate) fn start_page_command(
        &self,
        mut command: RendererPageCommand,
    ) -> Result<PendingPageCommand> {
        if let Some(attachment_id) = self.renderer_agent_attachment_id {
            command.bind_inspector_attachment(attachment_id);
        }
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
        Ok(PendingPageCommand {
            pending,
            renderer_agent_attachment_id: self.renderer_agent_attachment_id,
        })
    }

    fn start_full_page_command(
        &self,
        mut command: RendererPageCommand,
    ) -> Result<PendingPageCommand> {
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

    fn pending_runtime_inspector_command_dispatch(
        kind: PendingRuntimeInspectorCommandDispatchKind,
    ) -> PendingRuntimeInspectorCommandDispatch {
        PendingRuntimeInspectorCommandDispatch { kind }
    }

    pub(crate) fn pending_io_runtime_inspector_command_dispatch(
        route: RendererRuntimeInspectorIoCommandRoute,
    ) -> PendingRuntimeInspectorCommandDispatch {
        Self::pending_runtime_inspector_command_dispatch(
            PendingRuntimeInspectorCommandDispatchKind::Io(
                Self::pending_devtools_io_command_dispatch(route),
            ),
        )
    }

    pub(crate) fn pending_devtools_io_command_dispatch(
        route: RendererRuntimeInspectorIoCommandRoute,
    ) -> PendingDevToolsIoCommandDispatch {
        PendingDevToolsIoCommandDispatch { route }
    }

    pub(crate) fn pending_main_ingress_runtime_inspector_command_dispatch(
        route: RendererRuntimeInspectorMainCommandRoute,
    ) -> PendingRuntimeInspectorCommandDispatch {
        Self::pending_runtime_inspector_command_dispatch(
            PendingRuntimeInspectorCommandDispatchKind::MainIngress(Box::new(route)),
        )
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
            RendererPageReply::RuntimeEvaluationResult(_) => "a runtime evaluation result reply",
            RendererPageReply::Bool(_) => "a bool reply",
            RendererPageReply::OptionalBool(_) => "an optional bool reply",
            RendererPageReply::InputDispatchOutcome(_) => "an input dispatch outcome reply",
            RendererPageReply::RuntimeInspectorProtocolMessages(_) => {
                "runtime inspector protocol messages"
            }
            RendererPageReply::RuntimeConsoleMessageSnapshots(_) => "runtime console snapshots",
            RendererPageReply::RuntimeHeapUsage(_) => "runtime heap usage",
            RendererPageReply::PerformanceMetricSnapshot(_) => "performance metric snapshot",
            RendererPageReply::DomDebuggerEventListeners(_) => {
                "a DOMDebugger event listeners resolution"
            }
            RendererPageReply::DomDebuggerDomBreakpoint(_) => {
                "a DOMDebugger DOM breakpoint resolution"
            }
            RendererPageReply::RuntimeRealmInventory(_) => "runtime realm inventory",
            RendererPageReply::ExecutionContextId(_) => "an execution context reply",
            RendererPageReply::ExecutionContextIds(_) => "execution context replies",
            RendererPageReply::OptionalExecutionContextId(_) => {
                "an optional execution context reply"
            }
            RendererPageReply::OptionalDocumentNodeObjectSnapshot(_) => {
                "an optional document node object snapshot"
            }
            RendererPageReply::OptionalDomSnapshotCapturePayload(_) => {
                "an optional DOMSnapshot capture payload"
            }
            RendererPageReply::OptionalDocumentChildNodeSnapshots(_) => {
                "optional document child node snapshots"
            }
            RendererPageReply::OptionalDocumentChildNodeSnapshotEvents(_) => {
                "optional document child node snapshot events"
            }
            RendererPageReply::DocumentNodeAttributesResolution(_) => {
                "a document node attributes resolution"
            }
            RendererPageReply::DocumentNodeTextResolution(_) => "a document node text resolution",
            RendererPageReply::DocumentNodePropertyResolution(_) => {
                "a document node property resolution"
            }
            RendererPageReply::OptionalAccessibilityPayloads(_) => {
                "optional accessibility payloads"
            }
            RendererPageReply::OptionalAccessibilityPayload(_) => {
                "an optional accessibility payload"
            }
            RendererPageReply::OptionalAccessibilityPayloadsForObjectId(_) => {
                "optional accessibility payloads for object id"
            }
            RendererPageReply::DocumentQuerySelectorResolution(_) => {
                "a document query selector resolution"
            }
            RendererPageReply::DocumentQuerySelectorNode(_) => "a document query selector node",
            RendererPageReply::DocumentQuerySelectorWithChildNodeSnapshotEvents(_) => {
                "a document query selector with child node snapshot events reply"
            }
            RendererPageReply::DocumentPerformSearch(_) => {
                "a document perform search registration reply"
            }
            RendererPageReply::DocumentSearchResults(_) => "a document search results reply",
            RendererPageReply::DocumentSearchResultsDiscarded => {
                "a document search results discarded reply"
            }
            RendererPageReply::DocumentNodeStackTracesEnabled => {
                "a document node stack traces enabled reply"
            }
            RendererPageReply::DocumentNodeStackTrace(_) => "a document node stack trace reply",
            RendererPageReply::DocumentFrontendNodeBinding(_) => {
                "a document frontend node binding reply"
            }
            RendererPageReply::DocumentBidiNodeBinding(_) => "a document BiDi node binding reply",
            RendererPageReply::DocumentBidiNodeSharedId(_) => {
                "a document BiDi node shared id reply"
            }
            RendererPageReply::DocumentBidiNodeBindingRegistered => {
                "a document BiDi node binding registered reply"
            }
            RendererPageReply::OptionalStyleSheetPayload(_) => "an optional stylesheet payload",
            RendererPageReply::StyleSheetInventory(_) => "stylesheet inventory update",
            RendererPageReply::RuntimeRemoteObjectResolution(_) => {
                "a runtime remote object resolution reply"
            }
            RendererPageReply::BlobUuid(_) => "a Blob UUID reply",
            RendererPageReply::OptionalBlobBytes(_) => "optional Blob bytes",
            RendererPageReply::DocumentFrontendNodeIds(_) => {
                "a document frontend node ids resolution reply"
            }
            RendererPageReply::OptionalDocumentNodeReference(_) => {
                "an optional document node reference reply"
            }
            RendererPageReply::OptionalClientRect(_) => "a client rect reply",
            RendererPageReply::OptionalDocumentNodeClientRect(_) => {
                "an optional document node client rect reply"
            }
            RendererPageReply::OptionalDocumentNodeGeometry(_) => {
                "an optional document node geometry reply"
            }
            RendererPageReply::OptionalDocumentHitTest(_) => "an optional document hit-test reply",
            RendererPageReply::ScrollIntoViewResult(_) => "a scroll-into-view reply",
            RendererPageReply::ComputedStyleProperties(_) => "computed style properties",
            RendererPageReply::DomAttributeMutationOutcome(_) => {
                "a DOM attribute mutation outcome reply"
            }
            RendererPageReply::DomEditOutcome(_) => "a DOM edit outcome reply",
            RendererPageReply::DomFocusOutcome(_) => "a DOM focus outcome reply",
            RendererPageReply::AutofillTriggerOutcome(_) => "an Autofill trigger outcome reply",
            RendererPageReply::DocumentStorageKey(_) => "a document storage key reply",
            RendererPageReply::ResourceTextSearchOutcome(_) => {
                "a resource text search outcome reply"
            }
            RendererPageReply::NetworkResourceLoadPreparation(_) => {
                "a network resource load preparation"
            }
            RendererPageReply::AppManifestLoadPreparation(_) => "an app manifest load preparation",
            RendererPageReply::ChildFrameTreeSnapshots(_) => "child frame tree snapshots",
            RendererPageReply::OptionalString(_) => "an optional string reply",
            RendererPageReply::OptionalU64(_) => "an optional u64 reply",
            RendererPageReply::Usize(_) => "a usize reply",
            RendererPageReply::SetDocumentContentResult(_) => "a set-document-content result reply",
            RendererPageReply::DocumentStartScriptResult(_) => "a document-start result reply",
            RendererPageReply::PendingSubresourceContinueOutcome(_) => "a continue outcome reply",
            RendererPageReply::PageDiagnosticsSnapshot(_) => "a page diagnostics snapshot reply",
            RendererPageReply::CookieFacadeSnapshot(_) => "a cookie facade snapshot reply",
            RendererPageReply::LayoutMetrics(_) => "a layout metrics reply",
            RendererPageReply::CaptureScreenshot(_) => "a capture screenshot reply",
            RendererPageReply::Unit => "a unit reply",
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

impl PendingRuntimeInspectorCommandDispatch {
    pub async fn wait(self) -> Result<CompletedRuntimeInspectorCommandDispatch> {
        match self.kind {
            PendingRuntimeInspectorCommandDispatchKind::MainIngress(route) => {
                let renderer_agent_attachment_id = route.ticket().attachment();
                match route.wait_for_completion().await? {
                    RendererRuntimeInspectorMainCommandCompletion::Owner(mut output) => {
                        if let Some(attachment_id) = renderer_agent_attachment_id {
                            output.bind_renderer_agent_attachment(attachment_id);
                        }
                        Ok(CompletedRuntimeInspectorCommandDispatch::Owner(Box::new(
                            CompletedPageCommand {
                                output: *output,
                                renderer_agent_attachment_id,
                            },
                        )))
                    }
                    RendererRuntimeInspectorMainCommandCompletion::Inspector => {
                        Ok(CompletedRuntimeInspectorCommandDispatch::Inspector)
                    }
                    RendererRuntimeInspectorMainCommandCompletion::Page(_) => Err(anyhow::anyhow!(
                        "a Runtime Inspector command entered nested non-V8 Page dispatch"
                    )),
                    RendererRuntimeInspectorMainCommandCompletion::Canceled => {
                        Ok(CompletedRuntimeInspectorCommandDispatch::Canceled)
                    }
                }
            }
            PendingRuntimeInspectorCommandDispatchKind::Io(pending) => {
                match pending.wait().await? {
                    CompletedDevToolsIoCommandDispatch::Dispatched => {
                        Ok(CompletedRuntimeInspectorCommandDispatch::Inspector)
                    }
                    CompletedDevToolsIoCommandDispatch::Canceled => {
                        Ok(CompletedRuntimeInspectorCommandDispatch::Canceled)
                    }
                }
            }
        }
    }
}

impl PendingDevToolsIoCommandDispatch {
    pub async fn wait(self) -> Result<CompletedDevToolsIoCommandDispatch> {
        match self
            .route
            .wait_for_first_dispatch()
            .await
            .map_err(anyhow::Error::msg)?
        {
            RendererRuntimeInspectorIoCommandClaim::Dispatched => {
                Ok(CompletedDevToolsIoCommandDispatch::Dispatched)
            }
            RendererRuntimeInspectorIoCommandClaim::Canceled => {
                Ok(CompletedDevToolsIoCommandDispatch::Canceled)
            }
        }
    }
}
