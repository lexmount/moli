//! Body-only terminal settlement for main parser-owned classic scripts.
//!
//! Evaluation settlement reaches this module as an explicit fact. If evaluation
//! ran, its HTML algorithm checkpoint has already completed; a source failure
//! remains explicitly `NotSettled`. This component applies only the element
//! terminal body and returns its activity. The deferred carrier hands that fact
//! to the selected parser-task coordinator; the parser-blocking carrier hands it
//! to its bounded continuation coordinator. Neither path can re-run
//! prepared-script finishing or admit later parser work while this body is
//! executing.

use super::{
    CurrentScriptContextSpec, MainParserClassicCompletionBodyApplication,
    ParserOwnedClassicScriptCompletion, ParserOwnedClassicScriptCompletionApplication,
    ParserOwnedClassicScriptExecutionContext, ScriptTerminalBodyActivity, ScriptVm,
};
use crate::frame_owner_model::FrameDocumentTaskOwner;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MainParserClassicCompletionCarrier {
    ParserBlocking,
    Deferred,
}

impl ScriptVm {
    pub(crate) fn apply_main_parser_deferred_classic_completion_body(
        &mut self,
        expected_owner: FrameDocumentTaskOwner,
        completion: ParserOwnedClassicScriptCompletion,
    ) -> std::result::Result<MainParserClassicCompletionBodyApplication, String> {
        self.apply_main_parser_classic_completion_body(
            expected_owner,
            completion,
            MainParserClassicCompletionCarrier::Deferred,
        )
    }

    pub(crate) fn apply_main_parser_blocking_classic_completion_body(
        &mut self,
        expected_owner: FrameDocumentTaskOwner,
        completion: ParserOwnedClassicScriptCompletion,
    ) -> std::result::Result<MainParserClassicCompletionBodyApplication, String> {
        self.apply_main_parser_classic_completion_body(
            expected_owner,
            completion,
            MainParserClassicCompletionCarrier::ParserBlocking,
        )
    }

    fn apply_main_parser_classic_completion_body(
        &mut self,
        expected_owner: FrameDocumentTaskOwner,
        completion: ParserOwnedClassicScriptCompletion,
        carrier: MainParserClassicCompletionCarrier,
    ) -> std::result::Result<MainParserClassicCompletionBodyApplication, String> {
        let current_owner = self.current_main_document_task_owner();
        if current_owner != Some(expected_owner) {
            tracing::debug!(
                ?expected_owner,
                ?current_owner,
                ?carrier,
                "dropping stale main parser classic terminal body"
            );
            return Ok(MainParserClassicCompletionBodyApplication::new(
                ParserOwnedClassicScriptCompletionApplication::stale_owner(),
                ScriptTerminalBodyActivity::NoEventDispatch,
            ));
        }

        let (execution_context, script_element_event, evaluation) = completion.into_parts();
        match (&execution_context, carrier) {
            (
                ParserOwnedClassicScriptExecutionContext::ParserBlocking { .. },
                MainParserClassicCompletionCarrier::ParserBlocking,
            )
            | (
                ParserOwnedClassicScriptExecutionContext::Deferred,
                MainParserClassicCompletionCarrier::Deferred,
            ) => {}
            _ => {
                return Err(format!(
                    "main parser {carrier:?} completion received another carrier's authority"
                ));
            }
        }

        let mut application = ParserOwnedClassicScriptCompletionApplication::default();
        application.note_completion_applied(evaluation);
        let terminal_activity = if let Some(task) = script_element_event {
            let parser_bridge = execution_context.parser_bridge().cloned();
            self.document_runtime
                .set_current_script_context(CurrentScriptContextSpec {
                    handle: None,
                    parser_write_insertion_point_active: parser_bridge.is_some(),
                    parser_bridge,
                });
            let _parser_script_nesting = execution_context
                .is_parser_blocking()
                .then(|| self.document_runtime.enter_parser_script_nesting());
            let dispatch = self.dispatch_script_event_body(&task);
            self.document_runtime.clear_current_script_handle();
            match dispatch {
                Ok(()) => application.note_script_event_dispatched(),
                Err(error) => self.record_runtime_warning(format_args!(
                    "main parser {carrier:?} classic script {} body failed for `{}`: {error}",
                    task.event_name(),
                    task.handle
                )),
            }
            ScriptTerminalBodyActivity::EventDispatchAttempted
        } else {
            ScriptTerminalBodyActivity::NoEventDispatch
        };

        if self.current_main_document_task_owner() != Some(expected_owner) {
            application.note_stale_owner();
        }
        Ok(MainParserClassicCompletionBodyApplication::new(
            application,
            terminal_activity,
        ))
    }

    /// Finish the synchronous load/error callback nested inside a
    /// parser-blocking classic continuation.
    ///
    /// This is not an evaluation checkpoint. When evaluation occurred it was
    /// already settled before the terminal body; source failure had no
    /// evaluation to settle. This method does not run another Page task. It
    /// preserves the Chromium-compatible parser boundary that makes terminal
    /// reactions visible before tokenization resumes. The HTML Standard
    /// describes the surrounding script-element algorithm but does not expose
    /// this as a separate task or Moli carrier.
    pub(crate) fn finish_main_parser_blocking_classic_terminal_checkpoint(
        &mut self,
    ) -> anyhow::Result<()> {
        self.perform_owner_lane_task_microtask_checkpoints()
    }
}
