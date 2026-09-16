use std::collections::{BTreeMap, HashMap};

#[cfg(test)]
use moli_core::page::RuntimeConsoleMessageSnapshot;

#[cfg(test)]
use super::RuntimeObservableEmissionSnapshot;
use super::TargetRuntimeObservableSourceSummary;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TargetRuntimeObservableState {
    emitted_console_entries_by_context: HashMap<i64, usize>,
    emitted_exception_entries: usize,
}

impl TargetRuntimeObservableState {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(in crate::domains) fn has_unemitted_source(
        &self,
        summary: &TargetRuntimeObservableSourceSummary,
    ) -> bool {
        self.has_unemitted_console(summary.console_messages_by_context())
            || self.has_unemitted_exceptions(summary.lifecycle_errors())
    }

    fn has_unemitted_console(&self, console_messages_by_context: &BTreeMap<i64, usize>) -> bool {
        console_messages_by_context
            .iter()
            .any(|(execution_context_id, count)| {
                *count
                    > self
                        .emitted_console_entries_by_context
                        .get(execution_context_id)
                        .copied()
                        .unwrap_or_default()
            })
    }

    fn has_unemitted_exceptions(&self, lifecycle_error_count: usize) -> bool {
        lifecycle_error_count > self.emitted_exception_entries
    }

    #[cfg(test)]
    pub(crate) fn emitted_console_entries(&self) -> usize {
        self.emitted_console_entries_by_context.values().sum()
    }

    #[cfg(test)]
    pub(crate) fn emitted_exception_entries(&self) -> usize {
        self.emitted_exception_entries
    }

    pub(in crate::domains) fn source_exception_start(
        &self,
        start_summary: &TargetRuntimeObservableSourceSummary,
        summary: &TargetRuntimeObservableSourceSummary,
    ) -> Option<usize> {
        (summary.lifecycle_errors() >= start_summary.lifecycle_errors()).then(|| {
            self.emitted_exception_entries
                .max(start_summary.lifecycle_errors())
        })
    }

    pub(in crate::domains) fn source_exception_end(
        &self,
        summary: &TargetRuntimeObservableSourceSummary,
    ) -> usize {
        self.emitted_exception_entries
            .max(summary.lifecycle_errors())
    }

    pub(in crate::domains) fn source_context_console_counts(
        &self,
        default_execution_context_id: Option<i64>,
        summary: &TargetRuntimeObservableSourceSummary,
    ) -> HashMap<i64, usize> {
        self.summary_context_console_counts(default_execution_context_id, summary)
    }

    #[cfg(test)]
    pub(in crate::domains) fn emission_snapshot(
        &self,
        all_console_messages: Vec<RuntimeConsoleMessageSnapshot>,
        lifecycle_errors: &[String],
    ) -> RuntimeObservableEmissionSnapshot {
        let exception_start = self.emitted_exception_entries;
        let context_console_counts = console_counts_by_context(&all_console_messages);
        let mut seen_context_counts = HashMap::<i64, usize>::new();
        let console_messages = all_console_messages
            .into_iter()
            .filter(|message| {
                let seen = seen_context_counts
                    .entry(message.execution_context_id)
                    .or_default();
                let emitted =
                    self.emitted_console_entries_for_context(message.execution_context_id);
                let should_emit = *seen >= emitted;
                *seen += 1;
                should_emit
            })
            .collect();
        RuntimeObservableEmissionSnapshot::new(
            exception_start,
            console_messages,
            context_console_counts,
            lifecycle_errors
                .iter()
                .skip(exception_start)
                .cloned()
                .collect(),
        )
    }

    fn summary_context_console_counts(
        &self,
        default_execution_context_id: Option<i64>,
        summary: &TargetRuntimeObservableSourceSummary,
    ) -> HashMap<i64, usize> {
        let mut counts = self.emitted_console_entries_by_context.clone();
        if summary.console_messages_by_context().is_empty() {
            if let Some(execution_context_id) = default_execution_context_id {
                let count = counts.entry(execution_context_id).or_default();
                *count = (*count).max(summary.console_messages_with_context());
            }
            return counts;
        }
        for (execution_context_id, summary_count) in summary.console_messages_by_context() {
            let count = counts.entry(*execution_context_id).or_default();
            *count = (*count).max(*summary_count);
        }
        counts
    }

    pub(crate) fn emitted_console_entries_for_context(&self, execution_context_id: i64) -> usize {
        self.emitted_console_entries_by_context
            .get(&execution_context_id)
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn mark_emitted_console_counts(&mut self, counts: HashMap<i64, usize>) {
        self.emitted_console_entries_by_context = counts;
    }

    pub(crate) fn mark_emitted_exception_entries(&mut self, entries: usize) {
        self.emitted_exception_entries = entries;
    }

    pub(crate) fn advance_to_current(
        &mut self,
        console_counts_by_context: HashMap<i64, usize>,
        exception_entries: usize,
    ) {
        self.mark_emitted_console_counts(console_counts_by_context);
        self.emitted_exception_entries = exception_entries;
    }
}

#[cfg(test)]
fn console_counts_by_context(messages: &[RuntimeConsoleMessageSnapshot]) -> HashMap<i64, usize> {
    let mut counts = HashMap::new();
    for message in messages {
        *counts.entry(message.execution_context_id).or_default() += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use moli_core::page::{
        RendererActivityDiagnostics, RendererPageDiagnosticsSnapshot,
        RendererRuntimeObservableSourceSummary, RuntimeConsoleMessageSnapshot,
    };

    use super::{TargetRuntimeObservableSourceSummary, TargetRuntimeObservableState};

    #[test]
    fn runtime_observable_state_tracks_and_clears_exact_context_cursors() {
        let mut state = TargetRuntimeObservableState::default();
        state.mark_emitted_console_counts(HashMap::from([(1, 2), (7, 1)]));

        assert!(!state.has_unemitted_console(&BTreeMap::from([(1, 2), (7, 1)])));
        assert!(state.has_unemitted_console(&BTreeMap::from([(1, 2), (7, 2)])));
        assert_eq!(state.emitted_console_entries_for_context(1), 2);
        assert_eq!(state.emitted_console_entries_for_context(7), 1);
        assert_eq!(state.emitted_console_entries_for_context(9), 0);

        state.advance_to_current(HashMap::from([(42, 5)]), 3);
        assert_eq!(state.emitted_console_entries_for_context(42), 5);
        assert_eq!(state.emitted_console_entries_for_context(7), 0);
        state.advance_to_current(HashMap::new(), 3);
        assert_eq!(state.emitted_console_entries(), 0);
        assert_eq!(state.emitted_console_entries_for_context(42), 0);
        assert!(
            !state.has_unemitted_source(&TargetRuntimeObservableSourceSummary::from_counts(
                0,
                BTreeMap::new(),
                3,
            ))
        );
        assert!(
            state.has_unemitted_source(&TargetRuntimeObservableSourceSummary::from_counts(
                0,
                BTreeMap::new(),
                4,
            ))
        );
    }

    #[test]
    fn runtime_observable_source_summary_projects_renderer_diagnostics() {
        let summary = TargetRuntimeObservableSourceSummary::from_renderer_snapshot(
            &RendererPageDiagnosticsSnapshot::from_diagnostics(RendererActivityDiagnostics {
                runtime_console_messages_with_context: 3,
                runtime_console_messages_by_context: BTreeMap::from([(1, 2), (2, 1)]),
                runtime_lifecycle_errors: 4,
                ..Default::default()
            }),
        );

        let state = TargetRuntimeObservableState::default();
        assert!(state.has_unemitted_source(&summary));
    }

    #[test]
    fn runtime_observable_source_summary_prefers_typed_renderer_source() {
        let mut snapshot = RendererPageDiagnosticsSnapshot::from_runtime_observable_source(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![console_message(7, "typed")],
                vec!["first".to_owned(), "second".to_owned()],
            ),
        );
        snapshot.diagnostics = RendererActivityDiagnostics {
            runtime_console_messages_with_context: 99,
            runtime_console_messages_by_context: BTreeMap::from([(1, 99)]),
            runtime_lifecycle_errors: 99,
            ..Default::default()
        };
        let summary = TargetRuntimeObservableSourceSummary::from_renderer_snapshot(&snapshot);

        assert_eq!(summary.console_messages_with_context(), 1);
        assert_eq!(
            summary.console_messages_by_context(),
            &BTreeMap::from([(7, 1)])
        );
        assert_eq!(summary.lifecycle_errors(), 2);
    }

    #[test]
    fn runtime_observable_emission_snapshot_filters_unemitted_payloads_and_advances_cursors() {
        let mut state = TargetRuntimeObservableState::default();
        state.mark_emitted_console_counts(HashMap::from([(1, 1)]));
        state.mark_emitted_exception_entries(1);

        let snapshot = state.emission_snapshot(
            vec![
                console_message(1, "old-default"),
                console_message(1, "new-default"),
                console_message(2, "new-isolated"),
            ],
            &["old-error".to_owned(), "new-error".to_owned()],
        );

        let messages = snapshot
            .console_messages()
            .iter()
            .map(|message| message.message.as_str())
            .collect::<Vec<_>>();
        assert_eq!(messages, ["new-default", "new-isolated"]);
        assert_eq!(snapshot.exception_start(), 1);
        assert_eq!(snapshot.lifecycle_errors(), ["new-error"]);

        state.mark_emitted_console_counts(snapshot.context_console_counts().clone());
        state.mark_emitted_exception_entries(snapshot.exception_end());
        assert_eq!(state.emitted_console_entries_for_context(1), 2);
        assert_eq!(state.emitted_console_entries_for_context(2), 1);
        assert_eq!(state.emitted_exception_entries(), 2);
    }

    #[test]
    fn runtime_observable_lifecycle_error_cursor_advances_without_default_context() {
        let mut state = TargetRuntimeObservableState::default();
        let snapshot = state.emission_snapshot(Vec::new(), &["error".to_owned()]);

        assert_eq!(snapshot.lifecycle_errors(), ["error"]);
        state.mark_emitted_console_counts(snapshot.context_console_counts().clone());
        state.mark_emitted_exception_entries(snapshot.exception_end());

        assert_eq!(state.emitted_exception_entries(), 1);
        assert!(
            !state.has_unemitted_source(&TargetRuntimeObservableSourceSummary::from_counts(
                0,
                BTreeMap::new(),
                1,
            )),
            "lifecycle errors that cannot be emitted without a context must still advance the cursor"
        );
    }

    fn console_message(execution_context_id: i64, message: &str) -> RuntimeConsoleMessageSnapshot {
        RuntimeConsoleMessageSnapshot {
            execution_context_id,
            message: message.to_owned(),
            args: Vec::new(),
            stack: None,
        }
    }
}
