//! Execution and terminal facts for parser-owned classic scripts.
//!
//! Script evaluation, script-element terminal dispatch, and the enclosing
//! parser task are separate completion boundaries. These types keep their
//! facts distinct so blocking and deferred carriers cannot reconstruct
//! "finished" from a script handle, an event, or a compatibility boolean.

use super::{PreparedScriptBodyActivity, ScriptTerminalBodyActivity};
use crate::{document_runtime::ParserConnectedScriptBridge, host::ScriptEventTask};

#[derive(Debug)]
pub(crate) struct ParserOwnedClassicScriptExecutionError {
    message: String,
}

impl ParserOwnedClassicScriptExecutionError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub(crate) fn into_message(self) -> String {
        self.message
    }
}

/// Whether classic-script evaluation has already crossed its HTML algorithm
/// checkpoint before parser-owned terminal work is applied.
///
/// This is deliberately not a task-end policy. The execution primitive owns
/// the evaluation checkpoint; a later load/error event and its enclosing
/// parser task retain their own completion boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParserOwnedClassicScriptEvaluationSettlement {
    NotSettled,
    Settled,
}

pub(crate) struct ParserOwnedClassicScriptExecutionReport {
    result: std::result::Result<(), ParserOwnedClassicScriptExecutionError>,
    script_element_event: Option<ScriptEventTask>,
    evaluation: ParserOwnedClassicScriptEvaluationSettlement,
    body_activity: PreparedScriptBodyActivity,
}

impl ParserOwnedClassicScriptExecutionReport {
    pub(crate) fn new(
        result: std::result::Result<(), ParserOwnedClassicScriptExecutionError>,
        script_element_event: Option<ScriptEventTask>,
        evaluation: ParserOwnedClassicScriptEvaluationSettlement,
        body_activity: PreparedScriptBodyActivity,
    ) -> Self {
        Self {
            result,
            script_element_event,
            evaluation,
            body_activity,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        std::result::Result<(), ParserOwnedClassicScriptExecutionError>,
        Option<ScriptEventTask>,
        ParserOwnedClassicScriptEvaluationSettlement,
        PreparedScriptBodyActivity,
    ) {
        (
            self.result,
            self.script_element_event,
            self.evaluation,
            self.body_activity,
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) enum ParserOwnedClassicScriptExecutionContext {
    ParserBlocking {
        parser_bridge: Option<ParserConnectedScriptBridge>,
    },
    Deferred,
}

impl ParserOwnedClassicScriptExecutionContext {
    pub(crate) fn is_parser_blocking(&self) -> bool {
        matches!(self, Self::ParserBlocking { .. })
    }

    pub(crate) fn parser_bridge(&self) -> Option<&ParserConnectedScriptBridge> {
        match self {
            Self::ParserBlocking { parser_bridge } => parser_bridge.as_ref(),
            Self::Deferred => None,
        }
    }
}

pub(crate) struct ParserOwnedClassicScriptCompletion {
    execution_context: ParserOwnedClassicScriptExecutionContext,
    script_element_event: Option<ScriptEventTask>,
    evaluation: ParserOwnedClassicScriptEvaluationSettlement,
}

impl ParserOwnedClassicScriptCompletion {
    pub(crate) fn after_execution(
        execution_context: ParserOwnedClassicScriptExecutionContext,
        script_element_event: Option<ScriptEventTask>,
        evaluation: ParserOwnedClassicScriptEvaluationSettlement,
    ) -> Self {
        Self {
            execution_context,
            script_element_event,
            evaluation,
        }
    }

    pub(crate) fn parser_blocking_source_failure(
        parser_bridge: Option<ParserConnectedScriptBridge>,
        script_element_event: Option<ScriptEventTask>,
    ) -> Self {
        Self {
            execution_context: ParserOwnedClassicScriptExecutionContext::ParserBlocking {
                parser_bridge,
            },
            script_element_event,
            evaluation: ParserOwnedClassicScriptEvaluationSettlement::NotSettled,
        }
    }

    pub(crate) fn deferred_source_failure(script_element_event: Option<ScriptEventTask>) -> Self {
        Self {
            execution_context: ParserOwnedClassicScriptExecutionContext::Deferred,
            script_element_event,
            evaluation: ParserOwnedClassicScriptEvaluationSettlement::NotSettled,
        }
    }

    pub(crate) fn script_element_event(&self) -> Option<&ScriptEventTask> {
        self.script_element_event.as_ref()
    }

    pub(crate) fn evaluation(&self) -> ParserOwnedClassicScriptEvaluationSettlement {
        self.evaluation
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ParserOwnedClassicScriptExecutionContext,
        Option<ScriptEventTask>,
        ParserOwnedClassicScriptEvaluationSettlement,
    ) {
        (
            self.execution_context,
            self.script_element_event,
            self.evaluation,
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ParserOwnedClassicScriptCompletionApplication {
    evaluation: Option<ParserOwnedClassicScriptEvaluationSettlement>,
    script_event_dispatched: bool,
    stale_owner: bool,
}

/// Body-only terminal settlement of a main parser-owned classic script.
///
/// Evaluation settlement is an input fact, not work performed by this body.
/// If evaluation ran, its separate algorithm checkpoint has already completed;
/// source failures remain explicitly `NotSettled`. Blocking and deferred
/// carriers each hand the returned terminal activity to their own parser
/// completion coordinator; no terminal checkpoint or next-script admission
/// occurs while constructing it.
pub(crate) struct MainParserClassicCompletionBodyApplication {
    application: ParserOwnedClassicScriptCompletionApplication,
    terminal_activity: ScriptTerminalBodyActivity,
}

impl MainParserClassicCompletionBodyApplication {
    pub(crate) const fn new(
        application: ParserOwnedClassicScriptCompletionApplication,
        terminal_activity: ScriptTerminalBodyActivity,
    ) -> Self {
        Self {
            application,
            terminal_activity,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ParserOwnedClassicScriptCompletionApplication,
        ScriptTerminalBodyActivity,
    ) {
        (self.application, self.terminal_activity)
    }
}

impl ParserOwnedClassicScriptCompletionApplication {
    pub(crate) fn note_completion_applied(
        &mut self,
        evaluation: ParserOwnedClassicScriptEvaluationSettlement,
    ) {
        debug_assert!(
            self.evaluation.is_none(),
            "parser classic completion application must record evaluation settlement once"
        );
        self.evaluation = Some(evaluation);
    }

    pub(crate) fn note_script_event_dispatched(&mut self) {
        self.script_event_dispatched = true;
    }

    pub(crate) fn stale_owner() -> Self {
        Self {
            stale_owner: true,
            ..Self::default()
        }
    }

    pub(crate) fn note_stale_owner(&mut self) {
        self.stale_owner = true;
    }

    pub(crate) fn made_progress(self) -> bool {
        self.evaluation.is_some() || self.script_event_dispatched || self.stale_owner
    }

    pub(crate) fn completion_was_applied(self) -> bool {
        self.evaluation.is_some()
    }

    pub(crate) fn script_event_was_dispatched(self) -> bool {
        self.script_event_dispatched
    }

    pub(crate) fn evaluation(self) -> Option<ParserOwnedClassicScriptEvaluationSettlement> {
        self.evaluation
    }

    pub(crate) fn owner_was_stale(self) -> bool {
        self.stale_owner
    }
}
