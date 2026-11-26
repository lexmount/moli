use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use moli_layout::{
    LayoutCssImageReference, LayoutError, LayoutFlushReason, LayoutPassRequest, LayoutPassResult,
    LayoutSourceChange, LayoutViewport, ScrollProjectionRefresh,
};
use style::servo::restyle_damage::ServoRestyleDamage;

use super::realtime_animation_frames::RealtimeAnimationFrames;
use super::realtime_environment::RealtimeEnvironmentState;
use super::realtime_events::RealtimeRenderingEvents;
use super::realtime_rendering::RealtimeFrameState;
#[cfg(feature = "devtools")]
use super::screencast_paint::{RendererScreencastPaintSession, ScreenPaintConsumers};
use super::{OwnerDispatchScope, PageScriptHost, WindowDocumentTaskTarget};
use crate::{
    document_runtime::DomHandle,
    layout_renderer::{LayoutPassFeedback, screen_targets_allow_layout_reuse},
    style_engine::{ScreenStyleCoverage, StyleInvalidationScope, StyloStyleEnvironment},
};

/// The consumer explicitly decides whether a pass may update screen geometry.
/// Trigger reasons remain diagnostic and do not select publication behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LayoutPublicationRequest {
    Screen,
    Transient,
}

pub(crate) struct PreparedLayoutPublication(LayoutPublication);

enum LayoutPublication {
    LegacyLatest(DomHandle),
    CurrentScreen {
        document: DomHandle,
        inputs: ScreenLayoutInputs,
        source_dom_version: u64,
        retain_paint: bool,
    },
    Transient,
}

impl PreparedLayoutPublication {
    pub(crate) fn publishes_screen(&self) -> bool {
        !matches!(self.0, LayoutPublication::Transient)
    }

    pub(crate) fn publishes_current_screen(&self) -> bool {
        matches!(self.0, LayoutPublication::CurrentScreen { .. })
    }

    pub(crate) fn layout_request(&self, request: LayoutPassRequest) -> LayoutPassRequest {
        if let LayoutPublication::CurrentScreen { retain_paint, .. } = &self.0 {
            request
                .with_frame_coverage(moli_layout::LayoutFrameCoverage::AllLayoutBoxes)
                .with_scroll_projection(true)
                .with_screen_paint(*retain_paint)
        } else {
            request
        }
    }
}

/// The validity and lifecycle state for one Realtime screen publication.
/// Screen paint consumers request optional data on the document's publication.
pub(super) struct RealtimeLayoutState {
    non_dom_revision: Cell<Option<u64>>,
    scroll_projection_dirty: Cell<bool>,
    published: RefCell<Option<ScreenLayoutInputs>>,
    css_images: RefCell<Rc<[LayoutCssImageReference<DomHandle>]>>,
    css_images_need_preparation: Cell<bool>,
    #[cfg(feature = "devtools")]
    screen_paint_consumers: ScreenPaintConsumers,
    pub(super) frame: RefCell<RealtimeFrameState>,
    pub(super) animation_frames: RefCell<RealtimeAnimationFrames>,
    pub(super) environment: RefCell<RealtimeEnvironmentState>,
    pub(super) rendering_events: RefCell<RealtimeRenderingEvents>,
    pub(super) font_readiness: RefCell<super::font_loading::RealtimeFontReadiness>,
}

impl Default for RealtimeLayoutState {
    fn default() -> Self {
        Self {
            non_dom_revision: Cell::new(Some(0)),
            scroll_projection_dirty: Cell::new(false),
            published: RefCell::new(None),
            css_images: RefCell::default(),
            css_images_need_preparation: Cell::new(false),
            #[cfg(feature = "devtools")]
            screen_paint_consumers: ScreenPaintConsumers::default(),
            frame: RefCell::new(RealtimeFrameState::default()),
            animation_frames: RefCell::new(RealtimeAnimationFrames::default()),
            environment: RefCell::new(RealtimeEnvironmentState::default()),
            rendering_events: RefCell::new(RealtimeRenderingEvents::default()),
            font_readiness: RefCell::default(),
        }
    }
}

impl RealtimeLayoutState {
    pub(super) fn invalidate(&self) {
        // Exhaustion fails closed. Saturation or wraparound must not make a
        // later source compare equal to an earlier publication.
        self.non_dom_revision.set(
            self.non_dom_revision
                .get()
                .and_then(|value| value.checked_add(1)),
        );
    }

    fn has_screen_paint_consumers(&self) -> bool {
        #[cfg(feature = "devtools")]
        return self.screen_paint_consumers.is_active();
        #[cfg(not(feature = "devtools"))]
        false
    }

    pub(super) fn clear_publication(&self) {
        self.published.borrow_mut().take();
        self.css_images.take();
        self.css_images_need_preparation.set(false);
    }

    pub(super) fn reset(&self) {
        self.clear_publication();
        self.non_dom_revision.set(Some(0));
        self.scroll_projection_dirty.set(false);
        self.frame.borrow_mut().retire_document();
    }
}

#[derive(Debug, PartialEq)]
struct DocumentLayoutInputs {
    document: DomHandle,
    root: Option<DomHandle>,
    target: Option<WindowDocumentTaskTarget>,
    dom_revision: u64,
    stylesheet_sources: u64,
}

#[derive(Debug, PartialEq)]
struct ScreenLayoutInputs {
    non_dom: u64,
    viewport: LayoutViewport,
    environment: StyloStyleEnvironment,
    scrollbars_hidden: bool,
    // Membership and exact owners are inputs. Child viewports are results of
    // the parent pass and must not invalidate their own successful publication.
    documents: Vec<DocumentLayoutInputs>,
}

impl ScreenLayoutInputs {
    fn permits_reuse(&self) -> bool {
        // Existing source counters saturate. At exhaustion, rebuilding is
        // preferable to falsely treating different inputs as equal.
        self.documents.iter().all(|document| {
            document.dom_revision != u64::MAX && document.stylesheet_sources != u64::MAX
        })
    }
}

impl PageScriptHost {
    /// Capture the inputs before computing a fresh pass. A temporary viewport
    /// cannot replace Realtime's canonical screen tree or child viewports.
    pub(crate) fn prepare_layout_publication(
        &self,
        document: DomHandle,
        viewport: LayoutViewport,
        request: LayoutPublicationRequest,
    ) -> Result<PreparedLayoutPublication, LayoutError> {
        if request == LayoutPublicationRequest::Transient {
            return Ok(PreparedLayoutPublication(LayoutPublication::Transient));
        }
        if !self.layout_policy.is_realtime() {
            return Ok(PreparedLayoutPublication(LayoutPublication::LegacyLatest(
                document,
            )));
        }
        if document != self.document_handle()
            || viewport != self.layout_viewport_for_document(document)
        {
            return Ok(PreparedLayoutPublication(LayoutPublication::Transient));
        }
        if self.layout_pass_active.get() {
            return Err(LayoutError::ReentrantLayoutPass);
        }
        let state = self
            .realtime_layout_state
            .as_ref()
            .expect("Realtime policy installs its document state");
        let inputs = self.screen_layout_inputs(state)?;
        self.prepare_current_screen_styles(&inputs);
        Ok(PreparedLayoutPublication(
            LayoutPublication::CurrentScreen {
                document,
                inputs,
                source_dom_version: self.dom_host().dom_version(),
                retain_paint: state.has_screen_paint_consumers(),
            },
        ))
    }

    pub(crate) fn finish_layout_publication(
        &self,
        publication: PreparedLayoutPublication,
        mut pass: LayoutPassResult<DomHandle>,
        feedback: LayoutPassFeedback<'_>,
    ) -> Result<(), LayoutError> {
        match publication.0 {
            LayoutPublication::LegacyLatest(document) => {
                self.publish_layout_pass_for_document(document, pass, false);
            }
            LayoutPublication::CurrentScreen {
                document,
                inputs,
                source_dom_version,
                retain_paint,
            } => {
                let state = self.realtime_layout_state.as_ref().ok_or_else(|| {
                    LayoutError::source_contract(
                        "screen layout",
                        "layout policy changed during pass",
                    )
                })?;
                if pass.viewport != inputs.viewport
                    || self.dom_host().dom_version() != source_dom_version
                    || self.screen_layout_inputs(state)? != inputs
                {
                    return Err(LayoutError::source_contract(
                        "screen layout",
                        "source inputs or Document owners changed during layout",
                    ));
                }
                let images = pass.take_css_image_references();
                self.publish_layout_pass_for_document(
                    document,
                    pass,
                    retain_paint && state.has_screen_paint_consumers(),
                );
                state.css_images_need_preparation.set(!images.is_empty());
                *state.css_images.borrow_mut() = images.into();
                self.did_publish_screen_layout(state, inputs, feedback.commit());
            }
            LayoutPublication::Transient => {}
        }
        Ok(())
    }

    pub(crate) fn take_realtime_css_image_preparation(
        &self,
    ) -> Option<Rc<[LayoutCssImageReference<DomHandle>]>> {
        let state = self.realtime_layout_state.as_ref()?;
        (state.css_images_need_preparation.get()
            && crate::document_runtime::DocumentRuntime::css_image_fetch_enabled(self))
        .then(|| {
            state.css_images_need_preparation.set(false);
            state.css_images.borrow().clone()
        })
    }

    pub(crate) fn begin_screen_style_layout(&self, document: DomHandle) {
        self.style_engine.begin_screen_layout(document);
    }

    fn did_publish_screen_layout(
        &self,
        state: &RealtimeLayoutState,
        inputs: ScreenLayoutInputs,
        mut coverage: HashMap<DomHandle, ScreenStyleCoverage>,
    ) {
        if moli_trace::cpu_profile_enabled() {
            tracing::info!(
                target: "moli_cpu_profile",
                stage = "screen_style_publication",
                documents = coverage.len(),
                lazy_pseudo_owners = coverage
                    .values()
                    .map(|document| document.lazy_pseudo_owners.len())
                    .sum::<usize>(),
            );
        }
        for document in &inputs.documents {
            self.style_engine.publish_screen_styles(
                document.document,
                coverage.remove(&document.document).unwrap_or_default(),
            );
        }
        // Attach the verified inputs after the geometry update retires the previous validity.
        self.dom_host().track_layout_changes(
            self.document_layout_state
                .borrow()
                .has_reusable_layout()
                .then(|| self.document_handle()),
        );
        *state.published.borrow_mut() = Some(inputs);
        state.scroll_projection_dirty.set(false);
        crate::observer_runtime::did_publish_realtime_resize_observer_layout(self);
        self.did_publish_realtime_font_layout();
    }

    #[cfg(all(test, feature = "devtools"))]
    pub(crate) fn has_screencast_paint_for_test(&self) -> bool {
        self.document_layout_state
            .borrow()
            .has_retained_paint_for_test()
    }

    #[cfg(feature = "devtools")]
    pub(crate) fn paint_current_screen_layout(
        &self,
        session: &RendererScreencastPaintSession,
        viewport: LayoutViewport,
        capture: moli_layout::PaintCaptureRequest,
        reason: LayoutFlushReason,
    ) -> Result<Option<moli_layout::PaintSnapshot>, LayoutError> {
        let Some(state) = &self.realtime_layout_state else {
            return Ok(None);
        };
        let document = self.document_handle();
        if viewport != self.layout_viewport_for_document(document) {
            return Ok(None);
        }
        state
            .screen_paint_consumers
            .attach(session, &self.document_layout_state);
        self.ensure_current_screen_layout(reason)?;
        let layout = self.document_layout_state.borrow();
        let Some((tree, retained)) = layout.retained_screen_paint(document) else {
            return Ok(None);
        };
        let images = self.css_image_paint_snapshot();
        let css_image = |root, url: &str| {
            self.layout_document_for_source(root)
                .map_or_else(moli_layout::LayoutCssImageState::default, |document| {
                    images.image(document, url)
                })
        };
        let replaced_image =
            |source, kind| crate::layout_renderer::native_replaced_image(self, source, kind);
        retained
            .paint(tree, capture, &css_image, &replaced_image)
            .map(Some)
    }

    #[cfg(test)]
    pub(crate) fn realtime_layout_allocated_for_test(&self) -> bool {
        self.realtime_layout_state.is_some()
    }

    /// Called at native source commits, before any callback can observe them.
    /// Legacy modes do not allocate state, advance revisions, or enqueue work.
    pub(crate) fn note_realtime_layout_input_change(&self) {
        self.dom_host().track_layout_changes(None);
        self.note_realtime_tracked_input_change();
    }

    pub(super) fn note_realtime_tracked_input_change(&self) {
        if let Some(state) = &self.realtime_layout_state {
            state.invalidate();
            self.request_realtime_rendering_opportunity();
        }
    }

    pub(super) fn note_selection_paint_change(&self) {
        self.dom_host().track_layout_changes(None);
        #[cfg(feature = "devtools")]
        if self.realtime_layout_state.is_some() {
            let mut layout = self.document_layout_state.borrow_mut();
            layout.discard_screen_paint();
            layout.mark_visual_state_dirty();
        }
    }

    /// Scrolling changes the projection and lifecycle demand, but leaves the
    /// dimensions that were computed by the last layout pass intact.
    pub(crate) fn note_realtime_scroll_change(&self, target: Option<DomHandle>) {
        if let Some(state) = &self.realtime_layout_state {
            if let Some(target) = target {
                self.dom_host().note_tracked_layout_target(target);
            }
            state.scroll_projection_dirty.set(true);
            self.invalidate_layout_projection();
            self.request_realtime_rendering_opportunity();
        }
    }

    pub(crate) fn retained_box_tree_changes(
        &self,
        document: DomHandle,
    ) -> Option<Vec<LayoutSourceChange<DomHandle>>> {
        let host = self.dom_host();
        let mut changes: Vec<_> = host
            .tracked_layout_targets(document)?
            .into_iter()
            .map(LayoutSourceChange::Node)
            .collect();
        changes.extend(
            self.style_engine
                .screen_layout_style_targets(host, document)?
                .into_iter()
                .map(|(target, scope)| match scope {
                    StyleInvalidationScope::Element => LayoutSourceChange::Node(target),
                    StyleInvalidationScope::Subtree => LayoutSourceChange::Subtree(target),
                }),
        );
        Some(changes)
    }

    /// Public geometry and coordinate input share this policy decision.
    /// Explicitly sampled geometry continues to use the snapshot-only APIs.
    pub(crate) fn ensure_observable_layout(&self) -> Result<(), LayoutError> {
        match self.layout_policy {
            moli_page_types::LayoutPolicy::Mock | moli_page_types::LayoutPolicy::OnDemand => {
                self.ensure_initial_layout()
            }
            moli_page_types::LayoutPolicy::Realtime => {
                self.ensure_current_screen_layout(LayoutFlushReason::SynchronousGeometry)
            }
        }
    }

    fn screen_layout_inputs(
        &self,
        state: &RealtimeLayoutState,
    ) -> Result<ScreenLayoutInputs, LayoutError> {
        let non_dom = state.non_dom_revision.get().ok_or_else(|| {
            LayoutError::source_contract("screen layout", "source revision exhausted")
        })?;
        let document_input = |document, dispatch_scope| DocumentLayoutInputs {
            document,
            root: self
                .dom_host()
                .dom()
                .document_element_handle_for_document(document),
            target: self.current_window_document_task_target_for_dispatch_scope(dispatch_scope),
            dom_revision: self.dom_host().document_layout_version(document),
            stylesheet_sources: self
                .style_engine
                .source_set_generation_for_document(document),
        };
        let document = self.document_handle();
        let mut documents = vec![document_input(document, OwnerDispatchScope::Top)];
        documents.extend(
            self.child_browsing_context_handles_in_document_order()
                .into_iter()
                .filter(|frame| self.child_browsing_context_host_is_active(*frame))
                .filter_map(|frame| {
                    self.child_browsing_context_document_handle(frame)
                        .map(|document| document_input(document, OwnerDispatchScope::Child(frame)))
                }),
        );
        Ok(ScreenLayoutInputs {
            non_dom,
            viewport: self.layout_viewport_for_document(document),
            environment: StyloStyleEnvironment::from_emulated_media(self.emulated_media()),
            scrollbars_hidden: self.scrollbars_hidden,
            documents,
        })
    }

    pub(super) fn ensure_current_screen_layout(
        &self,
        reason: LayoutFlushReason,
    ) -> Result<(), LayoutError> {
        if self.layout_pass_active.get() {
            return Err(LayoutError::ReentrantLayoutPass);
        }
        let state = self
            .realtime_layout_state
            .as_ref()
            .expect("Realtime policy installs its document state");
        let inputs = self.screen_layout_inputs(state)?;
        let document = self.document_handle();
        let resolve_style_changes = self.has_screen_style_invalidations(&inputs);
        if resolve_style_changes {
            self.prepare_current_screen_styles(&inputs);
        }
        let inline_style_update = resolve_style_changes
            && self.published_layout_matches_inline_style_update(state, &inputs);
        if (self.screen_layout_matches_inputs(state, &inputs) || inline_style_update)
            && (!resolve_style_changes || self.screen_styles_allow_layout_reuse(&inputs))
            && (inputs.documents[0].root.is_none()
                || !state.scroll_projection_dirty.get()
                || self.refresh_published_scroll_projection()?
                    != ScrollProjectionRefresh::Unavailable)
        {
            if resolve_style_changes {
                if let Some(tree) = self.document_layout_state.borrow_mut().latest_tree_mut() {
                    tree.clear_inline_svg_geometry();
                }
                for document in &inputs.documents {
                    self.style_engine
                        .consume_screen_style_work(document.document);
                }
            }
            if inline_style_update {
                *state.published.borrow_mut() = Some(inputs);
                self.dom_host().track_layout_changes(Some(document));
            }
            state.scroll_projection_dirty.set(false);
            return Ok(());
        }

        // Native invalidation/geometry only: no input dispatch, callbacks,
        // network admission, or nested event-loop checkpoint at this boundary.
        if !resolve_style_changes {
            self.prepare_current_screen_styles(&inputs);
        }
        let source_dom_version = self.dom_host().dom_version();
        let retain_paint = state.has_screen_paint_consumers();
        let request = LayoutPassRequest::new(inputs.viewport, reason)
            .with_frame_coverage(moli_layout::LayoutFrameCoverage::AllLayoutBoxes)
            .with_scroll_projection(true)
            .with_screen_paint(retain_paint);
        let Some((pass, feedback)) =
            self.build_layout_pass_for_document_in_environment(document, request, None, true)?
        else {
            if self.dom_host().dom_version() != source_dom_version
                || self.screen_layout_inputs(state)? != inputs
            {
                return Err(LayoutError::source_contract(
                    "screen layout",
                    "source inputs or Document owners changed during layout",
                ));
            }
            // Publish an empty result and retire the previous geometry.
            self.clear_layout_rect_cache();
            self.document_layout_state
                .borrow_mut()
                .clear_latest_layout();
            state.clear_publication();
            self.did_publish_screen_layout(state, inputs, HashMap::new());
            return Ok(());
        };
        self.finish_layout_publication(
            PreparedLayoutPublication(LayoutPublication::CurrentScreen {
                document,
                inputs,
                source_dom_version,
                retain_paint,
            }),
            pass,
            feedback,
        )
    }

    pub(crate) fn has_published_realtime_screen_layout(&self) -> bool {
        self.realtime_layout_state
            .as_ref()
            .is_some_and(|state| state.published.borrow().is_some())
    }

    pub(crate) fn realtime_screen_layout_is_current(&self) -> bool {
        let Some(state) = &self.realtime_layout_state else {
            return false;
        };
        if !state
            .published
            .borrow()
            .as_ref()
            .is_some_and(|published| Some(published.non_dom) == state.non_dom_revision.get())
        {
            return false;
        }
        self.screen_layout_inputs(state).is_ok_and(|inputs| {
            self.screen_layout_matches_inputs(state, &inputs)
                && !self.has_screen_style_invalidations(&inputs)
        })
    }

    #[cfg(test)]
    pub(crate) fn observed_screen_style_damage_for_test(&self) -> ServoRestyleDamage {
        self.style_engine
            .observed_screen_damage_for_test(self.document_handle())
    }

    #[cfg(test)]
    pub(crate) fn screen_style_damage_for_node_for_test(
        &self,
        handle: DomHandle,
    ) -> Option<ServoRestyleDamage> {
        self.style_engine
            .screen_style_damage_for_node_for_test(self.document_handle(), handle)
    }

    #[cfg(test)]
    pub(crate) fn element_style_resolutions_for_test(&self) -> u64 {
        self.style_engine
            .element_style_resolution_count_for_document_for_test(self.document_handle())
    }

    fn screen_layout_matches_inputs(
        &self,
        state: &RealtimeLayoutState,
        inputs: &ScreenLayoutInputs,
    ) -> bool {
        inputs.permits_reuse()
            && state.published.borrow().as_ref() == Some(inputs)
            && (inputs.documents[0].root.is_none()
                || self
                    .document_layout_state
                    .borrow()
                    .latest_layout(self.document_handle())
                    .is_some())
    }

    fn published_layout_matches_inline_style_update(
        &self,
        state: &RealtimeLayoutState,
        inputs: &ScreenLayoutInputs,
    ) -> bool {
        let document = self.document_handle();
        if inputs.documents.len() != 1
            || !inputs.permits_reuse()
            || !self
                .dom_host()
                .tracked_layout_changes_are_inline_style_only(document)
            || !self.document_layout_state.borrow().has_reusable_layout()
        {
            return false;
        }
        let published = state.published.borrow();
        let Some(published) = published
            .as_ref()
            .filter(|previous| previous.documents.len() == 1)
        else {
            return false;
        };
        let previous_document = &published.documents[0];
        let current = &inputs.documents[0];
        published.viewport == inputs.viewport
            && published.environment == inputs.environment
            && published.scrollbars_hidden == inputs.scrollbars_hidden
            && previous_document.document == current.document
            && previous_document.target == current.target
            && previous_document.root == current.root
            && previous_document.stylesheet_sources == current.stylesheet_sources
    }

    fn has_screen_style_invalidations(&self, inputs: &ScreenLayoutInputs) -> bool {
        inputs.documents.iter().any(|document| {
            self.style_engine
                .has_screen_style_invalidation(document.document)
        })
    }

    fn screen_styles_allow_layout_reuse(&self, inputs: &ScreenLayoutInputs) -> bool {
        let supported_damage = if inputs.documents.len() == 1
            && self.document_layout_state.borrow().has_reusable_layout()
        {
            ServoRestyleDamage::RECALCULATE_OVERFLOW
        } else {
            ServoRestyleDamage::empty()
        };
        if !inputs.documents.iter().all(|document| {
            let Some(restyles) = self.style_engine.screen_restyle_targets(
                self.dom_host(),
                document.document,
                supported_damage,
            ) else {
                return false;
            };
            restyles.targets.is_empty()
                || screen_targets_allow_layout_reuse(
                    self,
                    document.document,
                    restyles.targets,
                    restyles.after,
                    supported_damage,
                )
        }) {
            return false;
        }
        if supported_damage.is_empty() {
            return true;
        }

        let document = self.document_handle();
        let Some(updates) = self
            .style_engine
            .screen_projection_style_updates(document, supported_damage)
        else {
            return false;
        };
        if updates.styles.is_empty() {
            return true;
        }
        if ServoRestyleDamage::REPAINT.contains(updates.damage) {
            return self
                .document_layout_state
                .borrow_mut()
                .update_repaint_styles(updates.styles);
        }

        let reprojected = self
            .document_layout_state
            .borrow_mut()
            .update_screen_style_projection(document, updates.styles, |source| {
                crate::layout_renderer::native_scroll_offset(self.dom_host(), source)
            });
        if reprojected {
            self.clear_layout_rect_cache();
        }
        reprojected
    }

    fn prepare_current_screen_styles(&self, inputs: &ScreenLayoutInputs) {
        for document in &inputs.documents {
            self.drain_pending_style_invalidations_for_computed_style_read_for_document(
                document.document,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_revision_exhaustion_does_not_alias_an_old_input() {
        let state = RealtimeLayoutState::default();
        state.non_dom_revision.set(Some(u64::MAX));
        state.invalidate();
        assert_eq!(state.non_dom_revision.get(), None);
        state.invalidate();
        assert_eq!(state.non_dom_revision.get(), None);
        state.reset();
        assert_eq!(state.non_dom_revision.get(), Some(0));
    }
}
