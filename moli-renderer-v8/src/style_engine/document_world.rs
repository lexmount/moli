use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::document_runtime::DomHandle;

use super::{
    cache::{ComputedStyleCache, ComputedStyleInputCache},
    pending_invalidation::PendingStyleInvalidations,
    pending_mutation::PendingStructuralStyleMutations,
    registered_properties::CssCustomPropertyRegistry,
    source::{
        adopted::AdoptedStyleSheetSources, inline::InlineStyleMetadataStore,
        linked::LinkedStylesheetSources,
    },
    source_owner_text::OwnerStyleSheetSources,
    state::StyleDocumentState,
};

pub(super) struct DocumentStyleWorld {
    pub(super) document: DomHandle,
    pub(super) registered_custom_properties: CssCustomPropertyRegistry,
    pub(super) document_state: StyleDocumentState,
    pub(super) pending_invalidations: PendingStyleInvalidations,
    pub(super) pending_structural_mutations: PendingStructuralStyleMutations,
    pub(super) computed_style_cache: ComputedStyleCache,
    pub(super) computed_style_input_cache: ComputedStyleInputCache,
    pub(super) owner_style_sheet_sources: RefCell<OwnerStyleSheetSources>,
    pub(super) linked_stylesheet_sources: RefCell<LinkedStylesheetSources>,
    pub(super) adopted_style_sheet_sources: RefCell<AdoptedStyleSheetSources>,
    pub(super) inline_style_metadata: InlineStyleMetadataStore,
}

pub(super) struct DocumentStyleWorlds {
    worlds: RefCell<HashMap<DomHandle, Rc<DocumentStyleWorld>>>,
}

impl DocumentStyleWorld {
    fn new(document: DomHandle) -> Self {
        Self {
            document,
            registered_custom_properties: CssCustomPropertyRegistry::new(),
            document_state: StyleDocumentState::new(),
            pending_invalidations: PendingStyleInvalidations::new(),
            pending_structural_mutations: PendingStructuralStyleMutations::new(),
            computed_style_cache: ComputedStyleCache::new(),
            computed_style_input_cache: ComputedStyleInputCache::new(),
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
        self.computed_style_cache.clear();
        self.computed_style_input_cache.clear();
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
        }
    }

    pub(super) fn for_document(&self, document: DomHandle) -> Rc<DocumentStyleWorld> {
        if let Some(world) = self.worlds.borrow().get(&document) {
            return Rc::clone(world);
        }
        let world = Rc::new(DocumentStyleWorld::new(document));
        self.worlds.borrow_mut().insert(document, Rc::clone(&world));
        world
    }

    pub(super) fn clear_for_document_replacement(&self, document: DomHandle) {
        self.for_document(document).clear_for_document_replacement();
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
}
