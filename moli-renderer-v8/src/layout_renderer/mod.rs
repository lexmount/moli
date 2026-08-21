mod inline_svg;
mod source_view;
mod style_resolver;

use std::collections::HashMap;

use moli_layout::{
    DocumentLayoutServices, EmbeddedFrameRenderer, LayoutPassRequest, LayoutPassResult,
    LayoutViewport, build_layout_pass_with_embedded_frames,
};

use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

pub(crate) fn current_native_stylesheet_web_font_resources(
    runtime: &JsContextHost,
    root: DomHandle,
) -> Vec<crate::css_resource_urls::StylesheetLoadBlockingResource> {
    let mut reads = crate::native_bridge::element::ComputedStyleReadScope::new(runtime);
    let sources = reads.read(root).stylesheet_source_snapshots();
    let mut resources = std::collections::BTreeMap::new();
    for (css_text, base_url) in sources {
        for resource in crate::css_resource_urls::stylesheet_load_blocking_resources(
            &css_text,
            &base_url,
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        ) {
            let Some(font) = resource.web_font() else {
                continue;
            };
            resources.entry(font.slot().to_owned()).or_insert(resource);
        }
    }
    resources.into_values().collect()
}

pub(crate) fn build_native_layout_pass(
    runtime: &JsContextHost,
    root: DomHandle,
    services: &mut DocumentLayoutServices,
    embedded_document_services: &mut HashMap<DomHandle, DocumentLayoutServices>,
    request: LayoutPassRequest,
) -> Result<LayoutPassResult<DomHandle>, moli_layout::LayoutError> {
    let mut document_stack = Vec::new();
    build_native_layout_pass_recursive(
        runtime,
        root,
        services,
        embedded_document_services,
        request,
        &mut document_stack,
    )
}

fn build_native_layout_pass_recursive(
    runtime: &JsContextHost,
    root: DomHandle,
    services: &mut DocumentLayoutServices,
    embedded_document_services: &mut HashMap<DomHandle, DocumentLayoutServices>,
    request: LayoutPassRequest,
    document_stack: &mut Vec<DomHandle>,
) -> Result<LayoutPassResult<DomHandle>, moli_layout::LayoutError> {
    let document = runtime
        .dom_host()
        .owner_document_handle(root)
        .unwrap_or_else(|| runtime.document_handle());
    style_resolver::prepare_layout_style_inputs(runtime, root, document, request.viewport);
    document_stack.push(document);
    let source = source_view::NativeLayoutSourceView::with_paint_resources(
        runtime,
        root,
        request.requests_paint(),
    );
    let mut styles =
        style_resolver::NativeLayoutStyleResolver::new(runtime, document, request.viewport);
    let result = {
        let mut frames = NativeEmbeddedFrameRenderer {
            runtime,
            reason: request.reason,
            capture_paint: request.requests_paint(),
            include_backgrounds: request.includes_backgrounds(),
            document_stack,
            embedded_document_services,
        };
        build_layout_pass_with_embedded_frames(&source, &mut styles, services, request, &mut frames)
    };
    document_stack.pop();
    result
}

struct NativeEmbeddedFrameRenderer<'a> {
    runtime: &'a JsContextHost,
    reason: moli_layout::LayoutFlushReason,
    capture_paint: bool,
    include_backgrounds: bool,
    document_stack: &'a mut Vec<DomHandle>,
    embedded_document_services: &'a mut HashMap<DomHandle, DocumentLayoutServices>,
}

impl EmbeddedFrameRenderer<DomHandle> for NativeEmbeddedFrameRenderer<'_> {
    fn render_embedded_frame(
        &mut self,
        frame: DomHandle,
        viewport: LayoutViewport,
    ) -> Result<Option<moli_layout::EmbeddedFrameSnapshot<DomHandle>>, moli_layout::LayoutError>
    {
        const MAX_EMBEDDED_DOCUMENT_DEPTH: usize = 32;

        let Some(document) = self.runtime.child_browsing_context_document_handle(frame) else {
            return Ok(None);
        };
        if self.document_stack.len() >= MAX_EMBEDDED_DOCUMENT_DEPTH
            || self.document_stack.contains(&document)
        {
            return Ok(None);
        }
        let Some(root) = self
            .runtime
            .dom_host()
            .dom()
            .document_element_handle_for_document(document)
        else {
            return Ok(None);
        };
        let mut services = self
            .embedded_document_services
            .remove(&document)
            .unwrap_or_default();
        let request = if self.capture_paint {
            let mut capture = moli_layout::PaintCaptureRequest::viewport();
            capture.include_backgrounds = self.include_backgrounds;
            LayoutPassRequest::with_capture(viewport, self.reason, capture)
        } else {
            LayoutPassRequest::new(viewport, self.reason)
        };
        let result = build_native_layout_pass_recursive(
            self.runtime,
            root,
            &mut services,
            self.embedded_document_services,
            request,
            self.document_stack,
        );
        self.embedded_document_services.insert(document, services);
        let output = result?;
        let (tree, paint, css_image_references) = output.into_embedded_parts();
        Ok(Some(moli_layout::EmbeddedFrameSnapshot::new(
            tree,
            paint,
            css_image_references,
        )))
    }
}

#[cfg(test)]
pub(crate) fn build_normalized_native_box_tree_for_test(
    runtime: &JsContextHost,
    root: DomHandle,
) -> Result<moli_layout::NormalizedBoxTree, moli_layout::LayoutError> {
    let document = runtime
        .dom_host()
        .owner_document_handle(root)
        .unwrap_or_else(|| runtime.document_handle());
    let viewport = runtime.layout_viewport_for_document(document);
    style_resolver::prepare_layout_style_inputs(runtime, root, document, viewport);
    let source = source_view::NativeLayoutSourceView::new(runtime, root);
    let mut styles = style_resolver::NativeLayoutStyleResolver::new(runtime, document, viewport);
    moli_layout::build_layout_world(&source, &mut styles).map(|world| world.normalized_tree())
}
