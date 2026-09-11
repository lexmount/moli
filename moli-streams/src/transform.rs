//! Runtime-independent `TransformStream` coordination.
//!
//! The renderer owns JavaScript chunks, reasons, callbacks, promises, and the
//! controller's finish promise/resolver pair. This module owns the decisions
//! that coordinate the readable and writable sides around those values.

use crate::readable::ReadableState;
use crate::strategy::StrategySnapshot;
use crate::writable::WritableState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformMode {
    Identity,
    Callback,
    TextEncoder,
    TextDecoder,
}

impl TransformMode {
    #[must_use]
    pub fn from_storage(mode: Option<&str>, has_transformer: bool) -> Self {
        match mode {
            Some("text-encoder") => Self::TextEncoder,
            Some("text-decoder") => Self::TextDecoder,
            _ if has_transformer => Self::Callback,
            _ => Self::Identity,
        }
    }

    #[must_use]
    pub const fn write_algorithm(self) -> TransformWriteAlgorithm {
        match self {
            Self::Identity => TransformWriteAlgorithm::Identity,
            Self::Callback => TransformWriteAlgorithm::Callback,
            Self::TextEncoder => TransformWriteAlgorithm::TextEncoder,
            Self::TextDecoder => TransformWriteAlgorithm::TextDecoder,
        }
    }

    #[must_use]
    pub const fn flush_algorithm(self) -> TransformFlushAlgorithm {
        match self {
            Self::Callback => TransformFlushAlgorithm::Callback,
            Self::TextDecoder => TransformFlushAlgorithm::TextDecoder,
            Self::Identity | Self::TextEncoder => TransformFlushAlgorithm::None,
        }
    }

    #[must_use]
    pub const fn cancel_algorithm(self) -> TransformCancelAlgorithm {
        match self {
            Self::Callback => TransformCancelAlgorithm::Callback,
            Self::Identity | Self::TextEncoder | Self::TextDecoder => {
                TransformCancelAlgorithm::None
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformWriteAlgorithm {
    Identity,
    Callback,
    TextEncoder,
    TextDecoder,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformFlushAlgorithm {
    None,
    Callback,
    TextDecoder,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformCancelAlgorithm {
    None,
    Callback,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransformReadableSnapshot {
    state: ReadableState,
    pending_read_count: usize,
    pipe_registered: bool,
    strategy: StrategySnapshot,
}

impl TransformReadableSnapshot {
    #[must_use]
    pub const fn new(
        state: ReadableState,
        pending_read_count: usize,
        pipe_registered: bool,
        strategy: StrategySnapshot,
    ) -> Self {
        Self {
            state,
            pending_read_count,
            pipe_registered,
            strategy,
        }
    }

    #[must_use]
    pub const fn state(self) -> ReadableState {
        self.state
    }

    #[must_use]
    pub const fn pending_read_count(self) -> usize {
        self.pending_read_count
    }

    #[must_use]
    pub const fn pipe_registered(self) -> bool {
        self.pipe_registered
    }

    #[must_use]
    pub fn backpressure(self) -> TransformBackpressure {
        if !matches!(self.state, ReadableState::Readable) {
            return TransformBackpressure::Terminal;
        }
        if self.pending_read_count > 0 || self.pipe_registered || self.strategy.has_capacity() {
            TransformBackpressure::Ready
        } else {
            TransformBackpressure::Backpressured
        }
    }

    #[must_use]
    pub fn can_accept_chunk(self) -> bool {
        matches!(self.backpressure(), TransformBackpressure::Ready)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformBackpressure {
    Ready,
    Backpressured,
    Terminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinishResidenceState {
    Available,
    Claimed,
}

impl FinishResidenceState {
    #[must_use]
    pub const fn from_storage(has_finish_promise: bool) -> Self {
        if has_finish_promise {
            Self::Claimed
        } else {
            Self::Available
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransformSnapshot {
    readable: TransformReadableSnapshot,
    writable_state: WritableState,
    mode: TransformMode,
    start_pending: bool,
    pending_operation_count: usize,
    finish: FinishResidenceState,
}

impl TransformSnapshot {
    #[must_use]
    pub const fn new(
        readable: TransformReadableSnapshot,
        writable_state: WritableState,
        mode: TransformMode,
        start_pending: bool,
        pending_operation_count: usize,
        finish: FinishResidenceState,
    ) -> Self {
        Self {
            readable,
            writable_state,
            mode,
            start_pending,
            pending_operation_count,
            finish,
        }
    }

    #[must_use]
    pub const fn readable(self) -> TransformReadableSnapshot {
        self.readable
    }

    #[must_use]
    pub const fn writable_state(self) -> WritableState {
        self.writable_state
    }

    #[must_use]
    pub const fn mode(self) -> TransformMode {
        self.mode
    }

    #[must_use]
    pub fn plan_write_admission(self) -> TransformWriteAdmissionPlan {
        if matches!(self.mode, TransformMode::Callback)
            || self.start_pending
            || self.pending_operation_count > 0
            || !self.readable.can_accept_chunk()
        {
            TransformWriteAdmissionPlan::Queue
        } else {
            TransformWriteAdmissionPlan::Run(self.mode.write_algorithm())
        }
    }

    #[must_use]
    pub const fn plan_queued_write_algorithm(self) -> TransformWriteAlgorithm {
        self.mode.write_algorithm()
    }

    #[must_use]
    pub const fn plan_close_admission(self) -> TransformCloseAdmissionPlan {
        if self.start_pending || self.pending_operation_count > 0 {
            TransformCloseAdmissionPlan::Queue
        } else {
            TransformCloseAdmissionPlan::Run
        }
    }

    #[must_use]
    pub const fn plan_finish(self, operation: FinishOperation) -> FinishClaimPlan {
        if matches!(self.finish, FinishResidenceState::Claimed) {
            return FinishClaimPlan::Reuse;
        }
        let algorithm = match operation {
            FinishOperation::WritableClose => FinishAlgorithm::Flush(self.mode.flush_algorithm()),
            FinishOperation::ReadableCancel | FinishOperation::WritableAbort => {
                FinishAlgorithm::Cancel(self.mode.cancel_algorithm())
            }
        };
        FinishClaimPlan::Claim { algorithm }
    }

    #[must_use]
    pub const fn plan_start_settlement(self, outcome: AlgorithmOutcome) -> StartSettlementPlan {
        match outcome {
            AlgorithmOutcome::Fulfilled => StartSettlementPlan::ClearPendingAndPump,
            AlgorithmOutcome::Rejected => StartSettlementPlan::ClearPendingAndErrorBoth,
        }
    }

    #[must_use]
    pub const fn plan_write_settlement(
        self,
        outcome: AlgorithmOutcome,
        direct_write_size: bool,
    ) -> WriteSettlementPlan {
        match outcome {
            AlgorithmOutcome::Fulfilled => WriteSettlementPlan::Fulfill {
                finish_direct_write: direct_write_size,
                drain_pipe: direct_write_size,
            },
            AlgorithmOutcome::Rejected => WriteSettlementPlan::Reject {
                finish_direct_write: direct_write_size,
                error: self.plan_error(),
            },
        }
    }

    #[must_use]
    pub const fn plan_writable_close_settlement(
        self,
        outcome: AlgorithmOutcome,
    ) -> WritableCloseSettlementPlan {
        match outcome {
            AlgorithmOutcome::Fulfilled => WritableCloseSettlementPlan::MarkClosed,
            AlgorithmOutcome::Rejected => {
                WritableCloseSettlementPlan::ClearInFlightAndErrorWritable
            }
        }
    }

    #[must_use]
    pub const fn plan_error(self) -> TransformErrorPlan {
        match self.readable.state {
            ReadableState::Readable => TransformErrorPlan {
                reason: ErrorReasonSource::Provided,
                readable: ReadableErrorAction::Error,
            },
            ReadableState::Closed => TransformErrorPlan {
                reason: ErrorReasonSource::Provided,
                readable: ReadableErrorAction::Keep,
            },
            ReadableState::Errored => TransformErrorPlan {
                reason: ErrorReasonSource::ReadableStored,
                readable: ReadableErrorAction::Keep,
            },
        }
    }

    #[must_use]
    pub const fn plan_terminate(self) -> TerminatePlan {
        TerminatePlan {
            readable: if matches!(self.readable.state, ReadableState::Readable) {
                ReadableTerminateAction::Close
            } else {
                ReadableTerminateAction::Keep
            },
        }
    }

    #[must_use]
    pub const fn plan_finish_settlement(
        self,
        operation: FinishOperation,
        outcome: AlgorithmOutcome,
    ) -> FinishSettlementPlan {
        match (operation, outcome) {
            (FinishOperation::ReadableCancel, AlgorithmOutcome::Fulfilled) => {
                if matches!(self.writable_state, WritableState::Errored) {
                    FinishSettlementPlan::RejectWithWritableStoredError
                } else {
                    FinishSettlementPlan::ErrorWritableWithOriginalReasonAndResolve
                }
            }
            (FinishOperation::ReadableCancel, AlgorithmOutcome::Rejected) => {
                FinishSettlementPlan::ErrorWritableWithCallbackErrorAndReject
            }
            (FinishOperation::WritableAbort, AlgorithmOutcome::Fulfilled) => {
                if matches!(self.readable.state, ReadableState::Errored) {
                    FinishSettlementPlan::RejectWithReadableStoredError
                } else {
                    FinishSettlementPlan::ErrorReadableWithOriginalReasonAndResolve
                }
            }
            (FinishOperation::WritableAbort, AlgorithmOutcome::Rejected) => {
                FinishSettlementPlan::ErrorReadableWithCallbackErrorAndReject
            }
            (FinishOperation::WritableClose, AlgorithmOutcome::Fulfilled) => {
                match self.readable.state {
                    ReadableState::Errored => FinishSettlementPlan::RejectWithReadableStoredError,
                    ReadableState::Readable => FinishSettlementPlan::CloseReadableAndResolve,
                    ReadableState::Closed => FinishSettlementPlan::Resolve,
                }
            }
            (FinishOperation::WritableClose, AlgorithmOutcome::Rejected) => {
                FinishSettlementPlan::ErrorReadableWithCallbackErrorAndReject
            }
        }
    }

    #[must_use]
    pub const fn plan_finish_setup_failure(
        self,
        operation: FinishOperation,
    ) -> FinishSetupFailurePlan {
        match operation {
            FinishOperation::ReadableCancel => {
                FinishSetupFailurePlan::ErrorWritableWithOriginalReasonAndReject
            }
            FinishOperation::WritableAbort => {
                FinishSetupFailurePlan::ErrorReadableWithOriginalReasonAndReject
            }
            FinishOperation::WritableClose => {
                FinishSetupFailurePlan::ErrorReadableWithUndefinedAndReject
            }
        }
    }

    #[must_use]
    pub const fn plan_enqueue_failure(
        self,
        failure: TransformEnqueueFailure,
    ) -> TransformEnqueueFailurePlan {
        match failure {
            TransformEnqueueFailure::ClosedOrErrored => TransformEnqueueFailurePlan {
                returned_error: EnqueueErrorSource::SynthesizedTypeError,
                propagation: self.plan_error(),
            },
            TransformEnqueueFailure::Strategy => {
                let propagation = self.plan_error();
                TransformEnqueueFailurePlan {
                    returned_error: match propagation.reason {
                        ErrorReasonSource::Provided => EnqueueErrorSource::Provided,
                        ErrorReasonSource::ReadableStored => EnqueueErrorSource::ReadableStored,
                    },
                    propagation,
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformWriteAdmissionPlan {
    Queue,
    Run(TransformWriteAlgorithm),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformCloseAdmissionPlan {
    Queue,
    Run,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinishOperation {
    ReadableCancel,
    WritableAbort,
    WritableClose,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinishAlgorithm {
    Cancel(TransformCancelAlgorithm),
    Flush(TransformFlushAlgorithm),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinishClaimPlan {
    Reuse,
    Claim { algorithm: FinishAlgorithm },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlgorithmOutcome {
    Fulfilled,
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartSettlementPlan {
    ClearPendingAndPump,
    ClearPendingAndErrorBoth,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteSettlementPlan {
    Fulfill {
        finish_direct_write: bool,
        drain_pipe: bool,
    },
    Reject {
        finish_direct_write: bool,
        error: TransformErrorPlan,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WritableCloseSettlementPlan {
    MarkClosed,
    ClearInFlightAndErrorWritable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorReasonSource {
    Provided,
    ReadableStored,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadableErrorAction {
    Keep,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransformErrorPlan {
    reason: ErrorReasonSource,
    readable: ReadableErrorAction,
}

impl TransformErrorPlan {
    #[must_use]
    pub const fn reason(self) -> ErrorReasonSource {
        self.reason
    }

    #[must_use]
    pub const fn readable(self) -> ReadableErrorAction {
        self.readable
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadableTerminateAction {
    Keep,
    Close,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminatePlan {
    readable: ReadableTerminateAction,
}

impl TerminatePlan {
    #[must_use]
    pub const fn readable(self) -> ReadableTerminateAction {
        self.readable
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinishSettlementPlan {
    Resolve,
    CloseReadableAndResolve,
    ErrorWritableWithOriginalReasonAndResolve,
    ErrorReadableWithOriginalReasonAndResolve,
    ErrorWritableWithCallbackErrorAndReject,
    ErrorReadableWithCallbackErrorAndReject,
    RejectWithWritableStoredError,
    RejectWithReadableStoredError,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinishSetupFailurePlan {
    ErrorWritableWithOriginalReasonAndReject,
    ErrorReadableWithOriginalReasonAndReject,
    ErrorReadableWithUndefinedAndReject,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformEnqueueFailure {
    ClosedOrErrored,
    Strategy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnqueueErrorSource {
    SynthesizedTypeError,
    Provided,
    ReadableStored,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransformEnqueueFailurePlan {
    returned_error: EnqueueErrorSource,
    propagation: TransformErrorPlan,
}

impl TransformEnqueueFailurePlan {
    #[must_use]
    pub const fn returned_error(self) -> EnqueueErrorSource {
        self.returned_error
    }

    #[must_use]
    pub const fn propagation(self) -> TransformErrorPlan {
        self.propagation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn readable(
        state: ReadableState,
        pending_reads: usize,
        pipe_registered: bool,
        high_water_mark: f64,
        total_size: f64,
    ) -> TransformReadableSnapshot {
        TransformReadableSnapshot::new(
            state,
            pending_reads,
            pipe_registered,
            StrategySnapshot::new(high_water_mark, total_size),
        )
    }

    fn snapshot(
        readable: TransformReadableSnapshot,
        writable_state: WritableState,
        mode: TransformMode,
    ) -> TransformSnapshot {
        TransformSnapshot::new(
            readable,
            writable_state,
            mode,
            false,
            0,
            FinishResidenceState::Available,
        )
    }

    #[test]
    fn storage_mode_owns_write_flush_and_cancel_routing() {
        let cases = [
            (
                None,
                false,
                TransformMode::Identity,
                TransformWriteAlgorithm::Identity,
                TransformFlushAlgorithm::None,
                TransformCancelAlgorithm::None,
            ),
            (
                None,
                true,
                TransformMode::Callback,
                TransformWriteAlgorithm::Callback,
                TransformFlushAlgorithm::Callback,
                TransformCancelAlgorithm::Callback,
            ),
            (
                Some("text-encoder"),
                true,
                TransformMode::TextEncoder,
                TransformWriteAlgorithm::TextEncoder,
                TransformFlushAlgorithm::None,
                TransformCancelAlgorithm::None,
            ),
            (
                Some("text-decoder"),
                false,
                TransformMode::TextDecoder,
                TransformWriteAlgorithm::TextDecoder,
                TransformFlushAlgorithm::TextDecoder,
                TransformCancelAlgorithm::None,
            ),
        ];
        for (stored, has_transformer, mode, write, flush, cancel) in cases {
            let decoded = TransformMode::from_storage(stored, has_transformer);
            assert_eq!(decoded, mode);
            assert_eq!(decoded.write_algorithm(), write);
            assert_eq!(decoded.flush_algorithm(), flush);
            assert_eq!(decoded.cancel_algorithm(), cancel);
        }
    }

    #[test]
    fn readable_backpressure_combines_lifecycle_demand_pipe_and_strategy() {
        assert_eq!(
            readable(ReadableState::Closed, 1, true, 1.0, 0.0).backpressure(),
            TransformBackpressure::Terminal
        );
        assert_eq!(
            readable(ReadableState::Readable, 1, false, 0.0, 0.0).backpressure(),
            TransformBackpressure::Ready
        );
        assert_eq!(
            readable(ReadableState::Readable, 0, true, 0.0, 0.0).backpressure(),
            TransformBackpressure::Ready
        );
        assert_eq!(
            readable(ReadableState::Readable, 0, false, 2.0, 1.0).backpressure(),
            TransformBackpressure::Ready
        );
        assert_eq!(
            readable(ReadableState::Readable, 0, false, 1.0, 1.0).backpressure(),
            TransformBackpressure::Backpressured
        );
    }

    #[test]
    fn write_and_close_admission_cover_callback_start_queue_and_capacity() {
        let ready = readable(ReadableState::Readable, 1, false, 0.0, 0.0);
        assert_eq!(
            snapshot(ready, WritableState::Writable, TransformMode::Identity)
                .plan_write_admission(),
            TransformWriteAdmissionPlan::Run(TransformWriteAlgorithm::Identity)
        );
        assert_eq!(
            snapshot(ready, WritableState::Writable, TransformMode::Callback)
                .plan_write_admission(),
            TransformWriteAdmissionPlan::Queue
        );

        for (start_pending, pending_count, backpressured) in
            [(true, 0, false), (false, 1, false), (false, 0, true)]
        {
            let readable = if backpressured {
                readable(ReadableState::Readable, 0, false, 0.0, 0.0)
            } else {
                ready
            };
            let current = TransformSnapshot::new(
                readable,
                WritableState::Writable,
                TransformMode::Identity,
                start_pending,
                pending_count,
                FinishResidenceState::Available,
            );
            assert_eq!(
                current.plan_write_admission(),
                TransformWriteAdmissionPlan::Queue
            );
            assert_eq!(
                current.plan_close_admission(),
                if start_pending || pending_count > 0 {
                    TransformCloseAdmissionPlan::Queue
                } else {
                    TransformCloseAdmissionPlan::Run
                }
            );
        }
    }

    #[test]
    fn finish_residence_is_claimed_once_with_operation_specific_algorithm() {
        let available = snapshot(
            readable(ReadableState::Readable, 0, false, 1.0, 0.0),
            WritableState::Writable,
            TransformMode::Callback,
        );
        assert_eq!(
            available.plan_finish(FinishOperation::ReadableCancel),
            FinishClaimPlan::Claim {
                algorithm: FinishAlgorithm::Cancel(TransformCancelAlgorithm::Callback),
            }
        );
        assert_eq!(
            available.plan_finish(FinishOperation::WritableClose),
            FinishClaimPlan::Claim {
                algorithm: FinishAlgorithm::Flush(TransformFlushAlgorithm::Callback),
            }
        );
        let claimed = TransformSnapshot::new(
            available.readable(),
            available.writable_state(),
            available.mode(),
            false,
            0,
            FinishResidenceState::Claimed,
        );
        assert_eq!(
            claimed.plan_finish(FinishOperation::WritableAbort),
            FinishClaimPlan::Reuse
        );
    }

    #[test]
    fn transform_error_preserves_the_first_readable_error() {
        let plan = snapshot(
            readable(ReadableState::Errored, 0, false, 1.0, 0.0),
            WritableState::Writable,
            TransformMode::Identity,
        )
        .plan_error();
        assert_eq!(plan.reason(), ErrorReasonSource::ReadableStored);
        assert_eq!(plan.readable(), ReadableErrorAction::Keep);

        let plan = snapshot(
            readable(ReadableState::Readable, 0, false, 1.0, 0.0),
            WritableState::Writable,
            TransformMode::Identity,
        )
        .plan_error();
        assert_eq!(plan.reason(), ErrorReasonSource::Provided);
        assert_eq!(plan.readable(), ReadableErrorAction::Error);
    }

    #[test]
    fn terminal_settlements_cover_both_sides_and_all_outcomes() {
        let readable_snapshot = readable(ReadableState::Readable, 0, false, 1.0, 0.0);
        let writable = snapshot(
            readable_snapshot,
            WritableState::Writable,
            TransformMode::Callback,
        );
        assert_eq!(
            writable.plan_finish_settlement(
                FinishOperation::ReadableCancel,
                AlgorithmOutcome::Fulfilled,
            ),
            FinishSettlementPlan::ErrorWritableWithOriginalReasonAndResolve
        );
        assert_eq!(
            writable.plan_finish_settlement(
                FinishOperation::ReadableCancel,
                AlgorithmOutcome::Rejected,
            ),
            FinishSettlementPlan::ErrorWritableWithCallbackErrorAndReject
        );
        assert_eq!(
            writable.plan_finish_settlement(
                FinishOperation::WritableAbort,
                AlgorithmOutcome::Fulfilled,
            ),
            FinishSettlementPlan::ErrorReadableWithOriginalReasonAndResolve
        );
        assert_eq!(
            writable.plan_finish_settlement(
                FinishOperation::WritableClose,
                AlgorithmOutcome::Fulfilled,
            ),
            FinishSettlementPlan::CloseReadableAndResolve
        );

        let readable_error = snapshot(
            readable(ReadableState::Errored, 0, false, 1.0, 0.0),
            WritableState::Writable,
            TransformMode::Callback,
        );
        assert_eq!(
            readable_error.plan_finish_settlement(
                FinishOperation::WritableClose,
                AlgorithmOutcome::Fulfilled,
            ),
            FinishSettlementPlan::RejectWithReadableStoredError
        );
        let writable_error = snapshot(
            readable_snapshot,
            WritableState::Errored,
            TransformMode::Callback,
        );
        assert_eq!(
            writable_error.plan_finish_settlement(
                FinishOperation::ReadableCancel,
                AlgorithmOutcome::Fulfilled,
            ),
            FinishSettlementPlan::RejectWithWritableStoredError
        );

        let writable_erroring = snapshot(
            readable_snapshot,
            WritableState::Erroring,
            TransformMode::Callback,
        );
        assert_eq!(
            writable_erroring.plan_finish_settlement(
                FinishOperation::ReadableCancel,
                AlgorithmOutcome::Fulfilled,
            ),
            FinishSettlementPlan::ErrorWritableWithOriginalReasonAndResolve
        );
    }

    #[test]
    fn setup_failures_and_enqueue_failures_preserve_error_ownership() {
        let current = snapshot(
            readable(ReadableState::Readable, 0, false, 1.0, 0.0),
            WritableState::Writable,
            TransformMode::Identity,
        );
        assert_eq!(
            current.plan_finish_setup_failure(FinishOperation::ReadableCancel),
            FinishSetupFailurePlan::ErrorWritableWithOriginalReasonAndReject
        );
        assert_eq!(
            current.plan_finish_setup_failure(FinishOperation::WritableClose),
            FinishSetupFailurePlan::ErrorReadableWithUndefinedAndReject
        );
        let closed = current.plan_enqueue_failure(TransformEnqueueFailure::ClosedOrErrored);
        assert_eq!(
            closed.returned_error(),
            EnqueueErrorSource::SynthesizedTypeError
        );

        let errored = snapshot(
            readable(ReadableState::Errored, 0, false, 1.0, 0.0),
            WritableState::Writable,
            TransformMode::Identity,
        )
        .plan_enqueue_failure(TransformEnqueueFailure::Strategy);
        assert_eq!(errored.returned_error(), EnqueueErrorSource::ReadableStored);
        assert_eq!(
            errored.propagation().reason(),
            ErrorReasonSource::ReadableStored
        );
    }

    #[test]
    fn start_write_and_terminate_plans_are_explicit() {
        let current = snapshot(
            readable(ReadableState::Readable, 0, false, 1.0, 0.0),
            WritableState::Writable,
            TransformMode::Identity,
        );
        assert_eq!(
            current.plan_start_settlement(AlgorithmOutcome::Fulfilled),
            StartSettlementPlan::ClearPendingAndPump
        );
        assert_eq!(
            current.plan_start_settlement(AlgorithmOutcome::Rejected),
            StartSettlementPlan::ClearPendingAndErrorBoth
        );
        assert_eq!(
            current.plan_write_settlement(AlgorithmOutcome::Fulfilled, true),
            WriteSettlementPlan::Fulfill {
                finish_direct_write: true,
                drain_pipe: true,
            }
        );
        assert_eq!(
            current.plan_write_settlement(AlgorithmOutcome::Rejected, false),
            WriteSettlementPlan::Reject {
                finish_direct_write: false,
                error: current.plan_error(),
            }
        );
        assert_eq!(
            current.plan_writable_close_settlement(AlgorithmOutcome::Fulfilled),
            WritableCloseSettlementPlan::MarkClosed
        );
        assert_eq!(
            current.plan_writable_close_settlement(AlgorithmOutcome::Rejected),
            WritableCloseSettlementPlan::ClearInFlightAndErrorWritable
        );
        assert_eq!(
            current.plan_terminate().readable(),
            ReadableTerminateAction::Close
        );
    }
}
