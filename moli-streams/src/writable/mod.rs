//! Runtime-independent `WritableStream` lifecycle and request planning.
//!
//! Stored errors, promises, sink/transformer objects, and queued JavaScript
//! payloads remain adapter-owned. This module only receives primitive facts
//! decoded from that storage and returns typed actions.

pub mod default_controller;
pub mod erroring;
pub mod writer;

use crate::strategy::StrategySnapshot;
pub use erroring::{
    AbortPlan, CloseOutcome, CloseSettlementPlan, ErrorPlan, ErroringSnapshot, FinishErroringPlan,
    PendingAbortState,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WritableState {
    Writable,
    Erroring,
    Closed,
    Errored,
}

impl WritableState {
    #[must_use]
    pub const fn from_storage(closed: bool, erroring: bool, errored: bool) -> Self {
        if errored {
            Self::Errored
        } else if erroring {
            Self::Erroring
        } else if closed {
            Self::Closed
        } else {
            Self::Writable
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WritableKind {
    Sink,
    Transform,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WritableAccessSnapshot {
    locked: bool,
}

impl WritableAccessSnapshot {
    #[must_use]
    pub const fn new(locked: bool) -> Self {
        Self { locked }
    }

    #[must_use]
    pub const fn locked(self) -> bool {
        self.locked
    }

    #[must_use]
    pub const fn plan_acquire(self) -> AcquireWriterPlan {
        if self.locked {
            AcquireWriterPlan::RejectLocked
        } else {
            AcquireWriterPlan::Acquire
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WritableSnapshot {
    state: WritableState,
    kind: WritableKind,
    locked: bool,
    close_requested: bool,
    start_pending: bool,
    pending_write_count: usize,
    transform_close_in_flight: bool,
    operation_in_flight: bool,
    pending_abort: PendingAbortState,
    strategy: StrategySnapshot,
}

impl WritableSnapshot {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        state: WritableState,
        kind: WritableKind,
        locked: bool,
        close_requested: bool,
        start_pending: bool,
        pending_write_count: usize,
        transform_close_in_flight: bool,
        operation_in_flight: bool,
        pending_abort: PendingAbortState,
        strategy: StrategySnapshot,
    ) -> Self {
        Self {
            state,
            kind,
            locked,
            close_requested,
            start_pending,
            pending_write_count,
            transform_close_in_flight,
            operation_in_flight,
            pending_abort,
            strategy,
        }
    }

    #[must_use]
    pub const fn state(self) -> WritableState {
        self.state
    }

    #[must_use]
    pub const fn kind(self) -> WritableKind {
        self.kind
    }

    #[must_use]
    pub const fn locked(self) -> bool {
        self.locked
    }

    #[must_use]
    pub const fn close_requested(self) -> bool {
        self.close_requested
    }

    #[must_use]
    pub const fn start_pending(self) -> bool {
        self.start_pending
    }

    #[must_use]
    pub const fn pending_write_count(self) -> usize {
        self.pending_write_count
    }

    #[must_use]
    pub const fn strategy(self) -> StrategySnapshot {
        self.strategy
    }

    #[must_use]
    pub const fn erroring(self) -> ErroringSnapshot {
        ErroringSnapshot::new(
            self.state,
            self.start_pending,
            self.operation_in_flight || self.transform_close_in_flight,
            self.pending_abort,
        )
    }

    #[must_use]
    pub const fn plan_acquire_writer(self) -> AcquireWriterPlan {
        if self.locked {
            AcquireWriterPlan::RejectLocked
        } else {
            AcquireWriterPlan::Acquire
        }
    }

    #[must_use]
    pub const fn plan_unlocked_abort_entry(self) -> UnlockedEntryPlan {
        if self.locked {
            UnlockedEntryPlan::RejectLocked
        } else {
            UnlockedEntryPlan::Continue
        }
    }

    #[must_use]
    pub const fn plan_unlocked_close_entry(self) -> UnlockedCloseEntryPlan {
        if self.locked {
            UnlockedCloseEntryPlan::RejectLocked
        } else if matches!(self.state, WritableState::Errored) {
            UnlockedCloseEntryPlan::RejectErrored
        } else {
            UnlockedCloseEntryPlan::Continue
        }
    }

    #[must_use]
    pub const fn plan_writer_write_entry(self) -> WriterWriteEntryPlan {
        if matches!(self.state, WritableState::Errored)
            || (matches!(self.state, WritableState::Erroring) && !self.close_requested)
        {
            WriterWriteEntryPlan::RejectStoredError
        } else {
            WriterWriteEntryPlan::Continue
        }
    }

    #[must_use]
    pub const fn plan_internal_write_entry(self) -> InternalWriteEntryPlan {
        match self.plan_writer_write_entry() {
            WriterWriteEntryPlan::RejectStoredError => InternalWriteEntryPlan::RejectStoredError,
            WriterWriteEntryPlan::Continue => InternalWriteEntryPlan::Continue,
        }
    }

    /// Revalidates the writer and stream after the strategy size algorithm has
    /// run arbitrary JavaScript. The ordering matches
    /// WritableStreamDefaultWriterWrite: ownership, terminal error, close,
    /// then a pending error.
    #[must_use]
    pub const fn plan_write_after_size(self, owner_is_current: bool) -> WriteAfterSizePlan {
        if !owner_is_current {
            WriteAfterSizePlan::RejectReleasedWriter
        } else if matches!(self.state, WritableState::Errored) {
            WriteAfterSizePlan::RejectStoredError
        } else if self.close_requested || matches!(self.state, WritableState::Closed) {
            WriteAfterSizePlan::RejectClosingOrClosed
        } else if matches!(self.state, WritableState::Erroring) {
            WriteAfterSizePlan::RejectStoredError
        } else {
            WriteAfterSizePlan::Continue
        }
    }

    #[must_use]
    pub const fn plan_write_route(self) -> WriteRoutePlan {
        match self.kind {
            WritableKind::Transform => WriteRoutePlan::Transform,
            WritableKind::Sink => WriteRoutePlan::QueueSink,
        }
    }

    #[must_use]
    pub const fn plan_close(self) -> ClosePlan {
        if self.close_requested || matches!(self.state, WritableState::Closed) {
            return ClosePlan::RejectAlreadyRequested;
        }
        if matches!(self.state, WritableState::Errored) {
            return ClosePlan::RejectTerminal;
        }
        if matches!(self.state, WritableState::Erroring) {
            return ClosePlan::RequestAndQueueForErroring;
        }
        match self.kind {
            WritableKind::Transform => ClosePlan::RequestTransform,
            WritableKind::Sink => ClosePlan::RequestAndQueueSink,
        }
    }

    /// Plans the internal close used by `ReadableStreamPipeTo` after the
    /// source closes. Unlike the public `close()` operation, an already
    /// queued close or an already closed destination is a successful no-op.
    #[must_use]
    pub const fn plan_close_with_error_propagation(self) -> CloseWithErrorPropagationPlan {
        if self.close_requested || matches!(self.state, WritableState::Closed) {
            CloseWithErrorPropagationPlan::Resolve
        } else if matches!(self.state, WritableState::Erroring | WritableState::Errored) {
            CloseWithErrorPropagationPlan::RejectStoredError
        } else {
            CloseWithErrorPropagationPlan::Close
        }
    }

    #[must_use]
    pub const fn plan_abort(self) -> AbortPlan {
        self.erroring().plan_abort()
    }

    #[must_use]
    pub const fn plan_error(self) -> ErrorPlan {
        self.erroring().plan_error()
    }

    #[must_use]
    pub const fn plan_finish_erroring(self) -> FinishErroringPlan {
        self.erroring().plan_finish()
    }

    #[must_use]
    pub fn plan_desired_size(self) -> DesiredSizePlan {
        match self.state {
            WritableState::Erroring | WritableState::Errored => DesiredSizePlan::Null,
            WritableState::Closed => DesiredSizePlan::Zero,
            WritableState::Writable => DesiredSizePlan::Value(self.strategy.desired_size()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcquireWriterPlan {
    Acquire,
    RejectLocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnlockedEntryPlan {
    Continue,
    RejectLocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnlockedCloseEntryPlan {
    Continue,
    RejectLocked,
    RejectErrored,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriterWriteEntryPlan {
    Continue,
    RejectStoredError,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InternalWriteEntryPlan {
    Continue,
    RejectStoredError,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteAfterSizePlan {
    Continue,
    RejectReleasedWriter,
    RejectStoredError,
    RejectClosingOrClosed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteRoutePlan {
    Transform,
    QueueSink,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClosePlan {
    RejectAlreadyRequested,
    RejectTerminal,
    RequestAndQueueForErroring,
    RequestTransform,
    RequestAndQueueSink,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseWithErrorPropagationPlan {
    Resolve,
    RejectStoredError,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DesiredSizePlan {
    Null,
    Zero,
    Value(f64),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(state: WritableState, kind: WritableKind) -> WritableSnapshot {
        WritableSnapshot::new(
            state,
            kind,
            false,
            false,
            false,
            0,
            false,
            false,
            PendingAbortState::None,
            StrategySnapshot::new(2.0, 0.5),
        )
    }

    #[test]
    fn storage_state_prioritizes_terminal_errors() {
        assert_eq!(
            WritableState::from_storage(false, false, false),
            WritableState::Writable
        );
        assert_eq!(
            WritableState::from_storage(true, false, false),
            WritableState::Closed
        );
        assert_eq!(
            WritableState::from_storage(false, true, false),
            WritableState::Erroring
        );
        assert_eq!(
            WritableState::from_storage(true, true, true),
            WritableState::Errored
        );
    }

    #[test]
    fn close_routes_the_whole_writable_surface() {
        assert_eq!(
            snapshot(WritableState::Closed, WritableKind::Sink).plan_close(),
            ClosePlan::RejectAlreadyRequested
        );
        assert_eq!(
            snapshot(WritableState::Errored, WritableKind::Sink).plan_close(),
            ClosePlan::RejectTerminal
        );
        assert_eq!(
            snapshot(WritableState::Writable, WritableKind::Sink).plan_close(),
            ClosePlan::RequestAndQueueSink
        );
        assert_eq!(
            snapshot(WritableState::Writable, WritableKind::Transform).plan_close(),
            ClosePlan::RequestTransform
        );

        let queued = WritableSnapshot::new(
            WritableState::Writable,
            WritableKind::Transform,
            false,
            false,
            true,
            2,
            false,
            false,
            PendingAbortState::None,
            StrategySnapshot::new(1.0, 0.0),
        );
        assert_eq!(queued.plan_close(), ClosePlan::RequestTransform);

        assert_eq!(
            snapshot(WritableState::Closed, WritableKind::Sink).plan_close_with_error_propagation(),
            CloseWithErrorPropagationPlan::Resolve
        );
        assert_eq!(
            snapshot(WritableState::Errored, WritableKind::Sink)
                .plan_close_with_error_propagation(),
            CloseWithErrorPropagationPlan::RejectStoredError
        );
        assert_eq!(
            snapshot(WritableState::Writable, WritableKind::Sink)
                .plan_close_with_error_propagation(),
            CloseWithErrorPropagationPlan::Close
        );

        let closing = WritableSnapshot::new(
            WritableState::Writable,
            WritableKind::Sink,
            false,
            true,
            false,
            0,
            false,
            false,
            PendingAbortState::None,
            StrategySnapshot::new(1.0, 0.0),
        );
        assert_eq!(
            closing.plan_close_with_error_propagation(),
            CloseWithErrorPropagationPlan::Resolve
        );
    }

    #[test]
    fn write_abort_and_error_preserve_current_lifecycle() {
        let writable = snapshot(WritableState::Writable, WritableKind::Transform);
        assert_eq!(writable.plan_write_route(), WriteRoutePlan::Transform);
        assert_eq!(
            writable.plan_abort(),
            AbortPlan::CreatePending {
                was_already_erroring: false,
                start_erroring: true,
            }
        );
        assert_eq!(writable.plan_desired_size(), DesiredSizePlan::Value(1.5));

        let errored_transform = snapshot(WritableState::Errored, WritableKind::Transform);
        assert_eq!(errored_transform.plan_abort(), AbortPlan::Resolve);
        assert_eq!(
            errored_transform.plan_internal_write_entry(),
            InternalWriteEntryPlan::RejectStoredError
        );
        assert_eq!(errored_transform.plan_desired_size(), DesiredSizePlan::Null);

        let errored_sink = snapshot(WritableState::Errored, WritableKind::Sink);
        assert_eq!(errored_sink.plan_abort(), AbortPlan::Resolve);
        assert_eq!(errored_sink.plan_error(), ErrorPlan::Ignore);

        let erroring_sink = snapshot(WritableState::Erroring, WritableKind::Sink);
        assert_eq!(erroring_sink.plan_desired_size(), DesiredSizePlan::Null);
    }

    #[test]
    fn lock_and_writer_error_entrypoints_are_independent() {
        let locked = WritableSnapshot::new(
            WritableState::Writable,
            WritableKind::Sink,
            true,
            false,
            false,
            0,
            false,
            false,
            PendingAbortState::None,
            StrategySnapshot::new(1.0, 0.0),
        );
        assert_eq!(
            locked.plan_acquire_writer(),
            AcquireWriterPlan::RejectLocked
        );
        assert_eq!(
            locked.plan_unlocked_abort_entry(),
            UnlockedEntryPlan::RejectLocked
        );
        assert_eq!(
            locked.plan_unlocked_close_entry(),
            UnlockedCloseEntryPlan::RejectLocked
        );

        assert_eq!(
            snapshot(WritableState::Writable, WritableKind::Sink).plan_writer_write_entry(),
            WriterWriteEntryPlan::Continue
        );
        assert_eq!(
            snapshot(WritableState::Writable, WritableKind::Sink).plan_internal_write_entry(),
            InternalWriteEntryPlan::Continue
        );
    }

    #[test]
    fn post_size_write_revalidation_preserves_spec_order() {
        let writable = snapshot(WritableState::Writable, WritableKind::Sink);
        assert_eq!(
            writable.plan_write_after_size(true),
            WriteAfterSizePlan::Continue
        );
        assert_eq!(
            writable.plan_write_after_size(false),
            WriteAfterSizePlan::RejectReleasedWriter
        );

        let errored = snapshot(WritableState::Errored, WritableKind::Sink);
        assert_eq!(
            errored.plan_write_after_size(true),
            WriteAfterSizePlan::RejectStoredError
        );
        assert_eq!(
            errored.plan_write_after_size(false),
            WriteAfterSizePlan::RejectReleasedWriter
        );

        for state in [WritableState::Writable, WritableState::Erroring] {
            let closing = WritableSnapshot::new(
                state,
                WritableKind::Sink,
                true,
                true,
                false,
                0,
                false,
                false,
                PendingAbortState::None,
                StrategySnapshot::new(1.0, 0.0),
            );
            assert_eq!(
                closing.plan_writer_write_entry(),
                WriterWriteEntryPlan::Continue
            );
            assert_eq!(
                closing.plan_internal_write_entry(),
                InternalWriteEntryPlan::Continue
            );
            assert_eq!(
                closing.plan_write_after_size(true),
                WriteAfterSizePlan::RejectClosingOrClosed
            );
        }
        assert_eq!(
            snapshot(WritableState::Closed, WritableKind::Sink).plan_write_after_size(true),
            WriteAfterSizePlan::RejectClosingOrClosed
        );
    }

    #[test]
    fn sink_close_always_uses_the_controller_queue() {
        for start_pending in [false, true] {
            for pending_write_count in [0, 1] {
                for kind in [WritableKind::Sink, WritableKind::Transform] {
                    let current = WritableSnapshot::new(
                        WritableState::Writable,
                        kind,
                        false,
                        false,
                        start_pending,
                        pending_write_count,
                        false,
                        false,
                        PendingAbortState::None,
                        StrategySnapshot::new(1.0, 0.0),
                    );
                    let expected = match kind {
                        WritableKind::Sink => ClosePlan::RequestAndQueueSink,
                        WritableKind::Transform => ClosePlan::RequestTransform,
                    };
                    assert_eq!(current.plan_close(), expected);
                }
            }
        }
    }

    #[test]
    fn transform_error_projection_waits_for_close_residence() {
        let snapshot = |state, in_flight| {
            WritableSnapshot::new(
                state,
                WritableKind::Transform,
                false,
                false,
                false,
                0,
                in_flight,
                false,
                PendingAbortState::None,
                StrategySnapshot::new(1.0, 0.0),
            )
        };
        assert_eq!(
            snapshot(WritableState::Writable, true).plan_error(),
            ErrorPlan::Start {
                finish_immediately: false,
            }
        );
        assert_eq!(
            snapshot(WritableState::Errored, true).plan_error(),
            ErrorPlan::Ignore
        );
        assert_eq!(
            snapshot(WritableState::Errored, false).plan_error(),
            ErrorPlan::Ignore
        );
    }
}
