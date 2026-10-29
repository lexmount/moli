use moli_selector::{StyloDomStyleAdapter, StyloStyleSourceScope as StyleSourceScope};

use crate::{
    document_runtime::DomHandle, dom::native::DomHost, protocol_types::EmulatedMediaOverrides,
    style_engine::StyleViewport,
};

use super::{
    cause::PendingStyleInvalidationCause,
    pending_invalidation::{PendingStyleInvalidationWork, PendingStyleInvalidations},
    planner::pending_work_for_pending_cause,
    source_document::DocumentStyleSourceStores,
    state::StyleDocumentState,
};

pub(super) fn queue_style_invalidation_for_scope(
    pending_invalidations: &PendingStyleInvalidations,
    document_state: &StyleDocumentState,
    source_stores: &DocumentStyleSourceStores<'_>,
    dom_adapter: &StyloDomStyleAdapter,
    host: &DomHost,
    document: DomHandle,
    emulated_media: &EmulatedMediaOverrides,
    viewport: StyleViewport,
    source_scope: Option<StyleSourceScope>,
    cause: PendingStyleInvalidationCause,
) {
    let profile_enabled = moli_trace::cpu_profile_enabled();
    let total_started = profile_enabled.then(std::time::Instant::now);
    let Some(source_scope) = source_scope else {
        return;
    };
    document_state.note_style_input_change();
    let plan_started = profile_enabled.then(std::time::Instant::now);
    let work = pending_work_for_pending_cause(
        host,
        source_stores,
        dom_adapter,
        emulated_media,
        viewport,
        document,
        cause,
        &source_scope,
    );
    let plan_us = plan_started
        .map(|started| started.elapsed().as_micros())
        .unwrap_or_default();
    let Some(work) = work else {
        if let Some(started) = total_started {
            let total_us = started.elapsed().as_micros();
            if total_us >= 500 {
                tracing::info!(
                    target: "moli_cpu_profile",
                    stage = "queue_style_invalidation_scope",
                    has_work = false,
                    plan_us,
                    extend_us = 0,
                    total_us,
                );
            }
        }
        return;
    };
    let extend_started = profile_enabled.then(std::time::Instant::now);
    queue_style_invalidation_targets(pending_invalidations, work);
    if let Some(started) = total_started {
        let total_us = started.elapsed().as_micros();
        if total_us >= 500 {
            tracing::info!(
                target: "moli_cpu_profile",
                stage = "queue_style_invalidation_scope",
                has_work = true,
                plan_us,
                extend_us = extend_started
                    .map(|started| started.elapsed().as_micros())
                    .unwrap_or_default(),
                total_us,
            );
        }
    }
}

pub(super) fn queue_style_invalidation_targets(
    pending_invalidations: &PendingStyleInvalidations,
    work: PendingStyleInvalidationWork,
) {
    pending_invalidations.extend_work(work);
}
