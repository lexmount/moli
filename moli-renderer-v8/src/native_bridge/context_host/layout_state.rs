use std::collections::HashMap;

use moli_layout::{DocumentLayoutServices, FrozenLayoutTree};

use super::layout_snapshot::LatestLayoutTreeCache;
use crate::{
    css_resource_urls::{CompletedStylesheetWebFont, StylesheetLoadBlockingResource},
    document_runtime::DomHandle,
    layout_renderer::{AutoDisplayLockState, IntrinsicSizeObserverState},
    script_vm::web_fonts::{
        DocumentWebFontCompletion, DocumentWebFontLoadCycleId, DocumentWebFontState,
    },
};

/// Layout-facing state whose lifetime is bounded by exactly one main Document.
///
/// `ScriptVm` outlives `document.open()`, so the main-document owner
/// transition replaces this value explicitly. Full layout passes borrow the
/// services and discard every working tree/cache. Only the latest frozen
/// layout tree and document-owned font, display-lock, text, and intrinsic-size
/// state survive a pass.
/// Embedded documents receive separate Parley services so a main-document
/// `@font-face` registration cannot leak across the browsing-context boundary.
/// The single snapshot slot may describe any exact Document in the current
/// document tree; its identity is stored alongside the tree.
pub(super) struct DocumentLayoutState {
    services: DocumentLayoutServices,
    embedded_document_services: HashMap<DomHandle, DocumentLayoutServices>,
    auto_display_locks: AutoDisplayLockState,
    intrinsic_size_observer: IntrinsicSizeObserverState,
    web_fonts: DocumentWebFontState,
    web_font_sources_dirty: bool,
    latest_layout: LatestLayoutTreeCache,
}

impl Default for DocumentLayoutState {
    fn default() -> Self {
        Self {
            services: DocumentLayoutServices::default(),
            embedded_document_services: HashMap::new(),
            auto_display_locks: AutoDisplayLockState::default(),
            intrinsic_size_observer: IntrinsicSizeObserverState::default(),
            web_fonts: DocumentWebFontState::default(),
            web_font_sources_dirty: true,
            latest_layout: LatestLayoutTreeCache::default(),
        }
    }
}

impl DocumentLayoutState {
    pub(super) fn auto_display_lock_is_locked(&self, element: DomHandle) -> bool {
        self.auto_display_locks.published_is_locked(element)
    }

    pub(super) fn mark_web_font_sources_dirty(&mut self) {
        self.web_font_sources_dirty = true;
    }

    pub(super) fn take_web_font_sources_dirty(&mut self) -> bool {
        std::mem::take(&mut self.web_font_sources_dirty)
    }

    pub(crate) fn with_layout_pass_state_for_document<T>(
        &mut self,
        document: DomHandle,
        main_document: DomHandle,
        consume: impl FnOnce(
            &mut DocumentLayoutServices,
            &mut HashMap<DomHandle, DocumentLayoutServices>,
            &mut AutoDisplayLockState,
            &mut IntrinsicSizeObserverState,
        ) -> T,
    ) -> T {
        if document == main_document {
            return consume(
                &mut self.services,
                &mut self.embedded_document_services,
                &mut self.auto_display_locks,
                &mut self.intrinsic_size_observer,
            );
        }

        // Remove the exact child service while its recursive pass runs so the
        // same map can lend distinct services to nested documents. Reinsert on
        // every ordinary Result path; no pointer into the map escapes.
        let mut services = self
            .embedded_document_services
            .remove(&document)
            .unwrap_or_default();
        let output = consume(
            &mut services,
            &mut self.embedded_document_services,
            &mut self.auto_display_locks,
            &mut self.intrinsic_size_observer,
        );
        self.embedded_document_services.insert(document, services);
        output
    }

    pub(super) fn retain_live_embedded_document_services(
        &mut self,
        mut is_live: impl FnMut(DomHandle) -> bool,
    ) {
        self.embedded_document_services
            .retain(|document, _| is_live(*document));
    }

    pub(super) fn latest_layout(
        &self,
        document: DomHandle,
    ) -> Option<&FrozenLayoutTree<DomHandle>> {
        self.latest_layout.get(document)
    }

    pub(super) fn publish_latest_layout(
        &mut self,
        document: DomHandle,
        tree: FrozenLayoutTree<DomHandle>,
    ) {
        self.latest_layout.publish(document, tree);
    }

    pub(super) fn clear_latest_layout(&mut self) {
        self.latest_layout.clear();
    }

    #[cfg(test)]
    pub(super) fn latest_layout_observability(
        &self,
    ) -> Option<(DomHandle, moli_layout::LayoutTreeRetentionMetrics)> {
        self.latest_layout.observability()
    }

    pub(super) fn retain_active_slots<'a>(
        &mut self,
        resources: impl IntoIterator<Item = &'a StylesheetLoadBlockingResource>,
    ) {
        self.web_fonts
            .retain_active_slots(resources, &mut self.services);
    }

    pub(super) fn admit(
        &mut self,
        resource: StylesheetLoadBlockingResource,
    ) -> Option<StylesheetLoadBlockingResource> {
        self.web_fonts.admit(resource, &mut self.services)
    }

    pub(super) fn reserve_web_font_ready_cycle<'a>(
        &mut self,
        resources: impl IntoIterator<Item = &'a StylesheetLoadBlockingResource>,
    ) -> Option<DocumentWebFontLoadCycleId> {
        self.web_fonts.reserve_for_ready(resources)
    }

    pub(super) fn active_web_font_load_cycle(&self) -> Option<DocumentWebFontLoadCycleId> {
        self.web_fonts.active_load_cycle()
    }

    pub(super) fn web_font_ready_layout_task_needed(&self) -> Option<DocumentWebFontLoadCycleId> {
        self.web_fonts.ready_layout_task_needed()
    }

    pub(super) fn web_font_cycle_ready_for_layout(&self) -> Option<DocumentWebFontLoadCycleId> {
        self.web_fonts.load_cycle_ready_for_layout()
    }

    pub(super) fn complete_web_font_cycle_after_layout(
        &mut self,
        cycle: DocumentWebFontLoadCycleId,
    ) -> bool {
        self.web_fonts.complete_after_layout(cycle)
    }

    pub(super) fn complete(
        &mut self,
        terminal: CompletedStylesheetWebFont,
    ) -> DocumentWebFontCompletion {
        self.web_fonts.complete(terminal, &mut self.services)
    }

    #[cfg(test)]
    pub(super) fn web_font_counts(&self) -> (usize, usize, usize) {
        (
            self.web_fonts.slot_count(),
            self.web_fonts.ready_slot_count(),
            self.services.web_font_count(),
        )
    }
}
