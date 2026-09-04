use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use crate::{document_runtime::DomHandle, dom::native::DomHost};

use super::{
    StyleSourceId, StyleViewport,
    request::RetainedSourceDependencyRequestPlan,
    source::store::{OwnerStyleSheetSource, StyloStylesheetSource},
    source_id::{StyleScopeId, StyleSourceKind},
    source_lifecycle::{
        StyleSourceLifecycleOwner, StyleSourceLifecycleReport,
        StyleSourceLifecycleRetainedSourceIdSink, StyleSourceLifecycleSourceRequests,
        StyleSourceLifecycleWithoutSourceReason, TrackedStyleSourceIds,
        TrackedStyleSourceLifecycleEntry,
    },
    source_record::{MatchingStyleDependencySource, RetainedStylesheetSourceRecord},
};

#[derive(Debug)]
struct InstalledOwnerStyleSheet {
    processing_source: Arc<OwnerStyleSheetSource>,
    cascade_source: StyloStylesheetSource,
    live_stylesheet: Option<crate::live_stylesheet::LiveStylesheetRef>,
}

impl InstalledOwnerStyleSheet {
    fn from_processing_source(processing_source: Arc<OwnerStyleSheetSource>) -> Self {
        Self {
            cascade_source: processing_source.source().clone(),
            processing_source,
            live_stylesheet: None,
        }
    }

    fn install_live_stylesheet(&mut self, stylesheet: crate::live_stylesheet::LiveStylesheetRef) {
        self.cascade_source = StyloStylesheetSource::from_live_stylesheet(&stylesheet);
        self.live_stylesheet = Some(stylesheet);
    }

    fn refresh_live_stylesheet(
        &mut self,
        stylesheet_id: crate::live_stylesheet::StylesheetId,
    ) -> bool {
        let Some(stylesheet) = self
            .live_stylesheet
            .as_ref()
            .filter(|stylesheet| stylesheet.id() == stylesheet_id)
        else {
            return false;
        };
        self.cascade_source = StyloStylesheetSource::from_live_stylesheet(stylesheet);
        true
    }
}

#[derive(Debug, Default)]
pub(super) struct OwnerStyleSheetSources {
    sources_by_owner: HashMap<DomHandle, InstalledOwnerStyleSheet>,
    // A CSSOM rule mutation does not rewrite the owning <style> text nodes.
    // Parser/runtime reconciliation must therefore preserve this source until
    // an actual DOM stylesheet-content mutation calls `set_source` again.
    cssom_authoritative_owners: HashSet<DomHandle>,
    // Projection of DocumentRuntime's CSP disposition. This set only suppresses
    // retained/cascade sources; it is not queried for request or event policy.
    csp_suppressed_owners: HashSet<DomHandle>,
}

impl OwnerStyleSheetSources {
    pub(super) fn has_document_retained_sources_for_document(
        &self,
        host: &DomHost,
        document: DomHandle,
    ) -> bool {
        self.tracked_owners_for_document(host, document)
            .any(|owner| {
                if !self.owner_has_retained_source(host, owner) {
                    return false;
                }
                StyleSourceId::owner_style_sheet(host, owner).is_some_and(|source_id| {
                    matches!(source_id.scope_id, StyleScopeId::Document(_))
                })
            })
    }

    pub(super) fn clear_all(&mut self) {
        self.sources_by_owner.clear();
        self.cssom_authoritative_owners.clear();
        self.csp_suppressed_owners.clear();
    }

    pub(super) fn set_source(
        &mut self,
        owner: DomHandle,
        css_text: Arc<str>,
        parser_base: url::Url,
    ) -> bool {
        self.cssom_authoritative_owners.remove(&owner);
        if self.sources_by_owner.get(&owner).is_some_and(|existing| {
            existing
                .processing_source
                .matches_processing_input(&css_text, &parser_base)
        }) {
            return false;
        }
        let processing_source = Arc::new(OwnerStyleSheetSource::from_shared_text(
            owner,
            css_text,
            parser_base,
        ));
        self.sources_by_owner.insert(
            owner,
            InstalledOwnerStyleSheet::from_processing_source(processing_source),
        );
        true
    }

    pub(super) fn replace_processed_source(
        &mut self,
        owner: DomHandle,
        css_text: Arc<str>,
        parser_base: url::Url,
    ) {
        self.cssom_authoritative_owners.remove(&owner);
        let processing_source = Arc::new(OwnerStyleSheetSource::from_shared_text(
            owner,
            css_text,
            parser_base,
        ));
        self.sources_by_owner.insert(
            owner,
            InstalledOwnerStyleSheet::from_processing_source(processing_source),
        );
    }

    pub(super) fn install_live_stylesheet(
        &mut self,
        owner: DomHandle,
        stylesheet: crate::live_stylesheet::LiveStylesheetRef,
    ) -> bool {
        let Some(installed) = self.sources_by_owner.get_mut(&owner) else {
            return false;
        };
        installed.install_live_stylesheet(stylesheet);
        true
    }

    pub(super) fn refresh_live_stylesheet(
        &mut self,
        owner: DomHandle,
        stylesheet_id: crate::live_stylesheet::StylesheetId,
    ) -> bool {
        let Some(installed) = self.sources_by_owner.get_mut(&owner) else {
            return false;
        };
        if !installed.refresh_live_stylesheet(stylesheet_id) {
            return false;
        }
        true
    }

    pub(super) fn live_stylesheet(
        &self,
        owner: DomHandle,
    ) -> Option<crate::live_stylesheet::LiveStylesheetRef> {
        if self.is_csp_suppressed(owner) {
            return None;
        }
        self.sources_by_owner
            .get(&owner)
            .and_then(|installed| installed.live_stylesheet.clone())
    }

    pub(super) fn remove_owner(&mut self, owner: DomHandle) -> bool {
        self.cssom_authoritative_owners.remove(&owner);
        self.csp_suppressed_owners.remove(&owner);
        self.sources_by_owner.remove(&owner).is_some()
    }

    pub(super) fn set_csp_suppressed(&mut self, owner: DomHandle, suppressed: bool) -> bool {
        if suppressed {
            self.csp_suppressed_owners.insert(owner)
        } else {
            self.csp_suppressed_owners.remove(&owner)
        }
    }

    pub(super) fn is_csp_suppressed(&self, owner: DomHandle) -> bool {
        self.csp_suppressed_owners.contains(&owner)
    }

    pub(super) fn mark_cssom_authoritative(&mut self, owner: DomHandle) {
        self.cssom_authoritative_owners.insert(owner);
    }

    pub(super) fn cssom_source_is_authoritative(&self, owner: DomHandle) -> bool {
        self.cssom_authoritative_owners.contains(&owner)
    }

    pub(super) fn text(&self, owner: DomHandle) -> Option<&str> {
        self.sources_by_owner
            .get(&owner)
            .map(|installed| installed.processing_source.css_text())
    }

    pub(super) fn text_owned(&self, owner: DomHandle) -> Option<String> {
        self.text(owner).map(str::to_owned)
    }

    pub(super) fn source(&self, owner: DomHandle) -> Option<StyloStylesheetSource> {
        if self.is_csp_suppressed(owner) {
            return None;
        }
        self.sources_by_owner
            .get(&owner)
            .map(|installed| installed.cascade_source.clone())
    }

    pub(super) fn processing_source(&self, owner: DomHandle) -> Option<Arc<OwnerStyleSheetSource>> {
        if self.is_csp_suppressed(owner) {
            return None;
        }
        self.sources_by_owner
            .get(&owner)
            .map(|installed| Arc::clone(&installed.processing_source))
    }

    pub(super) fn retained_source_record<'a>(
        &'a self,
        host: &DomHost,
        source_id: StyleSourceId,
        owner: DomHandle,
    ) -> Option<RetainedStylesheetSourceRecord<'a>> {
        if !self.owner_has_retained_source(host, owner) {
            return None;
        }
        let source = &self.sources_by_owner.get(&owner)?.cascade_source;
        Some(RetainedStylesheetSourceRecord::owner_style_sheet(
            source_id, source,
        ))
    }

    pub(super) fn for_each_retained_source_record_for_document<'a>(
        &'a self,
        host: &DomHost,
        document: DomHandle,
        mut callback: impl FnMut(RetainedStylesheetSourceRecord<'a>),
    ) {
        for owner in self.tracked_owners_for_document(host, document) {
            let Some(source_id) = StyleSourceId::owner_style_sheet(host, owner) else {
                continue;
            };
            if let Some(record) = self.retained_source_record(host, source_id, owner) {
                callback(record);
            }
        }
    }

    pub(super) fn matching_dependency_sources_for_document(
        &self,
        host: &DomHost,
        document: DomHandle,
        source_scope: &moli_selector::StyloStyleSourceScope,
        emulated_media: &crate::protocol_types::EmulatedMediaOverrides,
        viewport: StyleViewport,
    ) -> Vec<MatchingStyleDependencySource> {
        let mut sources = Vec::new();
        self.for_each_retained_source_record_for_document(host, document, |record| {
            if let Some(source) =
                record.matching_dependency_source(host, source_scope, emulated_media, viewport)
            {
                sources.push(source);
            }
        });
        sources
    }

    pub(super) fn has_dependency_match_for_document(
        &self,
        host: &DomHost,
        document: DomHandle,
        request_plan: &RetainedSourceDependencyRequestPlan,
    ) -> bool {
        self.tracked_owners_for_document(host, document)
            .any(|owner| {
                if !self.owner_has_retained_source(host, owner) {
                    return false;
                }
                self.sources_by_owner.get(&owner).is_some_and(|installed| {
                    request_plan.matches_dependency_summary(
                        installed
                            .cascade_source
                            .source_dependency_summary()
                            .as_ref(),
                    )
                })
            })
    }

    pub(super) fn tracked_owners(&self) -> impl Iterator<Item = DomHandle> + '_ {
        self.sources_by_owner.keys().copied()
    }

    pub(super) fn tracks_owner(&self, owner: DomHandle) -> bool {
        self.sources_by_owner.contains_key(&owner)
    }

    pub(super) fn tracked_lifecycle_entries_for_document<'a>(
        &'a self,
        host: &'a DomHost,
        document: DomHandle,
    ) -> impl Iterator<Item = TrackedStyleSourceLifecycleEntry> + 'a {
        self.tracked_owners_for_document(host, document)
            .filter_map(move |owner| self.tracked_lifecycle_entry(host, owner))
    }

    pub(super) fn tracked_lifecycle_entry(
        &self,
        host: &DomHost,
        owner: DomHandle,
    ) -> Option<TrackedStyleSourceLifecycleEntry> {
        if !self.tracks_owner(owner) {
            return None;
        }
        let lifecycle_owner = StyleSourceLifecycleOwner::OwnerStyleSheet { owner };
        Some(lifecycle_owner.tracked_entry(host, || self.tracked_source_ids(host, owner)))
    }

    pub(super) fn tracked_lifecycle_entries_for_source_ids<'a>(
        &'a self,
        host: &'a DomHost,
        source_ids: impl IntoIterator<Item = StyleSourceId> + 'a,
    ) -> impl Iterator<Item = TrackedStyleSourceLifecycleEntry> + 'a {
        StyleSourceLifecycleSourceRequests::from_source_ids(source_ids).into_tracked_entries(
            move |owner| match owner {
                StyleSourceLifecycleOwner::OwnerStyleSheet { owner } => {
                    self.tracked_lifecycle_entry(host, owner)
                }
                StyleSourceLifecycleOwner::LinkedStyleSheet { .. }
                | StyleSourceLifecycleOwner::DocumentAdoptedStyleSheets { .. }
                | StyleSourceLifecycleOwner::ShadowRootAdoptedStyleSheets { .. } => None,
            },
        )
    }

    pub(super) fn retained_source_records_for_lifecycle<'a>(
        &'a self,
        host: &DomHost,
        source_lifecycle: &StyleSourceLifecycleReport,
    ) -> Vec<RetainedStylesheetSourceRecord<'a>> {
        let mut requests = RetainedOwnerSourceRecordRequests::default();
        source_lifecycle.record_retained_source_ids_into(&mut requests);
        requests
            .source_ids
            .iter()
            .filter_map(|source_id| self.retained_source_record_for_id(host, source_id))
            .collect()
    }

    fn retained_source_record_for_id<'a>(
        &'a self,
        host: &DomHost,
        source_id: &StyleSourceId,
    ) -> Option<RetainedStylesheetSourceRecord<'a>> {
        match (&source_id.scope_id, &source_id.kind) {
            (
                StyleScopeId::Document(_) | StyleScopeId::ShadowRoot(_),
                StyleSourceKind::OwnerStyleSheet { owner },
            ) => self.retained_source_record(host, source_id.clone(), *owner),
            _ => None,
        }
    }

    fn tracked_source_ids(&self, host: &DomHost, owner: DomHandle) -> TrackedStyleSourceIds {
        if !self.sources_by_owner.contains_key(&owner) {
            return TrackedStyleSourceIds::WithoutSource(
                StyleSourceLifecycleWithoutSourceReason::OwnerStyleSheetSourceMissing,
            );
        }
        let source_ids: Vec<_> = StyleSourceId::owner_style_sheet(host, owner)
            .into_iter()
            .collect();
        TrackedStyleSourceIds::retained_or_without_source(
            source_ids,
            StyleSourceLifecycleWithoutSourceReason::SourceIdMissing,
        )
    }

    fn tracked_owners_for_document<'a>(
        &'a self,
        host: &'a DomHost,
        document: DomHandle,
    ) -> impl Iterator<Item = DomHandle> + 'a {
        self.tracked_owners()
            .filter(move |owner| host.owner_document_handle(*owner) == Some(document))
    }

    fn owner_has_retained_source(&self, host: &DomHost, owner: DomHandle) -> bool {
        !self.is_csp_suppressed(owner)
            && self.sources_by_owner.contains_key(&owner)
            && StyleSourceId::owner_style_sheet(host, owner).is_some()
            && (StyleSourceLifecycleOwner::OwnerStyleSheet { owner })
                .retained_source_records_are_available(host)
    }
}

#[derive(Default)]
struct RetainedOwnerSourceRecordRequests {
    source_ids: Vec<StyleSourceId>,
}

impl StyleSourceLifecycleRetainedSourceIdSink for RetainedOwnerSourceRecordRequests {
    fn record_source_lifecycle_retained_source_id(&mut self, source_id: &StyleSourceId) {
        self.source_ids.push(source_id.clone());
    }
}
