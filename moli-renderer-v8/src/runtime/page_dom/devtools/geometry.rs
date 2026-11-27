use super::super::*;
use crate::native_bridge::element::InputHitTestPolicy;
use crate::runtime::page_surface::RendererDocumentHitTestResult;
use crate::runtime::page_surface::{
    RendererDocumentBoxModel, RendererDocumentNodeClientRect, RendererDocumentNodeGeometry,
    RendererGeometryQuad,
};

impl PageVm {
    pub(crate) fn client_rect_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
    ) -> Result<Option<RendererDocumentNodeClientRect>> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(None);
        };
        self.client_rect_for_live_node_handle(handle)
    }

    pub(crate) fn client_rect_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
    ) -> Result<Option<RendererDocumentNodeClientRect>> {
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
            return Ok(None);
        };
        self.client_rect_for_live_node_handle(handle)
    }

    pub(crate) fn document_geometry_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
    ) -> Result<Option<RendererDocumentNodeGeometry>> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(None);
        };
        self.document_geometry_for_live_node_handle(handle)
            .map(Some)
    }

    pub(crate) fn document_geometry_for_node(
        &mut self,
        reference: RendererDomNodeReference,
    ) -> Result<Option<RendererDocumentNodeGeometry>> {
        let Some(handle) = self.live_handle_for_document_node_reference(reference) else {
            return Ok(None);
        };
        self.document_geometry_for_live_node_handle(handle)
            .map(Some)
    }

    fn document_geometry_for_live_node_handle(
        &mut self,
        handle: DomHandle,
    ) -> Result<RendererDocumentNodeGeometry> {
        let (is_element, is_text, document) = {
            let host = self.vm().document_runtime.dom_host();
            let Some(node) = host.node(handle) else {
                return Ok(RendererDocumentNodeGeometry::NoLayoutObject);
            };
            let Some(document) = host.owner_document_handle(handle) else {
                return Ok(RendererDocumentNodeGeometry::NoLayoutObject);
            };
            (node.is_element(), node.is_text(), document)
        };
        if !is_element && !is_text {
            return Ok(RendererDocumentNodeGeometry::NotElement);
        }

        let answers = self
            .vm_mut()
            .observable_geometry_batch_for_document(
                document,
                &moli_layout::LayoutQueryBatch::new(vec![
                    moli_layout::LayoutQuery::BoxModel { source: handle },
                    moli_layout::LayoutQuery::ContentQuads { source: handle },
                    moli_layout::LayoutQuery::ElementMetrics { source: handle },
                ]),
            )
            .map_err(|error| anyhow::anyhow!("failed to resolve document geometry: {error}"))?;
        let mut answers = answers.answers.into_iter();
        let Some(moli_layout::LayoutQueryAnswer::BoxModel(box_model)) = answers.next() else {
            return Err(anyhow::anyhow!(
                "geometry provider returned a mismatched box-model answer"
            ));
        };
        let Some(moli_layout::LayoutQueryAnswer::ContentQuads(content_quads)) = answers.next()
        else {
            return Err(anyhow::anyhow!(
                "geometry provider returned a mismatched content-quads answer"
            ));
        };
        let Some(moli_layout::LayoutQueryAnswer::ElementMetrics(element_metrics)) = answers.next()
        else {
            return Err(anyhow::anyhow!(
                "geometry provider returned a mismatched element-metrics answer"
            ));
        };
        let mut composed_quads = Vec::new();
        if let Some(model) = box_model {
            composed_quads.extend([model.content, model.padding, model.border, model.margin]);
        }
        let model_quad_count = composed_quads.len();
        composed_quads.extend(content_quads);
        self.vm_mut()
            .compose_layout_quads_to_top(document, &mut composed_quads)
            .map_err(|error| anyhow::anyhow!("failed to compose frame geometry: {error}"))?;
        let content_quads = composed_quads[model_quad_count..]
            .iter()
            .copied()
            .map(renderer_geometry_quad)
            .collect();

        if !is_element {
            return Ok(RendererDocumentNodeGeometry::FoundNonElement { content_quads });
        }
        let (Some(_), Some(element_metrics)) = (box_model, element_metrics) else {
            return Ok(if content_quads.is_empty() {
                RendererDocumentNodeGeometry::NoLayoutObject
            } else {
                RendererDocumentNodeGeometry::FoundElementContents { content_quads }
            });
        };
        let [content, padding, border, margin] = composed_quads[..model_quad_count] else {
            return Err(anyhow::anyhow!(
                "geometry provider returned an incomplete box model"
            ));
        };
        Ok(RendererDocumentNodeGeometry::FoundElement {
            box_model: Box::new(RendererDocumentBoxModel {
                content: renderer_geometry_quad(content),
                padding: renderer_geometry_quad(padding),
                border: renderer_geometry_quad(border),
                margin: renderer_geometry_quad(margin),
                width: rounded_css_integer(element_metrics.offset_size.width),
                height: rounded_css_integer(element_metrics.offset_size.height),
            }),
            content_quads,
        })
    }

    pub(crate) fn document_hit_test(
        &mut self,
        inspector_session_id: Option<&str>,
        x: f64,
        y: f64,
        include_user_agent_shadow_dom: bool,
        ignore_pointer_events_none: bool,
    ) -> Result<Option<RendererDocumentHitTestResult>> {
        // Generated UA shadow nodes are inspector projections rather than
        // layout sources, so their layout hit is already the live host.
        let _ = include_user_agent_shadow_dom;
        let policy = InputHitTestPolicy {
            ignore_pointer_events_none,
            // Chromium treats inert content as having a used pointer-events value of none.
            ignore_inert: ignore_pointer_events_none,
            ..InputHitTestPolicy::INTERACTIVE
        };
        let hit = self
            .vm_mut()
            .observable_deep_hit_test_for_current_document(
                moli_layout::LayoutPoint::new(x as f32, y as f32),
                policy,
            )
            .map_err(|error| anyhow::anyhow!("failed to hit test document: {error}"))?;
        let Some(mut handle) = hit else {
            return Ok(None);
        };
        loop {
            let Some(node) = self.vm().document_runtime.dom_host().node(handle) else {
                return Ok(None);
            };
            if node.is_element() {
                break;
            }
            let Some(parent) = node.parent_node() else {
                return Ok(None);
            };
            handle = parent;
        }
        let frame_id = self.vm().child_frame_id_for_live_node_handle(handle);
        let Some(node) =
            self.published_document_node_reference_for_live_handle(inspector_session_id, handle)
        else {
            return Ok(None);
        };
        Ok(Some(RendererDocumentHitTestResult { node, frame_id }))
    }

    fn client_rect_for_live_node_handle(
        &mut self,
        handle: DomHandle,
    ) -> Result<Option<RendererDocumentNodeClientRect>> {
        let Some((is_element, is_text)) = self
            .vm()
            .document_runtime
            .dom_host()
            .node(handle)
            .map(|node| (node.is_element(), node.is_text()))
        else {
            return Ok(None);
        };
        let rect = self
            .vm_mut()
            .client_rect_for_live_node_handle(handle)
            .map_err(|error| anyhow::anyhow!("failed to resolve client rect: {error}"))?;
        if is_element {
            Ok(rect.map(RendererDocumentNodeClientRect::Found))
        } else if is_text {
            Ok(rect.map(RendererDocumentNodeClientRect::FoundNonElement))
        } else {
            Ok(Some(RendererDocumentNodeClientRect::NotElement))
        }
    }

    pub(crate) fn node_has_geometry_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
    ) -> Result<Option<bool>> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(None);
        };
        self.node_has_geometry_for_live_node_handle(handle)
    }

    pub(crate) fn node_has_geometry_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
    ) -> Result<Option<bool>> {
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
            return Ok(None);
        };
        self.node_has_geometry_for_live_node_handle(handle)
    }

    pub(crate) fn scroll_node_into_view_if_needed_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
        rect: Option<moli_page_types::DomScrollIntoViewRect>,
    ) -> Result<RendererScrollIntoViewResult> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(RendererScrollIntoViewResult::NodeNotFound);
        };
        self.vm_mut()
            .scroll_live_node_handle_into_view_if_needed(handle, rect)
    }

    pub(crate) fn scroll_node_into_view_if_needed(
        &mut self,
        reference: RendererDomNodeReference,
        rect: Option<moli_page_types::DomScrollIntoViewRect>,
    ) -> Result<RendererScrollIntoViewResult> {
        let Some(handle) = self.live_handle_for_document_node_reference(reference) else {
            return Ok(RendererScrollIntoViewResult::NodeNotFound);
        };
        self.vm_mut()
            .scroll_live_node_handle_into_view_if_needed(handle, rect)
    }

    fn node_has_geometry_for_live_node_handle(
        &mut self,
        handle: DomHandle,
    ) -> Result<Option<bool>> {
        let Some((is_document, is_geometry_node, document)) = self
            .vm()
            .document_runtime
            .dom_host()
            .node(handle)
            .and_then(|node| {
                Some((
                    node.is_document(),
                    node.is_element() || node.is_text(),
                    self.vm()
                        .document_runtime
                        .dom_host()
                        .owner_document_handle(handle)?,
                ))
            })
        else {
            return Ok(None);
        };
        if is_document {
            return Ok(Some(true));
        }
        if !is_geometry_node {
            return Ok(Some(false));
        }
        let answer = self
            .vm()
            .observable_geometry_query_for_document(
                document,
                &moli_layout::LayoutQuery::ClientRects { source: handle },
            )
            .map_err(|error| anyhow::anyhow!("failed to test node geometry: {error}"))?;
        let moli_layout::LayoutQueryAnswer::ClientRects(rects) = answer else {
            return Err(anyhow::anyhow!(
                "geometry provider returned a mismatched client-rects answer"
            ));
        };
        Ok(Some(!rects.is_empty()))
    }
}

fn renderer_geometry_quad(quad: moli_layout::LayoutQuad) -> RendererGeometryQuad {
    let [top_left, top_right, bottom_right, bottom_left] = quad.points;
    RendererGeometryQuad {
        points: [
            f64::from(top_left.x),
            f64::from(top_left.y),
            f64::from(top_right.x),
            f64::from(top_right.y),
            f64::from(bottom_right.x),
            f64::from(bottom_right.y),
            f64::from(bottom_left.x),
            f64::from(bottom_left.y),
        ],
    }
}

fn rounded_css_integer(value: f32) -> i32 {
    value.round().clamp(i32::MIN as f32, i32::MAX as f32) as i32
}
