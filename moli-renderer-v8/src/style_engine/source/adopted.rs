use std::collections::{HashMap, HashSet};

use indexmap::IndexSet;

use crate::{document_runtime::DomHandle, dom::native::DomHost};

use super::super::{
    StyleSourceId, StyleViewport,
    request::RetainedSourceDependencyRequestPlan,
    source_lifecycle::{
        StyleSourceLifecycleOwner, StyleSourceLifecycleReport,
        StyleSourceLifecycleRetainedSourceIdSink, StyleSourceLifecycleSourceRequests,
        StyleSourceLifecycleWithoutSourceReason, TrackedStyleSourceIds,
        TrackedStyleSourceLifecycleEntry,
    },
    source_record::{MatchingStyleDependencySource, RetainedStylesheetSourceRecord},
};
use super::store::StyloStylesheetSource;

#[derive(Debug, Default)]
pub(in crate::style_engine) struct AdoptedStyleSheetSources {
    sources_by_owner: HashMap<AdoptedStyleSheetOwner, Vec<InstalledAdoptedStylesheetClient>>,
    tracked_owners: IndexSet<AdoptedStyleSheetOwner>,
    next_client_id: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct AdoptedStyleSheetInstallation {
    stylesheet: crate::live_stylesheet::LiveStylesheetRef,
}

impl AdoptedStyleSheetInstallation {
    pub(crate) fn new(stylesheet: crate::live_stylesheet::LiveStylesheetRef) -> Self {
        Self { stylesheet }
    }
}

#[derive(Clone, Debug)]
struct InstalledAdoptedStylesheetClient {
    source: StyloStylesheetSource,
    _live_stylesheet: Option<crate::live_stylesheet::LiveStylesheetRef>,
}

impl InstalledAdoptedStylesheetClient {
    fn from_installation(installation: AdoptedStyleSheetInstallation) -> Self {
        let stylesheet = installation.stylesheet;
        Self {
            source: StyloStylesheetSource::from_live_stylesheet(&stylesheet),
            _live_stylesheet: Some(stylesheet),
        }
    }

    #[cfg(test)]
    fn from_source_snapshot(mut source: StyloStylesheetSource) -> Self {
        if source.adopted_client_id().is_none() {
            let synthetic_client_id =
                source
                    .source_id()
                    .and_then(|source_id| match source_id.kind {
                        super::super::source_id::StyleSourceKind::DocumentAdoptedStyleSheet {
                            client_id,
                        }
                        | super::super::source_id::StyleSourceKind::ShadowRootAdoptedStyleSheet {
                            client_id,
                        } => Some(client_id),
                        super::super::source_id::StyleSourceKind::OwnerStyleSheet { .. }
                        | super::super::source_id::StyleSourceKind::LinkedStyleSheet { .. } => None,
                    });
            if let Some(client_id) = synthetic_client_id {
                source = source.with_adopted_client_id(client_id);
            }
        }
        Self {
            source,
            _live_stylesheet: None,
        }
    }

    fn source(&self) -> &StyloStylesheetSource {
        &self.source
    }
}

impl PartialEq for InstalledAdoptedStylesheetClient {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

impl Eq for InstalledAdoptedStylesheetClient {}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum AdoptedStyleSheetOwner {
    Document(DomHandle),
    ShadowRoot(DomHandle),
}

impl AdoptedStyleSheetSources {
    pub(in crate::style_engine) fn clear_all(&mut self) {
        self.sources_by_owner.clear();
        self.tracked_owners.clear();
        self.next_client_id = 0;
    }

    #[cfg(test)]
    pub(in crate::style_engine) fn set_document_sources(
        &mut self,
        document: DomHandle,
        sources: Vec<StyloStylesheetSource>,
    ) -> bool {
        self.set_clients(
            AdoptedStyleSheetOwner::Document(document),
            sources
                .into_iter()
                .map(InstalledAdoptedStylesheetClient::from_source_snapshot)
                .collect(),
        )
    }

    pub(in crate::style_engine) fn set_document_installations(
        &mut self,
        document: DomHandle,
        installations: Vec<AdoptedStyleSheetInstallation>,
    ) -> bool {
        self.set_installations(AdoptedStyleSheetOwner::Document(document), installations)
    }

    #[cfg(test)]
    pub(in crate::style_engine) fn set_shadow_root_sources(
        &mut self,
        root: DomHandle,
        sources: Vec<StyloStylesheetSource>,
    ) -> bool {
        self.set_clients(
            AdoptedStyleSheetOwner::ShadowRoot(root),
            sources
                .into_iter()
                .map(InstalledAdoptedStylesheetClient::from_source_snapshot)
                .collect(),
        )
    }

    pub(in crate::style_engine) fn set_shadow_root_installations(
        &mut self,
        root: DomHandle,
        installations: Vec<AdoptedStyleSheetInstallation>,
    ) -> bool {
        self.set_installations(AdoptedStyleSheetOwner::ShadowRoot(root), installations)
    }

    fn set_installations(
        &mut self,
        owner: AdoptedStyleSheetOwner,
        installations: Vec<AdoptedStyleSheetInstallation>,
    ) -> bool {
        self.set_clients(
            owner,
            installations
                .into_iter()
                .map(InstalledAdoptedStylesheetClient::from_installation)
                .collect(),
        )
    }

    fn set_clients(
        &mut self,
        owner: AdoptedStyleSheetOwner,
        mut clients: Vec<InstalledAdoptedStylesheetClient>,
    ) -> bool {
        self.tracked_owners.insert(owner);
        if clients.is_empty() {
            return self.sources_by_owner.remove(&owner).is_some();
        }
        let previous = self
            .sources_by_owner
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        let mut reused = vec![false; previous.len()];
        let mut assigned_client_ids = HashSet::new();
        for client in &mut clients {
            let reused_client_id = previous.iter().enumerate().find_map(|(index, candidate)| {
                (!reused[index]
                    && candidate
                        .source()
                        .has_same_installation_identity(client.source()))
                .then(|| {
                    reused[index] = true;
                    candidate.source().adopted_client_id()
                })
                .flatten()
            });
            let requested_client_id = client
                .source()
                .adopted_client_id()
                .filter(|client_id| !assigned_client_ids.contains(client_id));
            let client_id = reused_client_id
                .or(requested_client_id)
                .unwrap_or_else(|| self.allocate_client_id_avoiding(&assigned_client_ids));
            self.next_client_id = self.next_client_id.max(client_id);
            assigned_client_ids.insert(client_id);
            client.source = client.source.clone().with_adopted_client_id(client_id);
        }
        if self
            .sources_by_owner
            .get(&owner)
            .is_some_and(|existing| existing == &clients)
        {
            false
        } else {
            self.sources_by_owner.insert(owner, clients);
            true
        }
    }

    fn allocate_client_id(&mut self) -> u64 {
        self.next_client_id = self
            .next_client_id
            .checked_add(1)
            .expect("adopted stylesheet client identity space exhausted");
        self.next_client_id
    }

    fn allocate_client_id_avoiding(&mut self, assigned_client_ids: &HashSet<u64>) -> u64 {
        loop {
            let client_id = self.allocate_client_id();
            if !assigned_client_ids.contains(&client_id) {
                return client_id;
            }
        }
    }

    fn sources(
        &self,
        owner: AdoptedStyleSheetOwner,
    ) -> Option<&[InstalledAdoptedStylesheetClient]> {
        self.sources_by_owner.get(&owner).map(Vec::as_slice)
    }

    fn sources_or_empty(
        &self,
        owner: AdoptedStyleSheetOwner,
    ) -> &[InstalledAdoptedStylesheetClient] {
        self.sources(owner).unwrap_or(&[])
    }

    pub(in crate::style_engine) fn document_sources_or_empty_owned(
        &self,
        document: DomHandle,
    ) -> Vec<StyloStylesheetSource> {
        self.sources_or_empty(AdoptedStyleSheetOwner::Document(document))
            .iter()
            .map(|client| client.source().clone())
            .collect()
    }

    pub(in crate::style_engine) fn shadow_root_sources_or_empty_owned(
        &self,
        root: DomHandle,
    ) -> Vec<StyloStylesheetSource> {
        self.sources_or_empty(AdoptedStyleSheetOwner::ShadowRoot(root))
            .iter()
            .map(|client| client.source().clone())
            .collect()
    }

    pub(in crate::style_engine) fn document_source_count(&self, document: DomHandle) -> usize {
        self.sources_or_empty(AdoptedStyleSheetOwner::Document(document))
            .len()
    }

    pub(in crate::style_engine) fn shadow_root_source_count(&self, root: DomHandle) -> usize {
        self.sources_or_empty(AdoptedStyleSheetOwner::ShadowRoot(root))
            .len()
    }

    pub(in crate::style_engine) fn document_source_ids(
        &self,
        document: DomHandle,
    ) -> Vec<StyleSourceId> {
        self.source_ids(AdoptedStyleSheetOwner::Document(document), |client_id| {
            StyleSourceId::document_adopted_style_sheet(document, client_id)
        })
    }

    pub(in crate::style_engine) fn shadow_root_source_ids(
        &self,
        root: DomHandle,
    ) -> Vec<StyleSourceId> {
        self.source_ids(AdoptedStyleSheetOwner::ShadowRoot(root), |client_id| {
            StyleSourceId::shadow_root_adopted_style_sheet(root, client_id)
        })
    }

    fn source(
        &self,
        owner: AdoptedStyleSheetOwner,
        client_id: u64,
    ) -> Option<&StyloStylesheetSource> {
        self.sources_by_owner
            .get(&owner)?
            .iter()
            .find(|client| client.source().adopted_client_id() == Some(client_id))
            .map(InstalledAdoptedStylesheetClient::source)
    }

    fn retained_source_record<'a>(
        &'a self,
        source_id: StyleSourceId,
        owner: AdoptedStyleSheetOwner,
        client_id: u64,
    ) -> Option<RetainedStylesheetSourceRecord<'a>> {
        let source = self.source(owner, client_id)?;
        Some(RetainedStylesheetSourceRecord::registered(
            source_id, source,
        ))
    }

    pub(in crate::style_engine) fn for_each_retained_document_source_record_for_document<'a>(
        &'a self,
        host: &DomHost,
        document: DomHandle,
        mut callback: impl FnMut(RetainedStylesheetSourceRecord<'a>),
    ) {
        let Some(sources) = self
            .sources_by_owner
            .get(&AdoptedStyleSheetOwner::Document(document))
        else {
            return;
        };
        let lifecycle_owner = StyleSourceLifecycleOwner::DocumentAdoptedStyleSheets { document };
        if !lifecycle_owner.retained_source_records_are_available(host) {
            return;
        }
        for source in sources {
            let client_id = source
                .source()
                .adopted_client_id()
                .expect("installed adopted stylesheet must have a client id");
            let source_id = StyleSourceId::document_adopted_style_sheet(document, client_id);
            if let Some(record) = self.retained_source_record(
                source_id,
                AdoptedStyleSheetOwner::Document(document),
                client_id,
            ) {
                callback(record);
            }
        }
    }

    pub(in crate::style_engine) fn matching_dependency_sources_for_document(
        &self,
        host: &DomHost,
        document: DomHandle,
        source_scope: &moli_selector::StyloStyleSourceScope,
        emulated_media: &crate::protocol_types::EmulatedMediaOverrides,
        viewport: StyleViewport,
    ) -> Vec<MatchingStyleDependencySource> {
        let mut sources = Vec::new();
        self.for_each_retained_document_source_record_for_document(host, document, |record| {
            if let Some(source) =
                record.matching_dependency_source(host, source_scope, emulated_media, viewport)
            {
                sources.push(source);
            }
        });
        self.for_each_retained_shadow_root_source_record_for_document(host, document, |record| {
            if let Some(source) =
                record.matching_dependency_source(host, source_scope, emulated_media, viewport)
            {
                sources.push(source);
            }
        });
        sources
    }

    pub(in crate::style_engine) fn has_dependency_match_for_document(
        &self,
        host: &DomHost,
        document: DomHandle,
        request_plan: &RetainedSourceDependencyRequestPlan,
    ) -> bool {
        self.document_sources_have_dependency_match(host, document, request_plan)
            || self.shadow_root_sources_have_dependency_match(host, document, request_plan)
    }

    pub(in crate::style_engine) fn for_each_retained_shadow_root_source_record_for_document<'a>(
        &'a self,
        host: &DomHost,
        document: DomHandle,
        mut callback: impl FnMut(RetainedStylesheetSourceRecord<'a>),
    ) {
        for root in self.tracked_shadow_roots_for_document(host, document) {
            let Some(sources) = self
                .sources_by_owner
                .get(&AdoptedStyleSheetOwner::ShadowRoot(root))
            else {
                continue;
            };
            let lifecycle_owner = StyleSourceLifecycleOwner::ShadowRootAdoptedStyleSheets { root };
            if !lifecycle_owner.retained_source_records_are_available(host) {
                continue;
            }
            for source in sources {
                let client_id = source
                    .source()
                    .adopted_client_id()
                    .expect("installed adopted stylesheet must have a client id");
                let source_id = StyleSourceId::shadow_root_adopted_style_sheet(root, client_id);
                if let Some(record) = self.retained_source_record(
                    source_id,
                    AdoptedStyleSheetOwner::ShadowRoot(root),
                    client_id,
                ) {
                    callback(record);
                }
            }
        }
    }

    fn document_sources_have_dependency_match(
        &self,
        host: &DomHost,
        document: DomHandle,
        request_plan: &RetainedSourceDependencyRequestPlan,
    ) -> bool {
        let owner = AdoptedStyleSheetOwner::Document(document);
        let Some(sources) = self.sources_by_owner.get(&owner) else {
            return false;
        };
        if !(StyleSourceLifecycleOwner::DocumentAdoptedStyleSheets { document })
            .retained_source_records_are_available(host)
        {
            return false;
        }
        sources.iter().any(|source| {
            request_plan
                .matches_dependency_summary(source.source().source_dependency_summary().as_ref())
        })
    }

    fn shadow_root_sources_have_dependency_match(
        &self,
        host: &DomHost,
        document: DomHandle,
        request_plan: &RetainedSourceDependencyRequestPlan,
    ) -> bool {
        self.tracked_shadow_roots_for_document(host, document)
            .any(|root| {
                let owner = AdoptedStyleSheetOwner::ShadowRoot(root);
                let Some(sources) = self.sources_by_owner.get(&owner) else {
                    return false;
                };
                if !(StyleSourceLifecycleOwner::ShadowRootAdoptedStyleSheets { root })
                    .retained_source_records_are_available(host)
                {
                    return false;
                }
                sources.iter().any(|source| {
                    request_plan.matches_dependency_summary(
                        source.source().source_dependency_summary().as_ref(),
                    )
                })
            })
    }

    fn tracked_owners(&self) -> impl Iterator<Item = AdoptedStyleSheetOwner> + '_ {
        self.tracked_owners.iter().copied()
    }

    pub(in crate::style_engine) fn tracks_document(&self, document: DomHandle) -> bool {
        self.tracked_owners
            .contains(&AdoptedStyleSheetOwner::Document(document))
    }

    pub(in crate::style_engine) fn tracks_shadow_root(&self, root: DomHandle) -> bool {
        self.tracked_owners
            .contains(&AdoptedStyleSheetOwner::ShadowRoot(root))
    }

    #[cfg(test)]
    pub(in crate::style_engine) fn document_source_owner_count(&self) -> usize {
        self.sources_by_owner
            .keys()
            .filter(|owner| matches!(owner, AdoptedStyleSheetOwner::Document(_)))
            .count()
    }

    #[cfg(test)]
    pub(in crate::style_engine) fn shadow_root_source_owner_count(&self) -> usize {
        self.sources_by_owner
            .keys()
            .filter(|owner| matches!(owner, AdoptedStyleSheetOwner::ShadowRoot(_)))
            .count()
    }

    pub(in crate::style_engine) fn tracked_lifecycle_entries_for_document<'a>(
        &'a self,
        host: &'a DomHost,
        document: DomHandle,
    ) -> impl Iterator<Item = TrackedStyleSourceLifecycleEntry> + 'a {
        self.tracked_document_lifecycle_entry(host, document)
            .into_iter()
            .chain(self.tracked_shadow_root_lifecycle_entries_for_document(host, document))
    }

    pub(in crate::style_engine) fn tracked_lifecycle_entries_for_source_ids<'a>(
        &'a self,
        host: &'a DomHost,
        source_ids: impl IntoIterator<Item = StyleSourceId> + 'a,
    ) -> Vec<TrackedStyleSourceLifecycleEntry> {
        StyleSourceLifecycleSourceRequests::from_source_ids(source_ids)
            .into_tracked_entries(move |owner| match owner {
                StyleSourceLifecycleOwner::DocumentAdoptedStyleSheets { document } => {
                    self.tracked_document_lifecycle_entry(host, document)
                }
                StyleSourceLifecycleOwner::ShadowRootAdoptedStyleSheets { root } => {
                    self.tracked_shadow_root_lifecycle_entry(host, root)
                }
                StyleSourceLifecycleOwner::OwnerStyleSheet { .. }
                | StyleSourceLifecycleOwner::LinkedStyleSheet { .. } => None,
            })
            .collect()
    }

    pub(in crate::style_engine) fn retained_source_records_for_lifecycle<'a>(
        &'a self,
        source_lifecycle: &StyleSourceLifecycleReport,
    ) -> Vec<RetainedStylesheetSourceRecord<'a>> {
        let mut requests = RetainedAdoptedSourceRecordRequests::default();
        source_lifecycle.record_retained_source_ids_into(&mut requests);
        requests
            .source_ids
            .iter()
            .filter_map(|source_id| self.retained_source_record_for_id(source_id))
            .collect()
    }

    fn retained_source_record_for_id<'a>(
        &'a self,
        source_id: &StyleSourceId,
    ) -> Option<RetainedStylesheetSourceRecord<'a>> {
        match (&source_id.scope_id, &source_id.kind) {
            (
                super::super::source_id::StyleScopeId::Document(document),
                super::super::source_id::StyleSourceKind::DocumentAdoptedStyleSheet { client_id },
            ) => self.retained_source_record(
                source_id.clone(),
                AdoptedStyleSheetOwner::Document(*document),
                *client_id,
            ),
            (
                super::super::source_id::StyleScopeId::ShadowRoot(root),
                super::super::source_id::StyleSourceKind::ShadowRootAdoptedStyleSheet { client_id },
            ) => self.retained_source_record(
                source_id.clone(),
                AdoptedStyleSheetOwner::ShadowRoot(*root),
                *client_id,
            ),
            _ => None,
        }
    }

    pub(in crate::style_engine) fn tracked_shadow_root_lifecycle_entries_for_document<'a>(
        &'a self,
        host: &'a DomHost,
        document: DomHandle,
    ) -> impl Iterator<Item = TrackedStyleSourceLifecycleEntry> + 'a {
        self.tracked_shadow_roots_for_document(host, document)
            .filter_map(move |root| self.tracked_shadow_root_lifecycle_entry(host, root))
    }

    pub(in crate::style_engine) fn tracked_document_lifecycle_entry(
        &self,
        host: &DomHost,
        document: DomHandle,
    ) -> Option<TrackedStyleSourceLifecycleEntry> {
        if !self.tracks_document(document) {
            return None;
        }
        let lifecycle_owner = StyleSourceLifecycleOwner::DocumentAdoptedStyleSheets { document };
        Some(lifecycle_owner.tracked_entry(host, || {
            self.tracked_source_ids(
                AdoptedStyleSheetOwner::Document(document),
                |client_id| StyleSourceId::document_adopted_style_sheet(document, client_id),
                StyleSourceLifecycleWithoutSourceReason::EmptyDocumentAdoptedStyleSheets,
            )
        }))
    }

    pub(in crate::style_engine) fn tracked_shadow_root_lifecycle_entry(
        &self,
        host: &DomHost,
        root: DomHandle,
    ) -> Option<TrackedStyleSourceLifecycleEntry> {
        if !self.tracks_shadow_root(root) {
            return None;
        }
        let lifecycle_owner = StyleSourceLifecycleOwner::ShadowRootAdoptedStyleSheets { root };
        Some(lifecycle_owner.tracked_entry(host, || {
            self.tracked_source_ids(
                AdoptedStyleSheetOwner::ShadowRoot(root),
                |client_id| StyleSourceId::shadow_root_adopted_style_sheet(root, client_id),
                StyleSourceLifecycleWithoutSourceReason::EmptyShadowRootAdoptedStyleSheets,
            )
        }))
    }

    fn tracked_source_ids(
        &self,
        owner: AdoptedStyleSheetOwner,
        make_source_id: impl Fn(u64) -> StyleSourceId,
        empty_reason: StyleSourceLifecycleWithoutSourceReason,
    ) -> TrackedStyleSourceIds {
        let Some(sources) = self.sources(owner) else {
            return TrackedStyleSourceIds::WithoutSource(empty_reason);
        };
        let source_ids = sources
            .iter()
            .map(|client| {
                make_source_id(
                    client
                        .source()
                        .adopted_client_id()
                        .expect("installed adopted stylesheet must have a client id"),
                )
            })
            .collect();
        TrackedStyleSourceIds::retained_or_without_source(source_ids, empty_reason)
    }

    fn source_ids(
        &self,
        owner: AdoptedStyleSheetOwner,
        make_source_id: impl Fn(u64) -> StyleSourceId,
    ) -> Vec<StyleSourceId> {
        self.sources_or_empty(owner)
            .iter()
            .map(|client| {
                make_source_id(
                    client
                        .source()
                        .adopted_client_id()
                        .expect("installed adopted stylesheet must have a client id"),
                )
            })
            .collect()
    }

    fn tracked_shadow_roots_for_document<'a>(
        &'a self,
        host: &'a DomHost,
        document: DomHandle,
    ) -> impl Iterator<Item = DomHandle> + 'a {
        self.tracked_owners().filter_map(move |owner| match owner {
            AdoptedStyleSheetOwner::ShadowRoot(root)
                if host.owner_document_handle(root) == Some(document) =>
            {
                Some(root)
            }
            AdoptedStyleSheetOwner::Document(_) | AdoptedStyleSheetOwner::ShadowRoot(_) => None,
        })
    }

    pub(in crate::style_engine) fn has_document_retained_sources(
        &self,
        host: &DomHost,
        document: DomHandle,
    ) -> bool {
        self.sources(AdoptedStyleSheetOwner::Document(document))
            .is_some_and(|sources| !sources.is_empty())
            && StyleSourceLifecycleOwner::DocumentAdoptedStyleSheets { document }
                .retained_source_records_are_available(host)
    }
}

#[derive(Default)]
struct RetainedAdoptedSourceRecordRequests {
    source_ids: Vec<StyleSourceId>,
}

impl StyleSourceLifecycleRetainedSourceIdSink for RetainedAdoptedSourceRecordRequests {
    fn record_source_lifecycle_retained_source_id(&mut self, source_id: &StyleSourceId) {
        self.source_ids.push(source_id.clone());
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use style::{
        context::QuirksMode, servo_arc::Arc as ServoArc, shared_lock::SharedRwLock,
        stylesheets::AllowImportRules,
    };

    use super::{AdoptedStyleSheetInstallation, AdoptedStyleSheetSources};
    use crate::{
        dom::native::NativeNodeId, live_stylesheet::LiveStylesheetRegistry,
        style_engine::StyleSourceId,
    };

    fn live_stylesheet(
        registry: &LiveStylesheetRegistry,
        name: &str,
    ) -> crate::live_stylesheet::LiveStylesheetRef {
        registry.create(
            &format!(".{name} {{ color: red; }}"),
            url::Url::parse(&format!("https://example.test/{name}.css")).unwrap(),
            QuirksMode::NoQuirks,
            AllowImportRules::No,
            SharedRwLock::new(),
        )
    }

    #[test]
    fn live_adopted_occurrences_keep_stable_client_ids_and_the_same_parsed_stylesheet() {
        let registry = LiveStylesheetRegistry::default();
        let first = live_stylesheet(&registry, "first");
        let second = live_stylesheet(&registry, "second");
        let document = NativeNodeId::new(1);
        let mut sources = AdoptedStyleSheetSources::default();

        assert!(sources.set_document_installations(
            document,
            vec![
                AdoptedStyleSheetInstallation::new(Rc::clone(&first)),
                AdoptedStyleSheetInstallation::new(Rc::clone(&first)),
                AdoptedStyleSheetInstallation::new(Rc::clone(&second)),
            ],
        ));
        let initial = sources.document_sources_or_empty_owned(document);
        let initial_ids = initial
            .iter()
            .map(|source| source.adopted_client_id().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(initial_ids.len(), 3);
        assert_ne!(initial_ids[0], initial_ids[1]);
        assert_ne!(initial_ids[1], initial_ids[2]);
        assert_eq!(
            sources.document_source_ids(document),
            initial_ids
                .iter()
                .map(|client_id| {
                    StyleSourceId::document_adopted_style_sheet(document, *client_id)
                })
                .collect::<Vec<_>>()
        );
        assert!(ServoArc::ptr_eq(
            &initial[0].parsed_stylesheet().unwrap(),
            &first.stylesheet(),
        ));
        assert!(ServoArc::ptr_eq(
            &initial[1].parsed_stylesheet().unwrap(),
            &first.stylesheet(),
        ));

        assert!(sources.set_document_installations(
            document,
            vec![
                AdoptedStyleSheetInstallation::new(Rc::clone(&second)),
                AdoptedStyleSheetInstallation::new(Rc::clone(&first)),
                AdoptedStyleSheetInstallation::new(Rc::clone(&first)),
            ],
        ));
        let reordered = sources.document_sources_or_empty_owned(document);
        assert_eq!(
            reordered
                .iter()
                .map(|source| source.adopted_client_id().unwrap())
                .collect::<Vec<_>>(),
            vec![initial_ids[2], initial_ids[0], initial_ids[1]],
        );
        assert_eq!(
            sources.document_source_ids(document),
            [initial_ids[2], initial_ids[0], initial_ids[1]]
                .into_iter()
                .map(|client_id| StyleSourceId::document_adopted_style_sheet(document, client_id))
                .collect::<Vec<_>>()
        );

        assert!(sources.set_document_installations(
            document,
            vec![
                AdoptedStyleSheetInstallation::new(Rc::clone(&first)),
                AdoptedStyleSheetInstallation::new(Rc::clone(&second)),
            ],
        ));
        assert_eq!(
            sources
                .document_sources_or_empty_owned(document)
                .iter()
                .map(|source| source.adopted_client_id().unwrap())
                .collect::<Vec<_>>(),
            vec![initial_ids[0], initial_ids[2]],
        );
    }

    #[test]
    fn installed_adopted_client_keeps_live_stylesheet_alive_without_a_wrapper_lease() {
        let registry = LiveStylesheetRegistry::default();
        let stylesheet = live_stylesheet(&registry, "retained");
        let stylesheet_id = stylesheet.id();
        let document = NativeNodeId::new(2);
        let mut sources = AdoptedStyleSheetSources::default();

        assert!(sources.set_document_installations(
            document,
            vec![AdoptedStyleSheetInstallation::new(Rc::clone(&stylesheet))],
        ));
        drop(stylesheet);
        assert!(registry.get(stylesheet_id).is_some());

        assert!(sources.set_document_installations(document, Vec::new()));
        assert!(registry.get(stylesheet_id).is_none());
    }
}
