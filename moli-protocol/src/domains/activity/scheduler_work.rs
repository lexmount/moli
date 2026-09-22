use std::fmt;

use crate::conn::{
    BidiChannelOwnerAction, TargetStartupOwnerAction, TopLevelLocationNavigationOwnerAction,
};

use super::output_work::ProtocolOutputWork;

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

    BidiChannelOwnerAction,
    TopLevelLocationNavigationOwnerAction,
    TargetStartupOwnerAction,
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

    BidiChannelOwnerAction(BidiChannelOwnerAction),
    TopLevelLocationNavigationOwnerAction(TopLevelLocationNavigationOwnerAction),
    TargetStartupOwnerAction(TargetStartupOwnerAction),
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
            ProtocolSchedulerWorkPayload::TargetStartupOwnerAction(action) => {
                debug
                    .field("target_id", &action.target_id())
                    .field("decision", &action.decision());
            }
            ProtocolSchedulerWorkPayload::PageTargetTerminationOwnerAction(action) => {
                debug
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

    pub(crate) fn target_startup_owner_action(
        publish_sequence: ProtocolWorkPublishSequence,
        action: TargetStartupOwnerAction,
    ) -> Self {
        Self {
            publish_sequence,
            payload: ProtocolSchedulerWorkPayload::TargetStartupOwnerAction(action),
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

            ProtocolSchedulerWorkPayload::BidiChannelOwnerAction(_) => {
                ProtocolSchedulerWorkKind::BidiChannelOwnerAction
            }
            ProtocolSchedulerWorkPayload::TopLevelLocationNavigationOwnerAction(_) => {
                ProtocolSchedulerWorkKind::TopLevelLocationNavigationOwnerAction
            }
            ProtocolSchedulerWorkPayload::TargetStartupOwnerAction(_) => {
                ProtocolSchedulerWorkKind::TargetStartupOwnerAction
            }
            ProtocolSchedulerWorkPayload::PageTargetTerminationOwnerAction(_) => {
                ProtocolSchedulerWorkKind::PageTargetTerminationOwnerAction
            }
        }
    }

    /// Reports owner work that must complete inside the producing command's
    /// turn.
    ///
    /// A target startup decision can replace the command's own initial empty
    /// Document. Its release therefore crosses the ordinary client-turn
    /// predecessor, including `Runtime.runIfWaitingForDebugger` replies.
    pub fn is_command_followup(&self) -> bool {
        match &self.payload {
            ProtocolSchedulerWorkPayload::BidiChannelOwnerAction(_)
            | ProtocolSchedulerWorkPayload::TopLevelLocationNavigationOwnerAction(_)
            | ProtocolSchedulerWorkPayload::PageTargetTerminationOwnerAction(_) => true,
            ProtocolSchedulerWorkPayload::TargetStartupOwnerAction(_) => false,
            ProtocolSchedulerWorkPayload::ProtocolObservation(_) => false,
        }
    }

    pub fn navigation_gate_target_id(&self) -> Option<&str> {
        match &self.payload {
            ProtocolSchedulerWorkPayload::ProtocolObservation(output) => {
                output.navigation_gate_target_id()
            }

            ProtocolSchedulerWorkPayload::BidiChannelOwnerAction(action) => {
                action.owner().target_id()
            }
            ProtocolSchedulerWorkPayload::TopLevelLocationNavigationOwnerAction(action) => {
                action.target_id()
            }
            ProtocolSchedulerWorkPayload::TargetStartupOwnerAction(action) => {
                Some(action.target_id())
            }
            ProtocolSchedulerWorkPayload::PageTargetTerminationOwnerAction(action) => {
                Some(action.target_id())
            }
        }
    }

    pub fn is_top_level_location_navigation_owner_action(&self) -> bool {
        matches!(
            &self.payload,
            ProtocolSchedulerWorkPayload::TopLevelLocationNavigationOwnerAction(_)
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

    #[cfg(any(test, feature = "test-support"))]
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

    #[cfg(any(test, feature = "test-support"))]
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

    BidiChannelOwnerAction(BidiChannelOwnerAction),
    TopLevelLocationNavigationOwnerAction(TopLevelLocationNavigationOwnerAction),
    TargetStartupOwnerAction(TargetStartupOwnerAction),
    PageTargetTerminationOwnerAction(crate::domains::page::PageTargetTerminationOwnerAction),
}

impl ProtocolSchedulerWork {
    pub(crate) fn into_ready(self) -> ReadyProtocolSchedulerWork {
        match self.payload {
            ProtocolSchedulerWorkPayload::ProtocolObservation(output) => {
                ReadyProtocolSchedulerWork::ProtocolObservation(output)
            }

            ProtocolSchedulerWorkPayload::BidiChannelOwnerAction(action) => {
                ReadyProtocolSchedulerWork::BidiChannelOwnerAction(action)
            }
            ProtocolSchedulerWorkPayload::TopLevelLocationNavigationOwnerAction(action) => {
                ReadyProtocolSchedulerWork::TopLevelLocationNavigationOwnerAction(action)
            }
            ProtocolSchedulerWorkPayload::TargetStartupOwnerAction(action) => {
                ReadyProtocolSchedulerWork::TargetStartupOwnerAction(action)
            }
            ProtocolSchedulerWorkPayload::PageTargetTerminationOwnerAction(action) => {
                ReadyProtocolSchedulerWork::PageTargetTerminationOwnerAction(action)
            }
        }
    }
}
