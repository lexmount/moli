use std::collections::{HashMap, HashSet};

use moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE;
#[cfg(feature = "devtools")]
use moli_layout::LayoutTransform2D;
use moli_layout::{
    FrozenLayoutTree, GeometryProvider, LayoutAnswers, LayoutError, LayoutPassRequest,
    LayoutPassResult, LayoutQuery, LayoutQueryAnswer, LayoutQueryBatch, LayoutRect, LayoutViewport,
    ScrollProjectionRefresh,
};
use moli_page_types::LayoutPolicy;
use moli_selector::QueryContainerSizes;
use style::servo::restyle_damage::ServoRestyleDamage;

#[cfg(feature = "devtools")]
use super::layout_snapshot::PublishedFrameGeometry;
use super::layout_state::InferredFrameStyleViewportCacheKey;
use super::{LayoutPublicationRequest, PageScriptHost};
use crate::{
    css_resource_urls::{
        CompletedStylesheetWebFont, StylesheetLoadBlockingResource, StylesheetSubresourceRequest,
    },
    document_runtime::DomHandle,
    layout_renderer::{
        FrameOwnerStyles, LayoutPassFeedback, layout_viewport_for_surface,
        screen_targets_allow_layout_reuse,
    },
    native_bridge::element::{ComputedStyleRead, iframe_handle_viewport},
    script_vm::web_fonts::DocumentWebFontCompletion,
    style_engine::{StyleViewport, StyloStyleEnvironment, layout_damage_for_style_change},
};

/// Invalidation required by a changed native input. Publication is a separate
/// owner decision; these changes retain the last frozen layout for readers.
#[derive(Clone, Copy)]
pub(crate) enum LayoutInputChange {
    /// Media and viewport changes invalidate computed styles through Stylo.
    StyleEnvironment,
    /// Scrollbar policy and interactions change future geometry/paint output.
    UsedGeometry,
}

/// Selects the layout input that a geometry consumer may sample.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutQueryMode {
    Observable,
    Published,
}

struct FrameLayoutPublication {
    // Full layout publication tracks the content boxes for viewport updates.
    // An absent entry leaves the prior viewport alone. A visited frame
    // without a box records None so publication can apply the layout policy.
    content_boxes: Option<HashMap<DomHandle, Option<LayoutRect>>>,
    #[cfg(feature = "devtools")]
    geometry: PublishedFrameGeometry,
}

impl FrameLayoutPublication {
    fn collect(
        &mut self,
        host: &PageScriptHost,
        document: DomHandle,
        tree: &FrozenLayoutTree<DomHandle>,
        frames_by_document: &HashMap<DomHandle, HashSet<DomHandle>>,
        #[cfg(feature = "devtools")] root_to_viewport: Option<LayoutTransform2D>,
    ) {
        let Some(frames) = frames_by_document.get(&document) else {
            return;
        };
        let owner_boxes: HashMap<_, _> = tree
            .boxes
            .iter()
            .filter_map(|layout_box| {
                let source = layout_box.principal_source?;
                frames.contains(&source).then_some((source, layout_box))
            })
            .collect();
        let embedded: HashMap<_, _> = tree
            .embedded_frames()
            .map(|frame| (frame.source, frame.tree.as_ref()))
            .collect();
        for &frame in frames {
            let owner_box = owner_boxes.get(&frame);
            if let Some(content_boxes) = &mut self.content_boxes {
                content_boxes.insert(frame, owner_box.map(|owner_box| owner_box.content_box));
            }
            let Some(child_document) = host.child_browsing_context_document_handle(frame) else {
                continue;
            };
            let child_tree = embedded.get(&frame).copied();
            if child_tree.is_some_and(|child| {
                host.dom_host()
                    .dom()
                    .document_element_handle_for_document(child_document)
                    != Some(child.source_root())
            }) {
                continue;
            }
            #[cfg(feature = "devtools")]
            let child_transform = root_to_viewport
                .zip(owner_box.and_then(|owner_box| tree.frame_viewport_transform(owner_box.id)))
                .map(|(parent, child)| child.concatenate(parent))
                .filter(|transform| transform.is_finite());
            #[cfg(feature = "devtools")]
            if let Some(transform) = child_transform
                && let Some(owner) = host.current_window_document_task_target_for_dispatch_scope(
                    super::OwnerDispatchScope::Child(frame),
                )
            {
                self.geometry.insert(child_document, owner, transform);
            }
            if let Some(child_tree) = child_tree {
                self.collect(
                    host,
                    child_document,
                    child_tree,
                    frames_by_document,
                    #[cfg(feature = "devtools")]
                    child_transform,
                );
            }
        }
    }
}

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

impl PageScriptHost {
    pub(crate) fn set_layout_policy(&mut self, policy: moli_page_types::LayoutPolicy) {
        if self.layout_policy == policy && policy.is_realtime() {
            return;
        }
        // Production configuration is sealed before Page creation. Internal
        // test/initialization setters must not discard a live frame ticket and
        // then reuse its local id on re-entry to the same Document.
        assert!(
            self.realtime_layout_state
                .as_ref()
                .is_none_or(|state| state.frame.borrow().is_idle()
                    && state.animation_frames.borrow().is_empty()
                    && state.font_readiness.borrow().is_idle()),
            "layout policy can change only at an idle realtime boundary"
        );
        if !policy.uses_real_layout() {
            self.document_layout_state
                .borrow_mut()
                .clear_latest_layout();
        }
        if self.layout_policy != policy {
            let mut state = self.document_layout_state.borrow_mut();
            drop(state.take_retained_layout());
            state.on_demand_frames = Default::default();
            self.dom_host().track_layout_changes(None);
            self.realtime_layout_state = policy.is_realtime().then(Box::default);
        }
        self.layout_policy = policy;
        self.initialize_realtime_document_fonts();
        self.request_realtime_rendering_opportunity();
    }

    pub(crate) const fn layout_policy(&self) -> moli_page_types::LayoutPolicy {
        self.layout_policy
    }

    pub(crate) fn set_scrollbars_hidden(&mut self, hidden: bool) -> bool {
        let changed = self.scrollbars_hidden != hidden;
        if changed {
            self.scrollbars_hidden = hidden;
            self.invalidate_layout_inputs(LayoutInputChange::UsedGeometry);
        }
        changed
    }

    pub(crate) fn invalidate_layout_inputs(&self, change: LayoutInputChange) {
        self.note_realtime_layout_input_change();
        match change {
            LayoutInputChange::StyleEnvironment => self
                .style_engine
                .bump_target_context_epoch_for_document(self.document_handle()),
            LayoutInputChange::UsedGeometry => self.invalidate_layout_projection(),
        }
    }

    pub(super) fn invalidate_layout_projection(&self) {
        self.clear_layout_rect_cache();
        self.document_layout_state
            .borrow_mut()
            .mark_visual_state_dirty();
    }

    pub(crate) const fn scrollbars_hidden(&self) -> bool {
        self.scrollbars_hidden
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

    #[cfg(feature = "devtools")]
    pub(crate) fn visual_state_token(
        &self,
        document: crate::runtime::RendererDocumentLifecycleIdentity,
        viewport: moli_layout::PaintViewport,
        base_background_color: [u8; 4],
    ) -> crate::runtime::RendererVisualStateToken {
        crate::runtime::RendererVisualStateToken::new(
            document,
            self.dom_host().dom_version(),
            self.style_engine
                .visual_style_input_generations(self.active_layout_document_handles()),
            self.visual_state_generation(),
            self.visual_resource_generation(),
            viewport,
            base_background_color,
        )
    }

    pub(crate) fn layout_document_for_source(&self, source: DomHandle) -> Option<DomHandle> {
        self.dom_host().owner_document_handle(source)
    }

    pub(crate) fn layout_viewport_for_document(&self, document: DomHandle) -> LayoutViewport {
        let surface = self.viewport_surface().unwrap_or_default();
        if document == self.document_handle() {
            return layout_viewport_for_surface(surface);
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
            surface.device_pixel_ratio as f32,
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

    pub(crate) fn observed_frame_viewport(&self, frame: DomHandle) -> Option<LayoutViewport> {
        let (viewport, needs_validation) = {
            let state = self.document_layout_state.try_borrow().ok()?;
            let changes = &state.on_demand_frames;
            (
                state.frame_viewport(frame)?,
                changes.needs_layout || !changes.changed_owners.is_empty(),
            )
        };
        if needs_validation && !self.layout_pass_active.get() {
            // Keep the last exact viewport until a new screen pass publishes
            // the frame's used content box.
            let _ = self.ensure_initial_layout();
            return self.frame_viewport(frame);
        }
        Some(viewport)
    }

    pub(super) fn note_on_demand_frame_style_change(&self, frame: DomHandle) {
        let host = self.dom_host();
        if self.layout_policy != LayoutPolicy::OnDemand
            || !host.is_html_element_named(frame, "iframe")
            || !host.is_connected(frame)
        {
            return;
        }
        let mut state = self.document_layout_state.borrow_mut();
        if state.on_demand_frames.needs_layout
            || (state.frame_viewport(frame).is_none()
                && state.latest_layout(self.document_handle()).is_none())
        {
            return;
        }
        state.on_demand_frames.changed_owners.insert(frame);
    }

    fn validate_on_demand_frame_layout(&self) {
        if self.layout_policy != LayoutPolicy::OnDemand || self.layout_pass_active.get() {
            return;
        }
        let changed_owners = std::mem::take(
            &mut self
                .document_layout_state
                .borrow_mut()
                .on_demand_frames
                .changed_owners,
        );
        let host = self.dom_host();
        let mut owner_documents = HashSet::new();
        let owner_needs_layout = changed_owners.into_iter().any(|frame| {
            let Some(document) = host
                .owner_document_handle(frame)
                .filter(|_| host.is_connected(frame))
            else {
                return true;
            };
            owner_documents.insert(document);
            let previous = self.document_layout_state.borrow().frame_owner_style(frame);
            let current = ComputedStyleRead::new(self, frame).computed_values();
            previous.zip(current).is_none_or(|(previous, current)| {
                let damage = ServoRestyleDamage::compute_style_difference::<
                    moli_selector::StyloElement<'_>,
                >(&previous, &current)
                .damage
                    | layout_damage_for_style_change(Some(&previous), Some(&current));
                !ServoRestyleDamage::REPAINT.contains(damage)
            })
        });
        if owner_needs_layout
            || owner_documents
                .iter()
                .any(|&document| !self.on_demand_restyles_allow_layout_reuse(document))
        {
            // Snapshot readers can still use the published projection until
            // the replacement pass succeeds.
            self.document_layout_state
                .borrow_mut()
                .on_demand_frames
                .needs_layout = true;
        } else {
            for document in owner_documents {
                self.style_engine.consume_screen_style_work(document);
            }
        }
    }

    fn on_demand_restyles_allow_layout_reuse(&self, document: DomHandle) -> bool {
        // An owner's attribute can restyle its containing block or siblings
        // without changing the owner's computed dimensions.
        let repaint = ServoRestyleDamage::REPAINT;
        let Some(restyles) =
            self.style_engine
                .screen_restyle_targets(self.dom_host(), document, repaint)
        else {
            return false;
        };
        (restyles.targets.is_empty()
            || screen_targets_allow_layout_reuse(
                self,
                document,
                restyles.targets,
                restyles.after,
                repaint,
            ))
            && self
                .style_engine
                .screen_style_damage_is_within(document, repaint)
    }

    pub(crate) fn invalidate_mock_frame_viewports(
        &self,
        frames: impl IntoIterator<Item = DomHandle>,
    ) {
        if self.layout_policy != LayoutPolicy::Mock {
            return;
        }
        let frames = frames.into_iter().collect::<Vec<_>>();
        if frames.is_empty() {
            return;
        }
        let changed = {
            let mut state = self.document_layout_state.borrow_mut();
            let changed = state.update_frame_viewports(
                frames.into_iter().map(|frame| (frame, None)),
                FrameOwnerStyles::new(),
            );
            state.clear_latest_layout();
            changed
        };
        if changed {
            self.style_viewport_generation
                .set(self.style_viewport_generation.get().saturating_add(1));
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn style_viewport_generation(&self) -> u64 {
        self.style_viewport_generation.get()
    }

    /// Marks the exact style environment selected by an active layout pass
    /// before that pass reads the Document's styles.
    ///
    /// An embedded Document's persistent frame viewport is published only
    /// after the recursive pass succeeds. Advancing this input generation
    /// first also distinguishes an explicit main-Document capture viewport
    /// or media from ordinary DOM APIs selecting conflicting implicit inputs.
    /// This observation epoch does not invalidate canonical screen geometry or
    /// schedule lifecycle work: the live source inputs have not changed.
    pub(crate) fn note_layout_style_environment(
        &self,
        document: DomHandle,
        viewport: StyleViewport,
        environment_override: Option<StyloStyleEnvironment>,
    ) {
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
        let different_media = environment_override.is_some_and(|environment| {
            environment != StyloStyleEnvironment::from_emulated_media(self.emulated_media())
        });
        if !matches_cached || different_media {
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

    /// Lazily publishes a missing screen layout for this main Document.
    ///
    /// Every consumer shares the same recursive tree. Missing nodes or newly
    /// navigated frames in an existing tree must not refresh the whole page.
    pub(crate) fn ensure_initial_layout(&self) -> Result<(), LayoutError> {
        if !self.layout_policy.uses_real_layout() {
            return Ok(());
        }
        if self.layout_pass_active.get() {
            return Err(LayoutError::ReentrantLayoutPass);
        }
        self.validate_on_demand_frame_layout();
        let document = self.document_handle();
        {
            let state = self.document_layout_state.borrow();
            if state.latest_layout(document).is_some() && !state.on_demand_frames.needs_layout {
                return Ok(());
            }
        }
        let request = LayoutPassRequest::new(
            self.layout_viewport_for_document(document),
            moli_layout::LayoutFlushReason::SynchronousGeometry,
        );
        let publication = self.prepare_layout_publication(
            document,
            request.viewport,
            LayoutPublicationRequest::Screen,
        )?;
        let (pass, feedback) = self
            .build_layout_pass_for_document_in_environment(
                document,
                publication.layout_request(request),
                None,
                publication.input_retention(self.layout_policy),
            )?
            .ok_or(LayoutError::NoLayoutRoot)?;
        self.finish_layout_publication(publication, pass, feedback)
    }

    /// FontFace and Canvas use the document's font service without requesting a
    /// layout pass or changing the frozen synchronous DOM geometry snapshot.
    pub(crate) fn with_font_services<T>(
        &self,
        document: DomHandle,
        consume: impl FnOnce(&mut moli_layout::DocumentLayoutServices) -> T,
    ) -> T {
        self.document_layout_state
            .borrow_mut()
            .with_services_for_document(document, self.document_handle(), |services, _| {
                consume(services)
            })
    }

    pub(crate) fn invalidate_document_font_layout(&self, document: DomHandle) {
        if document != self.document_handle()
            && self
                .child_browsing_context_host_for_document_handle(document)
                .is_none()
        {
            return;
        }
        // A real font registration change invalidates sampled glyph advances.
        // Unchanged registration and read-only projections retain the tree.
        self.note_realtime_layout_input_change();
        self.note_realtime_font_loading(document);
        self.queue_realtime_font_readiness(document);
        self.clear_layout_rect_cache();
        self.document_layout_state
            .borrow_mut()
            .clear_latest_layout();
    }

    pub(crate) fn replace_layout_container_sizes(
        &self,
        document: DomHandle,
        sizes: QueryContainerSizes,
    ) -> Option<QueryContainerSizes> {
        self.style_engine
            .replace_layout_container_sizes(self.dom_host(), document, sizes)
    }

    #[cfg(test)]
    pub(crate) fn build_layout_pass_for_document(
        &self,
        document: DomHandle,
        request: LayoutPassRequest,
    ) -> Result<Option<LayoutPassResult<DomHandle>>, LayoutError> {
        use crate::layout_renderer::LayoutInputRetention;
        let inputs = if self.layout_policy == moli_page_types::LayoutPolicy::Lazy {
            LayoutInputRetention::Frozen
        } else {
            LayoutInputRetention::Transient
        };
        self.build_layout_pass_for_document_in_environment(document, request, None, inputs)
            .map(|pass| pass.map(|(layout, _)| layout))
    }

    #[cfg(test)]
    pub(crate) fn has_retained_layout_for_test(&self) -> bool {
        self.document_layout_state.borrow().has_reusable_layout()
    }

    pub(crate) fn build_layout_pass_for_document_in_environment(
        &self,
        document: DomHandle,
        request: LayoutPassRequest,
        environment_override: Option<StyloStyleEnvironment>,
        inputs: crate::layout_renderer::LayoutInputRetention,
    ) -> Result<Option<(LayoutPassResult<DomHandle>, LayoutPassFeedback<'_>)>, LayoutError> {
        let Some(root) = self
            .dom_host()
            .dom()
            .document_element_handle_for_document(document)
        else {
            return Ok(None);
        };
        let _active = ActiveLayoutPass::enter(&self.layout_pass_active)?;
        let realtime_screen = inputs == crate::layout_renderer::LayoutInputRetention::Realtime
            && self.layout_policy.is_realtime()
            && document == self.document_handle()
            && environment_override.is_none();
        let request = if realtime_screen {
            request.with_retained_layout_world()
        } else {
            request
        };
        let (mut pass, feedback) = {
            let mut state = self.document_layout_state.borrow_mut();
            let retained = if realtime_screen {
                state.take_retained_layout()
            } else {
                None
            };
            state.retain_live_embedded_document_services(|candidate| {
                self.child_browsing_context_host_for_document_handle(candidate)
                    .is_some()
            });
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
                        environment_override,
                        inputs,
                        retained,
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
                retained_box_count = metrics.retained_box_count,
                pruned_source_count = metrics.pruned_source_count,
                retained_embedded_box_count = metrics.retained_embedded_box_count,
                retained_numeric_layout_hits = metrics.retained_numeric_layout_hits,
                retained_numeric_tree = metrics.retained_numeric_tree,
                reused_inline_contexts = metrics.reused_inline_contexts,
                reused_overflow_projection = metrics.reused_overflow_projection,
                fragment_count = metrics.fragment_count,
                paint_event_count = metrics.paint_event_count,
                paint_culled_event_count = metrics.paint_culled_event_count,
                paint_text_line_count = metrics.paint_text_line_count,
                paint_culled_text_line_count = metrics.paint_culled_text_line_count,
                paint_operation_count = metrics.paint_operation_count,
                fallback_count = metrics.fallback_count,
            );
        }
        pass.tree.validate_retention_budget_with_svg_fallback(
            crate::layout_renderer::decode_frozen_svg_geometry,
        )?;
        Ok(Some((pass, feedback)))
    }

    pub(super) fn publish_layout_pass_for_document(
        &self,
        document: DomHandle,
        mut pass: LayoutPassResult<DomHandle>,
        retain_paint: bool,
        frame_owner_styles: FrameOwnerStyles,
    ) {
        if let Some(state) = &self.realtime_layout_state {
            state.clear_publication();
        }
        let metrics = pass.metrics;
        let pass_viewport = pass.viewport;
        let live_frames = self.child_browsing_context_handles_in_document_order();
        let frames =
            self.collect_frame_layout_publication(document, &pass.tree, &live_frames, true);
        let content_boxes = frames
            .content_boxes
            .expect("full layout publication collects frame content boxes");
        let frame_viewports = live_frames
            .into_iter()
            .filter_map(|frame| {
                let content = content_boxes.get(&frame);
                if content.is_none() && !self.layout_policy.is_realtime() {
                    return None;
                }
                let viewport = content.and_then(|content| *content).map(|content| {
                    LayoutViewport::new(
                        css_viewport_dimension(f64::from(content.width)),
                        css_viewport_dimension(f64::from(content.height)),
                        pass_viewport.device_pixel_ratio,
                    )
                });
                let viewport = if self.layout_policy.is_realtime() {
                    // Hiding a live frame removes its geometry, not its last
                    // Window viewport. Initially hidden frames start at zero.
                    viewport.or_else(|| {
                        Some(self.frame_viewport(frame).unwrap_or(LayoutViewport::new(
                            0,
                            0,
                            pass_viewport.device_pixel_ratio,
                        )))
                    })
                } else {
                    viewport
                };
                Some((frame, viewport))
            })
            .collect::<Vec<_>>();
        let sampled_viewports = if self.layout_policy.is_realtime() {
            frame_viewports
                .iter()
                .filter_map(|(frame, viewport)| {
                    let viewport = viewport.as_ref()?;
                    let target = self.current_window_document_task_target_for_dispatch_scope(
                        super::OwnerDispatchScope::Child(*frame),
                    )?;
                    Some((target, (viewport.css_width, viewport.css_height)))
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let retained = pass.take_retained_layout();
        let tree = pass.into_tree();
        #[cfg(feature = "devtools")]
        self.note_wheel_scroll_target_layout(document, &tree);
        let frame_viewports_changed = {
            let mut state = self.document_layout_state.borrow_mut();
            if document == self.document_handle() {
                state.on_demand_frames = Default::default();
            }
            state.retain_live_frame_viewports(|frame| self.child_browsing_context_is_live(frame));
            state.update_frame_viewports(frame_viewports, frame_owner_styles)
        };
        if frame_viewports_changed {
            self.style_viewport_generation
                .set(self.style_viewport_generation.get().saturating_add(1));
        }
        self.document_layout_state
            .borrow_mut()
            .publish_latest_layout(
                document,
                tree,
                retained,
                retain_paint,
                #[cfg(feature = "devtools")]
                frames.geometry,
            );
        for (target, size) in sampled_viewports {
            self.sample_realtime_frame_viewport(target, size);
        }
        self.last_layout_pass_metrics.set(Some(metrics));
        self.layout_snapshot_cache_publishes
            .set(self.layout_snapshot_cache_publishes.get().saturating_add(1));
    }

    fn collect_frame_layout_publication(
        &self,
        document: DomHandle,
        tree: &FrozenLayoutTree<DomHandle>,
        live_frames: &[DomHandle],
        collect_content_boxes: bool,
    ) -> FrameLayoutPublication {
        let mut frames_by_document: HashMap<_, HashSet<_>> = HashMap::new();
        for &frame in live_frames {
            if let Some(parent) = self.dom_host().owner_document_handle(frame) {
                frames_by_document.entry(parent).or_default().insert(frame);
            }
        }
        let mut frames = FrameLayoutPublication {
            content_boxes: collect_content_boxes.then(HashMap::new),
            #[cfg(feature = "devtools")]
            geometry: PublishedFrameGeometry::new(
                self.current_window_document_task_target_for_dispatch_scope(
                    super::OwnerDispatchScope::Top,
                ),
            ),
        };
        frames.collect(
            self,
            document,
            tree,
            &frames_by_document,
            #[cfg(feature = "devtools")]
            (document == self.document_handle()).then_some(LayoutTransform2D::IDENTITY),
        );
        frames
    }

    pub(super) fn refresh_published_scroll_projection(
        &self,
    ) -> Result<ScrollProjectionRefresh, LayoutError> {
        let document = self.document_handle();
        self.document_layout_state
            .borrow_mut()
            .refresh_scroll_projection(
                document,
                |source| crate::layout_renderer::native_scroll_offset(self.dom_host(), source),
                #[cfg(feature = "devtools")]
                |tree| {
                    let frames = self.child_browsing_context_handles_in_document_order();
                    self.collect_frame_layout_publication(document, tree, &frames, false)
                        .geometry
                },
            )
    }

    #[cfg(feature = "devtools")]
    pub(crate) fn published_frame_geometry(&self) -> Option<std::rc::Rc<PublishedFrameGeometry>> {
        self.document_layout_state
            .borrow()
            .frame_geometry()
            .cloned()
    }

    #[cfg(feature = "devtools")]
    pub(crate) fn input_frame_transform(
        &self,
        document: DomHandle,
        fallback_sample: Option<&PublishedFrameGeometry>,
    ) -> Option<LayoutTransform2D> {
        if document == self.document_handle() || !self.layout_policy().uses_real_layout() {
            return Some(LayoutTransform2D::IDENTITY);
        }
        let state = self.document_layout_state.borrow();
        // A current publication is authoritative even when the frame has no transform.
        let publication = state
            .frame_geometry()
            .map(std::rc::Rc::as_ref)
            .or(fallback_sample)?;
        publication.root_to_viewport(self, document)
    }

    pub(crate) fn answer_layout_for_document(
        &self,
        document: DomHandle,
        queries: &LayoutQueryBatch<DomHandle>,
        mode: LayoutQueryMode,
    ) -> Result<LayoutAnswers<DomHandle>, LayoutError> {
        self.with_layout_answers(document, &queries.queries, mode, |tree, metrics| {
            tree.answer_queries_with(queries, metrics, |query| {
                self.specialize_layout_query(tree, query)
            })
        })
    }

    pub(crate) fn answer_layout_query_for_document(
        &self,
        document: DomHandle,
        query: &LayoutQuery<DomHandle>,
        mode: LayoutQueryMode,
    ) -> Result<LayoutQueryAnswer<DomHandle>, LayoutError> {
        self.with_layout_answers(document, std::slice::from_ref(query), mode, |tree, _| {
            self.answer_layout_query(tree, query)
        })
    }

    fn with_layout_answers<T>(
        &self,
        document: DomHandle,
        queries: &[LayoutQuery<DomHandle>],
        mode: LayoutQueryMode,
        inspect: impl FnOnce(&FrozenLayoutTree<DomHandle>, moli_layout::LayoutPassMetrics) -> T,
    ) -> Result<T, LayoutError> {
        if mode == LayoutQueryMode::Observable {
            self.ensure_observable_layout()?;
        }
        // A snapshot-only reader can hydrate frozen inputs even when the live
        // DOM has changed. Realtime sampling requires a current publication.
        if self.layout_policy.uses_real_layout()
            && (!self.layout_policy.is_realtime() || mode == LayoutQueryMode::Observable)
        {
            self.prepare_svg_query_geometry(document, queries)?;
        }
        let value = self
            .with_latest_layout_tree_for_document(document, |tree| {
                self.last_layout_pass_metrics
                    .get()
                    .map(|metrics| inspect(tree, metrics))
            })
            .flatten();
        if let Some(value) = value {
            self.layout_snapshot_cache_hits
                .set(self.layout_snapshot_cache_hits.get().saturating_add(1));
            Ok(value)
        } else {
            self.layout_snapshot_cache_misses
                .set(self.layout_snapshot_cache_misses.get().saturating_add(1));
            Err(LayoutError::NoLayoutSnapshot)
        }
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
            .filter(|tree| Some(tree.source_root()) == root)
            .map(inspect)
    }

    fn prepare_svg_query_geometry(
        &self,
        document: DomHandle,
        queries: &[LayoutQuery<DomHandle>],
    ) -> Result<(), LayoutError> {
        let mut sources = queries
            .iter()
            .filter_map(|query| match *query {
                LayoutQuery::ClientRects { source }
                | LayoutQuery::ContentQuads { source }
                | LayoutQuery::ScrollIntoViewGeometry { source } => Some(source),
                LayoutQuery::IntersectionGeometry { target, .. } => Some(target),
                _ => None,
            })
            .peekable();
        if sources.peek().is_none() {
            return Ok(());
        }

        let prepared = if self.layout_policy.is_realtime() {
            let Some((root, viewport, pending)) = self
                .with_latest_layout_tree_for_document(document, |tree| {
                    let pending = crate::layout_renderer::pending_svg_geometry(self, tree, sources);
                    (!pending.is_empty()).then_some((tree.source_root(), tree.viewport, pending))
                })
                .flatten()
            else {
                return Ok(());
            };
            // Live style sampling runs outside the layout-state borrow.
            Some((
                root,
                crate::layout_renderer::prepare_svg_geometry(self, document, viewport, pending),
            ))
        } else {
            self.with_latest_layout_tree_for_document(document, |tree| {
                let mut seen = std::collections::HashSet::new();
                let geometry = sources
                    .filter_map(|source| tree.pending_frozen_svg_input(source))
                    .filter(|(root, _, _)| seen.insert(*root))
                    .map(|(root, size, input)| {
                        (
                            root,
                            crate::layout_renderer::decode_frozen_svg_geometry(root, input, size),
                        )
                    })
                    .collect();
                (tree.source_root(), geometry)
            })
        };
        let Some((root, geometry)) = prepared else {
            return Ok(());
        };
        if geometry.is_empty() {
            return Ok(());
        }
        self.document_layout_state
            .borrow_mut()
            .append_svg_geometry(root, geometry)
    }

    fn answer_layout_query(
        &self,
        tree: &FrozenLayoutTree<DomHandle>,
        query: &LayoutQuery<DomHandle>,
    ) -> LayoutQueryAnswer<DomHandle> {
        self.specialize_layout_query(tree, query)
            .unwrap_or_else(|| tree.answer_query(query))
    }

    fn specialize_layout_query(
        &self,
        tree: &FrozenLayoutTree<DomHandle>,
        query: &LayoutQuery<DomHandle>,
    ) -> Option<LayoutQueryAnswer<DomHandle>> {
        // Geometry and its coordinate environment come from the same
        // published frame, even after live viewport changes.
        Some(match query {
            LayoutQuery::ElementMetrics { source } => LayoutQueryAnswer::ElementMetrics(
                tree.element_metrics_for_source_with_offset_parent_filter(*source, |candidate| {
                    self.offset_parent_candidate_is_exposed(*source, candidate)
                }),
            ),
            // An out-of-viewport point needs no fragment walk.
            LayoutQuery::HitTest { point, .. } if !tree.viewport.contains(*point) => {
                LayoutQueryAnswer::HitTest(None)
            }
            LayoutQuery::HitTestAll { point, .. } if !tree.viewport.contains(*point) => {
                LayoutQueryAnswer::HitTestAll(Vec::new())
            }
            LayoutQuery::CaretPosition { point } if !tree.viewport.contains(*point) => {
                LayoutQueryAnswer::CaretPosition(None)
            }
            _ => return None,
        })
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
        if let Some(state) = &self.realtime_layout_state {
            state.reset();
        }
        self.prune_realtime_animation_frames();
        self.prune_media_queries();
        self.prune_realtime_environment();
        self.prune_realtime_rendering_events();
        if self.layout_policy().is_realtime() {
            crate::observer_runtime::prune_realtime_resize_observers(self);
        }
        self.style_viewport_generation
            .set(self.style_viewport_generation.get().saturating_add(1));
        self.request_realtime_rendering_opportunity();
    }

    pub(crate) fn invalidate_layout_after_interaction_state_change(&self) {
        self.invalidate_layout_inputs(LayoutInputChange::UsedGeometry);
    }

    pub(crate) fn document_web_font_resources_are_current(
        &self,
        document: DomHandle,
        generation: crate::style_engine::StylesheetResourceGeneration,
    ) -> bool {
        self.document_layout_state
            .borrow()
            .web_font_resources_are_current(document, self.document_handle(), generation)
    }

    pub(crate) fn document_web_font_sidecar_is_pristine(&self, document: DomHandle) -> bool {
        self.document_layout_state
            .borrow()
            .web_font_sidecar_is_pristine(document, self.document_handle())
    }

    pub(crate) fn publish_document_web_font_resource_generation(
        &self,
        document: DomHandle,
        generation: crate::style_engine::StylesheetResourceGeneration,
    ) {
        self.document_layout_state
            .borrow_mut()
            .publish_web_font_resource_generation(document, self.document_handle(), generation);
    }

    pub(crate) fn retain_document_web_font_slots<'a>(
        &self,
        document: DomHandle,
        resources: impl IntoIterator<Item = &'a StylesheetLoadBlockingResource>,
    ) {
        let changed = self.document_layout_state.borrow_mut().retain_active_slots(
            document,
            self.document_handle(),
            resources,
        );
        // Realtime passes select active CSS fonts from current styles. Retiring
        // obsolete registrations is cleanup, not another layout source change.
        if changed && !self.layout_policy().is_realtime() {
            self.invalidate_document_font_layout(document);
        }
    }

    pub(crate) fn admit_document_web_font(
        &self,
        document: DomHandle,
        resource: StylesheetLoadBlockingResource,
    ) -> Option<StylesheetSubresourceRequest> {
        let prepared = self.document_layout_state.borrow_mut().admit(
            document,
            self.document_handle(),
            resource,
        );
        if prepared.fonts_changed
            && (!self.layout_policy().is_realtime() || prepared.font_inputs_changed)
        {
            self.invalidate_document_font_layout(document);
        }
        if prepared.request.is_some() {
            self.note_realtime_font_loading(document);
        }
        prepared.request
    }

    pub(crate) fn complete_document_web_font(
        &self,
        document: DomHandle,
        terminal: CompletedStylesheetWebFont,
    ) -> DocumentWebFontCompletion {
        let completion = self.document_layout_state.borrow_mut().complete(
            document,
            self.document_handle(),
            terminal,
        );
        if let DocumentWebFontCompletion::Registered(outcome) = &completion {
            let changed = *outcome != moli_layout::WebFontRegistrationOutcome::Unchanged;
            if changed {
                self.note_realtime_layout_input_change();
            }
            // Geometry sampled while the request was pending contains fallback
            // glyph advances. Retire that tree before fonts.ready observers run.
            // Preserve the legacy policy; identical Realtime registrations
            // leave its valid sample reusable.
            if changed || !self.layout_policy().is_realtime() {
                self.document_layout_state
                    .borrow_mut()
                    .clear_latest_layout();
            }
        }
        if !matches!(completion, DocumentWebFontCompletion::Stale) {
            self.queue_realtime_font_readiness(document);
        }
        completion
    }

    pub(crate) fn document_web_fonts_have_pending_requests(&self, document: DomHandle) -> bool {
        self.document_layout_state
            .borrow()
            .web_fonts_have_pending_requests(document, self.document_handle())
    }

    pub(crate) fn stylesheet_web_font_document(
        &self,
        binding: crate::frame_owner_model::StylesheetSubresourceLoadDelayBinding,
    ) -> Option<DomHandle> {
        if !self.stylesheet_subresource_load_delay_is_current(binding) {
            return None;
        }
        match binding.child_handle() {
            Some(child) => self.child_browsing_context_document_handle(child),
            None => Some(self.document_handle()),
        }
    }

    pub(super) fn retire_embedded_document_fonts(&self, document: DomHandle) {
        self.document_layout_state
            .borrow_mut()
            .retire_embedded_document_fonts(document);
    }

    pub(crate) fn observe_document_font_face(
        &self,
        document: DomHandle,
        font: &crate::css_resource_urls::StylesheetWebFont,
    ) -> crate::font_loading::FontFaceLoad {
        self.document_layout_state
            .borrow_mut()
            .observe_font(document, self.document_handle(), font)
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
    pub(crate) fn register_layout_test_font(&self, family: &str, bytes: Vec<u8>) {
        let document = self.document_handle();
        self.document_layout_state
            .borrow_mut()
            .with_services_for_document(document, document, |services, _| {
                services
                    .register_web_font(moli_layout::WebFontRegistration::new(
                        "layout-test-font",
                        moli_layout::WebFontFace::new(family),
                        bytes,
                    ))
                    .expect("valid layout test font");
            });
    }

    #[cfg(test)]
    pub(crate) fn simulate_active_layout_pass_for_test(&self, active: bool) {
        self.layout_pass_active.set(active);
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

impl GeometryProvider for PageScriptHost {
    type NodeId = DomHandle;

    fn answer(
        &mut self,
        queries: &LayoutQueryBatch<Self::NodeId>,
    ) -> Result<LayoutAnswers<Self::NodeId>, LayoutError> {
        let document = self.document_handle();
        self.answer_layout_for_document(document, queries, LayoutQueryMode::Published)
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
