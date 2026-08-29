use moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE;
use moli_layout::{
    FrozenLayoutTree, GeometryProvider, LayoutAnswers, LayoutError, LayoutFlushReason,
    LayoutPassRequest, LayoutPassResult, LayoutQuery, LayoutQueryAnswer, LayoutQueryBatch,
    LayoutViewport,
};

use super::JsContextHost;
use super::layout_state::InferredFrameStyleViewportCacheKey;
use crate::{
    css_resource_urls::{CompletedStylesheetWebFont, StylesheetLoadBlockingResource},
    document_runtime::DomHandle,
    native_bridge::element::iframe_handle_viewport,
    script_vm::web_fonts::DocumentWebFontCompletion,
    style_engine::{StyleViewport, StyloStyleEnvironment},
};

/// Resets the entry flag on every return path, including unwinding.
///
/// This is a synchronous ownership guard, not a generation or a retry fence.
struct ActiveLayoutPass<'a> {
    active: &'a std::cell::Cell<bool>,
}

impl<'a> ActiveLayoutPass<'a> {
    fn enter(active: &'a std::cell::Cell<bool>) -> Result<Self, LayoutError> {
        if active.replace(true) {
            return Err(LayoutError::ReentrantLayoutPass);
        }
        Ok(Self { active })
    }
}

impl Drop for ActiveLayoutPass<'_> {
    fn drop(&mut self) {
        self.active.set(false);
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LayoutSnapshotCacheObservability {
    pub(crate) hits: u64,
    pub(crate) misses: u64,
    pub(crate) publishes: u64,
    pub(crate) cached: Option<(DomHandle, moli_layout::LayoutTreeRetentionMetrics)>,
}

impl JsContextHost {
    pub(crate) fn set_layout_policy(&mut self, policy: moli_page_types::LayoutPolicy) {
        if !policy.uses_real_layout() {
            self.document_layout_state.get_mut().clear_latest_layout();
        }
        self.layout_policy = policy;
    }

    pub(crate) const fn layout_policy(&self) -> moli_page_types::LayoutPolicy {
        self.layout_policy
    }

    pub(crate) fn active_layout_document_handles(&self) -> Vec<DomHandle> {
        let mut documents = vec![self.document_handle()];
        documents.extend(
            self.child_browsing_context_handles_in_document_order()
                .into_iter()
                .filter(|frame| self.child_browsing_context_host_is_active(*frame))
                .filter_map(|frame| self.child_browsing_context_document_handle(frame)),
        );
        documents.sort_by_key(|document| document.index());
        documents.dedup();
        documents
    }

    pub(crate) fn visual_state_generation(&self) -> u64 {
        self.document_layout_state
            .borrow()
            .visual_state_generation()
    }

    pub(crate) fn visual_state_token(
        &self,
        document: crate::runtime::RendererDocumentLifecycleIdentity,
        viewport: moli_layout::PaintViewport,
    ) -> crate::runtime::RendererVisualStateToken {
        crate::runtime::RendererVisualStateToken::new(
            document,
            self.dom_host().dom_version(),
            self.style_engine
                .computed_style_observation_generations(self.active_layout_document_handles()),
            self.visual_state_generation(),
            self.visual_resource_generation(),
            viewport,
        )
    }

    pub(crate) fn layout_document_for_source(&self, source: DomHandle) -> Option<DomHandle> {
        self.dom_host().owner_document_handle(source)
    }

    pub(crate) fn layout_viewport_for_document(&self, document: DomHandle) -> LayoutViewport {
        let surface = self.viewport_surface;
        let device_pixel_ratio = surface
            .map(|surface| surface.device_pixel_ratio as f32)
            .unwrap_or(DEFAULT_WINDOW_SURFACE_PROFILE.device_pixel_ratio as f32);
        if document == self.document_handle() {
            return LayoutViewport::new(
                surface
                    .map(|surface| surface.inner_width)
                    .unwrap_or(DEFAULT_WINDOW_SURFACE_PROFILE.inner_width as u32),
                surface
                    .map(|surface| surface.inner_height)
                    .unwrap_or(DEFAULT_WINDOW_SURFACE_PROFILE.inner_height as u32),
                device_pixel_ratio,
            );
        }

        let child_viewport = self
            .child_browsing_context_host_for_document_handle(document)
            .and_then(|frame| iframe_handle_viewport(self, frame));
        LayoutViewport::new(
            child_viewport
                .and_then(|viewport| viewport.width)
                .map(css_viewport_dimension)
                .unwrap_or(DEFAULT_WINDOW_SURFACE_PROFILE.inner_width as u32),
            child_viewport
                .and_then(|viewport| viewport.height)
                .map(css_viewport_dimension)
                .unwrap_or(DEFAULT_WINDOW_SURFACE_PROFILE.inner_height as u32),
            device_pixel_ratio,
        )
    }

    /// Returns the last exact used content viewport published by the iframe's
    /// parent layout. A non-blocking borrow is intentional: child style
    /// resolution can run while the document layout state is already lent to
    /// a recursive paint pass, in which case the caller uses its ordinary
    /// authored-style fallback.
    pub(crate) fn frame_viewport(&self, frame: DomHandle) -> Option<LayoutViewport> {
        self.document_layout_state
            .try_borrow()
            .ok()?
            .frame_viewport(frame)
    }

    #[cfg(debug_assertions)]
    pub(crate) fn style_viewport_generation(&self) -> u64 {
        self.style_viewport_generation.get()
    }

    /// Marks the exact viewport selected by an active layout pass before that
    /// pass reads the Document's styles.
    ///
    /// An embedded Document's persistent frame viewport is published only
    /// after the recursive pass succeeds. Advancing this input generation
    /// first also distinguishes an explicit main-Document capture viewport
    /// from ordinary DOM APIs selecting conflicting implicit viewports.
    pub(crate) fn note_layout_style_viewport(&self, document: DomHandle, viewport: StyleViewport) {
        let matches_cached = if document == self.document_handle() {
            viewport == self.style_viewport()
        } else {
            self.child_browsing_context_host_for_document_handle(document)
                .and_then(|frame| self.frame_viewport(frame))
                .is_some_and(|cached| {
                    viewport.width == Some(f64::from(cached.css_width))
                        && viewport.height == Some(f64::from(cached.css_height))
                })
        };
        if !matches_cached {
            self.style_viewport_generation
                .set(self.style_viewport_generation.get().saturating_add(1));
        }
    }

    pub(crate) fn inferred_frame_style_viewport(
        &self,
        frame: DomHandle,
        parent_document: DomHandle,
        parent_viewport: StyleViewport,
        compute: impl FnOnce() -> StyleViewport,
    ) -> StyleViewport {
        let key = InferredFrameStyleViewportCacheKey {
            dom_version: self.dom_host().dom_version(),
            parent_style_epoch: self
                .style_engine
                .target_context_epoch_for_document(parent_document),
            parent_viewport,
            environment: StyloStyleEnvironment::from_emulated_media(self.emulated_media()),
        };
        if let Ok(mut state) = self.document_layout_state.try_borrow_mut()
            && let Some(viewport) = state.inferred_frame_style_viewport(frame, key)
        {
            return viewport;
        }
        let viewport = compute();
        if let Ok(mut state) = self.document_layout_state.try_borrow_mut() {
            state.publish_inferred_frame_style_viewport(frame, key, viewport);
        }
        viewport
    }

    #[cfg(test)]
    pub(crate) fn inferred_frame_style_viewport_cache_observability(&self) -> (u64, u64, usize) {
        self.document_layout_state
            .borrow()
            .inferred_frame_style_viewport_cache_observability()
    }

    pub(crate) fn with_fresh_layout_pass_for_document<T>(
        &self,
        document: DomHandle,
        request: LayoutPassRequest,
        consume: impl FnOnce(&mut LayoutPassResult<DomHandle>) -> Result<T, LayoutError>,
    ) -> Result<Option<T>, LayoutError> {
        let Some(root) = self
            .dom_host()
            .dom()
            .document_element_handle_for_document(document)
        else {
            return Ok(None);
        };
        let pass_viewport = request.viewport;
        let _active = ActiveLayoutPass::enter(&self.layout_pass_active)?;
        let mut pass = {
            let mut state = self.document_layout_state.borrow_mut();
            state.retain_live_embedded_document_services(|candidate| {
                self.child_browsing_context_host_for_document_handle(candidate)
                    .is_some()
            });
            state.retain_live_frame_viewports(|frame| self.child_browsing_context_is_live(frame));
            state.with_services_for_document(
                document,
                self.document_handle(),
                |services, embedded_document_services| {
                    crate::layout_renderer::build_native_layout_pass(
                        self,
                        root,
                        services,
                        embedded_document_services,
                        request,
                    )
                },
            )?
        };
        self.completed_layout_pass_count
            .set(self.completed_layout_pass_count.get().saturating_add(1));
        self.completed_layout_pass_time.set(
            self.completed_layout_pass_time
                .get()
                .saturating_add(pass.metrics.elapsed),
        );
        if moli_trace::cpu_profile_enabled() {
            let metrics = pass.metrics;
            tracing::info!(
                target: "moli_cpu_profile",
                stage = "layout_pass",
                reason = ?metrics.reason,
                total_us = metrics.elapsed.as_micros(),
                box_tree_us = metrics.box_tree_elapsed.as_micros(),
                list_marker_us = metrics.list_marker_elapsed.as_micros(),
                form_control_us = metrics.form_control_elapsed.as_micros(),
                inline_preparation_us = metrics.inline_preparation_elapsed.as_micros(),
                numeric_layout_us = metrics.numeric_layout_elapsed.as_micros(),
                numeric_first_pass_us = metrics.numeric_first_pass_elapsed.as_micros(),
                numeric_followup_passes_us = metrics.numeric_followup_passes_elapsed.as_micros(),
                overflow_detection_us = metrics.overflow_detection_elapsed.as_micros(),
                scrollbar_feedback_us = metrics.scrollbar_feedback_elapsed.as_micros(),
                embedded_frame_us = metrics.embedded_frame_elapsed.as_micros(),
                projection_us = metrics.projection_elapsed.as_micros(),
                numeric_layout_pass_count = metrics.numeric_layout_pass_count,
                numeric_feedback_invalidated_node_count = metrics
                    .numeric_feedback_invalidated_node_count,
                numeric_feedback_overflow_recomputed_node_count = metrics
                    .numeric_feedback_overflow_recomputed_node_count,
                box_count = metrics.box_count,
                fragment_count = metrics.fragment_count,
                paint_event_count = metrics.paint_event_count,
                paint_culled_event_count = metrics.paint_culled_event_count,
                paint_text_line_count = metrics.paint_text_line_count,
                paint_culled_text_line_count = metrics.paint_culled_text_line_count,
                paint_operation_count = metrics.paint_operation_count,
                fallback_count = metrics.fallback_count,
            );
        }
        pass.validate_retention_budget()?;
        let consumed = consume(&mut pass)?;
        let metrics = pass.metrics;
        let frame_viewports = self
            .child_browsing_context_handles_in_document_order()
            .into_iter()
            .filter(|frame| self.dom_host().owner_document_handle(*frame) == Some(document))
            .map(|frame| {
                let viewport = pass
                    .tree
                    .local_content_box_for_source(frame)
                    .map(|content| {
                        LayoutViewport::new(
                            css_viewport_dimension(f64::from(content.width)),
                            css_viewport_dimension(f64::from(content.height)),
                            pass_viewport.device_pixel_ratio,
                        )
                    });
                (frame, viewport)
            })
            .collect::<Vec<_>>();
        let tree = pass.into_tree();
        let frame_viewports_changed = {
            let mut state = self.document_layout_state.borrow_mut();
            state.publish_latest_layout(document, tree);
            state.update_frame_viewports(frame_viewports)
        };
        if frame_viewports_changed {
            self.style_viewport_generation
                .set(self.style_viewport_generation.get().saturating_add(1));
        }
        self.last_layout_pass_metrics.set(Some(metrics));
        self.layout_snapshot_cache_publishes
            .set(self.layout_snapshot_cache_publishes.get().saturating_add(1));
        Ok(Some(consumed))
    }

    pub(crate) fn answer_layout_for_document(
        &self,
        document: DomHandle,
        reason: LayoutFlushReason,
        queries: &LayoutQueryBatch<DomHandle>,
    ) -> Result<LayoutAnswers<DomHandle>, LayoutError> {
        let viewport = self.layout_viewport_for_document(document);
        self.answer_layout(document, reason, viewport, queries, false)
    }

    pub(crate) fn can_answer_layout_from_snapshot(&self, document: DomHandle) -> bool {
        #[cfg(test)]
        if self.force_fresh_layout_reads_for_test {
            return false;
        }
        self.with_latest_layout_tree_for_document(document, |_| ())
            .is_some()
    }

    /// Inspects the member tree for one exact Document in the single latest
    /// recursively frozen snapshot.
    ///
    /// The callback cannot retain the tree or force a refresh. Consumers
    /// such as lazy-image admission may combine this sampled geometry with
    /// cheap live browser state, but must tolerate the snapshot being absent
    /// or stale after DOM/style mutation.
    pub(crate) fn with_latest_layout_tree_for_document<T>(
        &self,
        document: DomHandle,
        inspect: impl FnOnce(&FrozenLayoutTree<DomHandle>) -> T,
    ) -> Option<T> {
        let root = self
            .dom_host()
            .dom()
            .document_element_handle_for_document(document);
        let state = self.document_layout_state.borrow();
        state
            .latest_layout(document)
            .or_else(|| root.and_then(|root| state.latest_layout_for_root(root)))
            .map(inspect)
    }

    pub(crate) fn answer_layout_at_viewport(
        &self,
        document: DomHandle,
        reason: LayoutFlushReason,
        viewport: LayoutViewport,
        queries: &LayoutQueryBatch<DomHandle>,
    ) -> Result<LayoutAnswers<DomHandle>, LayoutError> {
        self.answer_layout(document, reason, viewport, queries, true)
    }

    /// Ensures one exact-viewport tree exists without manufacturing a geometry
    /// query. Hit-test demands additionally require the complete embedded-frame
    /// projection owned by that same frozen tree.
    pub(crate) fn ensure_layout_at_viewport(
        &self,
        document: DomHandle,
        reason: LayoutFlushReason,
        viewport: LayoutViewport,
    ) -> Result<(), LayoutError> {
        #[cfg(test)]
        let reuse_latest = !self.force_fresh_layout_reads_for_test;
        #[cfg(not(test))]
        let reuse_latest = true;
        let cached = reuse_latest
            && self
                .with_latest_layout_tree_for_document(document, |tree| {
                    layout_tree_satisfies_request(tree, reason, viewport, true)
                })
                .unwrap_or(false);
        if cached {
            self.layout_snapshot_cache_hits
                .set(self.layout_snapshot_cache_hits.get().saturating_add(1));
            return Ok(());
        }

        self.layout_snapshot_cache_misses
            .set(self.layout_snapshot_cache_misses.get().saturating_add(1));
        self.with_fresh_layout_pass_for_document(
            document,
            LayoutPassRequest::new(viewport, reason),
            |_| Ok(()),
        )?
        .ok_or(LayoutError::NoLayoutRoot)
    }

    fn answer_layout(
        &self,
        document: DomHandle,
        reason: LayoutFlushReason,
        viewport: LayoutViewport,
        queries: &LayoutQueryBatch<DomHandle>,
        exact_viewport: bool,
    ) -> Result<LayoutAnswers<DomHandle>, LayoutError> {
        #[cfg(test)]
        let reuse_latest = !self.force_fresh_layout_reads_for_test;
        #[cfg(not(test))]
        let reuse_latest = true;
        let cached = if reuse_latest {
            self.with_latest_layout_tree_for_document(document, |tree| {
                if !layout_tree_satisfies_request(tree, reason, viewport, exact_viewport) {
                    return None;
                }
                self.last_layout_pass_metrics
                    .get()
                    .map(|metrics| self.answer_layout_queries(tree, metrics, viewport, queries))
            })
            .flatten()
        } else {
            None
        };
        if let Some(answers) = cached {
            self.layout_snapshot_cache_hits
                .set(self.layout_snapshot_cache_hits.get().saturating_add(1));
            return Ok(answers);
        }

        self.layout_snapshot_cache_misses
            .set(self.layout_snapshot_cache_misses.get().saturating_add(1));
        self.with_fresh_layout_pass_for_document(
            document,
            LayoutPassRequest::new(viewport, reason),
            |pass| Ok(self.answer_layout_queries(&pass.tree, pass.metrics, viewport, queries)),
        )?
        .ok_or(LayoutError::NoLayoutRoot)
    }

    fn answer_layout_queries(
        &self,
        tree: &FrozenLayoutTree<DomHandle>,
        metrics: moli_layout::LayoutPassMetrics,
        viewport: LayoutViewport,
        queries: &LayoutQueryBatch<DomHandle>,
    ) -> LayoutAnswers<DomHandle> {
        let mut answers = tree.answer_queries(queries, metrics);
        for (query, answer) in queries.queries.iter().zip(&mut answers.answers) {
            match (query, answer) {
                (LayoutQuery::DocumentMetrics, LayoutQueryAnswer::DocumentMetrics(metrics)) => {
                    // The content extent and scroll position are sampled
                    // geometry, but the viewport is explicit browser state.
                    // Window/viewport protocol commands must observe a resize
                    // immediately without forcing a new layout pass.
                    metrics.viewport = viewport;
                }
                (
                    LayoutQuery::ElementMetrics { source },
                    LayoutQueryAnswer::ElementMetrics(metrics),
                ) => {
                    *metrics = tree.element_metrics_for_source_with_offset_parent_filter(
                        *source,
                        |candidate| self.offset_parent_candidate_is_exposed(*source, candidate),
                    );
                }
                _ => {}
            }
        }
        answers
    }

    /// Blink exposes only offset-parent candidates whose TreeScope is one of
    /// the queried element's ancestor TreeScopes. This makes a slotted light
    /// child skip positioned wrappers inside the host's shadow tree while an
    /// element physically inside that shadow tree can still return them.
    fn offset_parent_candidate_is_exposed(&self, source: DomHandle, candidate: DomHandle) -> bool {
        let dom = self.dom_host();
        let candidate_scope = dom
            .containing_shadow_root(candidate)
            .or_else(|| dom.owner_document_handle(candidate));
        let Some(candidate_scope) = candidate_scope else {
            return false;
        };

        let mut scope_node = source;
        loop {
            let Some(scope) = dom
                .containing_shadow_root(scope_node)
                .or_else(|| dom.owner_document_handle(scope_node))
            else {
                return false;
            };
            if scope == candidate_scope {
                return true;
            }
            if !dom.is_shadow_root(scope) {
                return false;
            }
            let Some(host) = dom.shadow_root_host(scope) else {
                return false;
            };
            scope_node = host;
        }
    }

    pub(crate) fn reset_document_layout_state(&self) {
        *self.document_layout_state.borrow_mut() = Default::default();
        self.style_viewport_generation
            .set(self.style_viewport_generation.get().saturating_add(1));
    }

    pub(crate) fn invalidate_layout_after_interaction_state_change(&self) {
        self.clear_layout_rect_cache();
        self.document_layout_state
            .borrow_mut()
            .clear_latest_layout();
    }

    pub(crate) fn document_web_font_resources_are_current(
        &self,
        generation: crate::style_engine::StylesheetResourceGeneration,
    ) -> bool {
        self.document_layout_state
            .borrow()
            .web_font_resources_are_current(generation)
    }

    pub(crate) fn publish_document_web_font_resource_generation(
        &self,
        generation: crate::style_engine::StylesheetResourceGeneration,
    ) {
        self.document_layout_state
            .borrow_mut()
            .publish_web_font_resource_generation(generation);
    }

    #[cfg(test)]
    pub(crate) fn force_fresh_layout_reads_for_test(&mut self) {
        self.force_fresh_layout_reads_for_test = true;
        self.document_layout_state.get_mut().clear_latest_layout();
    }

    pub(crate) fn retain_document_web_font_slots<'a>(
        &self,
        resources: impl IntoIterator<Item = &'a StylesheetLoadBlockingResource>,
    ) {
        self.document_layout_state
            .borrow_mut()
            .retain_active_slots(resources);
    }

    pub(crate) fn admit_document_web_font(
        &self,
        resource: StylesheetLoadBlockingResource,
    ) -> Option<StylesheetLoadBlockingResource> {
        self.document_layout_state.borrow_mut().admit(resource)
    }

    pub(crate) fn complete_document_web_font(
        &self,
        terminal: CompletedStylesheetWebFont,
    ) -> DocumentWebFontCompletion {
        self.document_layout_state.borrow_mut().complete(terminal)
    }

    #[cfg(test)]
    pub(crate) fn document_web_font_counts_for_test(&self) -> (usize, usize, usize) {
        self.document_layout_state.borrow().web_font_counts()
    }

    #[cfg(test)]
    pub(crate) fn layout_pass_observability_for_test(
        &self,
    ) -> (
        bool,
        u64,
        std::time::Duration,
        Option<moli_layout::LayoutPassMetrics>,
    ) {
        (
            self.layout_pass_active.get(),
            self.completed_layout_pass_count.get(),
            self.completed_layout_pass_time.get(),
            self.last_layout_pass_metrics.get(),
        )
    }

    #[cfg(test)]
    pub(crate) fn layout_snapshot_cache_observability_for_test(
        &self,
    ) -> LayoutSnapshotCacheObservability {
        LayoutSnapshotCacheObservability {
            hits: self.layout_snapshot_cache_hits.get(),
            misses: self.layout_snapshot_cache_misses.get(),
            publishes: self.layout_snapshot_cache_publishes.get(),
            cached: self
                .document_layout_state
                .borrow()
                .latest_layout_observability(),
        }
    }
}

fn layout_tree_satisfies_request(
    tree: &FrozenLayoutTree<DomHandle>,
    reason: LayoutFlushReason,
    viewport: LayoutViewport,
    exact_viewport: bool,
) -> bool {
    (!exact_viewport || tree.viewport == viewport)
        && (!matches!(reason, LayoutFlushReason::HitTest) || tree.embedded_frames_complete())
}

impl GeometryProvider for JsContextHost {
    type NodeId = DomHandle;

    fn answer(
        &mut self,
        reason: LayoutFlushReason,
        viewport: LayoutViewport,
        queries: &LayoutQueryBatch<Self::NodeId>,
    ) -> Result<LayoutAnswers<Self::NodeId>, LayoutError> {
        let document = self.document_handle();
        self.answer_layout(document, reason, viewport, queries, false)
    }
}

fn css_viewport_dimension(value: f64) -> u32 {
    value.round().clamp(0.0, f64::from(u32::MAX)) as u32
}

#[cfg(test)]
mod tests {
    use super::ActiveLayoutPass;

    #[test]
    fn active_layout_scope_rejects_reentry_and_resets_on_drop() {
        let active = std::cell::Cell::new(false);
        let outer = ActiveLayoutPass::enter(&active).expect("first pass should enter");
        assert!(active.get());
        assert!(matches!(
            ActiveLayoutPass::enter(&active),
            Err(moli_layout::LayoutError::ReentrantLayoutPass)
        ));
        drop(outer);
        assert!(!active.get());
        drop(ActiveLayoutPass::enter(&active).expect("later pass should enter"));
        assert!(!active.get());
    }
}
