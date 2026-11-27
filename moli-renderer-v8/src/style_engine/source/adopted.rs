use std::collections::{HashMap, HashSet};

use crate::document_runtime::DomHandle;

use super::super::StyleSourceId;
use super::store::StyloStylesheetSource;

#[derive(Debug, Default)]
pub(in crate::style_engine) struct AdoptedStyleSheetSources {
    sources_by_owner: HashMap<AdoptedStyleSheetOwner, Vec<InstalledAdoptedStylesheetClient>>,
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
