//! Task completion for main Page-owned DocumentScript execution.
//!
//! A selected DocumentScript can contain two distinct observable boundaries:
//! classic script evaluation drains its reactions before a synchronous
//! script-element terminal callback, while the surrounding HTML task ends only
//! after that terminal body. The evaluation checkpoint remains an algorithm
//! responsibility in `ScriptVm`; this coordinator exclusively owns the latter
//! task-end plus the legacy lifecycle/style prime.
//!
//! Every production carrier consumes the same execution-produced fact here.
//! The value is never queued and cannot influence scheduler selection.
//!
//! This coordinator deliberately does not use generic `CallbackCompletion`.
//! That boundary runs `finish_host_task_turn()` and can synchronously execute
//! runtime work admitted for a replacement Document. DocumentScript completion
//! instead checkpoints and reconciles its own consequences, publishes typed
//! runtime continuation readiness, primes lifecycle, and returns to the Page
//! scheduler for the next selection.

use anyhow::Result;

use crate::{document_script_scheduler::PageOwnedDocumentScriptBodyActivity, types::ScriptRun};

use super::{PageVm, page_owned_document_script::MainPageOwnedDocumentScriptExecution};

/// Execution-only task-end selected from an already-run DocumentScript body.
///
/// It cannot be queued or used as scheduler policy. The callback variant adds
/// child-record synchronization but, unlike generic `CallbackCompletion`,
/// never executes another runtime script inside this task.
enum MainPageOwnedDocumentScriptTaskEnd {
    CheckpointOnly,
    CallbackCheckpoint,
}

impl PageVm {
    pub(super) fn finish_main_page_owned_document_script_execution(
        &mut self,
        execution: MainPageOwnedDocumentScriptExecution,
    ) -> Result<ScriptRun> {
        let (run, completion) = execution.into_parts();
        let task_end = match completion.activity() {
            PageOwnedDocumentScriptBodyActivity::NoPageCodeOrEventDispatch => {
                MainPageOwnedDocumentScriptTaskEnd::CheckpointOnly
            }
            PageOwnedDocumentScriptBodyActivity::PageCodeOrEventDispatch => {
                MainPageOwnedDocumentScriptTaskEnd::CallbackCheckpoint
            }
        };
        match task_end {
            MainPageOwnedDocumentScriptTaskEnd::CheckpointOnly => {
                self.finish_selected_page_task_checkpoint()?;
            }
            MainPageOwnedDocumentScriptTaskEnd::CallbackCheckpoint => {
                self.vm_mut()
                    .finish_main_page_owned_document_script_callback_checkpoint()?;
            }
        }

        // This prime used to live in PageOwnedDocumentScriptRunner. It belongs
        // after task completion: terminal callbacks and their reactions must
        // settle before DCL/load or stylesheet successors become observable.
        self.vm_mut()
            .prime_document_lifecycle_and_publish_stylesheets();

        let owner_transition = completion.owner_transition();
        let target = match owner_transition.owner_after_body() {
            Some(owner_after_body) if owner_transition.owner_before() == owner_after_body => {
                "current-owner-retained"
            }
            Some(_) => "document-replaced-during-body",
            None => "document-target-disappeared-during-body",
        };
        tracing::debug!(
            body = ?completion.body(),
            activity = ?completion.activity(),
            target,
            "completed main page-owned DocumentScript task"
        );
        Ok(run)
    }
}
