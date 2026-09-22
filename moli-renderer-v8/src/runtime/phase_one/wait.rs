//! Phase-one restoration facts passed across the Page-slot admission boundary.
//!
//! These types deliberately do not contain parser/runtime state or concrete
//! tasks:
//!
//! - [`PendingPhaseOneResidence`](super::PendingPhaseOneResidence) owns the
//!   suspended parser/runtime state.
//! - the typed Page source owns each concrete runnable task;
//! - the enums in this module only describe the source-neutral owner action
//!   required after restoring the residence.
//!
//! Phase one never stores an ordinary Page task-source name. After restoration,
//! the common Page scheduler reads its complete production descriptor snapshot
//! and decides which concrete task is runnable.

/// Source-neutral condition that must be reconciled after a phase-one
/// residence becomes visible in its stable Page slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum PhaseOneRestoreRequirement {
    /// The parser owns a parser-blocking classic script whose external source
    /// has not produced a terminal outcome. This producer is parser-local and
    /// is therefore re-advertised separately from ordinary Page tasks.
    ParserBlockingSourceLoad,

    /// Closed-input parser progress depends on Page-owned work. The concrete
    /// task may already be resident, or its producer may still be in flight.
    /// Phase one remembers neither the source nor a speculative readiness bit.
    PageWork,

    /// No ordinary task was known to block progress. A future body-input or
    /// parser-owned producer transition may wake the residence.
    Producer,
}

/// One-shot source-neutral owner decision made after a phase-one residence is
/// restored.
///
/// This value is not stored in phase-one state and contains no task identity.
/// It closes the publish-before-restore boundary by consulting the production
/// scheduler snapshot after the residence is visible.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum PhaseOneResidenceAdmission {
    /// Re-advertise the already-pending parser-blocking source load after the
    /// parser/runtime residence becomes owner-visible.
    ParserBlockingSourceLoad,

    /// At least one production descriptor is runnable. The owner requests one
    /// ordinary scheduler turn; the scheduler, not phase one, selects it.
    ReadyPageTurn,

    /// No ready fact was observed during admission. The residence is visible,
    /// and the next producer transition is responsible for waking the owner.
    WaitingForProducer,
}

impl PhaseOneResidenceAdmission {
    /// Decide the one-shot wake needed after the stable residence is visible.
    ///
    /// `page_turn_is_runnable` is derived from the complete production
    /// descriptor snapshot and does not dequeue work. Input publication and
    /// continuation publication are separate producer steps: buffered input may
    /// be visible before its runnable descriptor. That producer still owns the
    /// subsequent wake, so admission must not infer readiness from raw input.
    pub(in crate::runtime) fn after_stable_restore(
        requirement: PhaseOneRestoreRequirement,
        page_turn_is_runnable: bool,
    ) -> Self {
        if requirement == PhaseOneRestoreRequirement::ParserBlockingSourceLoad {
            return Self::ParserBlockingSourceLoad;
        }
        if page_turn_is_runnable {
            return Self::ReadyPageTurn;
        }
        Self::WaitingForProducer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_page_turn_is_readmitted_after_its_producer_wake_was_spent() {
        let admission = PhaseOneResidenceAdmission::after_stable_restore(
            PhaseOneRestoreRequirement::PageWork,
            true,
        );

        assert_eq!(admission, PhaseOneResidenceAdmission::ReadyPageTurn);
    }

    #[test]
    fn unrelated_ready_source_uses_the_same_scheduler_admission() {
        let admission = PhaseOneResidenceAdmission::after_stable_restore(
            PhaseOneRestoreRequirement::Producer,
            true,
        );

        assert_eq!(admission, PhaseOneResidenceAdmission::ReadyPageTurn);
    }

    #[test]
    fn closed_input_page_producer_can_remain_pending_after_restore() {
        let admission = PhaseOneResidenceAdmission::after_stable_restore(
            PhaseOneRestoreRequirement::PageWork,
            false,
        );

        assert_eq!(admission, PhaseOneResidenceAdmission::WaitingForProducer);
    }
}
