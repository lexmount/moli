use std::time::{Duration, Instant};

const TRANSACTION_TIMEOUT: Duration = Duration::from_millis(500);
const LATCHING_SLOP: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelPhase {
    Unspecified,
    Began,
    Changed,
    Ended,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelSource {
    /// Direct CDP wheel events supply their own Began/End pair.
    Direct,
    /// Platform wheels may supply a phase or need Aura-style inference.
    Platform,
    /// Synthetic wheels use platform phase inference and identify their producer round.
    Synthetic(u64),
}

#[derive(Clone, Copy, Debug)]
pub struct WheelInput {
    pub position: (f64, f64),
    pub delta: (f64, f64),
    pub modifiers: u8,
    pub timestamp: Instant,
    pub phase: WheelPhase,
    pub source: WheelSource,
}

/// The result of dispatching a wheel event to a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelDispatchOutcome {
    Canceled,
    Scrolled,
    Unconsumed,
}

#[derive(Debug)]
struct InferredPhase {
    source: WheelSource,
    deadline: Instant,
    position: (f64, f64),
    delta: (f64, f64),
    modifiers: u8,
    first_scroll_consumed: Option<bool>,
}

/// Advances wheel transactions using document identities and targets supplied by the host.
#[derive(Debug)]
pub struct WheelInputController<D, E, S> {
    nonblocking: bool,
    document: Option<D>,
    inferred: Option<InferredPhase>,
    event_target: Option<E>,
    scroll_target: Option<S>,
    generation: u64,
    event_serial: u64,
}

/// A prepared dispatch with targets the host can validate or replace before acknowledging it.
pub struct PreparedWheel<E, S> {
    pub event_target: Option<E>,
    pub scroll_target: Option<S>,
    pub cancelable: bool,
    phase: WheelPhase,
    nonzero: bool,
    generation: u64,
    event_serial: u64,
}

impl<D, E, S> Default for WheelInputController<D, E, S> {
    fn default() -> Self {
        Self {
            nonblocking: false,
            document: None,
            inferred: None,
            event_target: None,
            scroll_target: None,
            generation: 0,
            event_serial: 0,
        }
    }
}

impl<D, E, S> WheelInputController<D, E, S>
where
    D: Copy + Eq,
    E: Clone,
    S: Clone,
{
    /// The deadline of an inferred platform sequence.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.inferred.as_ref().map(|phase| phase.deadline)
    }

    /// End the inferred platform sequence once its deadline is reached.
    pub fn expire(&mut self, now: Instant) {
        if self.next_deadline().is_some_and(|deadline| deadline <= now) {
            self.end_platform_sequence();
        }
    }

    /// Release the event target and scroll latch while preserving queue mode.
    pub fn end_transaction(&mut self) {
        self.event_target = None;
        self.scroll_target = None;
        self.generation = self.generation.wrapping_add(1);
    }

    /// Release the transaction and its inferred phase deadline.
    pub fn end_platform_sequence(&mut self) {
        self.inferred = None;
        self.end_transaction();
    }

    /// Release an interrupted producer's sequence if it still owns the current phase.
    pub fn cancel_synthetic_round(&mut self, document: D, round: u64) {
        if self.document == Some(document)
            && self
                .inferred
                .as_ref()
                .is_some_and(|phase| phase.source == WheelSource::Synthetic(round))
        {
            self.end_platform_sequence();
        }
    }

    /// Retire the current document and its platform sequence.
    pub fn retire_document(&mut self) {
        self.end_platform_sequence();
        self.document = None;
    }

    /// Resolve the phase and prepare a dispatch for the given document.
    pub fn prepare(&mut self, input: WheelInput, document: D) -> PreparedWheel<E, S> {
        if self.document != Some(document) {
            self.retire_document();
            self.document = Some(document);
        }
        self.expire(input.timestamp);
        let platform = input.source != WheelSource::Direct;
        let phase = if platform && input.phase == WheelPhase::Unspecified {
            self.infer_phase(input)
        } else {
            input.phase
        };
        if phase == WheelPhase::Began {
            self.nonblocking = false;
            if platform {
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
        // A later platform update can take over the shared sequence. Direct
        // CDP wheels leave the platform producer and its timer in place.
        if platform && let Some(inferred) = &mut self.inferred {
            inferred.source = input.source;
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

    /// Commit the dispatch result and the host-validated targets.
    pub fn acknowledge(
        &mut self,
        prepared: PreparedWheel<E, S>,
        outcome: Option<WheelDispatchOutcome>,
        document: Option<D>,
    ) {
        if self.event_serial != prepared.event_serial {
            return;
        }
        if self.document != document {
            self.retire_document();
        }
        let Some(outcome) = outcome else { return };
        let canceled = outcome == WheelDispatchOutcome::Canceled;
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
                    .get_or_insert(outcome == WheelDispatchOutcome::Scrolled);
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
                source: input.source,
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

#[cfg(test)]
#[path = "wheel_tests.rs"]
mod tests;
