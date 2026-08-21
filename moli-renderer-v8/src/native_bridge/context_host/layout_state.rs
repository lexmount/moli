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
/// The single snapshot slot records the exact root Document it was built for;
/// embedded Document member trees remain recursively owned by that same
/// snapshot instead of becoming separately keyed cache entries.
pub(super) struct DocumentLayoutState {
    services: DocumentLayoutServices,
    embedded_document_services: HashMap<DomHandle, DocumentLayoutServices>,
    web_fonts: DocumentWebFontState,
    web_font_sources_dirty: bool,
    visual_state_generation: u64,
    latest_layout: LatestLayoutTreeCache,
    /// Last used content viewport published by each live iframe owner's
    /// parent layout. Blink keeps the equivalent size on LocalFrameView; it is
    /// separate from the single latest-tree slot because a later fresh layout
    /// may replace that slot while the frame view remains live.
    frame_viewports: HashMap<DomHandle, LayoutViewport>,
}

impl Default for DocumentLayoutState {
    fn default() -> Self {
        Self {
            services: DocumentLayoutServices::default(),
            embedded_document_services: HashMap::new(),
            web_fonts: DocumentWebFontState::default(),
            web_font_sources_dirty: true,
            visual_state_generation: 0,
            latest_layout: LatestLayoutTreeCache::default(),
            frame_viewports: HashMap::new(),
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

    pub(super) const fn visual_state_generation(&self) -> u64 {
        self.visual_state_generation
    }

    pub(super) fn mark_visual_state_dirty(&mut self) {
        self.visual_state_generation = self.visual_state_generation.saturating_add(1);
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

    pub(super) fn latest_layout_for_root(
        &self,
        root: DomHandle,
    ) -> Option<&FrozenLayoutTree<DomHandle>> {
        self.latest_layout.get_for_root(root)
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
        self.mark_visual_state_dirty();
    }

    pub(super) fn frame_viewport(&self, frame: DomHandle) -> Option<LayoutViewport> {
        self.frame_viewports.get(&frame).copied()
    }

    pub(super) fn update_frame_viewports(
        &mut self,
        updates: impl IntoIterator<Item = (DomHandle, Option<LayoutViewport>)>,
    ) {
        for (frame, viewport) in updates {
            match viewport {
                Some(viewport) => {
                    self.frame_viewports.insert(frame, viewport);
                }
                None => {
                    self.frame_viewports.remove(&frame);
                }
            }
        }
    }

    pub(super) fn retain_live_frame_viewports(
        &mut self,
        mut is_live: impl FnMut(DomHandle) -> bool,
    ) {
        self.frame_viewports.retain(|frame, _| is_live(*frame));
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
        let completion = self.web_fonts.complete(terminal, &mut self.services);
        if !matches!(&completion, DocumentWebFontCompletion::Stale) {
            self.mark_visual_state_dirty();
        }
        completion
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
