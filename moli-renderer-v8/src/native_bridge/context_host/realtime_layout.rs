use std::cell::{Cell, RefCell};

use moli_layout::{
    LayoutError, LayoutFlushReason, LayoutPassRequest, LayoutPassResult, LayoutViewport,
    ScrollProjectionRefresh,
};

use super::realtime_animation_frames::RealtimeAnimationFrames;
use super::realtime_environment::RealtimeEnvironmentState;
use super::realtime_events::RealtimeRenderingEvents;
use super::realtime_rendering::RealtimeFrameState;
#[cfg(feature = "devtools")]
use super::screencast_paint::{
    RendererScreencastPaintSession, ScreenPaintEntry, ScreencastPaintState,
};
#[cfg(feature = "devtools")]
use super::visual_resource_generation::VisualResourceRevisions;
use super::{OwnerDispatchScope, PageScriptHost, WindowDocumentTaskTarget};
use crate::{
    document_runtime::DomHandle,
    layout_renderer::{LayoutPassFeedback, ScreenStyleBaseline, refreshable_screen_styles},
    style_engine::{StyloComputedStyleSnapshot, StyloStyleEnvironment},
};

/// The consumer explicitly decides whether a pass may update screen geometry.
/// Trigger reasons remain diagnostic and do not select publication behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LayoutPublicationRequest {
    Screen,
    Transient,
}

pub(crate) struct PreparedLayoutPublication(LayoutPublication);

#[cfg(not(feature = "devtools"))]
type VisualResourceRevisions = ();

enum LayoutPublication {
    LegacyLatest(DomHandle),
    CurrentScreen {
        document: DomHandle,
        inputs: ScreenLayoutInputs,
        source_dom_version: u64,
        paint_generation: Option<VisualResourceRevisions>,
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
        if let LayoutPublication::CurrentScreen {
            paint_generation, ..
        } = &self.0
        {
            request
                .with_frame_coverage(moli_layout::LayoutFrameCoverage::AllLayoutBoxes)
                .with_scroll_projection(true)
                .with_screen_paint(paint_generation.is_some())
        } else {
            request
        }
    }
}

/// The validity and lifecycle state for one Realtime screen publication.
/// An active screencast owns its optional paint input through a session lease.
pub(super) struct RealtimeLayoutState {
    non_dom_revision: Cell<Option<u64>>,
    scroll_projection_dirty: Cell<bool>,
    published: RefCell<Option<ScreenLayoutInputs>>,
    screen_styles: RefCell<ScreenStyleBaseline>,
    #[cfg(feature = "devtools")]
    screencast_paint: ScreencastPaintState,
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
            screen_styles: RefCell::default(),
            #[cfg(feature = "devtools")]
            screencast_paint: ScreencastPaintState::default(),
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

    pub(super) fn clear_publication(&self) {
        self.published.borrow_mut().take();
        self.screen_styles.borrow_mut().clear();
        #[cfg(feature = "devtools")]
        self.screencast_paint.publish(None);
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

#[derive(Default)]
struct ScreenStyleRefresh {
    documents: Vec<DomHandle>,
    backgrounds: Vec<(DomHandle, DomHandle, StyloComputedStyleSnapshot)>,
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
                paint_generation: self.screen_paint_generation(state),
            },
        ))
    }

    pub(crate) fn finish_layout_publication(
        &self,
        publication: PreparedLayoutPublication,
        pass: LayoutPassResult<DomHandle>,
        feedback: LayoutPassFeedback<'_>,
    ) -> Result<(), LayoutError> {
        match publication.0 {
            LayoutPublication::LegacyLatest(document) => {
                self.publish_layout_pass_for_document(document, pass);
            }
            LayoutPublication::CurrentScreen {
                document,
                inputs,
                source_dom_version,
                paint_generation,
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
                #[cfg(feature = "devtools")]
                let (pass, paint) = {
                    let mut pass = pass;
                    let paint = paint_generation.zip(pass.take_screen_paint()).map(
                        |(resource_generation, layout)| ScreenPaintEntry {
                            layout,
                            resource_generation,
                            css_images: pass.css_image_references().to_vec(),
                        },
                    );
                    (pass, paint)
                };
                #[cfg(not(feature = "devtools"))]
                let _ = paint_generation;
                self.publish_layout_pass_for_document(document, pass);
                *state.screen_styles.borrow_mut() = feedback.commit().unwrap_or_default();
                #[cfg(feature = "devtools")]
                state.screencast_paint.publish(paint);
                self.did_publish_screen_layout(state, inputs);
            }
            LayoutPublication::Transient => {}
        }
        Ok(())
    }

    fn did_publish_screen_layout(&self, state: &RealtimeLayoutState, inputs: ScreenLayoutInputs) {
        for document in &inputs.documents {
            self.style_engine.publish_screen_styles(document.document);
        }
        if moli_trace::cpu_profile_enabled() {
            let baseline = state.screen_styles.borrow();
            tracing::info!(
                target: "moli_cpu_profile",
                stage = "screen_style_baseline",
                element_count = baseline.values().map(|styles| styles.len()).sum::<usize>(),
                entry_storage_bytes = baseline.values().map(|styles| {
                    styles.capacity() * std::mem::size_of::<(
                        DomHandle,
                        crate::style_engine::StyloComputedStyleSnapshot,
                    )>()
                }).sum::<usize>(),
            );
        }
        // Attach the verified inputs after the geometry update retires the previous validity.
        *state.published.borrow_mut() = Some(inputs);
        state.scroll_projection_dirty.set(false);
        crate::observer_runtime::did_publish_realtime_resize_observer_layout(self);
        self.did_publish_realtime_font_layout();
    }

    fn screen_paint_generation(
        &self,
        state: &RealtimeLayoutState,
    ) -> Option<VisualResourceRevisions> {
        #[cfg(feature = "devtools")]
        if state.screencast_paint.is_active() {
            let generation = self.visual_resource_revisions();
            return (generation.generation() != u64::MAX).then_some(generation);
        }
        #[cfg(not(feature = "devtools"))]
        let _ = state;
        None
    }

    #[cfg(all(test, feature = "devtools"))]
    pub(crate) fn has_screencast_paint_for_test(&self) -> bool {
        self.realtime_layout_state
            .as_ref()
            .is_some_and(|state| state.screencast_paint.read(|_| ()).is_some())
    }

    #[cfg(feature = "devtools")]
    pub(crate) fn paint_current_screencast_layout(
        &self,
        session: &RendererScreencastPaintSession,
        viewport: LayoutViewport,
        capture: moli_layout::PaintCaptureRequest,
    ) -> Result<
        Option<(
            moli_layout::PaintSnapshot,
            Vec<moli_layout::LayoutCssImageReference<DomHandle>>,
        )>,
        LayoutError,
    > {
        let Some(state) = &self.realtime_layout_state else {
            return Ok(None);
        };
        let document = self.document_handle();
        if viewport != self.layout_viewport_for_document(document) {
            return Ok(None);
        }
        state.screencast_paint.attach(session);
        self.ensure_current_screen_layout(LayoutFlushReason::Screencast)?;
        let layout = self.document_layout_state.borrow();
        let Some(tree) = layout.latest_layout(document) else {
            return Ok(None);
        };
        let resource_generation = self.visual_resource_revisions();
        state
            .screencast_paint
            .update(|paint| {
                if paint.resource_generation.other_pixels != resource_generation.other_pixels
                    || resource_generation.generation() == u64::MAX
                {
                    return Ok(None);
                }
                if paint.resource_generation.css_images != resource_generation.css_images {
                    self.refresh_screen_css_images(paint);
                    if self.visual_resource_revisions() != resource_generation {
                        return Ok(None);
                    }
                    paint.resource_generation = resource_generation;
                }
                Ok(Some((
                    paint.layout.paint(tree, capture)?,
                    paint.css_images.clone(),
                )))
            })
            .unwrap_or(Ok(None))
    }

    #[cfg(feature = "devtools")]
    fn refresh_screen_css_images(&self, paint: &mut ScreenPaintEntry) {
        use std::collections::HashMap;

        let mut images_by_document: HashMap<
            DomHandle,
            HashMap<String, Option<moli_layout::LayoutImageResource>>,
        > = HashMap::new();
        paint.layout.update_css_image_resources(&mut |source, url| {
            let document = self.dom_host().owner_document_handle(source)?;
            let images = images_by_document.entry(document).or_default();
            if let Some(resource) = images.get(url) {
                return resource.clone();
            }
            let resource = self
                .ready_css_image_for_layout(document, url)
                .map(Into::into);
            images.insert(url.to_owned(), resource.clone());
            resource
        });
    }

    #[cfg(test)]
    pub(crate) fn realtime_layout_allocated_for_test(&self) -> bool {
        self.realtime_layout_state.is_some()
    }

    /// Called at native source commits, before any callback can observe them.
    /// Legacy modes do not allocate state, advance revisions, or enqueue work.
    pub(crate) fn note_realtime_layout_input_change(&self) {
        if let Some(state) = &self.realtime_layout_state {
            state.invalidate();
            self.request_realtime_rendering_opportunity();
        }
    }

    pub(super) fn note_selection_paint_change(&self) {
        #[cfg(feature = "devtools")]
        if let Some(state) = &self.realtime_layout_state {
            state.screencast_paint.publish(None);
            self.document_layout_state
                .borrow_mut()
                .mark_visual_state_dirty();
        }
    }

    /// Scrolling changes the projection and lifecycle demand, but leaves the
    /// dimensions that were computed by the last layout pass intact.
    pub(crate) fn note_realtime_scroll_change(&self) {
        if let Some(state) = &self.realtime_layout_state {
            state.scroll_projection_dirty.set(true);
            self.invalidate_layout_projection();
            self.request_realtime_rendering_opportunity();
        }
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
        let reusable_styles = if self.screen_layout_matches_inputs(state, &inputs) {
            if resolve_style_changes {
                self.prepare_screen_style_refresh(state, &inputs)
            } else {
                Some(ScreenStyleRefresh::default())
            }
        } else {
            None
        };
        if let Some(refresh) = reusable_styles
            && (inputs.documents[0].root.is_none()
                || !state.scroll_projection_dirty.get()
                || self.refresh_published_scroll_projection()?
                    != ScrollProjectionRefresh::Unavailable)
        {
            self.commit_screen_style_refresh(state, refresh);
            state.scroll_projection_dirty.set(false);
            return Ok(());
        }

        // Native invalidation/geometry only: no input dispatch, callbacks,
        // network admission, or nested event-loop checkpoint at this boundary.
        if !resolve_style_changes {
            self.prepare_current_screen_styles(&inputs);
        }
        let source_dom_version = self.dom_host().dom_version();
        let paint_generation = self.screen_paint_generation(state);
        let request = LayoutPassRequest::new(inputs.viewport, reason)
            .with_frame_coverage(moli_layout::LayoutFrameCoverage::AllLayoutBoxes)
            .with_scroll_projection(true)
            .with_screen_paint(paint_generation.is_some());
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
            self.did_publish_screen_layout(state, inputs);
            return Ok(());
        };
        self.finish_layout_publication(
            PreparedLayoutPublication(LayoutPublication::CurrentScreen {
                document,
                inputs,
                source_dom_version,
                paint_generation,
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

    fn has_screen_style_invalidations(&self, inputs: &ScreenLayoutInputs) -> bool {
        inputs.documents.iter().any(|document| {
            self.style_engine
                .has_screen_style_invalidation(document.document)
        })
    }

    fn prepare_screen_style_refresh(
        &self,
        state: &RealtimeLayoutState,
        inputs: &ScreenLayoutInputs,
    ) -> Option<ScreenStyleRefresh> {
        let baseline = state.screen_styles.borrow();
        let mut refresh = ScreenStyleRefresh::default();
        for document in &inputs.documents {
            let roots = self
                .style_engine
                .affected_screen_style_roots(document.document)?;
            if roots.is_empty() {
                continue;
            }
            let backgrounds = refreshable_screen_styles(self, document.document, roots, &baseline)?;
            refresh.documents.push(document.document);
            refresh.backgrounds.extend(
                backgrounds
                    .into_iter()
                    .map(|(node, style)| (document.document, node, style)),
            );
        }
        Some(refresh)
    }

    fn commit_screen_style_refresh(
        &self,
        state: &RealtimeLayoutState,
        refresh: ScreenStyleRefresh,
    ) {
        if !refresh.backgrounds.is_empty() {
            #[cfg(feature = "devtools")]
            state.screencast_paint.update(|paint| {
                let colors = refresh
                    .backgrounds
                    .iter()
                    .map(|(_, node, style)| (*node, style.computed_values()))
                    .collect();
                paint.layout.update_background_colors(&colors);
            });
            let mut baseline = state.screen_styles.borrow_mut();
            for (document, node, style) in refresh.backgrounds {
                baseline
                    .get_mut(&document)
                    .expect("checked screen styles")
                    .insert(node, style);
            }
        }
        for document in refresh.documents {
            self.style_engine.publish_screen_styles(document);
        }
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
