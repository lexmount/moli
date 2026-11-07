use super::ScriptVm;
use crate::ace_page_task::InspectionStyle;
use crate::document_runtime::{DomHandle, EventTargetHandle};
use crate::dom::native::NodeData;
use anyhow::Result;
use moli_layout::{LayoutRect, LayoutViewport};
use std::collections::HashMap;
impl ScriptVm {
    pub(crate) fn prepare_document_inspection(&mut self) {
        self.reconcile_document_font_resources();
    }
    pub(crate) fn node_screenshot_clip(
        &mut self,
        handle: DomHandle,
        viewport: LayoutViewport,
    ) -> Result<crate::runtime::RendererScreenshotClip> {
        self.reconcile_document_font_resources();
        let runtime = self._context_host.borrow();
        let host = runtime.dom_host();
        let document = runtime.document_handle();
        let node = host
            .node(handle)
            .ok_or_else(|| anyhow::anyhow!("Unknown node ID"))?;
        let is_document = handle == document;
        anyhow::ensure!(
            node.is_connected()
                && (is_document || host.owner_document_handle(handle) == Some(document)),
            "Node must be connected to the main document"
        );
        anyhow::ensure!(
            is_document
                || node.is_element()
                || matches!(node.data(), NodeData::Text(_) | NodeData::CDataSection(_)),
            "Node does not have screenshot bounds"
        );
        anyhow::ensure!(
            is_document
                || crate::native_bridge::element::ace_page_task::node_has_screenshot_layout(
                    &runtime, handle
                ),
            "Node does not have screenshot bounds"
        );
        let bounds = runtime
            .build_layout_pass_for_document(
                document,
                moli_layout::LayoutPassRequest::new(
                    viewport,
                    moli_layout::LayoutFlushReason::Screenshot,
                ),
            )?
            .and_then(|pass| {
                let tree = &pass.tree;
                if is_document {
                    Some(LayoutRect::new(
                        0.0,
                        0.0,
                        tree.content_size.width,
                        tree.content_size.height,
                    ))
                } else {
                    node_quads(tree, handle)
                        .iter()
                        .map(|quad| quad.bounding_rect())
                        .filter(|rect| rect.width > 0.0 && rect.height > 0.0)
                        .reduce(LayoutRect::union)
                        .map(|mut rect| {
                            rect.x += tree.viewport_scroll.x;
                            rect.y += tree.viewport_scroll.y;
                            rect
                        })
                }
            })
            .ok_or_else(|| anyhow::anyhow!("Node does not have screenshot bounds"))?;
        Ok(crate::runtime::RendererScreenshotClip {
            x: f64::from(bounds.x),
            y: f64::from(bounds.y),
            width: f64::from(bounds.width),
            height: f64::from(bounds.height),
            scale: 1.0,
        })
    }

    pub(crate) fn inspect_layout(
        &mut self,
        viewport: LayoutViewport,
        visitor: &mut dyn FnMut(&moli_layout::FrozenLayoutTree<DomHandle>),
    ) -> Result<bool> {
        let runtime = self._context_host.borrow();
        let reason = moli_layout::LayoutFlushReason::SynchronousGeometry;
        let pass = runtime.build_layout_pass_for_document(
            runtime.document_handle(),
            moli_layout::LayoutPassRequest::new(viewport, reason),
        )?;
        if let Some(pass) = &pass {
            // Temporary inspection geometry must never replace the page's
            // published layout or frame viewports.
            visitor(&pass.tree);
        }
        Ok(pass.is_some())
    }
    pub(crate) fn inspection_styles(
        &self,
        properties: &[&str],
        events: &[&str],
        nodes: &[DomHandle],
    ) -> HashMap<DomHandle, InspectionStyle> {
        let runtime = self._context_host.borrow();
        let mut reads = crate::native_bridge::element::StyleObservation::new(&runtime);
        nodes
            .iter()
            .copied()
            .filter(|handle| {
                runtime
                    .dom_host()
                    .node(*handle)
                    .is_some_and(|node| node.is_element())
            })
            .map(|handle| {
                let read = reads.read(handle);
                let resolved = read
                    .computed_values()
                    .map(moli_layout::ResolvedLayoutStyle::from_stylo);
                let style = InspectionStyle {
                    properties: properties
                        .iter()
                        .map(|name| {
                            (
                                (*name).to_owned(),
                                crate::native_bridge::element::ace_page_task::style_property(
                                    &read, name,
                                ),
                            )
                        })
                        .collect(),
                    authored_display: resolved.as_ref().map(|style| style.display()),
                    background_alpha: resolved
                        .as_ref()
                        .map_or(0.0, |style| style.background_color().alpha),
                    event_listeners: events
                        .iter()
                        .filter(|event| {
                            self.document_runtime
                                .has_event_listener(EventTargetHandle::Node(handle), event)
                        })
                        .map(|event| (*event).to_owned())
                        .collect(),
                    focusable: crate::native_bridge::element::ace_page_task::is_focusable(
                        &runtime, handle,
                    ),
                    disabled:
                        crate::native_bridge::element::ace_page_task::is_disabled_form_control(
                            &runtime, handle,
                        ),
                };
                (handle, style)
            })
            .collect()
    }
    pub(crate) fn inspection_disabled(&self, handle: DomHandle) -> bool {
        crate::native_bridge::element::ace_page_task::is_disabled_form_control(
            &self._context_host.borrow(),
            handle,
        )
    }
}
fn node_quads(
    tree: &moli_layout::FrozenLayoutTree<DomHandle>,
    handle: DomHandle,
) -> Vec<moli_layout::LayoutQuad> {
    let boxes = tree.client_rects_for_source(handle);
    if boxes.is_empty() {
        // CSSOM element client rects intentionally omit text fragments. A
        // Text node still has its own rendered fragment geometry and DOM ID.
        tree.content_quads_for_source(handle)
    } else {
        boxes
    }
}
