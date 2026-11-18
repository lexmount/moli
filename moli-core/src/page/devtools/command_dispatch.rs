use crate::RendererOutputFence;
use crate::page::Page;
use crate::page::RendererAgentAttachmentId;
use crate::page::RendererPageCommand;
use crate::page::RendererPageReply;
use crate::page::RendererRuntimeInspectorIoCommandClaim;
use crate::page::RendererRuntimeInspectorIoCommandRoute;
use crate::page::RendererRuntimeInspectorMainCommandCompletion;
use crate::page::RendererRuntimeInspectorMainCommandRoute;
use crate::page::command_dispatch::{
    CompletedPageCommand, PendingPageCommand, UnexpectedDocumentCommandReply,
};
use crate::{RendererDevToolsPageCommand, RendererDevToolsPageReply};
use anyhow::Result;

/// The original Page/DevToolsSession admission, held without a Page borrow
/// while Browser work runs. Dispatch never resolves a replacement active Page.
pub struct PreparedNativeCall {
    target: moli_renderer_v8::RendererPageProtocolCommandTarget,
    attachment: RendererAgentAttachmentId,
    session: Option<String>,
    response: moli_renderer_v8::RendererNativeCommandAdmission,
}
impl PreparedNativeCall {
    pub fn start(self, operation: crate::RendererNativeOperation) -> Result<PendingPageCommand> {
        let mut command = RendererPageCommand::DevTools(RendererDevToolsPageCommand::Native(
            self.response.bind(operation),
        ));
        command.bind_inspector_attachment(self.attachment);
        let pending = self.target.enqueue(command, self.session)?;
        Ok(PendingPageCommand {
            pending,
            renderer_agent_attachment_id: Some(self.attachment),
        })
    }
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompletedDevToolsIoCommandDispatch {
    Dispatched,
    SessionResponse {
        predecessor: RendererOutputFence,
        response_succeeded: bool,
    },
}
/// Identifies which execution owner consumed one routable Inspector command.
///
/// `Inspector` claims complete either in V8's interrupt callback or its nested
/// message loop while the Page actor is blocked. Callers must not synchronously
/// re-enter that Page before exposing the Inspector response.
pub enum CompletedRuntimeInspectorCommandDispatch {
    Owner(Box<CompletedPageCommand>),
    OwnerSessionResponse {
        completion: Box<CompletedPageCommand>,
        response_succeeded: bool,
    },
    OwnerSessionErrorSettled(RendererOutputFence),
    Inspector,
    InspectorSessionResponse {
        predecessor: RendererOutputFence,
        response_succeeded: bool,
    },
}
impl CompletedRuntimeInspectorCommandDispatch {
    pub fn renderer_output_predecessor(&self) -> Option<RendererOutputFence> {
        match self {
            Self::Owner(completion) | Self::OwnerSessionResponse { completion, .. } => {
                completion.renderer_output_predecessor()
            }
            Self::OwnerSessionErrorSettled(predecessor)
            | Self::InspectorSessionResponse { predecessor, .. } => Some(predecessor.clone()),
            Self::Inspector => None,
        }
    }
}
impl CompletedPageCommand {
    pub fn into_document_bidi_node_shared_id(
        self,
    ) -> Result<crate::page::RendererDomBidiNodeSharedIdResolution> {
        match self.output.into_reply_and_state().0 {
            RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentBidiNodeSharedId(
                result,
            )) => Ok(result),
            _ => Err(UnexpectedDocumentCommandReply.into()),
        }
    }

    pub fn into_document_bidi_node_binding(self) -> Result<()> {
        match self.output.into_reply_and_state().0 {
            RendererPageReply::DevTools(
                RendererDevToolsPageReply::DocumentBidiNodeBindingRegistered,
            ) => Ok(()),
            _ => Err(UnexpectedDocumentCommandReply.into()),
        }
    }
}
impl Page {
    pub fn prepare_cdp_call(
        &self,
        command_id: u64,
        frontend_session_id: Option<&str>,
    ) -> Result<(
        PreparedNativeCall,
        crate::RendererNativeCommandResponseGuard,
    )> {
        let attachment = self.renderer_agent_attachment_id.ok_or_else(|| {
            anyhow::anyhow!("native frontend command requires a renderer attachment")
        })?;
        let (response, guard) = crate::RendererCdpCall::reserve(
            moli_page_types::FrontendCommandId::new(command_id),
            moli_page_types::DevToolsSessionKey::from_wire_session_id(frontend_session_id),
            attachment,
        );
        Ok((
            PreparedNativeCall {
                target: self.handle.protocol_command_target(),
                attachment,
                session: self.renderer_devtools_command_session_id.clone(),
                response,
            },
            guard,
        ))
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
                    RendererRuntimeInspectorMainCommandCompletion::InspectorSessionResponse {
                        predecessor,
                        response_succeeded,
                    } => Ok(
                        CompletedRuntimeInspectorCommandDispatch::InspectorSessionResponse {
                            predecessor,
                            response_succeeded,
                        },
                    ),
                    RendererRuntimeInspectorMainCommandCompletion::OwnerSessionResponse {
                        mut output,
                        response_succeeded,
                    } => {
                        if let Some(attachment_id) = renderer_agent_attachment_id {
                            output.bind_renderer_agent_attachment(attachment_id);
                        }
                        Ok(
                            CompletedRuntimeInspectorCommandDispatch::OwnerSessionResponse {
                                completion: Box::new(CompletedPageCommand {
                                    output: *output,
                                    renderer_agent_attachment_id,
                                }),
                                response_succeeded,
                            },
                        )
                    }
                    RendererRuntimeInspectorMainCommandCompletion::OwnerSessionErrorSettled(
                        predecessor,
                    ) => Ok(
                        CompletedRuntimeInspectorCommandDispatch::OwnerSessionErrorSettled(
                            predecessor,
                        ),
                    ),
                    RendererRuntimeInspectorMainCommandCompletion::Page(_) => Err(anyhow::anyhow!(
                        "a Runtime Inspector command entered nested non-V8 Page dispatch"
                    )),
                    RendererRuntimeInspectorMainCommandCompletion::Canceled(message) => {
                        Err(anyhow::anyhow!(message))
                    }
                }
            }
            PendingRuntimeInspectorCommandDispatchKind::Io(pending) => {
                match pending.wait().await? {
                    CompletedDevToolsIoCommandDispatch::Dispatched => {
                        Ok(CompletedRuntimeInspectorCommandDispatch::Inspector)
                    }
                    CompletedDevToolsIoCommandDispatch::SessionResponse {
                        predecessor,
                        response_succeeded,
                    } => Ok(
                        CompletedRuntimeInspectorCommandDispatch::InspectorSessionResponse {
                            predecessor,
                            response_succeeded,
                        },
                    ),
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
            RendererRuntimeInspectorIoCommandClaim::SessionResponse {
                predecessor,
                response_succeeded,
            } => Ok(CompletedDevToolsIoCommandDispatch::SessionResponse {
                predecessor,
                response_succeeded,
            }),
            RendererRuntimeInspectorIoCommandClaim::Canceled(message) => {
                Err(anyhow::anyhow!(message))
            }
        }
    }
}

pub(in crate::page) fn devtools_page_reply_kind(reply: &RendererDevToolsPageReply) -> &'static str {
    match reply {
        RendererDevToolsPageReply::WebMcp(_) => "a WebMCP command result",
        RendererDevToolsPageReply::ElementClickDispatch(_) => "an element click dispatch reply",
        RendererDevToolsPageReply::OptionalBool(_) => "an optional bool reply",
        RendererDevToolsPageReply::InputDispatchOutcome(_) => "an input dispatch outcome reply",
        RendererDevToolsPageReply::PerformanceMetricSnapshot(_) => "performance metric snapshot",
        RendererDevToolsPageReply::DomDebuggerEventListeners(_) => {
            "a DOMDebugger event listeners resolution"
        }
        RendererDevToolsPageReply::DomDebuggerDomBreakpoint(_) => {
            "a DOMDebugger DOM breakpoint resolution"
        }
        RendererDevToolsPageReply::ExecutionContextId(_) => "an execution context reply",
        RendererDevToolsPageReply::OptionalDomSnapshotCapturePayload(_) => {
            "an optional DOMSnapshot capture payload"
        }
        RendererDevToolsPageReply::DocumentNodeAttributesResolution(_) => {
            "a document node attributes resolution"
        }
        RendererDevToolsPageReply::DocumentNodeTextResolution(_) => {
            "a document node text resolution"
        }
        RendererDevToolsPageReply::DocumentNodePropertyResolution(_) => {
            "a document node property resolution"
        }
        RendererDevToolsPageReply::OptionalAccessibilityPayloads(_) => {
            "optional accessibility payloads"
        }
        RendererDevToolsPageReply::OptionalAccessibilityPayload(_) => {
            "an optional accessibility payload"
        }
        RendererDevToolsPageReply::OptionalAccessibilityPayloadsForObjectId(_) => {
            "optional accessibility payloads for object id"
        }
        RendererDevToolsPageReply::DocumentQuerySelectorResolution(_) => {
            "a document query selector resolution"
        }
        RendererDevToolsPageReply::DocumentQuerySelectorWithChildNodeSnapshotEvents(_) => {
            "a document query selector with child node snapshot events reply"
        }
        RendererDevToolsPageReply::DocumentPerformSearch(_) => {
            "a document perform search registration reply"
        }
        RendererDevToolsPageReply::DocumentSearchResults { .. } => {
            "a document search results reply"
        }
        RendererDevToolsPageReply::DocumentSearchResultsDiscarded => {
            "a document search results discarded reply"
        }
        RendererDevToolsPageReply::DocumentNodeStackTracesEnabled => {
            "a document node stack traces enabled reply"
        }
        RendererDevToolsPageReply::DocumentNodeStackTrace(_) => "a document node stack trace reply",
        RendererDevToolsPageReply::DocumentFrontendNodeBinding(_) => {
            "a document frontend node binding reply"
        }
        RendererDevToolsPageReply::DocumentBidiNodeBinding(_) => {
            "a document BiDi node binding reply"
        }
        RendererDevToolsPageReply::DocumentBidiNodeSharedId(_) => {
            "a document BiDi node shared id reply"
        }
        RendererDevToolsPageReply::DocumentBidiNodeBindingRegistered => {
            "a document BiDi node binding registered reply"
        }
        RendererDevToolsPageReply::OptionalStyleSheetPayload(_) => "an optional stylesheet payload",
        RendererDevToolsPageReply::StyleSheetInventory(_) => "stylesheet inventory update",
        RendererDevToolsPageReply::DocumentFrontendNodeIds(_) => {
            "a document frontend node ids resolution reply"
        }
        RendererDevToolsPageReply::OptionalDocumentNodeClientRect(_) => {
            "an optional document node client rect reply"
        }
        RendererDevToolsPageReply::OptionalDocumentNodeGeometry(_) => {
            "an optional document node geometry reply"
        }
        RendererDevToolsPageReply::OptionalDocumentHitTest(_) => {
            "an optional document hit-test reply"
        }
        RendererDevToolsPageReply::ScrollIntoViewResult(_) => "a scroll-into-view reply",
        RendererDevToolsPageReply::ComputedStyleProperties(_) => "computed style properties",
        RendererDevToolsPageReply::DomAttributeMutationOutcome(_) => {
            "a DOM attribute mutation outcome reply"
        }
        RendererDevToolsPageReply::DomEditOutcome(_) => "a DOM edit outcome reply",
        RendererDevToolsPageReply::DomFocusOutcome(_) => "a DOM focus outcome reply",
        RendererDevToolsPageReply::AutofillTriggerOutcome(_) => "an Autofill trigger outcome reply",
        RendererDevToolsPageReply::ResourceTextSearchOutcome(_) => {
            "a resource text search outcome reply"
        }
        RendererDevToolsPageReply::NetworkResourceLoadPreparation(_) => {
            "a network resource load preparation"
        }
        RendererDevToolsPageReply::CaptureScreencastFrame(_) => "a capture screencast frame reply",
        RendererDevToolsPageReply::NativeCommandReady(_) => "an unsettled native frontend reply",
        RendererDevToolsPageReply::NativeCommandPublished => "a published native frontend reply",
        _ => "an unknown renderer DevTools reply",
    }
}
