use crate::document_runtime::DomHandle;

use super::{
    FullStyleWorldSnapshot,
    source::store::StyloStylesheetSource,
    source_dirty::StyleSourceDirtyScopeSnapshot,
    source_id::StyleSourceId,
    source_lifecycle::{
        StyleSourceLifecycleOwnerDetailTrace, StyleSourceLifecycleOwnerDetailTraceSink,
        StyleSourceLifecycleReport, StyleSourceLifecycleSnapshot, StyleSourceLifecycleSnapshotSink,
    },
    state::RetainedStyleSystem,
    world_key::StyleWorldKeyMismatchTrace,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RetainedStyleSystemChangeKind {
    InitialBuild,
    IncrementalUpdate,
    Replacement,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StyleSourceInputTrace {
    pub(super) document_stylesheet_source_count: usize,
    pub(super) document_source_ids: Vec<Option<StyleSourceId>>,
    pub(super) shadow_stylesheet_sources: Vec<StyleSourceInputShadowRootTrace>,
    pub(super) script_custom_property_registration_count: usize,
    pub(super) script_custom_property_base_urls: Vec<url::Url>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StyleSourceInputShadowRootTrace {
    pub(super) root: DomHandle,
    pub(super) source_count: usize,
    pub(super) source_ids: Vec<Option<StyleSourceId>>,
}

pub(super) fn trace_retained_style_system_change(
    inputs: &FullStyleWorldSnapshot,
    source_lifecycle: &StyleSourceLifecycleReport,
    source_dirty_scope: &StyleSourceDirtyScopeSnapshot,
    source_set_generation: u64,
    retained: &RetainedStyleSystem,
    change_kind: RetainedStyleSystemChangeKind,
    custom_property_history_is_append_only: bool,
    key_mismatch: Option<&StyleWorldKeyMismatchTrace>,
    document_context_documents: &[DomHandle],
) {
    let mut lifecycle_snapshot = RetainedStyleSystemRebuildLifecycleSnapshot::default();
    source_lifecycle.record_snapshot_into(&mut lifecycle_snapshot);
    source_lifecycle.record_owner_detail_trace_into(&mut lifecycle_snapshot);
    let source_input = style_source_input_trace(inputs);
    tracing::info!(
        change_kind = ?change_kind,
        stylist_identity = retained.stylist_identity,
        stylist_flush_count = retained.stylist.num_rebuilds(),
        custom_property_history_is_append_only,
        document_stylesheet_input_count = inputs.document_stylesheet_sources.len(),
        shadow_stylesheet_input_count = inputs.shadow_stylesheet_sources.len(),
        retained_shadow_cascade_count = retained.shadow_cascade_data.len(),
        retained_source_cascade_data_count = retained.source_cascade_projections.len(),
        document_context_documents = ?document_context_documents,
        source_set_generation,
        source_dirty_ids = ?source_dirty_scope.source_ids_vec(),
        source_dirty_scope_ids = ?source_dirty_scope.scope_ids_vec(),
        source_dirty_roots = ?source_dirty_scope.scoped_roots_vec(),
        source_dirty_reasons = ?source_dirty_scope.reasons_vec(),
        source_dirty_records = ?source_dirty_scope.records_vec(),
        source_input = ?source_input,
        key_mismatch = ?key_mismatch,
        source_lifecycle = ?lifecycle_snapshot.snapshot,
        source_lifecycle_owner_details = ?lifecycle_snapshot.owner_details,
        "retained style system change summary"
    );
}

fn style_source_input_trace(inputs: &FullStyleWorldSnapshot) -> StyleSourceInputTrace {
    StyleSourceInputTrace {
        document_stylesheet_source_count: inputs.document_stylesheet_sources.len(),
        document_source_ids: style_source_ids_for_trace(&inputs.document_stylesheet_sources),
        shadow_stylesheet_sources: inputs
            .shadow_stylesheet_sources
            .iter()
            .map(|(root, sources)| StyleSourceInputShadowRootTrace {
                root: *root,
                source_count: sources.len(),
                source_ids: style_source_ids_for_trace(sources),
            })
            .collect(),
        script_custom_property_registration_count: inputs
            .script_custom_property_registrations
            .len(),
        script_custom_property_base_urls: inputs
            .script_custom_property_registrations
            .iter()
            .map(|record| record.base_url.clone())
            .collect(),
    }
}

fn style_source_ids_for_trace(sources: &[StyloStylesheetSource]) -> Vec<Option<StyleSourceId>> {
    sources
        .iter()
        .map(|source| source.source_id().cloned())
        .collect()
}

#[cfg(test)]
pub(super) fn style_source_input_trace_for_test(
    inputs: &FullStyleWorldSnapshot,
) -> StyleSourceInputTrace {
    style_source_input_trace(inputs)
}

#[derive(Default)]
struct RetainedStyleSystemRebuildLifecycleSnapshot {
    snapshot: StyleSourceLifecycleSnapshot,
    owner_details: Vec<StyleSourceLifecycleOwnerDetailTrace>,
}

impl StyleSourceLifecycleSnapshotSink for RetainedStyleSystemRebuildLifecycleSnapshot {
    fn record_source_lifecycle_snapshot(&mut self, snapshot: StyleSourceLifecycleSnapshot) {
        self.snapshot = snapshot;
    }
}

impl StyleSourceLifecycleOwnerDetailTraceSink for RetainedStyleSystemRebuildLifecycleSnapshot {
    fn record_source_lifecycle_owner_detail_trace(
        &mut self,
        trace: StyleSourceLifecycleOwnerDetailTrace,
    ) {
        self.owner_details.push(trace);
    }
}
