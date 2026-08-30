use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use crate::document_runtime::DomHandle;

use super::{
    cache::PseudoStyleCache,
    pending_invalidation::PendingStyleInvalidations,
    pending_mutation::PendingStructuralStyleMutations,
    registered_properties::CssCustomPropertyRegistry,
    source::{
        adopted::AdoptedStyleSheetSources, inline::InlineStyleMetadataStore,
        linked::LinkedStylesheetSources,
    },
    source_owner_text::OwnerStyleSheetSources,
    state::{StyleDocumentGenerationSnapshot, StyleDocumentState},
};

pub(super) struct DocumentStyleWorld {
    pub(super) document: DomHandle,
    pub(super) registered_custom_properties: CssCustomPropertyRegistry,
    pub(super) document_state: StyleDocumentState,
    pub(super) pending_invalidations: PendingStyleInvalidations,
    pub(super) pending_structural_mutations: PendingStructuralStyleMutations,
    pub(super) pseudo_style_cache: PseudoStyleCache,
    pub(super) owner_style_sheet_sources: RefCell<OwnerStyleSheetSources>,
    pub(super) linked_stylesheet_sources: RefCell<LinkedStylesheetSources>,
    pub(super) adopted_style_sheet_sources: RefCell<AdoptedStyleSheetSources>,
    pub(super) inline_style_metadata: InlineStyleMetadataStore,
}

pub(super) struct DocumentStyleWorlds {
    worlds: RefCell<HashMap<DomHandle, Rc<DocumentStyleWorld>>>,
    /// Monotonic lower bound used when no active world exists. This keeps stale
    /// computed-style wrappers distinguishable from later worlds without an
    /// O(navigations) tombstone map keyed by every retired Document.
    lifecycle_generation_floor: Cell<u64>,
}

impl DocumentStyleWorld {
    fn new(document: DomHandle) -> Self {
        Self {
            document,
            registered_custom_properties: CssCustomPropertyRegistry::new(),
            document_state: StyleDocumentState::new(),
            pending_invalidations: PendingStyleInvalidations::new(),
            pending_structural_mutations: PendingStructuralStyleMutations::new(),
            pseudo_style_cache: PseudoStyleCache::new(),
            owner_style_sheet_sources: RefCell::new(OwnerStyleSheetSources::default()),
            linked_stylesheet_sources: RefCell::new(LinkedStylesheetSources::default()),
            adopted_style_sheet_sources: RefCell::new(AdoptedStyleSheetSources::default()),
            inline_style_metadata: InlineStyleMetadataStore::default(),
        }
    }

    pub(super) fn clear_for_document_replacement(&self) {
        self.registered_custom_properties.clear();
        self.pending_invalidations.clear();
        self.pending_structural_mutations.clear();
        self.pseudo_style_cache.clear();
        self.document_state.clear_retained_style_system();
        self.document_state.bump_source_set_generation();
        self.document_state.bump_computed_cache_generation();
        self.document_state.bump_target_context_epoch();
        self.owner_style_sheet_sources.borrow_mut().clear_all();
        *self.linked_stylesheet_sources.borrow_mut() = LinkedStylesheetSources::default();
        self.adopted_style_sheet_sources.borrow_mut().clear_all();
        self.inline_style_metadata.clear_all();
    }
}

impl DocumentStyleWorlds {
    pub(super) fn new() -> Self {
        Self {
            worlds: RefCell::new(HashMap::new()),
            lifecycle_generation_floor: Cell::new(0),
        }
    }

    pub(super) fn for_document(&self, document: DomHandle) -> Rc<DocumentStyleWorld> {
        if let Some(world) = self.active_world(document) {
            return world;
        }
        let world = Rc::new(DocumentStyleWorld::new(document));
        self.worlds.borrow_mut().insert(document, Rc::clone(&world));
        world
    }

    pub(super) fn active_world(&self, document: DomHandle) -> Option<Rc<DocumentStyleWorld>> {
        self.worlds.borrow().get(&document).map(Rc::clone)
    }

    /// Removes one Document's heavyweight style state without making a lookup
    /// create it first.
    ///
    /// The lifecycle generation floor replaces per-Document tombstones. A
    /// retired child/popup handle is terminal; same-handle `document.open()`
    /// remains active and uses `clear_for_document_replacement` instead.
    pub(super) fn retire_document(&self, document: DomHandle) -> bool {
        let world = self.worlds.borrow_mut().remove(&document);
        let Some(world) = world else {
            return false;
        };
        // Clear before dropping the map's Rc so a short-lived borrower cannot
        // keep the retired Stylist or stylesheet sources alive past this
        // lifecycle boundary.
        world.clear_for_document_replacement();
        self.raise_lifecycle_generation_floor(world.document_state.generation_snapshot());
        true
    }

    pub(super) fn generation_snapshot_for_document(
        &self,
        document: DomHandle,
    ) -> StyleDocumentGenerationSnapshot {
        if let Some(generations) = self
            .worlds
            .borrow()
            .get(&document)
            .map(|world| world.document_state.generation_snapshot())
        {
            return generations;
        }
        self.lifecycle_generation_snapshot()
    }

    fn lifecycle_generation_snapshot(&self) -> StyleDocumentGenerationSnapshot {
        lifecycle_generation_snapshot(self.lifecycle_generation_floor.get())
    }

    fn raise_lifecycle_generation_floor(&self, generations: StyleDocumentGenerationSnapshot) {
        self.lifecycle_generation_floor.set(
            self.lifecycle_generation_floor
                .get()
                .max(generations.computed_cache_generation)
                .max(generations.target_context_epoch),
        );
    }

    pub(super) fn clear_for_document_replacement(&self, document: DomHandle) {
        self.for_document(document).clear_for_document_replacement();
    }

    /// Retires one handle's cached pseudo styles from whichever Document world
    /// last exposed it. A DOM adoption can change `ownerDocument` before
    /// deferred style work is drained, so consulting only the current owner
    /// would leave the old world's pseudo values behind.
    pub(super) fn invalidate_cached_pseudos_for_handle(&self, handle: DomHandle) {
        for world in self.worlds.borrow().values() {
            world.pseudo_style_cache.invalidate_handles([handle]);
        }
    }

    pub(super) fn documents_with_adopted_style_sheets(&self) -> Vec<DomHandle> {
        self.worlds
            .borrow()
            .values()
            .filter_map(|world| {
                (world
                    .adopted_style_sheet_sources
                    .borrow()
                    .document_source_count(world.document)
                    != 0)
                    .then_some(world.document)
            })
            .collect()
    }

    pub(super) fn observation_generations(
        &self,
        documents: impl IntoIterator<Item = DomHandle>,
    ) -> Vec<(DomHandle, u64, u64, u64)> {
        let mut documents = documents.into_iter().collect::<Vec<_>>();
        documents.sort_by_key(|document| document.index());
        documents.dedup();
        documents
            .into_iter()
            .map(|document| {
                let world = self.for_document(document);
                let generation = world.document_state.generation_snapshot();
                (
                    document,
                    generation.source_set_generation,
                    generation.computed_cache_generation,
                    generation.target_context_epoch,
                )
            })
            .collect()
    }

    #[cfg(test)]
    pub(super) fn active_world_count(&self) -> usize {
        self.worlds.borrow().len()
    }

    #[cfg(test)]
    pub(super) fn contains_active_world(&self, document: DomHandle) -> bool {
        self.worlds.borrow().contains_key(&document)
    }
}

fn lifecycle_generation_snapshot(generation: u64) -> StyleDocumentGenerationSnapshot {
    StyleDocumentGenerationSnapshot {
        source_set_generation: 0,
        computed_cache_generation: generation,
        retained_style_system_generation: 0,
        target_context_epoch: generation,
    }
}
