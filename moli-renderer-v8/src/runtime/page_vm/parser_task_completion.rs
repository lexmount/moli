//! Task completion for one selected main-parser continuation.
//!
//! Parser-deferred classic and module work can arrive through more than one
//! production carrier, but they share one HTML-task boundary. Their bodies
//! report whether page code or an event-dispatch body was entered; this
//! component alone maps that execution fact to the parser task-end checkpoint.
//! The fact is produced only after execution and is never stored in a queue or
//! used as scheduler policy.
//!
//! This completion deliberately does not execute another Page task. Work made
//! ready by script reactions is published to stable sources, while an exact
//! DOMContentLoaded successor may be consumed separately by `ParserCompletion`
//! without reopening ordinary scheduler arbitration.

use anyhow::Result;

use crate::{
    document_script_scheduler::PageOwnedDocumentScriptBodyActivity,
    frame_owner_model::FrameDocumentTaskOwner,
    script_vm::{PreparedScriptBodyActivity, ScriptTerminalBodyActivity},
};

use super::PageVm;

/// Observable body activity produced by an already-selected parser task.
///
/// `NoPageCodeOrEventDispatch` still represents an applied HTML task and thus
/// still owns a task-end checkpoint. The distinction controls only the child-
/// record reconciliation required after callbacks or script evaluation.
#[must_use = "parser body activity determines the selected task completion"]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MainParserContinuationBodyActivity {
    NoPageCodeOrEventDispatch,
    PageCodeOrEventDispatch,
}

impl MainParserContinuationBodyActivity {
    pub(super) const fn from_prepared_script(activity: PreparedScriptBodyActivity) -> Self {
        match activity {
            PreparedScriptBodyActivity::NotEntered => Self::NoPageCodeOrEventDispatch,
            PreparedScriptBodyActivity::Entered => Self::PageCodeOrEventDispatch,
        }
    }

    pub(super) const fn from_page_owned_document_script(
        activity: PageOwnedDocumentScriptBodyActivity,
    ) -> Self {
        match activity {
            PageOwnedDocumentScriptBodyActivity::NoPageCodeOrEventDispatch => {
                Self::NoPageCodeOrEventDispatch
            }
            PageOwnedDocumentScriptBodyActivity::PageCodeOrEventDispatch => {
                Self::PageCodeOrEventDispatch
            }
        }
    }

    pub(super) const fn note_terminal(self, activity: ScriptTerminalBodyActivity) -> Self {
        match activity {
            ScriptTerminalBodyActivity::NoEventDispatch => self,
            ScriptTerminalBodyActivity::EventDispatchAttempted => Self::PageCodeOrEventDispatch,
        }
    }
}

/// Execution-produced authority to finish one selected parser continuation.
///
/// `NotApplied` means the claimed work became stale or was no longer ready
/// before its body began, so it must not manufacture a checkpoint for the
/// current Document. `Applied` retains the exact owner even if execution later
/// replaces that Document: the task itself still ends, but old parser/lifecycle
/// follow-up must then be rejected by the owner check after the checkpoint.
#[must_use = "an applied parser continuation must complete its task boundary"]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MainParserContinuationTaskEffect {
    NotApplied,
    Applied {
        owner: FrameDocumentTaskOwner,
        activity: MainParserContinuationBodyActivity,
    },
}

impl MainParserContinuationTaskEffect {
    pub(super) const fn applied(
        owner: FrameDocumentTaskOwner,
        activity: MainParserContinuationBodyActivity,
    ) -> Self {
        Self::Applied { owner, activity }
    }

    #[cfg(test)]
    pub(super) const fn checkpoint_only_for_test(owner: FrameDocumentTaskOwner) -> Self {
        Self::applied(
            owner,
            MainParserContinuationBodyActivity::NoPageCodeOrEventDispatch,
        )
    }

    #[cfg(test)]
    pub(super) const fn callback_for_test(owner: FrameDocumentTaskOwner) -> Self {
        Self::applied(
            owner,
            MainParserContinuationBodyActivity::PageCodeOrEventDispatch,
        )
    }
}

impl PageVm {
    /// Finish one parser continuation without selecting or executing a second
    /// Page task. Exact-owner currentness controls only parser/lifecycle
    /// follow-up here; an already-claimed DCL remains the lifecycle
    /// coordinator's responsibility to apply or stale-reject.
    pub(super) fn finish_main_parser_continuation_task(
        &mut self,
        effect: MainParserContinuationTaskEffect,
    ) -> Result<()> {
        let MainParserContinuationTaskEffect::Applied { owner, activity } = effect else {
            return Ok(());
        };

        match activity {
            MainParserContinuationBodyActivity::NoPageCodeOrEventDispatch => self
                .vm_mut()
                .finish_main_parser_continuation_checkpoint_only()?,
            MainParserContinuationBodyActivity::PageCodeOrEventDispatch => self
                .vm_mut()
                .finish_main_parser_continuation_callback_checkpoint()?,
        }

        let owner_retained = self.vm().current_main_document_task_owner() == Some(owner);
        if owner_retained {
            // Rearm the next ordered parser slot only after this task's
            // reactions have settled. If the queue drained, this is a no-op;
            // ParserCompletion has already claimed the exact DCL successor,
            // or proved that no such successor was available, before entering
            // this task-end boundary.
            self.vm_mut().start_pending_main_parser_deferred_scripts()?;
            self.vm_mut()
                .prime_document_lifecycle_and_publish_stylesheets();
        }
        Ok(())
    }
}
