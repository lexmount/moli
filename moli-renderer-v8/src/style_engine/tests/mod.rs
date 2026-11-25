use super::invalidation::handle_is_in_style_subtrees;
use super::source_dirty::StyleSourceDirtyReason;
use super::source_lifecycle::{
    StyleSourceDocumentKind, StyleSourceLifecycleAvailability, StyleSourceLifecycleOwner,
    StyleSourceLifecycleOwnerDetailTrace, StyleSourceLifecycleOwnerDetailTraceSink,
    StyleSourceLifecycleRecord, StyleSourceLifecycleReport, StyleSourceLifecycleUnavailableReason,
    StyleSourceLifecycleWithoutSourceReason,
};
use super::*;
use std::collections::HashSet;

use crate::dom::native::{NativeDom, NativeNodeId};
use selectors::{Element as SelectorsElement, matching::ElementSelectorFlags};
use style::dom::TElement;
use style::servo_arc::Arc as ServoArc;

fn test_host() -> DomHost {
    DomHost::from_dom(NativeDom::new_html(
        url::Url::parse("https://example.test/").expect("valid test url"),
    ))
}

fn lifecycle_availability_for(
    records: &[StyleSourceLifecycleRecord],
    owner: StyleSourceLifecycleOwner,
) -> Option<&StyleSourceLifecycleAvailability> {
    records
        .iter()
        .find(|record| record.owner() == owner)
        .map(StyleSourceLifecycleRecord::availability)
}
fn lifecycle_document_kind_for(
    records: &[StyleSourceLifecycleRecord],
    owner: StyleSourceLifecycleOwner,
) -> Option<StyleSourceDocumentKind> {
    records
        .iter()
        .find(|record| record.owner() == owner)
        .and_then(StyleSourceLifecycleRecord::document_kind)
}
fn lifecycle_owner_detail_for(
    details: &[StyleSourceLifecycleOwnerDetailTrace],
    owner: StyleSourceLifecycleOwner,
) -> Option<&StyleSourceLifecycleOwnerDetailTrace> {
    details.iter().find(|detail| detail.owner() == owner)
}
fn source_lifecycle_report_for_document_for_test(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document: DomHandle,
    document_context: StyleSourceDocumentContext<'_>,
) -> StyleSourceLifecycleReport {
    let world = engine.world_for_document(document);
    let source_stores = world.borrow_source_stores();
    source_stores.source_lifecycle_report(host, document_context)
}
mod char_child;
mod dependency;
mod invalidator;
mod lifecycle;
mod native_snapshots;
mod registered_properties;
mod slotted_focus;
mod subtree_cache;
mod subtree_context;
