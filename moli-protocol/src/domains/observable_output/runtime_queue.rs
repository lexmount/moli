use std::collections::{BTreeMap, HashMap};

#[cfg(test)]
use moli_core::page::{
    RendererPageDiagnosticsSnapshot,
    RendererRuntimeObservableSourceItem as CoreRendererRuntimeObservableSourceItem,
    RendererRuntimeObservableSourceSummary,
};
use moli_core::page::{RuntimeConsoleMessageSnapshot, ScriptObservableOutputItem};

use crate::conn::DocumentId;

use super::items::{
    ObservableRuntimePreparedItem, ObservableRuntimePreparedItems, runtime_console_api_called_item,
    runtime_exception_thrown_item,
};
use super::runtime_cursor::TargetRuntimeObservableState;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[cfg(test)]
pub(crate) struct TargetRuntimeObservableQueueSnapshot {
    pub(crate) observable_output_items: Vec<ScriptObservableOutputItem>,
    pub(crate) source_outputs: Vec<TargetRuntimeObservableSourceOutput>,
}

#[cfg(test)]
impl TargetRuntimeObservableQueueSnapshot {
    #[cfg(test)]
    pub(in crate::domains) fn latest_source_tail(
        &self,
    ) -> Option<TargetRuntimeObservableSourceOutput> {
        let latest = self.source_outputs.last()?;
        let sources = self.source_outputs.iter().filter(|source| {
            source.url() == latest.url() && source.document_id() == latest.document_id()
        });
        TargetRuntimeObservableSourceOutput::combine_same_identity(sources)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetRuntimeObservableSourceOutput {
    url: String,
    document_id: DocumentId,
    source_item_start_index: usize,
    source_item_end_index: usize,
    start_summary: TargetRuntimeObservableSourceSummary,
    default_execution_context_id: Option<i64>,
    source_items: Vec<TargetRuntimeObservableSourceItem>,
}

impl TargetRuntimeObservableSourceOutput {
    fn from_delta(
        url: String,
        document_id: DocumentId,
        source_item_start_index: usize,
        source_item_end_index: usize,
        start_summary: TargetRuntimeObservableSourceSummary,
        summary: TargetRuntimeObservableSourceSummary,
        source_items: Vec<TargetRuntimeObservableSourceItem>,
    ) -> Option<Self> {
        (source_item_start_index <= source_item_end_index
            && source_item_end_index - source_item_start_index == source_items.len()
            && !summary.is_empty())
        .then_some(())?;
        let derived_summary = start_summary
            .advance_with_items(summary.default_execution_context_id(), &source_items)?;
        (derived_summary == summary).then_some(Self {
            url,
            document_id,
            source_item_start_index,
            source_item_end_index,
            start_summary,
            default_execution_context_id: derived_summary.default_execution_context_id(),
            source_items,
        })
    }

    fn combine_same_identity<'a>(sources: impl IntoIterator<Item = &'a Self>) -> Option<Self> {
        let mut sources = sources.into_iter();
        let first = sources.next()?;
        let mut combined = first.clone();
        for source in sources {
            if combined.url != source.url
                || combined.document_id != source.document_id
                || combined.source_item_end_index != source.source_item_start_index
            {
                return None;
            }
            combined.source_item_end_index = source.source_item_end_index;
            combined.default_execution_context_id = source.default_execution_context_id;
            combined
                .source_items
                .extend(source.source_items.iter().cloned());
        }
        Some(combined)
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    pub(crate) fn document_id(&self) -> DocumentId {
        self.document_id
    }

    #[cfg(test)]
    pub(crate) fn source_item_start_index(&self) -> usize {
        self.source_item_start_index
    }

    #[cfg(test)]
    pub(crate) fn source_item_end_index(&self) -> usize {
        self.source_item_end_index
    }

    pub(crate) fn start_summary(&self) -> &TargetRuntimeObservableSourceSummary {
        &self.start_summary
    }

    #[cfg(test)]
    pub(crate) fn summary(&self) -> TargetRuntimeObservableSourceSummary {
        self.source_summary()
            .expect("RuntimeObservable source output should be constructed from valid source items")
    }

    #[cfg(test)]
    pub(in crate::domains) fn has_source_items(&self) -> bool {
        self.source_items_match_summaries()
    }

    pub(in crate::domains::observable_output) fn source_items_prepared_for_state(
        &self,
        owner_state: &TargetRuntimeObservableState,
        include_console_api_messages: bool,
    ) -> Option<ObservableRuntimePreparedItems> {
        let summary = self.source_summary()?;
        if !owner_state.has_unemitted_source(&summary) {
            return None;
        }
        let exception_start = owner_state.source_exception_start(self.start_summary(), &summary)?;
        let mut items = Vec::new();
        for item in &self.source_items {
            match item {
                TargetRuntimeObservableSourceItem::ConsoleMessage {
                    message,
                    context_count_end,
                } => {
                    if !include_console_api_messages {
                        continue;
                    }
                    let emitted = owner_state.emitted_console_entries_for_context(
                        message.execution_context_id,
                        summary.default_execution_context_id(),
                    );
                    if *context_count_end > emitted {
                        items.push(ObservableRuntimePreparedItem::output(
                            runtime_console_api_called_item(message),
                        ));
                    }
                }
                TargetRuntimeObservableSourceItem::LifecycleError {
                    text,
                    execution_context_id,
                    exception_index,
                } => {
                    if *exception_index >= exception_start
                        && let Some(execution_context_id) = execution_context_id
                    {
                        items.push(ObservableRuntimePreparedItem::output(
                            runtime_exception_thrown_item(
                                text.clone(),
                                self.url(),
                                *execution_context_id,
                                *exception_index,
                            ),
                        ));
                    }
                }
            }
        }
        let exception_end = owner_state.source_exception_end(&summary);
        if items.is_empty() && exception_end == exception_start {
            return None;
        }
        Some(ObservableRuntimePreparedItems::from_runtime_source_items(
            self.url.clone(),
            self.document_id,
            items,
            owner_state
                .source_context_console_counts(summary.default_execution_context_id(), &summary),
            exception_end,
        ))
    }

    pub(crate) fn observable_output_items(&self) -> Vec<ScriptObservableOutputItem> {
        self.source_items
            .iter()
            .filter_map(|item| match item {
                TargetRuntimeObservableSourceItem::ConsoleMessage { message, .. }
                    if Some(message.execution_context_id) == self.default_execution_context_id =>
                {
                    Some(ScriptObservableOutputItem::ConsoleMessage(
                        message.message.clone(),
                    ))
                }
                TargetRuntimeObservableSourceItem::ConsoleMessage { .. } => None,
                TargetRuntimeObservableSourceItem::LifecycleError { text, .. } => {
                    Some(ScriptObservableOutputItem::LifecycleError(text.clone()))
                }
            })
            .collect()
    }

    pub(in crate::domains::observable_output) fn cursor_end(
        &self,
    ) -> Option<(HashMap<i64, usize>, usize)> {
        let summary = self.source_summary()?;
        let mut context_console_counts = summary
            .console_messages_by_context()
            .iter()
            .map(|(execution_context_id, count)| (*execution_context_id, *count))
            .collect::<HashMap<_, _>>();
        if context_console_counts.is_empty()
            && summary.console_messages_with_context() > 0
            && let Some(default_execution_context_id) = summary.default_execution_context_id()
        {
            context_console_counts.insert(
                default_execution_context_id,
                summary.console_messages_with_context(),
            );
        }
        Some((context_console_counts, summary.lifecycle_errors()))
    }

    #[cfg(test)]
    fn source_items_match_summaries(&self) -> bool {
        self.source_summary().is_some()
    }

    fn source_summary(&self) -> Option<TargetRuntimeObservableSourceSummary> {
        self.start_summary
            .advance_with_items(self.default_execution_context_id, &self.source_items)
    }

    #[cfg(test)]
    pub(crate) fn source_items(&self) -> &[TargetRuntimeObservableSourceItem] {
        &self.source_items
    }

    #[cfg(test)]
    pub(in crate::domains::observable_output) fn source_console_messages(
        &self,
    ) -> Vec<RuntimeConsoleMessageSnapshot> {
        self.source_items
            .iter()
            .filter_map(|item| match item {
                TargetRuntimeObservableSourceItem::ConsoleMessage { message, .. } => Some(message),
                TargetRuntimeObservableSourceItem::LifecycleError { .. } => None,
            })
            .cloned()
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TargetRuntimeObservableSourceItem {
    ConsoleMessage {
        message: RuntimeConsoleMessageSnapshot,
        context_count_end: usize,
    },
    LifecycleError {
        text: String,
        execution_context_id: Option<i64>,
        exception_index: usize,
    },
}

impl TargetRuntimeObservableSourceItem {
    fn console_message(message: RuntimeConsoleMessageSnapshot, context_count_end: usize) -> Self {
        Self::ConsoleMessage {
            message,
            context_count_end,
        }
    }

    fn lifecycle_error(
        text: String,
        execution_context_id: Option<i64>,
        exception_index: usize,
    ) -> Self {
        Self::LifecycleError {
            text,
            execution_context_id,
            exception_index,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TargetRuntimeObservableSourceSummary {
    default_execution_context_id: Option<i64>,
    console_messages_with_context: usize,
    console_messages_by_context: BTreeMap<i64, usize>,
    lifecycle_errors: usize,
}

impl TargetRuntimeObservableSourceSummary {
    #[cfg(test)]
    pub(in crate::domains) fn from_counts(
        console_messages_with_context: usize,
        console_messages_by_context: BTreeMap<i64, usize>,
        lifecycle_errors: usize,
    ) -> Self {
        Self {
            default_execution_context_id: None,
            console_messages_with_context,
            console_messages_by_context,
            lifecycle_errors,
        }
    }

    fn zero_with_default_execution_context(default_execution_context_id: Option<i64>) -> Self {
        Self {
            default_execution_context_id,
            console_messages_with_context: 0,
            console_messages_by_context: BTreeMap::new(),
            lifecycle_errors: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_renderer_snapshot(snapshot: &RendererPageDiagnosticsSnapshot) -> Self {
        if let Some(source) = snapshot.runtime_observable_source() {
            let source_items = source_items_from_renderer_runtime_source(source);
            return Self::from_source_items(source.default_execution_context_id(), &source_items)
                .expect("renderer runtime observable source snapshot should have valid cursors");
        }

        Self {
            default_execution_context_id: None,
            console_messages_with_context: snapshot
                .diagnostics
                .runtime_console_messages_with_context,
            console_messages_by_context: snapshot
                .diagnostics
                .runtime_console_messages_by_context
                .clone(),
            lifecycle_errors: snapshot.diagnostics.runtime_lifecycle_errors,
        }
    }

    #[cfg(test)]
    fn from_source_items(
        default_execution_context_id: Option<i64>,
        source_items: &[TargetRuntimeObservableSourceItem],
    ) -> Option<Self> {
        Self::zero_with_default_execution_context(default_execution_context_id)
            .advance_with_items(default_execution_context_id, source_items)
    }

    fn advance_with_items(
        &self,
        default_execution_context_id: Option<i64>,
        source_items: &[TargetRuntimeObservableSourceItem],
    ) -> Option<Self> {
        let mut next = Self {
            default_execution_context_id,
            console_messages_with_context: self.console_messages_with_context,
            console_messages_by_context: self.console_messages_by_context.clone(),
            lifecycle_errors: self.lifecycle_errors,
        };
        for item in source_items {
            match item {
                TargetRuntimeObservableSourceItem::ConsoleMessage {
                    message,
                    context_count_end,
                } => {
                    let current = next
                        .console_messages_by_context
                        .get(&message.execution_context_id)
                        .copied()
                        .unwrap_or_default();
                    if *context_count_end != current.checked_add(1)? {
                        return None;
                    }
                    next.console_messages_by_context
                        .insert(message.execution_context_id, *context_count_end);
                    next.console_messages_with_context =
                        next.console_messages_with_context.checked_add(1)?;
                }
                TargetRuntimeObservableSourceItem::LifecycleError {
                    exception_index, ..
                } => {
                    if *exception_index != next.lifecycle_errors {
                        return None;
                    }
                    next.lifecycle_errors = next.lifecycle_errors.checked_add(1)?;
                }
            }
        }
        Some(next)
    }

    pub(crate) fn console_messages_with_context(&self) -> usize {
        self.console_messages_with_context
    }

    pub(crate) fn default_execution_context_id(&self) -> Option<i64> {
        self.default_execution_context_id
    }

    pub(crate) fn console_messages_by_context(&self) -> &BTreeMap<i64, usize> {
        &self.console_messages_by_context
    }

    pub(crate) fn lifecycle_errors(&self) -> usize {
        self.lifecycle_errors
    }

    fn is_empty(&self) -> bool {
        self.console_messages_with_context == 0 && self.lifecycle_errors == 0
    }
}

#[cfg(test)]
fn source_items_from_renderer_runtime_source(
    source: &RendererRuntimeObservableSourceSummary,
) -> Vec<TargetRuntimeObservableSourceItem> {
    source
        .source_items()
        .iter()
        .cloned()
        .map(|item| match item {
            CoreRendererRuntimeObservableSourceItem::ConsoleMessage {
                message,
                context_count_end,
            } => TargetRuntimeObservableSourceItem::console_message(message, context_count_end),
            CoreRendererRuntimeObservableSourceItem::LifecycleError {
                text,
                execution_context_id,
                exception_index,
            } => TargetRuntimeObservableSourceItem::lifecycle_error(
                text,
                execution_context_id,
                exception_index,
            ),
        })
        .collect()
}

#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub(in crate::domains) struct RuntimeObservableEmissionSnapshot {
    exception_start: usize,
    console_messages: Vec<RuntimeConsoleMessageSnapshot>,
    context_console_counts: HashMap<i64, usize>,
    lifecycle_errors: Vec<String>,
}

#[cfg(test)]
impl RuntimeObservableEmissionSnapshot {
    pub(in crate::domains) fn new(
        exception_start: usize,
        console_messages: Vec<RuntimeConsoleMessageSnapshot>,
        context_console_counts: HashMap<i64, usize>,
        lifecycle_errors: Vec<String>,
    ) -> Self {
        Self {
            exception_start,
            console_messages,
            context_console_counts,
            lifecycle_errors,
        }
    }

    pub(in crate::domains) fn exception_start(&self) -> usize {
        self.exception_start
    }

    pub(in crate::domains) fn exception_end(&self) -> usize {
        self.exception_start + self.lifecycle_errors.len()
    }

    pub(in crate::domains) fn console_messages(&self) -> &[RuntimeConsoleMessageSnapshot] {
        &self.console_messages
    }

    pub(in crate::domains) fn context_console_counts(&self) -> &HashMap<i64, usize> {
        &self.context_console_counts
    }

    pub(in crate::domains) fn lifecycle_errors(&self) -> &[String] {
        &self.lifecycle_errors
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TargetRuntimeObservableQueueState {
    observable_output_items: Vec<ScriptObservableOutputItem>,
    source_inspector_issues: Vec<moli_core::page::InspectorIssueSnapshot>,
    latest_source_tail: Option<TargetRuntimeObservableSourceOutput>,
    source_tails_by_identity: HashMap<(String, DocumentId), TargetRuntimeObservableSourceOutput>,
    source_outputs: Vec<TargetRuntimeObservableSourceOutput>,
}

impl TargetRuntimeObservableQueueState {
    pub(crate) fn reset(&mut self) {
        self.observable_output_items.clear();
        self.source_inspector_issues.clear();
        self.latest_source_tail = None;
        self.source_tails_by_identity.clear();
        self.source_outputs.clear();
    }

    pub(crate) fn reset_output_queue(&mut self) {
        self.reset();
    }

    /// Reconciles a cumulative report snapshot, including producer rewinds.
    pub(crate) fn ingest_observable_output_snapshot(
        &mut self,
        items: &[ScriptObservableOutputItem],
    ) {
        if self.observable_output_items.len() > items.len()
            || self
                .observable_output_items
                .iter()
                .zip(items.iter())
                .any(|(previous, next)| previous != next)
        {
            self.observable_output_items.clear();
        }

        let start_item_index = self.observable_output_items.len();
        self.observable_output_items
            .extend(items[start_item_index..].iter().cloned());
    }

    fn console_event_count(&self) -> usize {
        self.observable_output_items
            .iter()
            .filter(|item| matches!(item, ScriptObservableOutputItem::ConsoleMessage(_)))
            .count()
    }

    fn lifecycle_error_event_count(&self) -> usize {
        self.observable_output_items
            .iter()
            .filter(|item| matches!(item, ScriptObservableOutputItem::LifecycleError(_)))
            .count()
    }

    pub(crate) fn inspector_issues(&self) -> Vec<moli_core::page::InspectorIssueSnapshot> {
        let report_issues = self
            .observable_output_items
            .iter()
            .filter_map(|item| match item {
                ScriptObservableOutputItem::InspectorIssue(issue) => Some((**issue).clone()),
                ScriptObservableOutputItem::ConsoleMessage(_)
                | ScriptObservableOutputItem::LifecycleError(_) => None,
            })
            .collect::<Vec<_>>();
        if report_issues.len() <= self.source_inspector_issues.len()
            && report_issues
                .iter()
                .zip(&self.source_inspector_issues)
                .all(|(report, source)| report == source)
        {
            self.source_inspector_issues.clone()
        } else {
            report_issues
        }
    }

    #[cfg(test)]
    pub(crate) fn sync_source_from_renderer_snapshot(
        &mut self,
        url: String,
        document_id: DocumentId,
        source: &RendererPageDiagnosticsSnapshot,
    ) -> Option<TargetRuntimeObservableSourceOutput> {
        let renderer_source = source.runtime_observable_source()?;
        self.sync_source_from_renderer_runtime_source(url, document_id, renderer_source)
    }

    #[cfg(test)]
    pub(crate) fn sync_source_from_renderer_runtime_source(
        &mut self,
        url: String,
        document_id: DocumentId,
        renderer_source: &RendererRuntimeObservableSourceSummary,
    ) -> Option<TargetRuntimeObservableSourceOutput> {
        self.source_inspector_issues = renderer_source.inspector_issues().to_vec();
        let source_items = source_items_from_renderer_runtime_source(renderer_source);
        let summary = TargetRuntimeObservableSourceSummary::from_source_items(
            renderer_source.default_execution_context_id(),
            &source_items,
        )?;
        if summary.is_empty() {
            return None;
        }
        let source_url = url.clone();
        let source_item_count = source_items.len();
        let Some(cursor) = self.source_tail_for_identity(&url, document_id) else {
            self.append_source_output(
                url,
                document_id,
                0,
                source_item_count,
                TargetRuntimeObservableSourceSummary::zero_with_default_execution_context(
                    summary.default_execution_context_id(),
                ),
                summary,
                source_items,
            );
            return self.source_tail_for_identity(&source_url, document_id);
        };
        let Some(cursor_summary) = cursor.source_summary() else {
            self.rebuild_source_outputs_from_renderer_source(
                url,
                document_id,
                summary,
                source_items,
            );
            return self.source_tail_for_identity(&source_url, document_id);
        };
        if cursor_summary == summary {
            return self.source_tail_for_identity(&source_url, document_id);
        }
        if source_item_count < cursor.source_item_end_index {
            self.rebuild_source_outputs_from_renderer_source(
                url,
                document_id,
                summary,
                source_items,
            );
            return self.source_tail_for_identity(&source_url, document_id);
        }
        let delta_items = source_items[cursor.source_item_end_index..].to_vec();
        if delta_items.is_empty() {
            self.rebuild_source_outputs_from_renderer_source(
                url,
                document_id,
                summary,
                source_items,
            );
            return self.source_tail_for_identity(&source_url, document_id);
        }
        let Some(delta_summary) =
            cursor_summary.advance_with_items(summary.default_execution_context_id(), &delta_items)
        else {
            self.rebuild_source_outputs_from_renderer_source(
                url,
                document_id,
                summary,
                source_items,
            );
            return self.source_tail_for_identity(&source_url, document_id);
        };
        if delta_summary != summary {
            self.rebuild_source_outputs_from_renderer_source(
                url,
                document_id,
                summary,
                source_items,
            );
            return self.source_tail_for_identity(&source_url, document_id);
        }
        if !self.append_source_output(
            url.clone(),
            document_id,
            cursor.source_item_end_index,
            source_item_count,
            cursor_summary,
            delta_summary,
            delta_items,
        ) {
            self.rebuild_source_outputs_from_renderer_source(
                url,
                document_id,
                summary,
                source_items,
            );
        }
        self.source_tail_for_identity(&source_url, document_id)
    }

    /// Appends one concrete console fact already ordered by the renderer
    /// output stream.
    ///
    /// Unlike the legacy renderer summary path, this never rebuilds or diffs
    /// a source snapshot. Protocol owns the durable per-attachment cursor and
    /// advances it exactly once for the admitted record.
    pub(crate) fn append_renderer_console_message(
        &mut self,
        url: String,
        document_id: DocumentId,
        message: RuntimeConsoleMessageSnapshot,
    ) -> Option<TargetRuntimeObservableSourceOutput> {
        let key = (url.clone(), document_id);
        let (source_item_start_index, start_summary) = self
            .source_tails_by_identity
            .get(&key)
            .and_then(|tail| Some((tail.source_item_end_index, tail.source_summary()?)))
            .unwrap_or_else(|| {
                (
                    0,
                    TargetRuntimeObservableSourceSummary::zero_with_default_execution_context(None),
                )
            });
        let context_count_end = start_summary
            .console_messages_by_context
            .get(&message.execution_context_id)
            .copied()
            .unwrap_or_default()
            .checked_add(1)
            .expect("Runtime console source context count overflow");
        let source_items = vec![TargetRuntimeObservableSourceItem::console_message(
            message,
            context_count_end,
        )];
        let summary = start_summary
            .advance_with_items(start_summary.default_execution_context_id(), &source_items)
            .expect("one concrete Runtime console record must advance its source cursor");
        let source_item_end_index = source_item_start_index
            .checked_add(1)
            .expect("Runtime console source item index overflow");
        assert!(
            self.append_source_output(
                url.clone(),
                document_id,
                source_item_start_index,
                source_item_end_index,
                start_summary,
                summary,
                source_items,
            ),
            "one concrete Runtime console record must form a valid source delta"
        );
        self.source_tail_for_identity(&url, document_id)
    }

    /// Appends one concrete renderer lifecycle error in stream order.
    ///
    /// `exception_index` is protocol-owned cursor state. The renderer freezes
    /// the error text and source realm at production time; it does not need to
    /// copy the cumulative diagnostic queue's index into the protocol record.
    pub(crate) fn append_renderer_lifecycle_error(
        &mut self,
        url: String,
        document_id: DocumentId,
        text: String,
        execution_context_id: Option<i64>,
    ) -> Option<TargetRuntimeObservableSourceOutput> {
        let key = (url.clone(), document_id);
        let (source_item_start_index, start_summary) = self
            .source_tails_by_identity
            .get(&key)
            .and_then(|tail| Some((tail.source_item_end_index, tail.source_summary()?)))
            .unwrap_or_else(|| {
                (
                    0,
                    TargetRuntimeObservableSourceSummary::zero_with_default_execution_context(None),
                )
            });
        let source_items = vec![TargetRuntimeObservableSourceItem::lifecycle_error(
            text,
            execution_context_id,
            start_summary.lifecycle_errors(),
        )];
        let summary = start_summary
            .advance_with_items(start_summary.default_execution_context_id(), &source_items)
            .expect("one concrete Runtime lifecycle error must advance its source cursor");
        let source_item_end_index = source_item_start_index
            .checked_add(1)
            .expect("Runtime lifecycle error source item index overflow");
        assert!(
            self.append_source_output(
                url.clone(),
                document_id,
                source_item_start_index,
                source_item_end_index,
                start_summary,
                summary,
                source_items,
            ),
            "one concrete Runtime lifecycle error must form a valid source delta"
        );
        self.source_tail_for_identity(&url, document_id)
    }

    fn source_tail_for_identity(
        &self,
        url: &str,
        document_id: DocumentId,
    ) -> Option<TargetRuntimeObservableSourceOutput> {
        self.source_tails_by_identity
            .get(&(url.to_owned(), document_id))
            .cloned()
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> TargetRuntimeObservableQueueSnapshot {
        self.snapshot_with_source_outputs(Vec::new())
    }

    pub(crate) fn latest_source_tail(&self) -> Option<TargetRuntimeObservableSourceOutput> {
        self.latest_source_tail.clone()
    }

    pub(crate) fn observable_output_cursor_end(&self) -> Option<(usize, usize)> {
        (!self.observable_output_items.is_empty()).then(|| {
            (
                self.console_event_count(),
                self.lifecycle_error_event_count(),
            )
        })
    }

    #[cfg(test)]
    pub(crate) fn source_snapshot(&self) -> TargetRuntimeObservableQueueSnapshot {
        self.snapshot_with_source_outputs(self.source_outputs.clone())
    }

    #[cfg(test)]
    fn snapshot_with_source_outputs(
        &self,
        source_outputs: Vec<TargetRuntimeObservableSourceOutput>,
    ) -> TargetRuntimeObservableQueueSnapshot {
        TargetRuntimeObservableQueueSnapshot {
            observable_output_items: self.observable_output_items.clone(),
            source_outputs,
        }
    }

    fn append_source_output(
        &mut self,
        url: String,
        document_id: DocumentId,
        source_item_start_index: usize,
        source_item_end_index: usize,
        start_summary: TargetRuntimeObservableSourceSummary,
        summary: TargetRuntimeObservableSourceSummary,
        source_items: Vec<TargetRuntimeObservableSourceItem>,
    ) -> bool {
        let Some(output) = TargetRuntimeObservableSourceOutput::from_delta(
            url.clone(),
            document_id,
            source_item_start_index,
            source_item_end_index,
            start_summary,
            summary.clone(),
            source_items,
        ) else {
            return false;
        };
        self.remember_latest_source_tail(&output);
        self.source_outputs.push(output);
        true
    }

    fn remember_latest_source_tail(&mut self, output: &TargetRuntimeObservableSourceOutput) {
        let key = (output.url.clone(), output.document_id);
        let next = if let Some(current) = self.source_tails_by_identity.remove(&key) {
            TargetRuntimeObservableSourceOutput::combine_same_identity([&current, output])
                .unwrap_or_else(|| output.clone())
        } else {
            output.clone()
        };
        self.source_tails_by_identity.insert(key, next.clone());
        self.latest_source_tail = Some(next);
    }

    #[cfg(test)]
    fn rebuild_source_outputs_from_renderer_source(
        &mut self,
        url: String,
        document_id: DocumentId,
        summary: TargetRuntimeObservableSourceSummary,
        source_items: Vec<TargetRuntimeObservableSourceItem>,
    ) {
        let source_item_count = source_items.len();
        self.source_outputs.clear();
        self.latest_source_tail = None;
        self.source_tails_by_identity.clear();
        self.append_source_output(
            url,
            document_id,
            0,
            source_item_count,
            TargetRuntimeObservableSourceSummary::zero_with_default_execution_context(
                summary.default_execution_context_id(),
            ),
            summary,
            source_items,
        );
    }
}

#[cfg(test)]
mod tests {
    use moli_core::page::{
        RendererActivityDiagnostics, RendererPageDiagnosticsSnapshot,
        RendererRuntimeObservableSourceItem, RendererRuntimeObservableSourceSummary,
        RuntimeConsoleMessageSnapshot, ScriptObservableOutputItem,
    };

    use super::{
        TargetRuntimeObservableQueueSnapshot, TargetRuntimeObservableQueueState,
        TargetRuntimeObservableSourceItem, TargetRuntimeObservableSourceSummary,
    };
    use crate::conn::DocumentId;

    fn document_id(raw: u64) -> DocumentId {
        DocumentId::from_raw_for_test(raw)
    }

    fn renderer_source_snapshot(
        source: RendererRuntimeObservableSourceSummary,
    ) -> RendererPageDiagnosticsSnapshot {
        RendererPageDiagnosticsSnapshot::from_runtime_observable_source(source)
    }

    fn observable_output_items(
        console: &[&str],
        lifecycle_errors: &[&str],
    ) -> Vec<ScriptObservableOutputItem> {
        let mut events = console
            .iter()
            .map(|message| ScriptObservableOutputItem::ConsoleMessage((*message).to_owned()))
            .collect::<Vec<_>>();
        events.extend(
            lifecycle_errors
                .iter()
                .map(|error| ScriptObservableOutputItem::LifecycleError((*error).to_owned())),
        );
        events
    }

    fn apply_observable_page_output_update(
        queue: &mut TargetRuntimeObservableQueueState,
        items: &[ScriptObservableOutputItem],
    ) {
        queue.ingest_observable_output_snapshot(items);
    }

    #[test]
    fn target_runtime_observable_queue_appends_matching_tail() {
        let mut queue = TargetRuntimeObservableQueueState::default();

        apply_observable_page_output_update(
            &mut queue,
            &[
                ScriptObservableOutputItem::ConsoleMessage("console-a".to_owned()),
                ScriptObservableOutputItem::LifecycleError("error-a".to_owned()),
            ],
        );
        apply_observable_page_output_update(
            &mut queue,
            &[
                ScriptObservableOutputItem::ConsoleMessage("console-a".to_owned()),
                ScriptObservableOutputItem::LifecycleError("error-a".to_owned()),
                ScriptObservableOutputItem::ConsoleMessage("console-b".to_owned()),
                ScriptObservableOutputItem::LifecycleError("error-b".to_owned()),
            ],
        );

        assert_eq!(
            queue.snapshot(),
            TargetRuntimeObservableQueueSnapshot {
                observable_output_items: vec![
                    ScriptObservableOutputItem::ConsoleMessage("console-a".to_owned()),
                    ScriptObservableOutputItem::LifecycleError("error-a".to_owned()),
                    ScriptObservableOutputItem::ConsoleMessage("console-b".to_owned()),
                    ScriptObservableOutputItem::LifecycleError("error-b".to_owned()),
                ],
                source_outputs: Vec::new(),
            }
        );
    }

    #[test]
    fn target_runtime_observable_queue_append_update_preserves_producer_item_order() {
        let mut queue = TargetRuntimeObservableQueueState::default();
        let first_items = vec![ScriptObservableOutputItem::ConsoleMessage(
            "console-a".to_owned(),
        )];
        let all_items = vec![
            ScriptObservableOutputItem::ConsoleMessage("console-a".to_owned()),
            ScriptObservableOutputItem::LifecycleError("error-a".to_owned()),
            ScriptObservableOutputItem::ConsoleMessage("console-b".to_owned()),
        ];

        apply_observable_page_output_update(&mut queue, &first_items);
        apply_observable_page_output_update(&mut queue, &all_items);

        assert_eq!(
            queue.snapshot(),
            TargetRuntimeObservableQueueSnapshot {
                observable_output_items: vec![
                    ScriptObservableOutputItem::ConsoleMessage("console-a".to_owned()),
                    ScriptObservableOutputItem::LifecycleError("error-a".to_owned()),
                    ScriptObservableOutputItem::ConsoleMessage("console-b".to_owned()),
                ],
                source_outputs: Vec::new(),
            },
            "observable producer item append update should append from the source item cursor instead of regrouping by event family"
        );
        assert_eq!(queue.observable_output_items.len(), 3);
    }

    #[test]
    fn target_runtime_observable_queue_reports_owner_output_cursor_end() {
        let mut queue = TargetRuntimeObservableQueueState::default();
        assert_eq!(
            queue.observable_output_cursor_end(),
            None,
            "empty owner output queue should not masquerade as a synced cursor source"
        );

        apply_observable_page_output_update(
            &mut queue,
            &observable_output_items(&["console-a", "console-b"], &["error-a"]),
        );

        assert_eq!(
            queue.observable_output_cursor_end(),
            Some((2, 1)),
            "aggregate cursor should be derived from already-ingested owner output only"
        );
    }

    #[test]
    fn target_runtime_observable_queue_recovers_producer_items_on_prefix_drift() {
        let mut queue = TargetRuntimeObservableQueueState::default();
        let first_items = vec![ScriptObservableOutputItem::ConsoleMessage("old".to_owned())];
        let replacement_items = vec![ScriptObservableOutputItem::LifecycleError(
            "replacement".to_owned(),
        )];

        apply_observable_page_output_update(&mut queue, &first_items);
        apply_observable_page_output_update(&mut queue, &replacement_items);

        assert_eq!(
            queue.snapshot(),
            TargetRuntimeObservableQueueSnapshot {
                observable_output_items: vec![ScriptObservableOutputItem::LifecycleError(
                    "replacement".to_owned()
                )],
                source_outputs: Vec::new(),
            },
            "observable producer item prefix drift must rebuild same-count replacement output instead of trusting the previous item cursor"
        );
        assert_eq!(queue.observable_output_items.len(), 1);
    }

    #[test]
    fn target_runtime_observable_queue_rebuilds_on_rewind_or_replacement() {
        let mut queue = TargetRuntimeObservableQueueState::default();

        apply_observable_page_output_update(
            &mut queue,
            &[
                ScriptObservableOutputItem::ConsoleMessage("console-a".to_owned()),
                ScriptObservableOutputItem::ConsoleMessage("console-b".to_owned()),
                ScriptObservableOutputItem::LifecycleError("error-a".to_owned()),
                ScriptObservableOutputItem::LifecycleError("error-b".to_owned()),
            ],
        );
        apply_observable_page_output_update(
            &mut queue,
            &[ScriptObservableOutputItem::ConsoleMessage(
                "console-new".to_owned(),
            )],
        );

        assert_eq!(
            queue.snapshot(),
            TargetRuntimeObservableQueueSnapshot {
                observable_output_items: observable_output_items(&["console-new"], &[]),
                source_outputs: Vec::new(),
            }
        );
    }

    #[test]
    fn target_runtime_observable_queue_appends_source_outputs_until_reset() {
        let mut queue = TargetRuntimeObservableQueueState::default();
        let source_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![runtime_console_message(7, "source console")],
                vec!["source lifecycle".to_owned()],
            ),
        );

        queue.sync_source_from_renderer_snapshot(
            "http://example.test/runtime-source".to_owned(),
            document_id(17),
            &source_snapshot,
        );

        assert_eq!(
            queue.snapshot().source_outputs,
            Vec::new(),
            "plain backlog snapshots should not prepare RuntimeObservable source output"
        );
        let source_outputs = queue.source_snapshot().source_outputs;
        assert_eq!(
            source_outputs.len(),
            1,
            "source snapshots should read RuntimeObservable source outputs owned by the queue",
        );
        let output = source_outputs
            .last()
            .expect("source output should be available");
        assert_eq!(output.url(), "http://example.test/runtime-source");
        assert_eq!(output.document_id().get(), 17);
        assert_eq!(output.source_item_start_index(), 0);
        assert_eq!(output.source_item_end_index(), 2);
        assert_eq!(
            output.summary(),
            TargetRuntimeObservableSourceSummary::from_renderer_snapshot(&source_snapshot),
        );
        assert_eq!(
            output.source_items().len(),
            2,
            "source output should carry concrete source items, not just the count summary"
        );

        queue.reset();

        assert_eq!(
            queue.source_snapshot().source_outputs,
            Vec::new(),
            "target reset should clear the owned RuntimeObservable source output"
        );
    }

    #[test]
    fn target_runtime_observable_queue_deduplicates_unchanged_source_output() {
        let mut queue = TargetRuntimeObservableQueueState::default();
        let first_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![runtime_console_message(7, "first source console")],
                Vec::new(),
            ),
        );
        let second_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![
                    runtime_console_message(7, "first source console"),
                    runtime_console_message(7, "second source console"),
                ],
                Vec::new(),
            ),
        );

        queue.sync_source_from_renderer_snapshot(
            "http://example.test/runtime-source".to_owned(),
            document_id(17),
            &first_snapshot,
        );
        queue.sync_source_from_renderer_snapshot(
            "http://example.test/runtime-source".to_owned(),
            document_id(17),
            &first_snapshot,
        );
        queue.sync_source_from_renderer_snapshot(
            "http://example.test/runtime-source".to_owned(),
            document_id(17),
            &second_snapshot,
        );

        let source_outputs = queue.source_snapshot().source_outputs;
        assert_eq!(
            source_outputs.len(),
            2,
            "unchanged source snapshots should not append duplicate source items, but advanced summaries should append"
        );
        assert_eq!(
            source_outputs[0].summary(),
            TargetRuntimeObservableSourceSummary::from_renderer_snapshot(&first_snapshot)
        );
        assert_eq!(
            source_outputs[1].summary(),
            TargetRuntimeObservableSourceSummary::from_renderer_snapshot(&second_snapshot)
        );
        assert_eq!(
            source_outputs[0].source_items().len(),
            1,
            "the first appended source item should carry the initial source item"
        );
        assert_eq!(source_outputs[0].source_item_start_index(), 0);
        assert_eq!(source_outputs[0].source_item_end_index(), 1);
        assert_eq!(
            source_outputs[1].source_items().len(),
            1,
            "advanced source snapshots should append only the new source item delta"
        );
        assert_eq!(
            source_outputs[1].source_item_start_index(),
            1,
            "advanced source output should be cut from the queue-owned producer cursor"
        );
        assert_eq!(source_outputs[1].source_item_end_index(), 2);
        assert!(matches!(
            &source_outputs[1].source_items()[0],
            TargetRuntimeObservableSourceItem::ConsoleMessage { message, .. }
                if message.message == "second source console"
        ));
    }

    #[test]
    fn target_runtime_observable_queue_snapshot_exposes_latest_source_tail() {
        let mut queue = TargetRuntimeObservableQueueState::default();
        let first_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![runtime_console_message(7, "first identity console")],
                Vec::new(),
            ),
        );
        let second_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![
                    runtime_console_message(7, "second identity first console"),
                    runtime_console_message(7, "second identity second console"),
                ],
                Vec::new(),
            ),
        );

        queue.sync_source_from_renderer_snapshot(
            "http://example.test/first-source".to_owned(),
            document_id(17),
            &first_snapshot,
        );
        queue.sync_source_from_renderer_snapshot(
            "http://example.test/second-source".to_owned(),
            document_id(18),
            &second_snapshot,
        );

        let source = queue
            .source_snapshot()
            .latest_source_tail()
            .expect("latest source identity should expose a prepared tail");

        assert_eq!(source.url(), "http://example.test/second-source");
        assert_eq!(source.document_id().get(), 18);
        assert_eq!(
            source.source_console_messages(),
            vec![
                runtime_console_message(7, "second identity first console"),
                runtime_console_message(7, "second identity second console"),
            ],
            "latest source tail should be queue-owned and should not include older source identities"
        );
    }

    #[test]
    fn target_runtime_observable_queue_caches_source_tails_by_identity() {
        let mut queue = TargetRuntimeObservableQueueState::default();
        let first_initial_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![runtime_console_message(7, "first identity first console")],
                Vec::new(),
            ),
        );
        let second_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(9),
                vec![runtime_console_message(9, "second identity console")],
                Vec::new(),
            ),
        );
        let first_advanced_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![
                    runtime_console_message(7, "first identity first console"),
                    runtime_console_message(7, "first identity second console"),
                ],
                Vec::new(),
            ),
        );

        queue.sync_source_from_renderer_snapshot(
            "http://example.test/first-source".to_owned(),
            document_id(17),
            &first_initial_snapshot,
        );
        queue.sync_source_from_renderer_snapshot(
            "http://example.test/second-source".to_owned(),
            document_id(18),
            &second_snapshot,
        );
        queue.sync_source_from_renderer_snapshot(
            "http://example.test/first-source".to_owned(),
            document_id(17),
            &first_advanced_snapshot,
        );

        let source_outputs = queue.source_snapshot().source_outputs;
        assert_eq!(
            source_outputs
                .last()
                .expect("advanced source output should be appended")
                .source_item_start_index(),
            1,
            "source tail cache should resume the prior identity cursor instead of rebuilding from zero"
        );
        let first_tail = queue
            .source_tail_for_identity("http://example.test/first-source", document_id(17))
            .expect(
                "first identity source tail should stay cached after a different latest source",
            );
        assert_eq!(
            first_tail.source_console_messages(),
            vec![
                runtime_console_message(7, "first identity first console"),
                runtime_console_message(7, "first identity second console"),
            ]
        );
        assert_eq!(
            queue
                .latest_source_tail()
                .expect("latest source tail should track the most recent append")
                .url(),
            "http://example.test/first-source"
        );
    }

    #[test]
    fn target_runtime_observable_queue_does_not_return_stale_tail_for_empty_source_snapshot() {
        let mut queue = TargetRuntimeObservableQueueState::default();
        let source_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![runtime_console_message(7, "cached source console")],
                Vec::new(),
            ),
        );

        assert!(
            queue
                .sync_source_from_renderer_snapshot(
                    "http://example.test/source".to_owned(),
                    document_id(17),
                    &source_snapshot,
                )
                .is_some(),
            "initial source snapshot should populate the owner source tail cache"
        );
        assert!(
            queue
                .sync_source_from_renderer_snapshot(
                    "http://example.test/source".to_owned(),
                    document_id(17),
                    &RendererPageDiagnosticsSnapshot::default(),
                )
                .is_none(),
            "a source snapshot without RuntimeObservable source must not return the previously cached tail"
        );
        assert!(
            queue.latest_source_tail().is_some(),
            "empty source snapshots should not clear the existing cache; they should only avoid producing output for this source boundary"
        );
    }

    #[test]
    fn target_runtime_observable_queue_derives_source_cursor_from_interleaved_outputs() {
        let mut queue = TargetRuntimeObservableQueueState::default();
        let first_a_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![runtime_console_message(7, "source-a first console")],
                Vec::new(),
            ),
        );
        let b_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![runtime_console_message(7, "source-b console")],
                Vec::new(),
            ),
        );
        let second_a_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![
                    runtime_console_message(7, "source-a first console"),
                    runtime_console_message(7, "source-a second console"),
                ],
                Vec::new(),
            ),
        );

        queue.sync_source_from_renderer_snapshot(
            "http://example.test/source-a".to_owned(),
            document_id(17),
            &first_a_snapshot,
        );
        queue.sync_source_from_renderer_snapshot(
            "http://example.test/source-b".to_owned(),
            document_id(18),
            &b_snapshot,
        );
        queue.sync_source_from_renderer_snapshot(
            "http://example.test/source-a".to_owned(),
            document_id(17),
            &second_a_snapshot,
        );

        let source_outputs = queue.source_snapshot().source_outputs;
        assert_eq!(
            source_outputs.len(),
            3,
            "interleaved source identities should not force a rebuild when an older identity advances"
        );
        assert_eq!(source_outputs[2].url(), "http://example.test/source-a");
        assert_eq!(source_outputs[2].source_item_start_index(), 1);
        assert_eq!(source_outputs[2].source_item_end_index(), 2);
        assert_eq!(
            source_outputs[2].source_console_messages(),
            vec![runtime_console_message(7, "source-a second console")],
            "the source cursor should be derived from prior queue-owned outputs for the same identity"
        );
    }

    #[test]
    fn target_runtime_observable_queue_rebuilds_source_outputs_on_renderer_source_rewind() {
        let mut queue = TargetRuntimeObservableQueueState::default();
        let first_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![
                    runtime_console_message(7, "first source console"),
                    runtime_console_message(7, "second source console"),
                ],
                Vec::new(),
            ),
        );
        let rewind_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![runtime_console_message(7, "replacement source console")],
                Vec::new(),
            ),
        );

        queue.sync_source_from_renderer_snapshot(
            "http://example.test/runtime-source".to_owned(),
            document_id(17),
            &first_snapshot,
        );
        queue.sync_source_from_renderer_snapshot(
            "http://example.test/runtime-source".to_owned(),
            document_id(17),
            &rewind_snapshot,
        );

        let source_outputs = queue.source_snapshot().source_outputs;
        assert_eq!(
            source_outputs.len(),
            1,
            "renderer source rewind should rebuild the queue-owned source cursor instead of appending an invalid delta"
        );
        assert_eq!(source_outputs[0].source_item_start_index(), 0);
        assert_eq!(source_outputs[0].source_item_end_index(), 1);
        assert!(matches!(
            &source_outputs[0].source_items()[0],
            TargetRuntimeObservableSourceItem::ConsoleMessage { message, .. }
                if message.message == "replacement source console"
        ));
    }

    #[test]
    fn target_runtime_observable_queue_rebuilds_source_outputs_on_summary_change_without_new_items()
    {
        let mut queue = TargetRuntimeObservableQueueState::default();
        let first_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![runtime_console_message(7, "source console")],
                Vec::new(),
            ),
        );
        let default_context_change_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(9),
                vec![runtime_console_message(7, "source console")],
                Vec::new(),
            ),
        );

        queue.sync_source_from_renderer_snapshot(
            "http://example.test/runtime-source".to_owned(),
            document_id(17),
            &first_snapshot,
        );
        queue.sync_source_from_renderer_snapshot(
            "http://example.test/runtime-source".to_owned(),
            document_id(17),
            &default_context_change_snapshot,
        );

        let source_outputs = queue.source_snapshot().source_outputs;
        assert_eq!(
            source_outputs.len(),
            1,
            "summary changes without a higher item cursor should rebuild instead of leaving stale prepared source state"
        );
        assert_eq!(
            source_outputs[0].summary(),
            TargetRuntimeObservableSourceSummary::from_renderer_snapshot(
                &default_context_change_snapshot
            )
        );
        assert_eq!(source_outputs[0].source_item_start_index(), 0);
        assert_eq!(source_outputs[0].source_item_end_index(), 1);
    }

    #[test]
    fn target_runtime_observable_queue_rejects_diagnostics_only_source_output() {
        let mut queue = TargetRuntimeObservableQueueState::default();
        let diagnostics_only_snapshot =
            RendererPageDiagnosticsSnapshot::from_diagnostics(RendererActivityDiagnostics {
                runtime_console_messages_with_context: 2,
                runtime_lifecycle_errors: 1,
                ..Default::default()
            });

        queue.sync_source_from_renderer_snapshot(
            "http://example.test/diagnostics-only-runtime-source".to_owned(),
            document_id(41),
            &diagnostics_only_snapshot,
        );

        assert_eq!(
            queue.source_snapshot().source_outputs,
            Vec::new(),
            "RuntimeObservable source output should require typed renderer source items instead of diagnostics-only counts"
        );
    }

    #[test]
    fn target_runtime_observable_queue_rejects_nonconsecutive_source_item_cursor() {
        let mut queue = TargetRuntimeObservableQueueState::default();
        let invalid_source_snapshot =
            renderer_source_snapshot(RendererRuntimeObservableSourceSummary::from_source_items(
                Some(7),
                vec![RendererRuntimeObservableSourceItem::ConsoleMessage {
                    message: runtime_console_message(7, "source console with skipped cursor"),
                    context_count_end: 2,
                }],
            ));

        queue.sync_source_from_renderer_snapshot(
            "http://example.test/invalid-runtime-source".to_owned(),
            document_id(41),
            &invalid_source_snapshot,
        );

        assert_eq!(
            queue.source_snapshot().source_outputs,
            Vec::new(),
            "RuntimeObservable source output should be derived from source item cursor tags and reject skipped per-context cursors"
        );
    }

    #[test]
    fn target_runtime_observable_queue_does_not_store_empty_source_output() {
        let mut queue = TargetRuntimeObservableQueueState::default();

        queue.sync_source_from_renderer_snapshot(
            "http://example.test/empty-runtime-source".to_owned(),
            document_id(29),
            &RendererPageDiagnosticsSnapshot::default(),
        );

        assert_eq!(
            queue.source_snapshot().source_outputs,
            Vec::new(),
            "empty RuntimeObservable source snapshots should not create prepared output"
        );
    }

    #[test]
    fn target_runtime_observable_source_output_owns_concrete_source_items() {
        let mut queue = TargetRuntimeObservableQueueState::default();
        let source_snapshot = renderer_source_snapshot(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(11),
                vec![runtime_console_message(11, "source item payload")],
                vec!["source lifecycle error".to_owned()],
            ),
        );

        queue.sync_source_from_renderer_snapshot(
            "http://example.test/runtime-source-items".to_owned(),
            document_id(31),
            &source_snapshot,
        );

        let output = queue
            .source_snapshot()
            .source_outputs
            .pop()
            .expect("source payload should produce source output");
        assert_eq!(
            output.summary(),
            TargetRuntimeObservableSourceSummary::from_renderer_snapshot(&source_snapshot),
            "source output should derive the count/context summary from its cursor-tagged source items"
        );
        assert_eq!(
            output.source_items().len(),
            2,
            "source output should own concrete console and lifecycle source items outside the summary"
        );
        assert!(matches!(
            &output.source_items()[0],
            TargetRuntimeObservableSourceItem::ConsoleMessage { message, .. }
                if message.execution_context_id == 11 && message.message == "source item payload"
        ));
        assert!(
            matches!(
                &output.source_items()[1],
                TargetRuntimeObservableSourceItem::LifecycleError { text, .. }
                    if text == "source lifecycle error"
            ),
            "source output should own concrete lifecycle source item payload"
        );
        assert!(
            matches!(
                &output.source_items()[0],
                TargetRuntimeObservableSourceItem::ConsoleMessage {
                    message,
                    context_count_end: 1,
                } if message.execution_context_id == 11
                    && message.message == "source item payload"
            ),
            "source output should tag console source items with their append-time context cursor"
        );
        assert!(
            matches!(
                &output.source_items()[1],
                TargetRuntimeObservableSourceItem::LifecycleError {
                    text,
                    execution_context_id: Some(11),
                    exception_index: 0,
                } if text == "source lifecycle error"
            ),
            "source output should tag lifecycle source items with append-time exception index and context"
        );
    }

    fn runtime_console_message(
        execution_context_id: i64,
        message: &str,
    ) -> RuntimeConsoleMessageSnapshot {
        RuntimeConsoleMessageSnapshot {
            execution_context_id,
            message: message.to_owned(),
            args: Vec::new(),
            stack: None,
        }
    }
}
