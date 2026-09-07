use crate::document_task_lane::DocumentTaskQueue;
use crate::dom::NodeId;

use crate::{
    frame_owner_model::MainDocumentScriptLoadDelayLease,
    planning::{
        PreparedScript, PreparedScriptSourceLoadOutcome, SharedScriptSourceLoad,
        prepared_script_with_loaded_source,
    },
    types::{ScriptKind, ScriptMode, ScriptSourceKind},
};

use super::{
    ParseTimeDocumentScriptTask, ParseTimeTurn, ParseTimeTurnTrigger,
    completion_port::ParseTimeAsyncCompletionPort, post_parse_task::PostParseDocumentScriptTask,
    source_load_port::DocumentScriptSourceLoadPort,
};

#[derive(Debug, Clone)]
pub(super) struct AsyncLoadCompletion {
    pub(super) node_id: NodeId,
    pub(super) outcome: PreparedScriptSourceLoadOutcome,
}

pub(super) struct AsyncFallbackQueue {
    pub(super) entries: Vec<AsyncFallbackEntry>,
}

pub(super) struct AsyncFallbackEntry {
    pub(super) script: PreparedScript,
    pub(super) load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    pub(super) awaiting_completion: bool,
    pub(super) source_load: Option<SharedScriptSourceLoad>,
    pub(super) load_failure: Option<PreparedScriptSourceLoadOutcome>,
}

/// Parse-time async queue: owns classic external async scripts that can become
/// ready during parsing.
///
/// Readiness is tracked through two mechanisms:
/// 1. **Direct completion notification**: when a fetch completes, the queue
///    notifies an owner-provided completion port. The owner maps that
///    notification to its task lane.
/// 2. **Synchronous readiness check**: at each parse-time turn, the queue
///    checks if any already-ready tasks exist and returns them without waiting.
///
/// There are no compat bridges, no wall-clock timeouts, and no local yield
/// loops. Completion is the wake source, not parser checkpoints.
pub(super) struct AsyncParseTimeQueue {
    pub(super) parse_time_entries: Vec<ParseTimeAsyncEntry>,
    pub(super) ready_tasks: DocumentTaskQueue<ParseTimeDocumentScriptTask>,
    pub(super) parse_time_completion_port: Option<ParseTimeAsyncCompletionPort>,
}

pub(super) struct ParseTimeAsyncEntry {
    pub(super) original: PreparedScript,
    pub(super) load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    pub(super) claimed_at_handoff: bool,
    pub(super) completion: Option<PreparedScriptSourceLoadOutcome>,
    pub(super) source_load: Option<SharedScriptSourceLoad>,
}

impl AsyncParseTimeQueue {
    pub(super) fn new() -> Self {
        Self {
            parse_time_entries: Vec::new(),
            ready_tasks: DocumentTaskQueue::default(),
            parse_time_completion_port: None,
        }
    }

    pub(super) fn bind_parse_time_async_completion_port(
        &mut self,
        port: ParseTimeAsyncCompletionPort,
    ) {
        self.parse_time_completion_port = Some(port);
    }

    pub(super) fn retire_parse_time_async_completion_port(&mut self) {
        if let Some(port) = self.parse_time_completion_port.take() {
            port.retire();
        }
    }

    /// Activate the handoff for an async script that should already be owned by
    /// the parse-time async queue.
    ///
    /// Parser discovery is responsible for creating the queue entry and
    /// starting the background fetch as early as possible. The later script
    /// handoff should only "claim visibility" for that existing owner entry.
    ///
    /// A missing discovery entry is treated as a recovery path rather than the
    /// expected flow: we still create the entry so behavior remains correct, but
    /// the debug assertion keeps the owner-model drift visible while this
    /// substrate rebuild is in progress.
    pub(super) fn activate_parser_discovered_async_handoff(
        &mut self,
        recovery_script: PreparedScript,
        source_load_port: &DocumentScriptSourceLoadPort,
        shared_load: Option<SharedScriptSourceLoad>,
        document_character_set: Option<&str>,
        bind_load_delay: impl FnOnce(&PreparedScript) -> Option<MainDocumentScriptLoadDelayLease>,
    ) -> bool {
        if !should_prefetch_classic_async(&recovery_script) {
            return false;
        }

        if self.activate_existing_handoff(recovery_script.node_id) {
            true
        } else {
            debug_assert!(
                false,
                "parser handoff activated async script without an earlier discovery-owned entry"
            );
            let load_delay_binding = bind_load_delay(&recovery_script);
            self.insert_parse_time_entry(
                recovery_script,
                load_delay_binding,
                true,
                source_load_port,
                shared_load,
                document_character_set,
            );
            true
        }
    }

    pub(super) fn activate_existing_handoff(&mut self, node_id: NodeId) -> bool {
        if let Some(index) = self
            .parse_time_entries
            .iter()
            .position(|entry| entry.original.node_id == node_id)
        {
            let entry = &mut self.parse_time_entries[index];
            entry.claimed_at_handoff = true;
            if let Some(outcome) = entry.completion.take() {
                let entry = self.parse_time_entries.remove(index);
                self.enqueue_ready_task_in_completion_order(parse_time_task_from_load_outcome(
                    entry.original,
                    outcome,
                    entry.load_delay_binding,
                ));
            }
            true
        } else {
            false
        }
    }

    pub(super) fn on_parser_discovered_async_candidate(
        &mut self,
        script: PreparedScript,
        source_load_port: &DocumentScriptSourceLoadPort,
        shared_load: Option<SharedScriptSourceLoad>,
        document_character_set: Option<&str>,
        bind_load_delay: impl FnOnce(&PreparedScript) -> Option<MainDocumentScriptLoadDelayLease>,
    ) -> bool {
        if !should_prefetch_classic_async(&script) {
            return false;
        }
        if self
            .parse_time_entries
            .iter()
            .any(|entry| entry.original.node_id == script.node_id)
        {
            return true;
        }

        let load_delay_binding = bind_load_delay(&script);
        self.insert_parse_time_entry(
            script,
            load_delay_binding,
            false,
            source_load_port,
            shared_load,
            document_character_set,
        );
        true
    }

    pub(super) fn apply_completion(&mut self, completion: AsyncLoadCompletion) -> bool {
        for index in 0..self.parse_time_entries.len() {
            if self.parse_time_entries[index].original.node_id != completion.node_id {
                continue;
            }
            if self.parse_time_entries[index].claimed_at_handoff {
                let entry = self.parse_time_entries.remove(index);
                self.enqueue_ready_task_in_completion_order(parse_time_task_from_load_outcome(
                    entry.original,
                    completion.outcome,
                    entry.load_delay_binding,
                ));
                return true;
            } else {
                self.parse_time_entries[index].completion = Some(completion.outcome);
                return false;
            }
        }
        false
    }

    fn insert_parse_time_entry(
        &mut self,
        script: PreparedScript,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
        claimed_at_handoff: bool,
        source_load_port: &DocumentScriptSourceLoadPort,
        shared_load: Option<SharedScriptSourceLoad>,
        document_character_set: Option<&str>,
    ) {
        let load = shared_load.unwrap_or_else(|| {
            source_load_port
                .start_with_document_character_set(script.clone(), document_character_set)
        });
        let completion = load.try_outcome();
        if claimed_at_handoff && let Some(outcome) = completion {
            self.enqueue_ready_task_in_completion_order(parse_time_task_from_load_outcome(
                script,
                outcome,
                load_delay_binding,
            ));
            return;
        }
        if completion.is_none() {
            self.spawn_async_prefetch_completion(script.node_id, load.clone());
        }
        self.parse_time_entries.push(ParseTimeAsyncEntry {
            original: script,
            load_delay_binding,
            claimed_at_handoff,
            completion,
            source_load: Some(load),
        });
    }

    fn spawn_async_prefetch_completion(&self, node_id: NodeId, load: SharedScriptSourceLoad) {
        let Some(parse_time_completion_port) = self.parse_time_completion_port.clone() else {
            return;
        };
        let completed_load = load.clone();
        load.register_completion_wake(move || {
            let outcome = completed_load
                .try_outcome()
                .expect("script source completion callback requires a terminal outcome");
            let _ = parse_time_completion_port.send(node_id, outcome);
        });
    }

    pub(super) fn enqueue_ready_task_in_completion_order(
        &mut self,
        task: ParseTimeDocumentScriptTask,
    ) {
        self.ready_tasks.push_back(task);
    }

    /// Synchronous readiness check: return the next ready task, if any.
    ///
    /// This never waits. In the readiness-driven model, the only way new tasks
    /// become ready is through `apply_completion()`, which is called when the
    /// coordinator receives a `ParseTimeAsyncCompletion` from the page task queue.
    fn take_ready_now(&mut self) -> Option<ParseTimeDocumentScriptTask> {
        self.ready_tasks.pop_front()
    }

    /// Synchronous turn decision for each trigger type.
    ///
    /// Every trigger path is now a simple "check readiness, return immediately"
    /// operation. No compat bridges, no wall-clock waits, no local yields.
    pub(super) fn next_turn(&mut self, trigger: ParseTimeTurnTrigger) -> ParseTimeTurn {
        match trigger {
            ParseTimeTurnTrigger::BeforeParserStep {
                default_chunk_bytes,
            } => ParseTimeTurn {
                parser_step_bytes: Some(default_chunk_bytes),
                ready_task: self.take_ready_now(),
            },
            ParseTimeTurnTrigger::AfterClassicAsyncTaskExecuted => ParseTimeTurn {
                parser_step_bytes: None,
                ready_task: self.take_ready_now(),
            },
        }
    }

    pub(super) fn accept_injected_completion(
        &mut self,
        completion: AsyncLoadCompletion,
    ) -> (Option<ParseTimeDocumentScriptTask>, bool) {
        let ready_task_enqueued = self.apply_completion(completion);
        (self.take_ready_now(), ready_task_enqueued)
    }

    pub(super) fn take_remaining_entries(&mut self) -> Vec<ParseTimeAsyncEntry> {
        std::mem::take(&mut self.parse_time_entries)
    }

    pub(super) fn into_remaining_async_phase_tasks(self) -> Vec<PostParseDocumentScriptTask> {
        let mut resolved = resolve_remaining_async_phase_entries(self.parse_time_entries);
        resolved.extend(
            self.ready_tasks
                .into_iter()
                .map(async_phase_task_from_parse_time_task),
        );
        resolved
    }
}

impl AsyncFallbackQueue {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(super) fn push(&mut self, script: PreparedScript) {
        self.push_with_load_delay_binding(script, None);
    }

    pub(super) fn push_with_load_delay_binding(
        &mut self,
        script: PreparedScript,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    ) {
        self.entries.push(AsyncFallbackEntry {
            script,
            load_delay_binding,
            awaiting_completion: false,
            source_load: None,
            load_failure: None,
        });
    }

    pub(super) fn push_failed_parse_time_task(&mut self, task: ParseTimeDocumentScriptTask) {
        let ParseTimeDocumentScriptTask::AsyncScriptFailure(failure) = task else {
            debug_assert!(
                false,
                "only async script failure tasks can be absorbed as failed parse-time tasks"
            );
            return;
        };
        let (script, error, network_result, load_delay_binding) = failure.into_parts();
        self.entries.push(AsyncFallbackEntry {
            script,
            load_delay_binding,
            awaiting_completion: false,
            source_load: None,
            load_failure: Some(PreparedScriptSourceLoadOutcome {
                source_result: Err(error),
                source_bytes: None,
                network_result,
                muted_errors: false,
            }),
        });
    }

    pub(super) fn extend_parse_visible_entries(&mut self, entries: Vec<ParseTimeAsyncEntry>) {
        self.entries.extend(
            entries
                .into_iter()
                .map(fallback_entry_from_parse_time_entry),
        );
    }

    pub(super) fn extend_ready_parse_time_tasks<I>(&mut self, tasks: I)
    where
        I: IntoIterator<Item = ParseTimeDocumentScriptTask>,
    {
        for task in tasks {
            match task {
                ParseTimeDocumentScriptTask::ClassicAsyncScript(script) => {
                    let (script, load_delay_binding) = script.into_parts();
                    self.push_with_load_delay_binding(script, load_delay_binding);
                }
                ParseTimeDocumentScriptTask::AsyncScriptFailure(_) => {
                    self.push_failed_parse_time_task(task)
                }
            }
        }
    }

    pub(super) fn accept_late_completion(&mut self, completion: AsyncLoadCompletion) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.awaiting_completion && entry.script.node_id == completion.node_id)
        {
            match completion.outcome.source_result {
                Ok(source) => {
                    entry.script = prepared_script_with_loaded_source(
                        entry.script.clone(),
                        source,
                        completion.outcome.source_bytes,
                        completion.outcome.muted_errors,
                    );
                    entry.load_failure = None;
                }
                Err(error) => {
                    entry.load_failure = Some(PreparedScriptSourceLoadOutcome {
                        source_result: Err(error),
                        source_bytes: completion.outcome.source_bytes,
                        network_result: completion.outcome.network_result,
                        muted_errors: false,
                    });
                }
            }
            entry.awaiting_completion = false;
            entry.source_load = None;
        }
    }

    pub(super) fn into_async_phase_tasks(self) -> Vec<PostParseDocumentScriptTask> {
        self.entries
            .into_iter()
            .map(|entry| {
                if entry.awaiting_completion
                    && let Some(source_load) = entry.source_load
                {
                    return PostParseDocumentScriptTask::async_script_waiting_for_source(
                        entry.script,
                        source_load,
                        entry.load_delay_binding,
                    );
                }
                if let Some(outcome) = entry.load_failure
                    && let Err(error) = outcome.source_result
                {
                    return PostParseDocumentScriptTask::async_script_load_failure(
                        entry.script,
                        error,
                        outcome.network_result,
                        entry.load_delay_binding,
                    );
                }
                PostParseDocumentScriptTask::async_script(entry.script, entry.load_delay_binding)
            })
            .collect()
    }
}

fn resolve_remaining_async_phase_entries(
    parse_time_entries: Vec<ParseTimeAsyncEntry>,
) -> Vec<PostParseDocumentScriptTask> {
    let mut resolved = Vec::with_capacity(parse_time_entries.len());
    for entry in parse_time_entries {
        // These are the async entries that never became ready at any parse-time
        // checkpoint. Preserve post-DCL execution timing, but keep a completed
        // fetch failure terminal so a failed prefetch is not retried as a fresh
        // external script before or after DCL.
        resolved.push(match entry.completion {
            Some(outcome) => async_phase_page_task_from_load_outcome(
                entry.original,
                outcome,
                entry.load_delay_binding,
            ),
            None => {
                PostParseDocumentScriptTask::async_script(entry.original, entry.load_delay_binding)
            }
        });
    }
    resolved
}

fn async_phase_task_from_parse_time_task(
    task: ParseTimeDocumentScriptTask,
) -> PostParseDocumentScriptTask {
    match task {
        ParseTimeDocumentScriptTask::ClassicAsyncScript(script) => {
            let (script, load_delay_binding) = script.into_parts();
            PostParseDocumentScriptTask::async_script(script, load_delay_binding)
        }
        ParseTimeDocumentScriptTask::AsyncScriptFailure(failure) => {
            let (script, error, source_network_result, load_delay_binding) = failure.into_parts();
            PostParseDocumentScriptTask::async_script_load_failure(
                script,
                error,
                source_network_result,
                load_delay_binding,
            )
        }
    }
}

fn parse_time_task_from_load_outcome(
    script: PreparedScript,
    outcome: PreparedScriptSourceLoadOutcome,
    load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
) -> ParseTimeDocumentScriptTask {
    let PreparedScriptSourceLoadOutcome {
        source_result,
        source_bytes,
        network_result,
        muted_errors,
    } = outcome;
    match source_result {
        Ok(source) => ParseTimeDocumentScriptTask::classic_async_script(
            prepared_script_with_loaded_source(script, source, source_bytes, muted_errors),
            load_delay_binding,
        ),
        Err(error) => ParseTimeDocumentScriptTask::async_script_failure(
            script,
            error,
            network_result,
            load_delay_binding,
        ),
    }
}

fn async_phase_page_task_from_load_outcome(
    script: PreparedScript,
    outcome: PreparedScriptSourceLoadOutcome,
    load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
) -> PostParseDocumentScriptTask {
    let PreparedScriptSourceLoadOutcome {
        source_result,
        source_bytes,
        network_result,
        muted_errors,
    } = outcome;
    match source_result {
        Ok(source) => PostParseDocumentScriptTask::async_script(
            prepared_script_with_loaded_source(script, source, source_bytes, muted_errors),
            load_delay_binding,
        ),
        Err(error) => PostParseDocumentScriptTask::async_script_load_failure(
            script,
            error,
            network_result,
            load_delay_binding,
        ),
    }
}

fn fallback_entry_from_parse_time_entry(entry: ParseTimeAsyncEntry) -> AsyncFallbackEntry {
    match entry.completion {
        Some(PreparedScriptSourceLoadOutcome {
            source_result: Ok(source),
            source_bytes,
            network_result: _,
            muted_errors,
        }) => AsyncFallbackEntry {
            script: prepared_script_with_loaded_source(
                entry.original,
                source,
                source_bytes,
                muted_errors,
            ),
            load_delay_binding: entry.load_delay_binding,
            awaiting_completion: false,
            source_load: None,
            load_failure: None,
        },
        Some(outcome) => AsyncFallbackEntry {
            script: entry.original,
            load_delay_binding: entry.load_delay_binding,
            awaiting_completion: false,
            source_load: None,
            load_failure: Some(outcome),
        },
        None => AsyncFallbackEntry {
            script: entry.original,
            load_delay_binding: entry.load_delay_binding,
            awaiting_completion: true,
            source_load: entry.source_load,
            load_failure: None,
        },
    }
}

fn should_prefetch_classic_async(script: &PreparedScript) -> bool {
    script.kind == ScriptKind::Classic
        && script.mode == ScriptMode::Async
        && script.source_kind == ScriptSourceKind::External
}
