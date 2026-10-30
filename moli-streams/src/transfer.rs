//! Runtime-independent transferable stream channel coordination.
//!
//! MessagePort endpoints, structured-clone payloads, JavaScript chunks and
//! errors, stream wrappers, and promises remain adapter-owned. This module
//! owns the four-message protocol and the sender/receiver decisions derived
//! from primitive channel state.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferMessageKind {
    Pull,
    Chunk,
    Close,
    Error,
}

impl TransferMessageKind {
    #[must_use]
    pub const fn wire_code(self) -> u32 {
        match self {
            Self::Pull => 0,
            Self::Chunk => 1,
            Self::Close => 2,
            Self::Error => 3,
        }
    }
}

impl TryFrom<u32> for TransferMessageKind {
    type Error = InvalidTransferMessageKind;

    fn try_from(value: u32) -> Result<Self, InvalidTransferMessageKind> {
        match value {
            0 => Ok(Self::Pull),
            1 => Ok(Self::Chunk),
            2 => Ok(Self::Close),
            3 => Ok(Self::Error),
            _ => Err(InvalidTransferMessageKind),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidTransferMessageKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransferEntrySnapshot {
    source_locked: bool,
}

impl TransferEntrySnapshot {
    #[must_use]
    pub const fn new(source_locked: bool) -> Self {
        Self { source_locked }
    }

    #[must_use]
    pub const fn plan(self) -> TransferEntryPlan {
        if self.source_locked {
            TransferEntryPlan::RejectLocked
        } else {
            TransferEntryPlan::Prepare
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferEntryPlan {
    Prepare,
    RejectLocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransferSnapshot {
    active: bool,
    read_in_flight: bool,
    pull_demand: u32,
    has_staged_chunk: bool,
}

impl TransferSnapshot {
    #[must_use]
    pub const fn new(
        active: bool,
        read_in_flight: bool,
        pull_demand: u32,
        has_staged_chunk: bool,
    ) -> Self {
        Self {
            active,
            read_in_flight,
            pull_demand,
            has_staged_chunk,
        }
    }

    #[must_use]
    pub const fn initial() -> Self {
        Self::new(true, false, 0, false)
    }

    #[must_use]
    pub const fn active(self) -> bool {
        self.active
    }

    #[must_use]
    pub const fn read_in_flight(self) -> bool {
        self.read_in_flight
    }

    #[must_use]
    pub const fn pull_demand(self) -> u32 {
        self.pull_demand
    }

    #[must_use]
    pub const fn plan_finish(self) -> TransferFinishPlan {
        if self.active {
            TransferFinishPlan::DeactivateAndClosePort
        } else {
            TransferFinishPlan::AlreadyFinished
        }
    }

    #[must_use]
    pub const fn plan_sender_start_read(self) -> SenderStartReadPlan {
        if self.active && !self.has_staged_chunk && !self.read_in_flight {
            SenderStartReadPlan::StartRead
        } else {
            SenderStartReadPlan::Ignore
        }
    }

    #[must_use]
    pub const fn plan_sender_read_reaction(self) -> SenderReadReactionPlan {
        if self.active {
            SenderReadReactionPlan::InspectResult
        } else {
            SenderReadReactionPlan::Ignore
        }
    }

    #[must_use]
    pub const fn plan_sender_read_fulfilled(
        self,
        result: SenderReadResultSnapshot,
    ) -> SenderReadFulfillmentPlan {
        if !self.active {
            return SenderReadFulfillmentPlan::Ignore;
        }
        if !result.valid_result {
            return SenderReadFulfillmentPlan::FailInvalidResult;
        }
        if result.done {
            return SenderReadFulfillmentPlan::PostCloseAndFinish;
        }
        if self.pull_demand == 0 {
            SenderReadFulfillmentPlan::StageChunk
        } else {
            SenderReadFulfillmentPlan::PostChunk {
                pull_demand_after: self.pull_demand - 1,
            }
        }
    }

    #[must_use]
    pub const fn plan_sender_read_rejected(self) -> SenderReadRejectionPlan {
        if self.active {
            SenderReadRejectionPlan::PostErrorAndFinish
        } else {
            SenderReadRejectionPlan::Ignore
        }
    }

    #[must_use]
    pub const fn plan_sender_message(
        self,
        message: Option<TransferMessageKind>,
    ) -> SenderMessagePlan {
        if !self.active {
            return SenderMessagePlan::IgnoreLateMessage;
        }
        match message {
            Some(TransferMessageKind::Pull) => {
                let pull_demand = self.pull_demand.saturating_add(1);
                let next = Self::new(
                    self.active,
                    self.read_in_flight,
                    pull_demand,
                    self.has_staged_chunk,
                );
                SenderMessagePlan::RecordPull {
                    pull_demand,
                    drain: next.plan_sender_drain(),
                }
            }
            Some(TransferMessageKind::Error) => SenderMessagePlan::CancelSourceAndFinish,
            Some(TransferMessageKind::Chunk | TransferMessageKind::Close) | None => {
                SenderMessagePlan::FailProtocol
            }
        }
    }

    #[must_use]
    pub const fn plan_sender_drain(self) -> SenderDrainPlan {
        if !self.active || self.pull_demand == 0 {
            return SenderDrainPlan::Ignore;
        }
        if self.has_staged_chunk {
            SenderDrainPlan::PostStagedChunk {
                pull_demand_after: self.pull_demand - 1,
            }
        } else {
            match self.plan_sender_start_read() {
                SenderStartReadPlan::StartRead => SenderDrainPlan::StartRead,
                SenderStartReadPlan::Ignore => SenderDrainPlan::Ignore,
            }
        }
    }

    #[must_use]
    pub const fn plan_sender_failure(self) -> SenderFailurePlan {
        if self.active {
            SenderFailurePlan::PostErrorCancelSourceAndFinish
        } else {
            SenderFailurePlan::Ignore
        }
    }

    #[must_use]
    pub const fn plan_receiver_message(
        self,
        message: Option<TransferMessageKind>,
    ) -> ReceiverMessagePlan {
        if !self.active {
            return ReceiverMessagePlan::IgnoreLateMessage;
        }
        match message {
            Some(TransferMessageKind::Chunk) => ReceiverMessagePlan::EnqueueChunk,
            Some(TransferMessageKind::Close) => ReceiverMessagePlan::CloseStreamAndFinish,
            Some(TransferMessageKind::Error) => ReceiverMessagePlan::ErrorStreamAndFinish,
            Some(TransferMessageKind::Pull) | None => ReceiverMessagePlan::FailProtocol,
        }
    }

    #[must_use]
    pub const fn plan_receiver_message_error(self) -> ReceiverMessageErrorPlan {
        if self.active {
            ReceiverMessageErrorPlan::ErrorStreamAndFinish
        } else {
            ReceiverMessageErrorPlan::Ignore
        }
    }

    #[must_use]
    pub const fn plan_receiver_pull(self) -> ReceiverPullPlan {
        if self.active {
            ReceiverPullPlan::PostPull
        } else {
            ReceiverPullPlan::Ignore
        }
    }

    #[must_use]
    pub const fn plan_receiver_cancel(self) -> ReceiverCancelPlan {
        if self.active {
            ReceiverCancelPlan::PostErrorAndFinish
        } else {
            ReceiverCancelPlan::Ignore
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferFinishPlan {
    AlreadyFinished,
    DeactivateAndClosePort,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SenderStartReadPlan {
    Ignore,
    StartRead,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SenderReadReactionPlan {
    Ignore,
    InspectResult,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SenderReadResultSnapshot {
    valid_result: bool,
    done: bool,
}

impl SenderReadResultSnapshot {
    #[must_use]
    pub const fn new(valid_result: bool, done: bool) -> Self {
        Self { valid_result, done }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SenderReadFulfillmentPlan {
    Ignore,
    FailInvalidResult,
    PostCloseAndFinish,
    StageChunk,
    PostChunk { pull_demand_after: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SenderReadRejectionPlan {
    Ignore,
    PostErrorAndFinish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SenderMessagePlan {
    IgnoreLateMessage,
    RecordPull {
        pull_demand: u32,
        drain: SenderDrainPlan,
    },
    CancelSourceAndFinish,
    FailProtocol,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SenderDrainPlan {
    Ignore,
    StartRead,
    PostStagedChunk { pull_demand_after: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SenderFailurePlan {
    Ignore,
    PostErrorCancelSourceAndFinish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiverMessagePlan {
    IgnoreLateMessage,
    EnqueueChunk,
    CloseStreamAndFinish,
    ErrorStreamAndFinish,
    FailProtocol,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiverEnqueueOutcome {
    Enqueued,
    StreamTerminal,
    StrategyError,
}

impl ReceiverEnqueueOutcome {
    #[must_use]
    pub const fn plan(self) -> ReceiverEnqueuePlan {
        match self {
            Self::Enqueued | Self::StreamTerminal => ReceiverEnqueuePlan::Continue,
            Self::StrategyError => ReceiverEnqueuePlan::Finish,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiverEnqueuePlan {
    Continue,
    Finish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiverMessageErrorPlan {
    Ignore,
    ErrorStreamAndFinish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiverPullPlan {
    Ignore,
    PostPull,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiverCancelPlan {
    Ignore,
    PostErrorAndFinish,
}

/// Primitive state for the WritableStream endpoint reconstructed in the
/// receiving realm. JavaScript chunks and promise capabilities stay in the
/// renderer; this snapshot owns admission and protocol ordering only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WritableTransferSnapshot {
    active: bool,
    pull_demand: u32,
    pending_write: bool,
}

impl WritableTransferSnapshot {
    #[must_use]
    pub const fn new(active: bool, pull_demand: u32, pending_write: bool) -> Self {
        Self {
            active,
            pull_demand,
            pending_write,
        }
    }

    #[must_use]
    pub const fn initial() -> Self {
        Self::new(true, 0, false)
    }

    #[must_use]
    pub const fn active(self) -> bool {
        self.active
    }

    #[must_use]
    pub const fn pull_demand(self) -> u32 {
        self.pull_demand
    }

    #[must_use]
    pub const fn plan_write(self) -> WritableTransferWritePlan {
        if !self.active {
            return WritableTransferWritePlan::RejectInactive;
        }
        if self.pending_write {
            return WritableTransferWritePlan::RejectConcurrentWrite;
        }
        if self.pull_demand == 0 {
            WritableTransferWritePlan::WaitForPull
        } else {
            WritableTransferWritePlan::PostChunk {
                pull_demand_after: self.pull_demand - 1,
            }
        }
    }

    #[must_use]
    pub const fn plan_message(
        self,
        message: Option<TransferMessageKind>,
    ) -> WritableTransferMessagePlan {
        if !self.active {
            return WritableTransferMessagePlan::IgnoreLateMessage;
        }
        match message {
            Some(TransferMessageKind::Pull) if self.pending_write => {
                WritableTransferMessagePlan::PostPendingChunk
            }
            Some(TransferMessageKind::Pull) => WritableTransferMessagePlan::RecordPull {
                pull_demand: self.pull_demand.saturating_add(1),
            },
            Some(TransferMessageKind::Error) => WritableTransferMessagePlan::ErrorStreamAndFinish,
            Some(TransferMessageKind::Chunk | TransferMessageKind::Close) | None => {
                WritableTransferMessagePlan::FailProtocol
            }
        }
    }

    #[must_use]
    pub const fn plan_terminal(self) -> WritableTransferTerminalPlan {
        if self.active {
            WritableTransferTerminalPlan::PostAndFinish
        } else {
            WritableTransferTerminalPlan::Ignore
        }
    }

    #[must_use]
    pub const fn plan_message_error(self) -> WritableTransferMessageErrorPlan {
        if self.active {
            WritableTransferMessageErrorPlan::ErrorStreamAndFinish
        } else {
            WritableTransferMessageErrorPlan::Ignore
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WritableTransferWritePlan {
    PostChunk { pull_demand_after: u32 },
    WaitForPull,
    RejectInactive,
    RejectConcurrentWrite,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WritableTransferMessagePlan {
    IgnoreLateMessage,
    RecordPull { pull_demand: u32 },
    PostPendingChunk,
    ErrorStreamAndFinish,
    FailProtocol,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WritableTransferTerminalPlan {
    Ignore,
    PostAndFinish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WritableTransferMessageErrorPlan {
    Ignore,
    ErrorStreamAndFinish,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(
        active: bool,
        read_in_flight: bool,
        pull_demand: u32,
        has_staged_chunk: bool,
    ) -> TransferSnapshot {
        TransferSnapshot::new(active, read_in_flight, pull_demand, has_staged_chunk)
    }

    #[test]
    fn protocol_wire_codes_are_exact_and_reject_unknown_values() {
        for (kind, code) in [
            (TransferMessageKind::Pull, 0),
            (TransferMessageKind::Chunk, 1),
            (TransferMessageKind::Close, 2),
            (TransferMessageKind::Error, 3),
        ] {
            assert_eq!(kind.wire_code(), code);
            assert_eq!(TransferMessageKind::try_from(code), Ok(kind));
        }
        assert_eq!(
            TransferMessageKind::try_from(4),
            Err(InvalidTransferMessageKind)
        );
        assert_eq!(
            TransferMessageKind::try_from(u32::MAX),
            Err(InvalidTransferMessageKind)
        );
    }

    #[test]
    fn entry_rejects_only_a_locked_source() {
        assert_eq!(
            TransferEntrySnapshot::new(false).plan(),
            TransferEntryPlan::Prepare
        );
        assert_eq!(
            TransferEntrySnapshot::new(true).plan(),
            TransferEntryPlan::RejectLocked
        );
    }

    #[test]
    fn initial_channel_is_active_idle_and_without_demand() {
        let initial = TransferSnapshot::initial();
        assert!(initial.active());
        assert!(!initial.read_in_flight());
        assert_eq!(initial.pull_demand(), 0);
        assert_eq!(
            initial.plan_sender_start_read(),
            SenderStartReadPlan::StartRead
        );
        assert_eq!(
            initial.plan_sender_read_reaction(),
            SenderReadReactionPlan::InspectResult
        );
        assert_eq!(
            initial.plan_finish(),
            TransferFinishPlan::DeactivateAndClosePort
        );
    }

    #[test]
    fn writable_endpoint_serializes_writes_against_remote_pull_demand() {
        let initial = WritableTransferSnapshot::initial();
        assert_eq!(initial.plan_write(), WritableTransferWritePlan::WaitForPull);
        assert_eq!(
            initial.plan_message(Some(TransferMessageKind::Pull)),
            WritableTransferMessagePlan::RecordPull { pull_demand: 1 }
        );
        assert_eq!(
            WritableTransferSnapshot::new(true, 1, false).plan_write(),
            WritableTransferWritePlan::PostChunk {
                pull_demand_after: 0
            }
        );
        assert_eq!(
            WritableTransferSnapshot::new(true, 0, true)
                .plan_message(Some(TransferMessageKind::Pull)),
            WritableTransferMessagePlan::PostPendingChunk
        );
    }

    #[test]
    fn writable_endpoint_routes_terminal_and_invalid_messages_once() {
        let active = WritableTransferSnapshot::initial();
        assert_eq!(
            active.plan_message(Some(TransferMessageKind::Error)),
            WritableTransferMessagePlan::ErrorStreamAndFinish
        );
        assert_eq!(
            active.plan_message(Some(TransferMessageKind::Chunk)),
            WritableTransferMessagePlan::FailProtocol
        );
        assert_eq!(
            active.plan_terminal(),
            WritableTransferTerminalPlan::PostAndFinish
        );

        let inactive = WritableTransferSnapshot::new(false, 1, true);
        assert_eq!(
            inactive.plan_message(Some(TransferMessageKind::Pull)),
            WritableTransferMessagePlan::IgnoreLateMessage
        );
        assert_eq!(
            inactive.plan_terminal(),
            WritableTransferTerminalPlan::Ignore
        );
        assert_eq!(
            inactive.plan_message_error(),
            WritableTransferMessageErrorPlan::Ignore
        );
    }

    #[test]
    fn sender_start_read_exhaustively_requires_active_idle_unstaged_state() {
        for active in [false, true] {
            for read_in_flight in [false, true] {
                for has_staged_chunk in [false, true] {
                    let current = snapshot(active, read_in_flight, 0, has_staged_chunk);
                    assert_eq!(
                        current.plan_sender_start_read(),
                        if active && !read_in_flight && !has_staged_chunk {
                            SenderStartReadPlan::StartRead
                        } else {
                            SenderStartReadPlan::Ignore
                        }
                    );
                }
            }
        }
    }

    #[test]
    fn sender_read_fulfillment_prioritizes_terminal_and_validation() {
        let inactive = snapshot(false, false, 1, false);
        assert_eq!(
            inactive.plan_sender_read_fulfilled(SenderReadResultSnapshot::new(false, false)),
            SenderReadFulfillmentPlan::Ignore
        );
        let active = snapshot(true, false, 0, false);
        assert_eq!(
            active.plan_sender_read_fulfilled(SenderReadResultSnapshot::new(false, false)),
            SenderReadFulfillmentPlan::FailInvalidResult
        );
        assert_eq!(
            active.plan_sender_read_fulfilled(SenderReadResultSnapshot::new(true, true)),
            SenderReadFulfillmentPlan::PostCloseAndFinish
        );
        assert_eq!(
            active.plan_sender_read_fulfilled(SenderReadResultSnapshot::new(true, false)),
            SenderReadFulfillmentPlan::StageChunk
        );
        assert_eq!(
            snapshot(true, false, 3, false)
                .plan_sender_read_fulfilled(SenderReadResultSnapshot::new(true, false)),
            SenderReadFulfillmentPlan::PostChunk {
                pull_demand_after: 2,
            }
        );
    }

    #[test]
    fn sender_drain_exhaustively_routes_stage_read_and_wait() {
        for active in [false, true] {
            for read_in_flight in [false, true] {
                for demand in [0, 1, u32::MAX] {
                    for staged in [false, true] {
                        let current = snapshot(active, read_in_flight, demand, staged);
                        let expected = if !active || demand == 0 {
                            SenderDrainPlan::Ignore
                        } else if staged {
                            SenderDrainPlan::PostStagedChunk {
                                pull_demand_after: demand - 1,
                            }
                        } else if read_in_flight {
                            SenderDrainPlan::Ignore
                        } else {
                            SenderDrainPlan::StartRead
                        };
                        assert_eq!(current.plan_sender_drain(), expected);
                    }
                }
            }
        }
    }

    #[test]
    fn pull_message_saturates_demand_and_freezes_the_matching_drain_plan() {
        assert_eq!(
            snapshot(true, false, 0, true).plan_sender_message(Some(TransferMessageKind::Pull)),
            SenderMessagePlan::RecordPull {
                pull_demand: 1,
                drain: SenderDrainPlan::PostStagedChunk {
                    pull_demand_after: 0,
                },
            }
        );
        assert_eq!(
            snapshot(true, true, u32::MAX, false)
                .plan_sender_message(Some(TransferMessageKind::Pull)),
            SenderMessagePlan::RecordPull {
                pull_demand: u32::MAX,
                drain: SenderDrainPlan::Ignore,
            }
        );
    }

    #[test]
    fn sender_protocol_accepts_only_pull_and_remote_error() {
        let active = TransferSnapshot::initial();
        assert_eq!(
            active.plan_sender_message(Some(TransferMessageKind::Error)),
            SenderMessagePlan::CancelSourceAndFinish
        );
        for message in [
            Some(TransferMessageKind::Chunk),
            Some(TransferMessageKind::Close),
            None,
        ] {
            assert_eq!(
                active.plan_sender_message(message),
                SenderMessagePlan::FailProtocol
            );
        }
        assert_eq!(
            snapshot(false, false, 0, false).plan_sender_message(Some(TransferMessageKind::Pull)),
            SenderMessagePlan::IgnoreLateMessage
        );
    }

    #[test]
    fn sender_rejection_and_failure_are_one_shot() {
        let active = TransferSnapshot::initial();
        assert_eq!(
            active.plan_sender_read_rejected(),
            SenderReadRejectionPlan::PostErrorAndFinish
        );
        assert_eq!(
            active.plan_sender_failure(),
            SenderFailurePlan::PostErrorCancelSourceAndFinish
        );
        let inactive = snapshot(false, false, 0, false);
        assert_eq!(
            inactive.plan_sender_read_rejected(),
            SenderReadRejectionPlan::Ignore
        );
        assert_eq!(
            inactive.plan_sender_read_reaction(),
            SenderReadReactionPlan::Ignore
        );
        assert_eq!(inactive.plan_sender_failure(), SenderFailurePlan::Ignore);
        assert_eq!(inactive.plan_finish(), TransferFinishPlan::AlreadyFinished);
    }

    #[test]
    fn receiver_protocol_routes_data_terminal_and_invalid_messages() {
        let active = TransferSnapshot::initial();
        assert_eq!(
            active.plan_receiver_message(Some(TransferMessageKind::Chunk)),
            ReceiverMessagePlan::EnqueueChunk
        );
        assert_eq!(
            active.plan_receiver_message(Some(TransferMessageKind::Close)),
            ReceiverMessagePlan::CloseStreamAndFinish
        );
        assert_eq!(
            active.plan_receiver_message(Some(TransferMessageKind::Error)),
            ReceiverMessagePlan::ErrorStreamAndFinish
        );
        assert_eq!(
            active.plan_receiver_message(Some(TransferMessageKind::Pull)),
            ReceiverMessagePlan::FailProtocol
        );
        assert_eq!(
            active.plan_receiver_message(None),
            ReceiverMessagePlan::FailProtocol
        );
        assert_eq!(
            snapshot(false, false, 0, false)
                .plan_receiver_message(Some(TransferMessageKind::Chunk)),
            ReceiverMessagePlan::IgnoreLateMessage
        );
    }

    #[test]
    fn receiver_enqueue_messageerror_pull_and_cancel_are_typed() {
        assert_eq!(
            ReceiverEnqueueOutcome::Enqueued.plan(),
            ReceiverEnqueuePlan::Continue
        );
        assert_eq!(
            ReceiverEnqueueOutcome::StreamTerminal.plan(),
            ReceiverEnqueuePlan::Continue
        );
        assert_eq!(
            ReceiverEnqueueOutcome::StrategyError.plan(),
            ReceiverEnqueuePlan::Finish
        );
        let active = TransferSnapshot::initial();
        assert_eq!(
            active.plan_receiver_message_error(),
            ReceiverMessageErrorPlan::ErrorStreamAndFinish
        );
        assert_eq!(active.plan_receiver_pull(), ReceiverPullPlan::PostPull);
        assert_eq!(
            active.plan_receiver_cancel(),
            ReceiverCancelPlan::PostErrorAndFinish
        );
        let inactive = snapshot(false, false, 0, false);
        assert_eq!(
            inactive.plan_receiver_message_error(),
            ReceiverMessageErrorPlan::Ignore
        );
        assert_eq!(inactive.plan_receiver_pull(), ReceiverPullPlan::Ignore);
        assert_eq!(inactive.plan_receiver_cancel(), ReceiverCancelPlan::Ignore);
    }
}
