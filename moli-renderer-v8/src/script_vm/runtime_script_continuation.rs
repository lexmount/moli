//! Main-Document runtime-script continuation body.
//!
//! A continuation advances the stable runtime-script owner far enough to
//! publish concrete typed successors. It is not itself a script executor and
//! it does not dispatch script terminal events. The pre-DOMContentLoaded
//! dynamic-script owner advances by at most one item per continuation. Script
//! work moves into its existing DocumentScript or lifecycle carrier before
//! this body returns. The selected Page-task dispatcher owns the
//! continuation's one ordinary task-end checkpoint.

use super::ScriptVm;

/// Exact state transition performed by one current runtime continuation.
///
/// These are execution-produced facts, not scheduler policy. In particular,
/// `WaitingForProducer` means the stable owner still retains work but no
/// concrete task is runnable yet; `ReservationSpent` means the selected
/// continuation found that its producer-side reservation had already been
/// consumed. Both remain current selected tasks and therefore still own their
/// task-end checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeScriptContinuationBodyEffect {
    AdvancedRuntimeOwner(RuntimeScriptOwnerAdvance),
    WaitingForProducer,
    ReservationSpent,
}

/// Concrete result of advancing the dynamic-script owner once.
///
/// Starting a module graph can suspend on fetch without immediately publishing
/// another Page task, so it is intentionally distinct from the three
/// publication variants. None of these effects means that page script code or
/// a script terminal callback ran inside the continuation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeScriptOwnerAdvance {
    StartedModuleGraph,
    PublishedDocumentScript,
    PublishedModuleContinuation,
    PublishedSourceFailure,
}

impl ScriptVm {
    fn publish_one_runtime_script_successor(
        &mut self,
    ) -> Option<RuntimeScriptContinuationBodyEffect> {
        if let Some(advance) = self.send_ready_runtime_tasks() {
            self.enqueue_runtime_script_signal_if_needed();
            return Some(RuntimeScriptContinuationBodyEffect::AdvancedRuntimeOwner(
                advance,
            ));
        }
        None
    }

    /// Advance one exact current runtime-script continuation without running
    /// the successor it publishes.
    pub(crate) fn continue_main_document_runtime_script_task_body(
        &mut self,
        document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> RuntimeScriptContinuationBodyEffect {
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .begin_continuation_turn();
        if self.queued_main_document_runtime_continuation_owner == Some(document_owner) {
            self.queued_main_document_runtime_continuation_owner = None;
        }
        assert_eq!(
            Some(document_owner),
            self.current_main_document_task_owner(),
            "an exact-owner-authorized runtime continuation must target the current Document"
        );

        self.resume_runtime_script_work_after_deferred_page_tasks();
        // The selected action now owns the producer reservation. Materialize
        // one authoritative successor before allowing any later insertion to
        // reserve another continuation.
        if let Some(effect) = self.publish_one_runtime_script_successor() {
            return effect;
        }
        if !self.has_pending_runtime_script_work() {
            return RuntimeScriptContinuationBodyEffect::ReservationSpent;
        }

        // A pending network/module producer needs a stable callback route.
        // Arming the owner does not execute work; it only makes the next
        // readiness transition capable of publishing another concrete task.
        assert!(
            self.arm_runtime_script_work_continuation_if_needed(),
            "non-idle runtime work must accept a stable continuation producer"
        );
        if !self.has_runnable_runtime_script_work_now() {
            return RuntimeScriptContinuationBodyEffect::WaitingForProducer;
        }

        self.resume_runtime_script_work_after_deferred_page_tasks();
        if let Some(effect) = self.publish_one_runtime_script_successor() {
            return effect;
        }
        panic!(
            "runnable runtime-script work must materialize a concrete typed successor; the continuation must not fall back to direct execution"
        );
    }
}
