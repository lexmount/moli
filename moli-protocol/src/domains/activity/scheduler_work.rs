use std::fmt;

use moli_core::RendererOutputTransportMessage;

use crate::{
    conn::{
        BidiChannelOwnerAction, CdpConnection, DeferredMainDocumentLoadCompletionOutputAction,
        DeferredMainDocumentLoadCompletionOutputInterest,
        PendingDeferredMainDocumentLoadCompletion, PopupTargetActivationAction,
        PopupTargetNavigationOwnerAction, TopLevelLocationNavigationOwnerAction,
    },
    devtools_runtime::DevToolsCommandContext,
};

use super::{
    main_document::DeferredMainDocumentLoadCompletionActivity, output_work::ProtocolOutputWork,
};

/// Monotonic sequence assigned when protocol-owned scheduler work becomes
/// durable.
///
/// This sequence orders work published by one `CdpConnection`; it is not an
/// HTML task sequence and is not comparable with a renderer stream-local
/// `RendererOutputCursor`. Cross-owner ordering must therefore use an explicit
/// predecessor rather than comparing unrelated counters.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProtocolWorkPublishSequence(u64);

impl ProtocolWorkPublishSequence {
    pub(crate) fn new(value: u64) -> Self {
        assert_ne!(value, 0, "protocol work publish sequence starts at one");
        Self(value)
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

/// The semantic responsibility carried by one durable protocol work item.
///
/// This classification deliberately preserves the P6-R1 split. An
/// observation only projects an already-settled fact. An owner action must
/// remain resident and complete even when no frontend is listening.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolSchedulerWorkKind {
    ProtocolObservation,
    MainDocumentLoadOwnerAction,
    BidiChannelOwnerAction,
    TopLevelLocationNavigationOwnerAction,
    PopupTargetNavigationOwnerAction,
    PopupTargetActivationAction,
    PageTargetTerminationOwnerAction,
}

/// Durable protocol-owned work with concrete payload, exact route and one
/// connection-local publication sequence.
///
/// This move-only value never asks a later turn to scan a source. The private
/// payload is either a ready protocol observation or an exact browser-owner
/// continuation. The common wrapper exists only to give both classes one
/// scheduler residence and one ordering contract; it does not make an owner
/// action listener-dependent.
pub struct ProtocolSchedulerWork {
    publish_sequence: ProtocolWorkPublishSequence,
    payload: ProtocolSchedulerWorkPayload,
}

enum ProtocolSchedulerWorkPayload {
    ProtocolObservation(ProtocolOutputWork),
    MainDocumentLoadOwnerAction(Box<DeferredMainDocumentLoadCompletionActivity>),
    BidiChannelOwnerAction(BidiChannelOwnerAction),
    TopLevelLocationNavigationOwnerAction(TopLevelLocationNavigationOwnerAction),
    PopupTargetNavigationOwnerAction(PopupTargetNavigationOwnerAction),
    PopupTargetActivationAction(PopupTargetActivationAction),
    PageTargetTerminationOwnerAction(crate::domains::page::PageTargetTerminationOwnerAction),
}

impl fmt::Debug for ProtocolSchedulerWork {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("ProtocolSchedulerWork");
        debug
            .field("publish_sequence", &self.publish_sequence)
            .field("kind", &self.kind());
        match &self.payload {
            ProtocolSchedulerWorkPayload::ProtocolObservation(output) => {
                debug.field("payload", output);
            }
            ProtocolSchedulerWorkPayload::MainDocumentLoadOwnerAction(completion) => {
                debug
                    .field("observation_id", &completion.observation_id())
                    .field("session_id", &completion.session_id())
                    .field(
                        "renderer_page",
                        &completion.renderer_page_residence_identity(),
                    )
                    .field(
                        "renderer_document",
                        &completion.renderer_document_identity(),
                    )
                    .field("terminal", &completion.has_terminal_lifecycle_observation());
            }
            ProtocolSchedulerWorkPayload::BidiChannelOwnerAction(action) => {
                debug
                    .field("action", &action.kind())
                    .field("session_id", &action.owner().session_id());
            }
            ProtocolSchedulerWorkPayload::TopLevelLocationNavigationOwnerAction(action) => {
                debug
                    .field("session_id", &action.session_id())
                    .field("source_document", &action.source_document())
                    .field("url", &action.url());
            }
            ProtocolSchedulerWorkPayload::PopupTargetNavigationOwnerAction(action) => {
                debug
                    .field("browser_context_id", &action.browser_context_id())
                    .field("target_id", &action.target_id())
                    .field("url", &action.url())
                    .field("navigation_kind", &action.kind());
            }
            ProtocolSchedulerWorkPayload::PopupTargetActivationAction(action) => {
                debug
                    .field("browser_context_id", &action.browser_context_id())
                    .field("target_id", &action.target_id());
            }
            ProtocolSchedulerWorkPayload::PageTargetTerminationOwnerAction(action) => {
                debug
                    .field("action", &action.kind())
                    .field("session_id", &action.owner_scope().session_id())
                    .field("target_id", &action.target_id());
            }
        }
        debug.finish()
    }
}

impl ProtocolSchedulerWork {
    pub(crate) fn protocol_observation(
        publish_sequence: ProtocolWorkPublishSequence,
        output: ProtocolOutputWork,
    ) -> Self {
        Self {
            publish_sequence,
            payload: ProtocolSchedulerWorkPayload::ProtocolObservation(output),
        }
    }

    pub(crate) fn main_document_load_owner_action(
        publish_sequence: ProtocolWorkPublishSequence,
        completion: DeferredMainDocumentLoadCompletionActivity,
    ) -> Self {
        Self {
            publish_sequence,
            payload: ProtocolSchedulerWorkPayload::MainDocumentLoadOwnerAction(Box::new(
                completion,
            )),
        }
    }

    pub(crate) fn bidi_channel_owner_action(
        publish_sequence: ProtocolWorkPublishSequence,
        action: BidiChannelOwnerAction,
    ) -> Self {
        Self {
            publish_sequence,
            payload: ProtocolSchedulerWorkPayload::BidiChannelOwnerAction(action),
        }
    }

    pub(crate) fn top_level_location_navigation_owner_action(
        publish_sequence: ProtocolWorkPublishSequence,
        action: TopLevelLocationNavigationOwnerAction,
    ) -> Self {
        Self {
            publish_sequence,
            payload: ProtocolSchedulerWorkPayload::TopLevelLocationNavigationOwnerAction(action),
        }
    }

    pub(crate) fn popup_target_navigation_owner_action(
        publish_sequence: ProtocolWorkPublishSequence,
        action: PopupTargetNavigationOwnerAction,
    ) -> Self {
        Self {
            publish_sequence,
            payload: ProtocolSchedulerWorkPayload::PopupTargetNavigationOwnerAction(action),
        }
    }

    pub(crate) fn popup_target_activation_action(
        publish_sequence: ProtocolWorkPublishSequence,
        action: PopupTargetActivationAction,
    ) -> Self {
        Self {
            publish_sequence,
            payload: ProtocolSchedulerWorkPayload::PopupTargetActivationAction(action),
        }
    }

    pub(crate) fn page_target_termination_owner_action(
        publish_sequence: ProtocolWorkPublishSequence,
        action: crate::domains::page::PageTargetTerminationOwnerAction,
    ) -> Self {
        Self {
            publish_sequence,
            payload: ProtocolSchedulerWorkPayload::PageTargetTerminationOwnerAction(action),
        }
    }

    pub fn publish_sequence(&self) -> ProtocolWorkPublishSequence {
        self.publish_sequence
    }

    pub fn kind(&self) -> ProtocolSchedulerWorkKind {
        match &self.payload {
            ProtocolSchedulerWorkPayload::ProtocolObservation(_) => {
                ProtocolSchedulerWorkKind::ProtocolObservation
            }
            ProtocolSchedulerWorkPayload::MainDocumentLoadOwnerAction(_) => {
                ProtocolSchedulerWorkKind::MainDocumentLoadOwnerAction
            }
            ProtocolSchedulerWorkPayload::BidiChannelOwnerAction(_) => {
                ProtocolSchedulerWorkKind::BidiChannelOwnerAction
            }
            ProtocolSchedulerWorkPayload::TopLevelLocationNavigationOwnerAction(_) => {
                ProtocolSchedulerWorkKind::TopLevelLocationNavigationOwnerAction
            }
            ProtocolSchedulerWorkPayload::PopupTargetNavigationOwnerAction(_) => {
                ProtocolSchedulerWorkKind::PopupTargetNavigationOwnerAction
            }
            ProtocolSchedulerWorkPayload::PopupTargetActivationAction(_) => {
                ProtocolSchedulerWorkKind::PopupTargetActivationAction
            }
            ProtocolSchedulerWorkPayload::PageTargetTerminationOwnerAction(_) => {
                ProtocolSchedulerWorkKind::PageTargetTerminationOwnerAction
            }
        }
    }

    /// Reports whether this work can be completed without blocking its
    /// scheduler.
    ///
    /// Protocol observations and already-materialized BiDi owner actions are
    /// intrinsically ready. A main-document load action becomes ready only
    /// when its exact lifecycle observer has published a typed terminal.
    pub fn is_ready(&self) -> bool {
        match &self.payload {
            ProtocolSchedulerWorkPayload::ProtocolObservation(_) => true,
            ProtocolSchedulerWorkPayload::MainDocumentLoadOwnerAction(completion) => {
                completion.has_terminal_lifecycle_observation()
            }
            ProtocolSchedulerWorkPayload::BidiChannelOwnerAction(_) => true,
            ProtocolSchedulerWorkPayload::TopLevelLocationNavigationOwnerAction(_) => true,
            ProtocolSchedulerWorkPayload::PopupTargetNavigationOwnerAction(_) => true,
            ProtocolSchedulerWorkPayload::PopupTargetActivationAction(_) => true,
            ProtocolSchedulerWorkPayload::PageTargetTerminationOwnerAction(_) => true,
        }
    }

    /// Reports owner work that must complete inside the producing command's
    /// turn.
    ///
    /// Popup navigation is safe to start while completing the producing
    /// command: it retains its exact target route and cannot replace the
    /// command's active renderer. Popup activation is deliberately excluded.
    /// It can replace that renderer and must therefore cross the ordinary
    /// client-turn predecessor so the opener's command result is collected
    /// from its original owner first.
    pub fn is_command_followup(&self) -> bool {
        matches!(
            &self.payload,
            ProtocolSchedulerWorkPayload::BidiChannelOwnerAction(_)
                | ProtocolSchedulerWorkPayload::TopLevelLocationNavigationOwnerAction(_)
                | ProtocolSchedulerWorkPayload::PopupTargetNavigationOwnerAction(_)
                | ProtocolSchedulerWorkPayload::PageTargetTerminationOwnerAction(_)
        )
    }

    pub fn navigation_gate_target_id(&self) -> Option<&str> {
        match &self.payload {
            ProtocolSchedulerWorkPayload::ProtocolObservation(output) => {
                output.navigation_gate_target_id()
            }
            ProtocolSchedulerWorkPayload::MainDocumentLoadOwnerAction(completion) => {
                Some(completion.target_id())
            }
            ProtocolSchedulerWorkPayload::BidiChannelOwnerAction(action) => {
                action.owner().target_id()
            }
            ProtocolSchedulerWorkPayload::TopLevelLocationNavigationOwnerAction(action) => {
                action.target_id()
            }
            ProtocolSchedulerWorkPayload::PopupTargetNavigationOwnerAction(action) => {
                Some(action.target_id())
            }
            ProtocolSchedulerWorkPayload::PopupTargetActivationAction(action) => {
                Some(action.target_id())
            }
            ProtocolSchedulerWorkPayload::PageTargetTerminationOwnerAction(action) => {
                Some(action.target_id())
            }
        }
    }

    /// Reports work whose transition is independent of an in-flight document
    /// load on the same target.
    ///
    /// Foreground selection happens when Chromium accepts a user activation,
    /// not when the selected target finishes loading. This also lets a target
    /// paused by `waitForDebuggerOnStart` become active before its initial
    /// navigation is released. The exact target id above still preserves
    /// target-local ordering against earlier scheduler residences.
    pub fn bypasses_inflight_navigation_gate(&self) -> bool {
        matches!(
            &self.payload,
            ProtocolSchedulerWorkPayload::PopupTargetActivationAction(_)
        )
    }

    pub fn is_top_level_location_navigation_owner_action(&self) -> bool {
        matches!(
            &self.payload,
            ProtocolSchedulerWorkPayload::TopLevelLocationNavigationOwnerAction(_)
        )
    }

    /// Reports work that requires the scheduler-owned background navigation
    /// channels rather than the command fixture's inline fallback.
    ///
    /// Popup target creation is projected before the causing Runtime response,
    /// while its URL load is deliberately independent. A protocol-only test
    /// harness without those channels must retain this action instead of
    /// accidentally turning it into a blocking navigation wait.
    #[cfg(test)]
    pub(crate) fn requires_background_navigation_scheduler(&self) -> bool {
        matches!(
            &self.payload,
            ProtocolSchedulerWorkPayload::PopupTargetNavigationOwnerAction(_)
        )
    }

    #[cfg(test)]
    pub(crate) fn bidi_channel_owner_action_kind(
        &self,
    ) -> Option<crate::conn::BidiChannelOwnerActionKind> {
        let ProtocolSchedulerWorkPayload::BidiChannelOwnerAction(action) = &self.payload else {
            return None;
        };
        Some(action.kind())
    }

    pub fn is_root_frame_stopped_loading(&self) -> bool {
        matches!(
            &self.payload,
            ProtocolSchedulerWorkPayload::ProtocolObservation(output)
                if output.is_root_frame_stopped_loading()
        )
    }

    pub fn main_document_load_output_interest(
        &self,
    ) -> Option<DeferredMainDocumentLoadCompletionOutputInterest> {
        let ProtocolSchedulerWorkPayload::MainDocumentLoadOwnerAction(completion) = &self.payload
        else {
            return None;
        };
        Some(DeferredMainDocumentLoadCompletionOutputInterest::new(
            completion.renderer_page_residence_identity(),
            completion.renderer_document_identity(),
        ))
    }

    pub fn main_document_load_observation_id(
        &self,
    ) -> Option<crate::conn::DeferredMainDocumentLoadObservationId> {
        let ProtocolSchedulerWorkPayload::MainDocumentLoadOwnerAction(completion) = &self.payload
        else {
            return None;
        };
        Some(completion.observation_id())
    }

    #[cfg(test)]
    pub(crate) fn main_document_load_session_id(&self) -> Option<&str> {
        let ProtocolSchedulerWorkPayload::MainDocumentLoadOwnerAction(completion) = &self.payload
        else {
            return None;
        };
        completion.session_id()
    }

    pub fn route_renderer_output_while_main_document_load_waits(
        &self,
        output: &RendererOutputTransportMessage,
    ) -> Option<DeferredMainDocumentLoadCompletionOutputAction> {
        self.main_document_load_output_interest()
            .map(|interest| interest.route_output_while_waiting(output))
    }

    pub fn observes_main_document_load_for_devtools_context(
        &self,
        conn: &CdpConnection,
        context: &DevToolsCommandContext,
    ) -> bool {
        let ProtocolSchedulerWorkPayload::MainDocumentLoadOwnerAction(completion) = &self.payload
        else {
            return false;
        };
        conn.command_owner_scope_for_devtools_context(context)
            .is_some_and(|owner_scope| completion.owner_scope() == &owner_scope)
    }

    pub fn start_main_document_load_wait(self) -> PendingDeferredMainDocumentLoadCompletion {
        match self.payload {
            ProtocolSchedulerWorkPayload::MainDocumentLoadOwnerAction(completion) => {
                PendingDeferredMainDocumentLoadCompletion::new((*completion).start_scheduler_step())
            }
            ProtocolSchedulerWorkPayload::ProtocolObservation(_)
            | ProtocolSchedulerWorkPayload::BidiChannelOwnerAction(_)
            | ProtocolSchedulerWorkPayload::TopLevelLocationNavigationOwnerAction(_)
            | ProtocolSchedulerWorkPayload::PopupTargetNavigationOwnerAction(_)
            | ProtocolSchedulerWorkPayload::PopupTargetActivationAction(_)
            | ProtocolSchedulerWorkPayload::PageTargetTerminationOwnerAction(_) => {
                panic!("only main-document load owner work can start a lifecycle wait")
            }
        }
    }

    #[cfg(feature = "test-support")]
    pub(crate) fn root_frame_stopped_loading_for_test_support(
        publish_sequence: u64,
        session_ids: Vec<Option<String>>,
        frame_id: String,
        loader_id: String,
    ) -> Self {
        Self::protocol_observation(
            ProtocolWorkPublishSequence::new(publish_sequence),
            ProtocolOutputWork::root_frame_stopped_loading_for_test_support(
                session_ids,
                frame_id,
                loader_id,
            ),
        )
    }

    #[cfg(feature = "test-support")]
    pub(crate) fn root_frame_stopped_loading_for_target_test_support(
        publish_sequence: u64,
        session_ids: Vec<Option<String>>,
        browser_context_id: String,
        target_id: String,
        frame_id: String,
        loader_id: String,
    ) -> Self {
        Self::protocol_observation(
            ProtocolWorkPublishSequence::new(publish_sequence),
            ProtocolOutputWork::root_frame_stopped_loading_for_target_test_support(
                session_ids,
                browser_context_id,
                target_id,
                frame_id,
                loader_id,
            ),
        )
    }
}

pub(crate) enum ReadyProtocolSchedulerWork {
    ProtocolObservation(ProtocolOutputWork),
    MainDocumentLoadOwnerAction(
        Box<super::main_document::CompletedDeferredMainDocumentLoadCompletionActivity>,
    ),
    BidiChannelOwnerAction(BidiChannelOwnerAction),
    TopLevelLocationNavigationOwnerAction(TopLevelLocationNavigationOwnerAction),
    PopupTargetNavigationOwnerAction(PopupTargetNavigationOwnerAction),
    PopupTargetActivationAction(PopupTargetActivationAction),
    PageTargetTerminationOwnerAction(crate::domains::page::PageTargetTerminationOwnerAction),
}

impl ProtocolSchedulerWork {
    pub(crate) fn into_ready(self) -> ReadyProtocolSchedulerWork {
        match self.payload {
            ProtocolSchedulerWorkPayload::ProtocolObservation(output) => {
                ReadyProtocolSchedulerWork::ProtocolObservation(output)
            }
            ProtocolSchedulerWorkPayload::MainDocumentLoadOwnerAction(completion) => {
                let completion = completion.try_complete().unwrap_or_else(|_| {
                    panic!(
                        "pending main-document load work cannot be completed by a nonblocking scheduler turn"
                    )
                });
                ReadyProtocolSchedulerWork::MainDocumentLoadOwnerAction(Box::new(completion))
            }
            ProtocolSchedulerWorkPayload::BidiChannelOwnerAction(action) => {
                ReadyProtocolSchedulerWork::BidiChannelOwnerAction(action)
            }
            ProtocolSchedulerWorkPayload::TopLevelLocationNavigationOwnerAction(action) => {
                ReadyProtocolSchedulerWork::TopLevelLocationNavigationOwnerAction(action)
            }
            ProtocolSchedulerWorkPayload::PopupTargetNavigationOwnerAction(action) => {
                ReadyProtocolSchedulerWork::PopupTargetNavigationOwnerAction(action)
            }
            ProtocolSchedulerWorkPayload::PopupTargetActivationAction(action) => {
                ReadyProtocolSchedulerWork::PopupTargetActivationAction(action)
            }
            ProtocolSchedulerWorkPayload::PageTargetTerminationOwnerAction(action) => {
                ReadyProtocolSchedulerWork::PageTargetTerminationOwnerAction(action)
            }
        }
    }
}
