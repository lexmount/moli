use std::collections::HashMap;

use moli_layout::{DocumentLayoutServices, FrozenLayoutTree, LayoutViewport};

use super::layout_snapshot::LatestLayoutTreeCache;
use crate::{
    css_resource_urls::{CompletedStylesheetWebFont, StylesheetLoadBlockingResource},
    document_runtime::DomHandle,
    script_vm::web_fonts::{DocumentWebFontCompletion, DocumentWebFontState},
};

/// Layout-facing state whose lifetime is bounded by exactly one main Document.
///
/// `ScriptVm` outlives `document.open()`, so the main-document owner
/// transition replaces this value explicitly. Full layout passes borrow the
/// services and discard every working tree/cache. Only the latest frozen
/// layout tree and document-owned font/text sidecars survive a pass.
/// Embedded documents receive separate Parley services so a main-document
/// `@font-face` registration cannot leak across the browsing-context boundary.
/// The single snapshot slot may describe any exact Document in the current
/// document tree; its identity is stored alongside the tree.
pub(super) struct DocumentLayoutState {
    services: DocumentLayoutServices,
    embedded_document_services: HashMap<DomHandle, DocumentLayoutServices>,
    web_fonts: DocumentWebFontState,
    web_font_sources_dirty: bool,
    latest_layout: LatestLayoutTreeCache,
    /// Last used content viewport published by each live iframe owner's
    /// parent layout. Blink keeps the equivalent size on LocalFrameView; it is
    /// separate from the single latest-tree slot because parent and child
    /// layout queries replace that slot while the frame view remains live.
    embedded_frame_viewports: HashMap<DomHandle, LayoutViewport>,
}

impl Default for DocumentLayoutState {
    fn default() -> Self {
        Self {
            services: DocumentLayoutServices::default(),
            embedded_document_services: HashMap::new(),
            web_fonts: DocumentWebFontState::default(),
            web_font_sources_dirty: true,
            latest_layout: LatestLayoutTreeCache::default(),
            embedded_frame_viewports: HashMap::new(),
        }
    }
}

impl DocumentLayoutState {
    pub(super) fn mark_web_font_sources_dirty(&mut self) {
        self.web_font_sources_dirty = true;
    }

    pub(super) fn take_web_font_sources_dirty(&mut self) -> bool {
        std::mem::take(&mut self.web_font_sources_dirty)
    }

    pub(crate) fn with_services_for_document<T>(
        &mut self,
        document: DomHandle,
        main_document: DomHandle,
        consume: impl FnOnce(
            &mut DocumentLayoutServices,
            &mut HashMap<DomHandle, DocumentLayoutServices>,
        ) -> T,
    ) -> T {
        if document == main_document {
            return consume(&mut self.services, &mut self.embedded_document_services);
        }

        // Remove the exact child service while its recursive pass runs so the
        // same map can lend distinct services to nested documents. Reinsert on
        // every ordinary Result path; no pointer into the map escapes.
        let mut services = self
            .embedded_document_services
            .remove(&document)
            .unwrap_or_default();
        let output = consume(&mut services, &mut self.embedded_document_services);
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

    pub(super) fn embedded_frame_viewport(&self, frame: DomHandle) -> Option<LayoutViewport> {
        self.embedded_frame_viewports.get(&frame).copied()
    }

    pub(super) fn publish_embedded_frame_viewports(
        &mut self,
        updates: impl IntoIterator<Item = (DomHandle, Option<LayoutViewport>)>,
    ) {
        for (frame, viewport) in updates {
            match viewport {
                Some(viewport) => {
                    self.embedded_frame_viewports.insert(frame, viewport);
                }
                None => {
                    self.embedded_frame_viewports.remove(&frame);
                }
            }
        }
    }

    pub(super) fn retain_live_embedded_frame_viewports(
        &mut self,
        mut is_live: impl FnMut(DomHandle) -> bool,
    ) {
        self.embedded_frame_viewports
            .retain(|frame, _| is_live(*frame));
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
