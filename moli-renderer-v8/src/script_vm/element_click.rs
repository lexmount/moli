use super::ScriptVm;
use super::input_helpers::clear_input_dispatch_state;
use crate::document_runtime::DomHandle;
use crate::dom::{forms::InputType, native::Node};
use crate::native_bridge::element::{read_client_rects, scroll_node_into_view_at_center};
use crate::runtime::{
    RendererElementClickError as ClickError, RendererElementClickTarget as ClickTarget,
    RendererInputDispatchOutcome, RendererPointerEventProperties, RendererPreparedPointerClick,
};
use moli_layout::{LayoutPoint, LayoutQuad};

fn layout_error(error: impl std::fmt::Display) -> ClickError {
    ClickError::LayoutUnavailable(error.to_string())
}

impl ScriptVm {
    pub(crate) fn prepare_element_click(
        &mut self,
        handle: DomHandle,
    ) -> Result<ClickTarget, ClickError> {
        {
            let host = self._context_host.borrow();
            let element = host
                .dom_host()
                .node(handle)
                .and_then(Node::as_element)
                .filter(|_| host.dom_host().is_connected(handle))
                .ok_or(ClickError::StaleNode)?;
            if element.is_html_input() && element.input_type() == InputType::File {
                return Ok(ClickTarget::FileInput);
            }
            if element.local_name() == "option" {
                return Ok(ClickTarget::Option);
            }
            if !host.layout_policy().uses_real_layout() {
                return Ok(ClickTarget::DomActivation);
            }
            let document = host
                .layout_document_for_source(handle)
                .ok_or(ClickError::StaleNode)?;
            host.with_latest_layout_tree_for_document(document, |_| ())
                .ok_or_else(|| layout_error(moli_layout::LayoutError::NoLayoutSnapshot))?;
        }
        self.with_default_context_scope(|scope, runtime_ptr| {
            Ok(scroll_node_into_view_at_center(scope, runtime_ptr, handle)?)
        })
        .map_err(layout_error)?
        .ok_or(ClickError::NoClickableRect)?;
        self.prepare_pointer_click_geometry(handle)
            .map(ClickTarget::Pointer)
    }

    fn prepare_pointer_click_geometry(
        &mut self,
        handle: DomHandle,
    ) -> Result<RendererPreparedPointerClick, ClickError> {
        let (document, point, root_document) = {
            let host = self._context_host.borrow();
            if !host.dom_host().is_connected(handle) {
                return Err(ClickError::StaleNode);
            }
            let document = host
                .dom_host()
                .owner_document_handle(handle)
                .ok_or(ClickError::StaleNode)?;
            let rect = read_client_rects(&host, handle)
                .map_err(layout_error)?
                .into_iter()
                .next()
                .ok_or(ClickError::NoClickableRect)?;
            let viewport = host
                .with_latest_layout_tree_for_document(document, |tree| tree.viewport)
                .ok_or_else(|| layout_error(moli_layout::LayoutError::NoLayoutSnapshot))?;
            let left = rect.left.max(0.0);
            let top = rect.top.max(0.0);
            let right = rect.right.min(f64::from(viewport.css_width));
            let bottom = rect.bottom.min(f64::from(viewport.css_height));
            if right <= left || bottom <= top {
                return Err(ClickError::NoClickableRect);
            }
            (
                document,
                LayoutPoint::new(
                    ((left + right) / 2.0).floor() as f32,
                    ((top + bottom) / 2.0).floor() as f32,
                ),
                host.root_document_lifecycle_identity(),
            )
        };
        // Use the same frame geometry composition as DOM geometry. The server
        // must never reconstruct this transform from serialized box quads.
        let mut quad = [LayoutQuad { points: [point; 4] }];
        self.compose_layout_quads_to_top(document, &mut quad)
            .map_err(layout_error)?;
        let point = quad[0].points[0];
        let hit = self
            .observable_deep_hit_test_for_current_document(point, false)
            .map_err(layout_error)?
            .ok_or(ClickError::Obscured)?;
        let mut documents = Vec::new();
        let mut document_cursor = self
            ._context_host
            .borrow()
            .dom_host()
            .owner_document_handle(hit)
            .ok_or(ClickError::MissingBrowsingContext)?;
        loop {
            let id = self
                .document_id_for_live_node_handle(document_cursor)
                .ok_or(ClickError::MissingBrowsingContext)?;
            documents.push((document_cursor, id));
            let host = self._context_host.borrow();
            if document_cursor == host.document_handle() {
                break;
            }
            let frame = host
                .child_browsing_context_host_for_document_handle(document_cursor)
                .ok_or(ClickError::MissingBrowsingContext)?;
            document_cursor = host
                .dom_host()
                .owner_document_handle(frame)
                .ok_or(ClickError::MissingBrowsingContext)?;
        }
        // The deep hit already passed every ancestor's clipping and paint
        // order. Walk the live composed ancestry, including frame owners, to
        // accept descendants without accidentally accepting sibling overlays.
        let host = self._context_host.borrow();
        let mut cursor = Some(hit);
        while let Some(candidate) = cursor {
            if candidate == handle {
                return Ok(RendererPreparedPointerClick {
                    target: handle,
                    root_x: f64::from(point.x),
                    root_y: f64::from(point.y),
                    root_document,
                    documents,
                });
            }
            cursor = host
                .dom_host()
                .parent_node(candidate)
                .or_else(|| host.dom_host().shadow_root_host(candidate))
                .or_else(|| host.child_browsing_context_host_for_document_handle(candidate));
        }
        Err(ClickError::Obscured)
    }

    fn click_documents_are_current(&self, click: &RendererPreparedPointerClick) -> bool {
        self._context_host
            .borrow()
            .root_document_lifecycle_identity()
            == click.root_document
            && click
                .documents
                .iter()
                .all(|(handle, id)| self.document_id_for_live_node_handle(*handle) == Some(*id))
    }

    pub(crate) fn dispatch_prepared_element_click(
        &mut self,
        click: RendererPreparedPointerClick,
    ) -> Result<RendererInputDispatchOutcome, ClickError> {
        if !self.click_documents_are_current(&click) {
            return Err(ClickError::StaleNode);
        }
        if !self
            ._context_host
            .borrow()
            .dom_host()
            .is_connected(click.target)
        {
            return Err(ClickError::StaleNode);
        }
        // Preserve the point sampled during preparation. Each pointer phase
        // uses the existing hit-test/snapshot policy; DOM or style mutations
        // must not introduce an extra layout refresh or replay the sequence.
        let mut outcome = RendererInputDispatchOutcome::default();
        for (event, buttons) in [("mousemove", 0), ("mousedown", 1), ("mouseup", 0)] {
            if !self.click_documents_are_current(&click) {
                clear_input_dispatch_state(self);
                break;
            }
            let dispatched = self.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                click.root_x,
                click.root_y,
                event,
                0,
                Some(buttons),
                i32::from(event != "mousemove"),
                0.0,
                0.0,
                RendererPointerEventProperties::default(),
                0,
            );
            let next = match dispatched {
                Ok(next) => next,
                Err(error) => {
                    clear_input_dispatch_state(self);
                    return Err(layout_error(error));
                }
            };
            outcome.handled |= next.handled;
            outcome.triggered_top_level_navigation |=
                next.triggered_top_level_navigation || self.has_pending_location_navigation();
            outcome.pending_download = next.pending_download.or(outcome.pending_download);
            outcome.pending_file_chooser =
                next.pending_file_chooser.or(outcome.pending_file_chooser);
            if outcome.triggered_top_level_navigation {
                clear_input_dispatch_state(self);
                break;
            }
        }
        Ok(outcome)
    }
}
