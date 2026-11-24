//! Page-owned wheel queue semantics, independent of the input producer.
use std::time::{Duration, Instant};

use crate::{
    document_runtime::DomHandle,
    native_bridge::{WindowDocumentTaskTarget, element::WheelScrollTarget},
    runtime::RendererWheelDispatchOutcome,
};

const TRANSACTION_TIMEOUT: Duration = Duration::from_millis(500);
const LATCHING_SLOP: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WheelPhase {
    Unspecified,
    Began,
    Changed,
    Ended,
    /// Explicit cancellation is supplied by the native viewport adapter.
    #[cfg_attr(not(feature = "molio"), allow(dead_code))]
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WheelSource {
    /// Direct CDP wheel events supply their own Began/End pair.
    Direct,
    /// Platform wheels may supply a phase or need Aura-style inference.
    Platform,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WheelInput {
    pub position: (f64, f64),
    pub delta: (f64, f64),
    pub modifiers: u8,
    pub timestamp: Instant,
    pub phase: WheelPhase,
    pub source: WheelSource,
}

#[derive(Clone, Debug)]
pub(crate) struct WheelEventTarget {
    pub node: DomHandle,
    pub owner: WindowDocumentTaskTarget,
    pub ancestors: Vec<DomHandle>,
}

#[derive(Debug)]
struct InferredPhase {
    deadline: Instant,
    position: (f64, f64),
    delta: (f64, f64),
    modifiers: u8,
    first_scroll_consumed: Option<bool>,
}

#[derive(Debug, Default)]
pub(crate) struct WheelInputController {
    nonblocking: bool,
    document: Option<WindowDocumentTaskTarget>,
    inferred: Option<InferredPhase>,
    event_target: Option<WheelEventTarget>,
    scroll_target: Option<WheelScrollTarget>,
    generation: u64,
    event_serial: u64,
}

pub(crate) struct PreparedWheel {
    pub event_target: Option<WheelEventTarget>,
    pub scroll_target: Option<WheelScrollTarget>,
    pub cancelable: bool,
    phase: WheelPhase,
    nonzero: bool,
    generation: u64,
    event_serial: u64,
}

impl WheelInputController {
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.inferred.as_ref().map(|phase| phase.deadline)
    }

    pub(crate) fn expire(&mut self, now: Instant) {
        if self.next_deadline().is_some_and(|deadline| deadline <= now) {
            self.end_platform_sequence();
        }
    }

    /// End releases the transaction, but does not fabricate a wheel ACK.
    pub(crate) fn end_transaction(&mut self) {
        self.event_target = None;
        self.scroll_target = None;
        self.generation = self.generation.wrapping_add(1);
    }

    pub(crate) fn end_platform_sequence(&mut self) {
        self.inferred = None;
        self.end_transaction();
    }

    pub(crate) fn retire_document(&mut self) {
        self.end_platform_sequence();
        self.document = None;
    }

    pub(crate) fn prepare(
        &mut self,
        input: WheelInput,
        document: WindowDocumentTaskTarget,
    ) -> PreparedWheel {
        if self.document != Some(document) {
            self.retire_document();
            self.document = Some(document);
        }
        self.expire(input.timestamp);
        let phase = match (input.source, input.phase) {
            (WheelSource::Platform, WheelPhase::Unspecified) => self.infer_phase(input),
            (_, phase) => phase,
        };
        if phase == WheelPhase::Began {
            self.nonblocking = false;
            if input.source == WheelSource::Platform {
                if input.phase == WheelPhase::Unspecified {
                    self.end_transaction();
                } else {
                    self.end_platform_sequence();
                }
            } else {
                // Raw Began hits a fresh DOM target. Its default action still
                // belongs to the active scroll until the paired End arrives.
                self.event_target = None;
            }
        }
        self.event_serial = self.event_serial.wrapping_add(1);
        PreparedWheel {
            event_target: self.event_target.clone(),
            scroll_target: self.scroll_target.clone(),
            cancelable: !self.nonblocking,
            phase,
            nonzero: input.delta != (0.0, 0.0),
            generation: self.generation,
            event_serial: self.event_serial,
        }
    }

    pub(crate) fn acknowledge(
        &mut self,
        prepared: PreparedWheel,
        outcome: Option<RendererWheelDispatchOutcome>,
        document: Option<WindowDocumentTaskTarget>,
    ) {
        if self.event_serial != prepared.event_serial {
            return;
        }
        if self.document != document {
            self.retire_document();
        }
        let Some(outcome) = outcome else { return };
        let canceled = outcome == RendererWheelDispatchOutcome::Canceled;
        if !canceled && prepared.nonzero && prepared.phase == WheelPhase::Began {
            self.nonblocking = true;
        }
        if self.generation != prepared.generation {
            // Document retirement clears targets, independently of Page ACK mode.
            return;
        }
        self.event_target = if canceled {
            None
        } else {
            prepared.event_target
        };
        if !canceled && prepared.nonzero {
            self.scroll_target = prepared.scroll_target;
            if let Some(inferred) = &mut self.inferred {
                inferred
                    .first_scroll_consumed
                    .get_or_insert(outcome == RendererWheelDispatchOutcome::Scrolled);
            }
        }
    }

    fn infer_phase(&mut self, input: WheelInput) -> WheelPhase {
        if self.inferred.as_ref().is_some_and(|initial| {
            let distance = (input.position.0 - initial.position.0)
                .hypot(input.position.1 - initial.position.1);
            let consistent = |a: f64, b: f64| (a == 0.0 && b == 0.0) || a * b > 0.0;
            distance >= LATCHING_SLOP
                || input.modifiers != initial.modifiers
                || (initial.first_scroll_consumed == Some(false)
                    && initial.delta != (0.0, 0.0)
                    && input.delta != (0.0, 0.0)
                    && (!consistent(initial.delta.0, input.delta.0)
                        || !consistent(initial.delta.1, input.delta.1)))
        }) {
            self.end_platform_sequence();
        }
        if let Some(inferred) = &mut self.inferred {
            if inferred.delta == (0.0, 0.0) {
                inferred.delta = input.delta;
            }
            inferred.deadline = input.timestamp + TRANSACTION_TIMEOUT;
            WheelPhase::Changed
        } else {
            self.inferred = Some(InferredPhase {
                deadline: input.timestamp + TRANSACTION_TIMEOUT,
                position: input.position,
                delta: input.delta,
                modifiers: input.modifiers,
                first_scroll_consumed: None,
            });
            WheelPhase::Began
        }
    }
}
