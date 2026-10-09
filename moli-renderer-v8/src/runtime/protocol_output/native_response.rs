//! Ready native replies share the renderer's producer-ordered output journal.
//!
//! The adapter chooses terminal delivery at admission and freezes Browser
//! metadata here. Internal queries continue to return their typed values to
//! their continuation; they never acquire this frontend publication capability.

use std::convert::Infallible;

use moli_page_types::{DevToolsSessionKey, FrontendCommandId, RendererAgentAttachmentId};

use super::{
    PendingRendererOutputRecord, RendererCommandResponseAuthority, RendererCommandResponseLease,
    RendererOutputFence, RendererOutputItem, RendererTurnOutputJournal,
};
use crate::runtime::{RendererPageCommand, RendererPageReply};

/// A resolved native frontend reply. The projection captures only immutable
/// protocol metadata at admission; it cannot borrow a Browser or live Page.
/// Both success and backend errors are published at the renderer boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct RendererNativeProtocolResponse {
    pub result: Result<serde_json::Value, RendererNativeProtocolError>,
    pub notifications: Vec<RendererNativeProtocolNotification>,
    pub state_updates: Vec<RendererNativeProtocolStateUpdate>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererNativeProtocolError {
    pub code: i32,
    pub message: String,
}

/// Native notifications keep the semantic sidecars needed by non-CDP
/// observers. JSON-only events and DOM automation events share one ordered
/// prefix; neither can be reconstructed from mutable Page state at drain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererNativeProtocolNotification {
    Json(serde_json::Value),
    DomSetChildNodes {
        parent_node_id: u32,
        nodes: Vec<serde_json::Value>,
    },
}

impl From<serde_json::Value> for RendererNativeProtocolNotification {
    fn from(value: serde_json::Value) -> Self {
        Self::Json(value)
    }
}

impl RendererNativeProtocolResponse {
    pub fn success(result: serde_json::Value) -> Self {
        Self {
            result: Ok(result),
            notifications: Vec::new(),
            state_updates: Vec::new(),
        }
    }

    pub fn error(code: i32, message: impl Into<String>) -> Self {
        Self {
            result: Err(RendererNativeProtocolError {
                code,
                message: message.into(),
            }),
            notifications: Vec::new(),
            state_updates: Vec::new(),
        }
    }
}

/// Browser mirrors committed with a native terminal. These are frozen data,
/// never callbacks into a live Page. Attachment retirement discards document
/// updates; command-local replay can instead fail the terminal at projection.
#[derive(Clone, Debug, PartialEq)]
pub enum RendererNativeProtocolStateUpdate {
    /// Frozen V8 replay and state, projected by the ordered protocol consumer
    /// before this terminal. Retirement can still turn enable into an error;
    /// neither the adapter waiter nor a replacement attachment owns that choice.
    RuntimeSubscription {
        enabled: bool,
        output: Box<crate::runtime::RendererRuntimeCommandOutput>,
    },
    WebMcpEnabled(bool),
    /// Register object IDs from the successful terminal result before delivery.
    /// The reply already owns the payload; this update only retains its group.
    RemoteObjects {
        object_group: Option<String>,
    },
    DomRemoteObjectNode {
        object_id: String,
        node: serde_json::Value,
    },
}

type NativeResponseProjection = Box<
    dyn FnOnce(
            anyhow::Result<RendererPageReply>,
            &crate::runtime::RendererPageState,
        ) -> RendererNativeProtocolResponse
        + Send,
>;

/// A complete reply to the original frontend call, not a live Page observation.
/// Its metadata remains valid even if the Page is replaced before projection.
/// The frontend router uses its globally allocated call id to deliver it once.
#[derive(Clone, Debug, PartialEq)]
pub struct RendererNativeCommandTerminal {
    pub command_id: FrontendCommandId,
    pub session: DevToolsSessionKey,
    pub attachment_id: RendererAgentAttachmentId,
    pub reply: RendererNativeProtocolResponse,
}

type Authority = RendererCommandResponseAuthority<Infallible, RendererOutputFence>;
type Lease = RendererCommandResponseLease<Infallible, RendererOutputFence>;

/// Kept by the adapter continuation until its native operation settles.
/// Dropping a pending continuation cancels publication. Dropping it after the
/// producer claimed completion cannot undo the already committed terminal.
pub struct RendererNativeCommandResponseGuard {
    authority: Authority,
    publication: Option<tokio::sync::oneshot::Receiver<RendererOutputFence>>,
}

impl RendererNativeCommandResponseGuard {
    /// After losing the adapter result, cancel only work that has not claimed
    /// completion. If publication won that race, return its exact fence so the
    /// adapter cannot generate a second terminal error. This never delays the
    /// renderer or allocates a journal slot for unfinished work.
    pub async fn cancel_or_published(mut self) -> Option<RendererOutputFence> {
        self.authority.cancel();
        self.publication
            .take()
            .expect("native publication receipt is consumed once")
            .await
            .ok()
    }
}

impl Drop for RendererNativeCommandResponseGuard {
    fn drop(&mut self) {
        self.authority.cancel();
    }
}

/// Native frontend operations use their ordinary backend dispatch. The
/// synchronous projection freezes the complete response before it enters
/// the common journal, without involving an adapter waiter.
pub struct RendererCdpCall {
    pub(crate) operation: RendererNativeOperation,
    response: RendererNativeCommandResponse,
}

struct RendererNativeCommandResponse {
    command_id: FrontendCommandId,
    session: DevToolsSessionKey,
    attachment_id: RendererAgentAttachmentId,
    lease: Lease,
}

impl RendererCdpCall {
    pub fn new(
        command_id: FrontendCommandId,
        session: DevToolsSessionKey,
        attachment_id: RendererAgentAttachmentId,
        operation: RendererNativeOperation,
    ) -> (Self, RendererNativeCommandResponseGuard) {
        let authority = Authority::session();
        let lease = authority
            .activate()
            .expect("new native response authority is open");
        let publication = lease
            .take_session_response_settlement_receiver()
            .expect("native frontend calls retain a publication receipt");
        (
            Self {
                operation,
                response: RendererNativeCommandResponse {
                    command_id,
                    session,
                    attachment_id,
                    lease,
                },
            },
            RendererNativeCommandResponseGuard {
                authority,
                publication: Some(publication),
            },
        )
    }
}

/// Created by the native backend with fully resolved payload, then published
/// by command-turn settlement after earlier facts have reached the journal.
/// Creating this value neither reserves a sequence nor blocks later commands.
pub struct RendererNativeCommandReadyResponse {
    response: RendererNativeCommandResponse,
    reply: NativeReadyReply,
    journal: RendererTurnOutputJournal,
}

impl RendererNativeCommandReadyResponse {
    pub(crate) fn publish(
        self,
        page_state: &crate::runtime::RendererPageState,
    ) -> anyhow::Result<RendererOutputFence> {
        let Self {
            response,
            reply,
            journal,
        } = self;
        let receipt = response
            .lease
            .claim_session()
            .map_err(|()| anyhow::anyhow!("native response was canceled"))?
            .expect("native frontend calls retain a publication receipt");
        // Keep the backend reply alive through owner lifecycle settlement.
        // In particular input navigation is reconciled before projection.
        let reply = match reply {
            NativeReadyReply::Resolved(reply) => reply,
            NativeReadyReply::Projected { reply, project } => project(reply, page_state),
        };
        let terminal = RendererNativeCommandTerminal {
            command_id: response.command_id,
            session: response.session,
            attachment_id: response.attachment_id,
            reply,
        };
        let fence = journal
            .try_publish_terminal_records_and_declare_fence([
                PendingRendererOutputRecord::from_parts(
                    None,
                    RendererOutputItem::NativeTerminal(terminal),
                ),
            ])
            .ok_or_else(|| anyhow::anyhow!("native response output stream closed"))?;
        let _ = receipt.send(fence.clone());
        Ok(fence)
    }
}

/// A bounded native handler. Lookups may select the next backend operation
/// inside the same renderer turn; only the terminal projection can publish a
/// frontend response. These continuations never await Browser state or I/O.
pub struct RendererNativeOperation {
    // Breaks the Native call -> operation -> page command type cycle.
    pub(crate) command: Box<RendererPageCommand>,
    completion: NativeOperationCompletion,
    nested_main: bool,
}

pub enum RendererNativeOperationStep {
    Continue(RendererNativeOperation),
    Complete(RendererNativeProtocolResponse),
}

enum NativeOperationCompletion {
    Terminal(NativeResponseProjection),
    Lookup(
        Box<dyn FnOnce(anyhow::Result<RendererPageReply>) -> RendererNativeOperationStep + Send>,
    ),
}

impl RendererNativeOperation {
    pub fn new(
        command: RendererPageCommand,
        project: impl FnOnce(anyhow::Result<RendererPageReply>) -> RendererNativeProtocolResponse
        + Send
        + 'static,
    ) -> Self {
        Self::with_page_state(command, move |reply, _state| project(reply))
    }

    /// A terminal may use the immutable state captured by this renderer turn.
    /// This avoids consulting a lagging Browser Page mirror, and ensures the
    /// reply and its earlier observations describe the same completed turn.
    pub fn with_page_state(
        command: RendererPageCommand,
        project: impl FnOnce(
            anyhow::Result<RendererPageReply>,
            &crate::runtime::RendererPageState,
        ) -> RendererNativeProtocolResponse
        + Send
        + 'static,
    ) -> Self {
        let nested_main = command.nested_dispatch()
            == crate::devtools::command::RendererDevToolsMainNestedDispatch::PageAgent;
        Self {
            nested_main,
            command: Box::new(command),
            completion: NativeOperationCompletion::Terminal(Box::new(project)),
        }
    }

    /// An opaque continuation may enter the isolate. Keep the entire chain
    /// on the owner turn unless its author explicitly supplies a native-only
    /// continuation with `then_on_nested_main`.
    pub fn then(
        command: RendererPageCommand,
        next: impl FnOnce(anyhow::Result<RendererPageReply>) -> RendererNativeOperationStep
        + Send
        + 'static,
    ) -> Self {
        Self {
            command: Box::new(command),
            completion: NativeOperationCompletion::Lookup(Box::new(next)),
            nested_main: false,
        }
    }

    /// The lookup and every continuation use the existing nested Page-agent
    /// boundary. They must not acquire an owner-only Inspector/isolate borrow.
    pub fn then_on_nested_main(
        command: RendererPageCommand,
        next: impl FnOnce(anyhow::Result<RendererPageReply>) -> RendererNativeOperationStep
        + Send
        + 'static,
    ) -> Self {
        let nested_main = command.nested_dispatch()
            == crate::devtools::command::RendererDevToolsMainNestedDispatch::PageAgent;
        let mut operation = Self::then(command, next);
        operation.nested_main = nested_main;
        operation
    }

    /// Preserve an enclosing handler's isolate-entry requirement even when
    /// its first lookup happens to be a native node/realm query.
    pub fn require_owner_turn(mut self) -> Self {
        self.nested_main = false;
        self
    }

    pub(crate) fn can_dispatch_on_nested_main(&self) -> bool {
        self.nested_main
    }
}

enum NativeReadyReply {
    Resolved(RendererNativeProtocolResponse),
    Projected {
        reply: anyhow::Result<RendererPageReply>,
        project: NativeResponseProjection,
    },
}

impl RendererCdpCall {
    pub(crate) fn dispatch(
        self,
        vm: &mut crate::runtime::PageVm,
    ) -> RendererNativeCommandReadyResponse {
        let Self {
            mut operation,
            response,
        } = self;
        let nested_main = operation.nested_main;
        let reply = loop {
            // A lookup may construct another Inspector operation. Bind
            // each one to the attachment captured for the original call.
            operation
                .command
                .bind_inspector_attachment(response.attachment_id);
            let reply = vm.dispatch_renderer_page_command(*operation.command);
            match operation.completion {
                NativeOperationCompletion::Terminal(project) => {
                    break NativeReadyReply::Projected { reply, project };
                }
                NativeOperationCompletion::Lookup(next) => match next(reply) {
                    RendererNativeOperationStep::Continue(next) => {
                        assert!(
                            !nested_main || next.nested_main,
                            "a nested native continuation cannot acquire an owner-only isolate"
                        );
                        operation = next;
                    }
                    RendererNativeOperationStep::Complete(reply) => {
                        break NativeReadyReply::Resolved(reply);
                    }
                },
            }
        };
        RendererNativeCommandReadyResponse {
            response,
            reply,
            journal: vm.vm().renderer_command_output_journal(),
        }
    }
}

impl RendererNativeCommandReadyResponse {
    pub(crate) fn input_dispatch_outcome(
        &self,
    ) -> Option<&crate::runtime::RendererInputDispatchOutcome> {
        match &self.reply {
            NativeReadyReply::Projected {
                reply: Ok(reply), ..
            } => reply.input_dispatch_outcome(),
            _ => None,
        }
    }

    pub(crate) fn input_dispatch_outcome_mut(
        &mut self,
    ) -> Option<&mut crate::runtime::RendererInputDispatchOutcome> {
        match &mut self.reply {
            NativeReadyReply::Projected {
                reply: Ok(reply), ..
            } => reply.input_dispatch_outcome_mut(),
            _ => None,
        }
    }
}
