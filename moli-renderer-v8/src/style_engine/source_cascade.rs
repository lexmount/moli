use std::collections::{HashMap, HashSet};

use style::{
    author_styles::AuthorStyles,
    servo_arc::Arc as ServoArc,
    shared_lock::SharedRwLock,
    stylesheets::{CustomMediaMap, DocumentStyleSheet},
    stylist::{CascadeData, Stylist},
};

use super::{
    active_stylesheets::{ActiveStylesheet, ActiveStylesheetCollection},
    shadow_scopes::ShadowScopeStyles,
    source::store::{StyloStylesheetSource, stylesheet_sources_cache_key},
    source_id::{StyleScopeId, StyleSourceId},
    source_record::RetainedStylesheetSourceRecord,
    state::{RetainedStyleSystem, SourceCascadeProjection},
};

type InstalledSourceGroup = (Vec<StyloStylesheetSource>, Vec<DocumentStyleSheet>);
type SourceCascadeProjections = HashMap<StyleSourceId, SourceCascadeProjection>;

#[cfg(test)]
thread_local! {
    static SOURCE_CASCADE_REBUILD_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
pub(crate) fn reset_source_cascade_rebuild_count_for_test() {
    SOURCE_CASCADE_REBUILD_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn source_cascade_rebuild_count_for_test() -> usize {
    SOURCE_CASCADE_REBUILD_COUNT.with(std::cell::Cell::get)
}

/// Reconciles the source-local cascade projections that have already been
/// materialized.
///
/// The document and shadow collections remain the canonical installed-sheet
/// state. A source gets its own `CascadeData` only after an invalidation query
/// actually targets it; full-world reconciliation must not eagerly project
/// every installed source again.
pub(super) fn reconcile_materialized_source_cascade_data(
    retained: &mut RetainedStyleSystem,
    shared_lock: &SharedRwLock,
    retained_source_records: &[RetainedStylesheetSourceRecord<'_>],
    mut install: impl FnMut(&StyloStylesheetSource) -> ActiveStylesheet,
) {
    let source_ids = retained
        .source_cascade_projections
        .keys()
        .cloned()
        .collect::<HashSet<_>>();
    if source_ids.is_empty() {
        return;
    }

    let previous = std::mem::take(&mut retained.source_cascade_projections);
    retained.source_cascade_projections = build_source_cascade_data_for_source_ids(
        &mut retained.stylist,
        shared_lock,
        &retained.document_stylesheets,
        &retained.shadow_scopes,
        retained_source_records,
        &source_ids,
        Some(&previous),
        &mut install,
    );
}

pub(super) fn materialized_source_cascade_ids(
    retained: &RetainedStyleSystem,
) -> HashSet<StyleSourceId> {
    retained
        .source_cascade_projections
        .keys()
        .cloned()
        .collect()
}

pub(super) fn materialized_source_cascade_ids_affected_by_update(
    retained: &RetainedStyleSystem,
    dirty_source_ids: &HashSet<StyleSourceId>,
    dirty_scopes: &HashSet<StyleScopeId>,
    device_changed: bool,
) -> HashSet<StyleSourceId> {
    retained
        .source_cascade_projections
        .keys()
        .filter(|source_id| {
            device_changed
                || dirty_source_ids.contains(*source_id)
                || dirty_scopes.contains(&source_id.scope_id)
        })
        .cloned()
        .collect()
}

/// Materializes only source-local cascade projections requested by an
/// invalidation batch.
pub(super) fn ensure_source_cascade_data_for_source_ids<'a>(
    retained: &mut RetainedStyleSystem,
    shared_lock: &SharedRwLock,
    retained_source_records: &[RetainedStylesheetSourceRecord<'_>],
    source_ids: impl IntoIterator<Item = &'a StyleSourceId>,
    mut install: impl FnMut(&StyloStylesheetSource) -> ActiveStylesheet,
) {
    let missing_source_ids = source_ids
        .into_iter()
        .filter(|source_id| !retained.source_cascade_projections.contains_key(*source_id))
        .cloned()
        .collect::<HashSet<_>>();
    if missing_source_ids.is_empty() {
        return;
    }

    let projections = build_source_cascade_data_for_source_ids(
        &mut retained.stylist,
        shared_lock,
        &retained.document_stylesheets,
        &retained.shadow_scopes,
        retained_source_records,
        &missing_source_ids,
        None,
        &mut install,
    );
    retained.source_cascade_projections.extend(projections);
}

fn build_source_cascade_data_for_source_ids(
    stylist: &mut Stylist,
    shared_lock: &SharedRwLock,
    document_stylesheets: &ActiveStylesheetCollection,
    shadow_scopes: &[ShadowScopeStyles],
    retained_source_records: &[RetainedStylesheetSourceRecord<'_>],
    source_ids: &HashSet<StyleSourceId>,
    previous: Option<&SourceCascadeProjections>,
    install: &mut impl FnMut(&StyloStylesheetSource) -> ActiveStylesheet,
) -> SourceCascadeProjections {
    let mut sources_by_id =
        installed_sources_by_id(document_stylesheets, shadow_scopes, source_ids);
    add_retained_source_records(
        &mut sources_by_id,
        retained_source_records,
        source_ids,
        install,
    );

    let mut projections = HashMap::with_capacity(sources_by_id.len());
    for (source_id, (sources, stylesheets)) in sources_by_id {
        let key = stylesheet_sources_cache_key(&sources);
        let retained_data = previous.and_then(|previous| {
            previous
                .get(&source_id)
                .filter(|projection| projection.key == key)
                .map(|projection| projection.data.clone())
                .filter(|data| {
                    source_cascade_matches_device(data, &stylesheets, stylist, shared_lock)
                })
        });
        let data = retained_data
            .unwrap_or_else(|| build_author_cascade_data(stylist, shared_lock, &stylesheets));
        projections.insert(source_id, SourceCascadeProjection { data, key });
    }
    projections
}

/// Reprojects the requested materialized entries. Every other entry retains
/// both its key and `CascadeData` allocation.
pub(super) fn update_materialized_source_cascade_data(
    retained: &mut RetainedStyleSystem,
    shared_lock: &SharedRwLock,
    retained_source_records: &[RetainedStylesheetSourceRecord<'_>],
    source_ids: &HashSet<StyleSourceId>,
    mut install: impl FnMut(&StyloStylesheetSource) -> ActiveStylesheet,
) {
    if source_ids.is_empty() {
        return;
    }

    let mut previous = std::mem::take(&mut retained.source_cascade_projections);
    let updated = build_source_cascade_data_for_source_ids(
        &mut retained.stylist,
        shared_lock,
        &retained.document_stylesheets,
        &retained.shadow_scopes,
        retained_source_records,
        source_ids,
        Some(&previous),
        &mut install,
    );
    previous.retain(|source_id, _| !source_ids.contains(source_id));
    previous.extend(updated);
    retained.source_cascade_projections = previous;
}

fn source_cascade_matches_device(
    data: &CascadeData,
    stylesheets: &[DocumentStyleSheet],
    stylist: &Stylist,
    shared_lock: &SharedRwLock,
) -> bool {
    let guard = shared_lock.read();
    stylesheets.iter().all(|stylesheet| {
        data.media_feature_affected_matches(
            stylesheet,
            &guard,
            stylist.device(),
            stylist.quirks_mode(),
        )
    })
}

fn installed_sources_by_id(
    document_stylesheets: &ActiveStylesheetCollection,
    shadow_scopes: &[ShadowScopeStyles],
    source_ids: &HashSet<StyleSourceId>,
) -> HashMap<StyleSourceId, InstalledSourceGroup> {
    let mut sources_by_id = HashMap::<StyleSourceId, InstalledSourceGroup>::new();
    for entry in document_stylesheets.entries().iter().chain(
        shadow_scopes
            .iter()
            .flat_map(|scope| scope.active_stylesheets().entries()),
    ) {
        let Some(source_id) = entry.source().source_id().cloned() else {
            continue;
        };
        if !source_ids.contains(&source_id) {
            continue;
        }
        let (sources, stylesheets) = sources_by_id.entry(source_id).or_default();
        if let Some(index) = stylesheets
            .iter()
            .position(|stylesheet| stylesheet == entry.stylesheet())
        {
            sources.remove(index);
            stylesheets.remove(index);
        }
        sources.push(entry.source().clone());
        stylesheets.push(entry.stylesheet().clone());
    }
    sources_by_id
}

fn add_retained_source_records(
    sources_by_id: &mut HashMap<StyleSourceId, InstalledSourceGroup>,
    records: &[RetainedStylesheetSourceRecord<'_>],
    source_ids: &HashSet<StyleSourceId>,
    install: &mut impl FnMut(&StyloStylesheetSource) -> ActiveStylesheet,
) {
    for record in records {
        if !source_ids.contains(record.id()) || sources_by_id.contains_key(record.id()) {
            continue;
        }
        let source = record.to_stylo_source();
        let installed = install(&source);
        sources_by_id.insert(
            record.id().clone(),
            (vec![source], vec![installed.stylesheet().clone()]),
        );
    }
}

fn build_author_cascade_data(
    stylist: &mut Stylist,
    shared_lock: &SharedRwLock,
    stylesheets: &[DocumentStyleSheet],
) -> ServoArc<CascadeData> {
    #[cfg(test)]
    SOURCE_CASCADE_REBUILD_COUNT.with(|count| count.set(count.get().saturating_add(1)));
    let mut author_styles = AuthorStyles::<DocumentStyleSheet>::new();
    let custom_media = CustomMediaMap::default();
    let guard = shared_lock.read();
    for stylesheet in stylesheets {
        author_styles.stylesheets.append_stylesheet(
            None,
            &custom_media,
            stylesheet.clone(),
            &guard,
        );
    }
    author_styles.flush(stylist, &guard);
    author_styles.data
}
