use std::cell::{Cell, RefCell};

use moli_layout::{
    LayoutError, LayoutFlushReason, LayoutPassRequest, LayoutPassResult, LayoutViewport,
};

use super::realtime_animation_frames::RealtimeAnimationFrames;
use super::realtime_rendering::RealtimeFrameState;
use super::{JsContextHost, OwnerDispatchScope, WindowDocumentTaskTarget};
use crate::{document_runtime::DomHandle, style_engine::StyloStyleEnvironment};

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
    },
    Transient,
}

impl PreparedLayoutPublication {
    pub(crate) fn publishes_screen(&self) -> bool {
        !matches!(self.0, LayoutPublication::Transient)
    }
}

/// Only allocated for Realtime. This owns validity metadata, never another
/// layout tree or a per-node working cache. The existing single snapshot slot
/// remains the only owner of published geometry.
pub(super) struct RealtimeLayoutState {
    non_dom_revision: Cell<Option<u64>>,
    published: RefCell<Option<ScreenLayoutInputs>>,
    pub(super) frame: RefCell<RealtimeFrameState>,
    pub(super) animation_frames: RefCell<RealtimeAnimationFrames>,
}

impl Default for RealtimeLayoutState {
    fn default() -> Self {
        Self {
            non_dom_revision: Cell::new(Some(0)),
            published: RefCell::new(None),
            frame: RefCell::new(RealtimeFrameState::default()),
            animation_frames: RefCell::new(RealtimeAnimationFrames::default()),
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
    }

    pub(super) fn reset(&self) {
        self.clear_publication();
        self.non_dom_revision.set(Some(0));
        self.frame.borrow_mut().retire_document();
    }
}

#[derive(Debug, PartialEq)]
struct DocumentLayoutInputs {
    document: DomHandle,
    root: Option<DomHandle>,
    target: Option<WindowDocumentTaskTarget>,
    stylesheet_sources: u64,
}

#[derive(Debug, PartialEq)]
struct ScreenLayoutInputs {
    dom: u64,
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
        self.dom != u64::MAX
            && self
                .documents
                .iter()
                .all(|document| document.stylesheet_sources != u64::MAX)
    }
}

impl JsContextHost {
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
            LayoutPublication::CurrentScreen { document, inputs },
        ))
    }

    pub(crate) fn finish_layout_publication(
        &self,
        publication: PreparedLayoutPublication,
        pass: LayoutPassResult<DomHandle>,
    ) -> Result<(), LayoutError> {
        match publication.0 {
            LayoutPublication::LegacyLatest(document) => {
                self.publish_layout_pass_for_document(document, pass);
            }
            LayoutPublication::CurrentScreen { document, inputs } => {
                let state = self.realtime_layout_state.as_ref().ok_or_else(|| {
                    LayoutError::source_contract(
                        "screen layout",
                        "layout policy changed during pass",
                    )
                })?;
                if pass.viewport != inputs.viewport || self.screen_layout_inputs(state)? != inputs {
                    return Err(LayoutError::source_contract(
                        "screen layout",
                        "source inputs or Document owners changed during layout",
                    ));
                }
                self.publish_layout_pass_for_document(document, pass);
                // Attach exactly the inputs captured before this pass. The
                // low-level publication has retired its previous validity.
                *state.published.borrow_mut() = Some(inputs);
            }
            LayoutPublication::Transient => {}
        }
        Ok(())
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
            dom: self.dom_host().dom_version(),
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
        if inputs.permits_reuse()
            && state.published.borrow().as_ref() == Some(&inputs)
            && self
                .document_layout_state
                .borrow()
                .latest_layout(self.document_handle())
                .is_some()
        {
            return Ok(());
        }

        // Native invalidation/geometry only: no input dispatch, callbacks,
        // network admission, or nested event-loop checkpoint at this boundary.
        self.prepare_current_screen_styles(&inputs);
        let request = LayoutPassRequest::new(inputs.viewport, reason);
        let document = self.document_handle();
        let pass = self
            .build_layout_pass_for_document(document, request)?
            .ok_or(LayoutError::NoLayoutRoot)?;
        self.finish_layout_publication(
            PreparedLayoutPublication(LayoutPublication::CurrentScreen { document, inputs }),
            pass,
        )
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
