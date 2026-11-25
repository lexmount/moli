use moli_layout::{IsolatedSizeLayout, LayoutError, LayoutSize, LayoutUsedSize};

use super::{RealtimeLayoutState, ScreenLayoutInputs};
use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};

pub(super) struct IsolatedSizeCache {
    inputs: ScreenLayoutInputs,
    dom_version: u64,
    style_generations: Vec<(DomHandle, u64, u64)>,
    sizes: IsolatedSizeLayout<DomHandle>,
}

impl PageScriptHost {
    /// Size-only demands may update an independent positioned region without
    /// replacing the screen publication. Its pending document invalidations
    /// remain visible to rendering, observers and all other geometry queries.
    pub(crate) fn try_isolated_offset_size(
        &self,
        source: DomHandle,
    ) -> Result<Option<LayoutSize>, LayoutError> {
        let Some(state) = self.realtime_layout_state.as_deref() else {
            return Ok(None);
        };
        if !self.scrollbars_hidden
            || self.layout_document_for_source(source) != Some(self.document_handle())
            || !self.dom_host().is_connected(source)
        {
            return Ok(None);
        }
        if self.layout_pass_active.get() {
            return Err(LayoutError::ReentrantLayoutPass);
        }
        let inputs = self.screen_layout_inputs(state)?;
        if !self.only_main_document_layout_changed(state, &inputs)
            || (self.screen_layout_matches_inputs(state, &inputs)
                && !self.has_screen_style_invalidations(&inputs))
        {
            return Ok(None);
        }
        let dom_version = self.dom_host().dom_version();
        if let Some(cache) = state.isolated_sizes.borrow().as_ref()
            && self.isolated_cache_matches(cache, &inputs, dom_version)
            && let Some(size) = cache.sizes.offset_size(source)
        {
            return Ok(Some(size));
        }

        self.prepare_current_screen_styles(&inputs);
        let document = self.document_handle();
        let _active = super::super::layout::ActiveLayoutPass::enter(&self.layout_pass_active)?;
        let sizes = self
            .document_layout_state
            .borrow_mut()
            .with_services_for_document(document, document, |services, _| {
                crate::layout_renderer::try_build_isolated_sizes(self, source, services)
            })?;
        let Some(sizes) = sizes else {
            return Ok(None);
        };
        if !self.only_region_changed(&sizes) {
            return Ok(None);
        }
        if self.dom_host().dom_version() != dom_version
            || self.screen_layout_inputs(state)? != inputs
        {
            return Err(LayoutError::source_contract(
                "isolated sizes",
                "layout inputs changed during measurement",
            ));
        }
        let answer = sizes.offset_size(source);
        #[cfg(test)]
        state
            .isolated_size_layouts
            .set(state.isolated_size_layouts.get().saturating_add(1));
        if moli_trace::cpu_profile_enabled() {
            let metrics = sizes.metrics();
            tracing::info!(target: "moli_cpu_profile", stage = "isolated_size_layout",
                total_us = metrics.elapsed.as_micros(), box_count = metrics.box_count,
                fragment_count = metrics.fragment_count);
        }
        let style_generations = self.style_engine.visual_style_input_generations([document]);
        if dom_version != u64::MAX
            && inputs.permits_reuse()
            && style_generations
                .iter()
                .all(|&(_, source, visual)| source != u64::MAX && visual != u64::MAX)
        {
            *state.isolated_sizes.borrow_mut() = Some(IsolatedSizeCache {
                inputs,
                dom_version,
                style_generations,
                sizes,
            });
        }
        Ok(answer)
    }

    /// CSSOM remains snapshot-only. Sizes produced by a preceding offset read
    /// supersede that region's older document snapshot, without forcing layout.
    pub(crate) fn cached_isolated_used_size(
        &self,
        source: DomHandle,
    ) -> Option<Option<LayoutUsedSize>> {
        let state = self.realtime_layout_state.as_deref()?;
        let cache = state.isolated_sizes.borrow();
        let cache = cache.as_ref()?;
        let inputs = self.screen_layout_inputs(state).ok()?;
        self.isolated_cache_matches(cache, &inputs, self.dom_host().dom_version())
            .then(|| cache.sizes.used_size(source))
            .flatten()
    }

    fn isolated_cache_matches(
        &self,
        cache: &IsolatedSizeCache,
        inputs: &ScreenLayoutInputs,
        dom_version: u64,
    ) -> bool {
        dom_version != u64::MAX
            && inputs.permits_reuse()
            && cache.dom_version == dom_version
            && cache.inputs == *inputs
            && cache.style_generations
                == self
                    .style_engine
                    .visual_style_input_generations([self.document_handle()])
    }

    fn only_main_document_layout_changed(
        &self,
        state: &RealtimeLayoutState,
        inputs: &ScreenLayoutInputs,
    ) -> bool {
        if !inputs.permits_reuse()
            || inputs.documents.iter().any(|document| {
                document.document != self.document_handle()
                    && self
                        .style_engine
                        .has_screen_style_invalidation(document.document)
            })
        {
            return false;
        }
        let publication = state.published.borrow();
        let Some(published) = publication.as_ref() else {
            return false;
        };
        let mut expected = published.clone();
        for document in &mut expected.documents {
            if document.document == self.document_handle() {
                document.dom_revision = self.dom_host().document_layout_version(document.document);
            }
        }
        expected == *inputs
    }

    fn only_region_changed(&self, sizes: &IsolatedSizeLayout<DomHandle>) -> bool {
        let root = sizes.root();
        let document = self.document_handle();
        let Some(native_roots) = self.dom_host().document_layout_change_roots(document) else {
            return false;
        };
        let Some(style_changes) = self
            .style_engine
            .screen_style_changes(self.dom_host(), document)
        else {
            return false;
        };
        let within = |mut node| {
            // Use flat ancestry, just like the measured region. Bound the walk
            // and reject unknown ownership/topology instead of guessing.
            for _ in 0..64 {
                if node == root {
                    return true;
                }
                let Some(parent) =
                    crate::dom_presentation::flat_parent(self.dom_host(), document, node)
                else {
                    return false;
                };
                node = parent;
            }
            false
        };
        if !native_roots.into_iter().all(within) {
            return false;
        }
        let external_targets = style_changes
            .targets
            .iter()
            .any(|(handle, _)| !within(*handle));
        (!external_targets
            || self
                .style_engine
                .with_screen_style_coverage(document, |coverage| {
                    crate::layout_renderer::validate_region_external_styles(
                        self,
                        document,
                        coverage,
                        |handle| !within(handle),
                    )
                })
                .unwrap_or(false))
            && self
                .style_engine
                .has_supported_screen_style_damage_for_nodes(
                    document,
                    style::servo::restyle_damage::ServoRestyleDamage::empty(),
                    |handle| self.dom_host().is_connected(handle) && !within(handle),
                )
    }

    #[cfg(test)]
    pub(crate) fn isolated_size_layout_count_for_test(&self) -> u64 {
        self.realtime_layout_state
            .as_deref()
            .map_or(0, |state| state.isolated_size_layouts.get())
    }
}
