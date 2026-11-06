use crate::planning::{PreparedScript, PreparedScriptSourceLoadOutcome, SharedScriptSourceLoad};
#[cfg(test)]
use crate::types::ScriptMode;
use crate::{dom::NodeId, frame_owner_model::MainDocumentScriptLoadDelayLease};

use super::parse_time_task::ParseTimeDocumentScriptTask;
use super::{
    DocumentScriptScheduler, async_queues::AsyncLoadCompletion, runner::DocumentScriptRunner,
    source_load_port::DocumentScriptSourceLoadPort,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParseVisibleAsyncLaneState {
    Open,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParseVisibleAsyncReevaluationCredit {
    None,
    Outstanding(ParseVisibleReevaluationCreditReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParseVisibleReevaluationCreditReason {
    ClaimedParseVisibleAsyncWithoutImmediateProgress,
}

/// Result of a synchronous parse-time readiness check.
///
/// Unlike the old `ParseTimeTurn`, this struct never carries a timeout or a
/// wall-clock wait instruction. The coordinator loop handles async wakeup
/// via `tokio::select!` on the page task queue instead.
#[derive(Debug)]
pub(crate) struct ParseTimeTurn {
    pub(crate) parser_step_bytes: Option<usize>,
    pub(crate) ready_task: Option<ParseTimeDocumentScriptTask>,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PostParseScriptClaimDisposition {
    Other,
    ParseTimeAsyncClaimedAtHandoff,
}

/// Trigger events that cause the scheduler to check readiness.
///
/// After the readiness-driven refactor, these triggers are purely synchronous
/// decision points — none of them carry wall-clock waits or compat bridges.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ParseTimeTurnTrigger {
    /// Before the parser advances: check if any async task is ready to run first.
    BeforeParserStep { default_chunk_bytes: usize },
    /// A classic async task just finished execution: check if another is ready.
    AfterClassicAsyncTaskExecuted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParseVisibleReadyTurnDisposition {
    DrainReadyTasks,
    YieldToParserBoundary,
    FinishNoTask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParseVisibleReadyTurnPhase {
    Parsing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParseVisibleReevaluationCreditGrant {
    Granted,
    NotGranted(ParseVisibleReevaluationCreditGrantRefusalReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParseVisibleReevaluationCreditGrantRefusalReason {
    LaneClosed,
    NoPendingClaimedAsync,
    AlreadyArmed,
}

impl DocumentScriptScheduler {
    #[cfg(test)]
    pub(crate) fn claim_parser_post_parse_script(
        &mut self,
        script: PreparedScript,
    ) -> PostParseScriptClaimDisposition {
        match script.mode {
            ScriptMode::Async => {
                if self.claim_existing_parse_time_async_handoff(script.node_id)
                    || self.recover_parse_time_async_handoff(script)
                {
                    PostParseScriptClaimDisposition::ParseTimeAsyncClaimedAtHandoff
                } else {
                    PostParseScriptClaimDisposition::Other
                }
            }
            _ => {
                self.claim_parser_non_async_post_parse_script(script);
                PostParseScriptClaimDisposition::Other
            }
        }
    }
}

impl<
    Target,
    ParserModuleEvaluation,
    ParserModuleGraphFailure,
    ParserClassicReady,
    ParserClassicSourceFailure,
>
    DocumentScriptRunner<
        Target,
        ParserModuleEvaluation,
        ParserModuleGraphFailure,
        ParserClassicReady,
        ParserClassicSourceFailure,
    >
{
    /// Synchronous readiness check for parse-time async scripts.
    ///
    /// This is the core of the readiness-driven model. Unlike the old design,
    /// this method never waits, never yields, and never sleeps. It simply checks
    /// whether any async script is already ready to run and returns immediately.
    ///
    /// Async wakeup is handled externally: completion delivery notifies an
    /// owner-provided completion port, and the owner task lane wakes the
    /// coordinator. The scheduler still owns the synchronous parse-visible
    /// lane/credit state machine, but the async waiting itself stays in the
    /// coordinator.
    pub(crate) fn parse_time_turn(&mut self, trigger: ParseTimeTurnTrigger) -> ParseTimeTurn {
        self.async_parse_time_queue.next_turn(trigger)
    }

    pub(crate) fn plan_parse_visible_ready_turn(
        &self,
        phase: ParseVisibleReadyTurnPhase,
        drained_once: bool,
    ) -> ParseVisibleReadyTurnDisposition {
        if !drained_once {
            return ParseVisibleReadyTurnDisposition::DrainReadyTasks;
        }
        if matches!(phase, ParseVisibleReadyTurnPhase::Parsing)
            && self.has_outstanding_parse_visible_reevaluation_credit()
        {
            return ParseVisibleReadyTurnDisposition::YieldToParserBoundary;
        }
        ParseVisibleReadyTurnDisposition::FinishNoTask
    }

    /// Accept an injected completion that arrived via the owner completion port.
    ///
    /// When the owner receives a parse-time async completion notification, it
    /// calls this method to let the scheduler absorb the completion and
    /// potentially produce a runnable `ClassicAsyncScript` task.
    pub(crate) fn accept_injected_parse_time_async_completion(
        &mut self,
        node_id: NodeId,
        outcome: PreparedScriptSourceLoadOutcome,
    ) -> Option<ParseTimeDocumentScriptTask> {
        let completion = AsyncLoadCompletion { node_id, outcome };
        if self.parse_visible_async_lane_is_closed() {
            self.async_fallback_queue.accept_late_completion(completion);
            None
        } else {
            let (page_task, ready_task_enqueued) = self
                .async_parse_time_queue
                .accept_injected_completion(completion);
            if ready_task_enqueued {
                self.consume_parse_visible_reevaluation_credit();
            }
            page_task
        }
    }

    pub(crate) fn seal_parse_visible_async_cutoff(&mut self) {
        if self.parse_visible_async_lane_is_closed() {
            return;
        }
        self.async_parse_time_queue
            .retire_parse_time_async_completion_port();
        self.parse_visible_async_lane_state = ParseVisibleAsyncLaneState::Closed;
        self.parse_visible_async_reevaluation_credit = ParseVisibleAsyncReevaluationCredit::None;
        self.async_fallback_queue
            .extend_parse_visible_entries(self.async_parse_time_queue.take_remaining_entries());
        self.async_fallback_queue
            .extend_ready_parse_time_tasks(self.async_parse_time_queue.ready_tasks.drain_all());
    }

    pub(super) fn absorb_stranded_parse_time_document_script_task(
        &mut self,
        task: ParseTimeDocumentScriptTask,
    ) {
        match task {
            ParseTimeDocumentScriptTask::ClassicAsyncScript(script) => {
                let (script, network_result, load_delay_binding) = script.into_parts();
                self.async_fallback_queue.push_with_network_result(
                    script,
                    network_result,
                    load_delay_binding,
                );
            }
            ParseTimeDocumentScriptTask::AsyncScriptFailure(task) => {
                self.async_fallback_queue.push_failed_parse_time_task(
                    ParseTimeDocumentScriptTask::AsyncScriptFailure(task),
                );
            }
        }
    }

    #[cfg(test)]
    pub(super) fn seal_parse_time_async_cutoff(&mut self) {
        self.seal_parse_visible_async_cutoff();
    }

    #[cfg(test)]
    pub(super) fn parse_time_async_cutoff_sealed(&self) -> bool {
        self.parse_visible_async_lane_is_closed()
    }

    /// Whether any parse-time async entries are still awaiting completion.
    ///
    /// The coordinator uses this to decide whether to wait on the page task
    /// queue for completions before proceeding with the next parser step.
    #[cfg(test)]
    pub(super) fn has_pending_parse_time_async_entries(&self) -> bool {
        !self.async_parse_time_queue.parse_time_entries.is_empty()
    }

    fn has_pending_claimed_parse_time_async_entries(&self) -> bool {
        self.async_parse_time_queue
            .parse_time_entries
            .iter()
            .any(|entry| entry.claimed_at_handoff)
    }

    pub(crate) fn has_parse_visible_pending_claimed_async(&self) -> bool {
        matches!(
            self.parse_visible_async_lane_state,
            ParseVisibleAsyncLaneState::Open
        ) && self.has_pending_claimed_parse_time_async_entries()
    }

    pub(crate) fn has_outstanding_parse_visible_reevaluation_credit(&self) -> bool {
        matches!(
            self.parse_visible_async_reevaluation_credit,
            ParseVisibleAsyncReevaluationCredit::Outstanding(_)
        ) && self.has_parse_visible_pending_claimed_async()
    }

    pub(crate) fn grant_parse_visible_reevaluation_credit(
        &mut self,
    ) -> ParseVisibleReevaluationCreditGrant {
        if self.parse_visible_async_lane_is_closed() {
            return ParseVisibleReevaluationCreditGrant::NotGranted(
                ParseVisibleReevaluationCreditGrantRefusalReason::LaneClosed,
            );
        }
        if !self.has_pending_claimed_parse_time_async_entries() {
            return ParseVisibleReevaluationCreditGrant::NotGranted(
                ParseVisibleReevaluationCreditGrantRefusalReason::NoPendingClaimedAsync,
            );
        }
        if matches!(
            self.parse_visible_async_reevaluation_credit,
            ParseVisibleAsyncReevaluationCredit::None
        ) {
            self.parse_visible_async_reevaluation_credit =
                ParseVisibleAsyncReevaluationCredit::Outstanding(
                ParseVisibleReevaluationCreditReason::ClaimedParseVisibleAsyncWithoutImmediateProgress,
            );
            ParseVisibleReevaluationCreditGrant::Granted
        } else {
            ParseVisibleReevaluationCreditGrant::NotGranted(
                ParseVisibleReevaluationCreditGrantRefusalReason::AlreadyArmed,
            )
        }
    }

    pub(crate) fn consume_parse_visible_reevaluation_credit(&mut self) {
        if matches!(
            self.parse_visible_async_reevaluation_credit,
            ParseVisibleAsyncReevaluationCredit::Outstanding(
                ParseVisibleReevaluationCreditReason::ClaimedParseVisibleAsyncWithoutImmediateProgress
            )
        ) {
            self.parse_visible_async_reevaluation_credit = ParseVisibleAsyncReevaluationCredit::None;
        }
    }

    fn parse_visible_async_lane_is_closed(&self) -> bool {
        matches!(
            self.parse_visible_async_lane_state,
            ParseVisibleAsyncLaneState::Closed
        )
    }

    pub(super) fn on_parser_discovered_async_candidate_with_source_load_port(
        &mut self,
        script: PreparedScript,
        source_load_port: &DocumentScriptSourceLoadPort,
        shared_load: Option<SharedScriptSourceLoad>,
        document_character_set: Option<&str>,
        bind_load_delay: impl FnOnce(&PreparedScript) -> Option<MainDocumentScriptLoadDelayLease>,
    ) -> bool {
        if self.parse_visible_async_lane_is_closed() {
            return false;
        }
        self.async_parse_time_queue
            .on_parser_discovered_async_candidate(
                script,
                source_load_port,
                shared_load,
                document_character_set,
                bind_load_delay,
            )
    }

    pub(crate) fn activate_existing_parse_time_async_handoff(&mut self, node_id: NodeId) -> bool {
        self.async_parse_time_queue
            .activate_existing_handoff(node_id)
    }

    pub(crate) fn claim_existing_parse_time_async_handoff(&mut self, node_id: NodeId) -> bool {
        self.activate_existing_parse_time_async_handoff(node_id)
    }

    pub(super) fn recover_parse_time_async_handoff_with_source_load_port(
        &mut self,
        script: PreparedScript,
        source_load_port: &DocumentScriptSourceLoadPort,
        shared_load: Option<SharedScriptSourceLoad>,
        document_character_set: Option<&str>,
        bind_load_delay: impl FnOnce(&PreparedScript) -> Option<MainDocumentScriptLoadDelayLease>,
    ) -> bool {
        if self
            .async_parse_time_queue
            .activate_parser_discovered_async_handoff(
                script.clone(),
                source_load_port,
                shared_load,
                document_character_set,
                bind_load_delay,
            )
        {
            true
        } else {
            self.async_fallback_queue.push(script);
            false
        }
    }
}

impl<
    Target,
    ParserModuleEvaluation,
    ParserModuleGraphFailure,
    ParserClassicReady,
    ParserClassicSourceFailure,
>
    DocumentScriptScheduler<
        Target,
        ParserModuleEvaluation,
        ParserModuleGraphFailure,
        ParserClassicReady,
        ParserClassicSourceFailure,
    >
{
    pub(crate) fn parse_time_turn(&mut self, trigger: ParseTimeTurnTrigger) -> ParseTimeTurn {
        self.runner.parse_time_turn(trigger)
    }

    pub(crate) fn plan_parse_visible_ready_turn(
        &self,
        phase: ParseVisibleReadyTurnPhase,
        drained_once: bool,
    ) -> ParseVisibleReadyTurnDisposition {
        self.runner
            .plan_parse_visible_ready_turn(phase, drained_once)
    }

    pub(crate) fn accept_injected_parse_time_async_completion(
        &mut self,
        node_id: NodeId,
        outcome: PreparedScriptSourceLoadOutcome,
    ) -> Option<ParseTimeDocumentScriptTask> {
        self.runner
            .accept_injected_parse_time_async_completion(node_id, outcome)
    }

    pub(crate) fn seal_parse_visible_async_cutoff(&mut self) {
        self.runner.seal_parse_visible_async_cutoff();
    }

    #[cfg(test)]
    pub(super) fn seal_parse_time_async_cutoff(&mut self) {
        self.runner.seal_parse_time_async_cutoff();
    }

    #[cfg(test)]
    pub(super) fn parse_time_async_cutoff_sealed(&self) -> bool {
        self.runner.parse_time_async_cutoff_sealed()
    }

    #[cfg(test)]
    pub(super) fn has_pending_parse_time_async_entries(&self) -> bool {
        self.runner.has_pending_parse_time_async_entries()
    }

    #[cfg(test)]
    pub(crate) fn has_parse_visible_pending_claimed_async(&self) -> bool {
        self.runner.has_parse_visible_pending_claimed_async()
    }

    pub(crate) fn has_outstanding_parse_visible_reevaluation_credit(&self) -> bool {
        self.runner
            .has_outstanding_parse_visible_reevaluation_credit()
    }

    pub(crate) fn grant_parse_visible_reevaluation_credit(
        &mut self,
    ) -> ParseVisibleReevaluationCreditGrant {
        self.runner.grant_parse_visible_reevaluation_credit()
    }

    #[cfg(test)]
    pub(crate) fn consume_parse_visible_reevaluation_credit(&mut self) {
        self.runner.consume_parse_visible_reevaluation_credit();
    }

    pub(crate) fn claim_existing_parse_time_async_handoff(&mut self, node_id: NodeId) -> bool {
        self.runner.claim_existing_parse_time_async_handoff(node_id)
    }
}
