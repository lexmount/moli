mod inline_svg;
mod source_view;
mod style_resolver;

use std::collections::HashMap;

use moli_layout::{
    DocumentLayoutServices, EmbeddedFrameRenderer, LayoutPassRequest, LayoutPassResult,
    LayoutViewport, build_layout_pass_with_embedded_frames,
};

use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

pub(crate) fn build_native_layout_pass(
    runtime: &JsContextHost,
    root: DomHandle,
    services: &mut DocumentLayoutServices,
    embedded_document_services: &mut HashMap<DomHandle, DocumentLayoutServices>,
    request: LayoutPassRequest,
    environment_override: Option<crate::style_engine::StyloStyleEnvironment>,
) -> Result<LayoutPassResult<DomHandle>, moli_layout::LayoutError> {
    let mut document_stack = Vec::new();
    build_native_layout_pass_recursive(
        runtime,
        root,
        services,
        embedded_document_services,
        request,
        environment_override,
        &mut document_stack,
    )
}

fn build_native_layout_pass_recursive(
    runtime: &JsContextHost,
    root: DomHandle,
    services: &mut DocumentLayoutServices,
    embedded_document_services: &mut HashMap<DomHandle, DocumentLayoutServices>,
    request: LayoutPassRequest,
    environment_override: Option<crate::style_engine::StyloStyleEnvironment>,
    document_stack: &mut Vec<DomHandle>,
) -> Result<LayoutPassResult<DomHandle>, moli_layout::LayoutError> {
    let profile_started = moli_trace::cpu_profile_enabled().then(std::time::Instant::now);
    let document = runtime
        .dom_host()
        .owner_document_handle(root)
        .unwrap_or_else(|| runtime.document_handle());
    let request = request.with_environment(moli_layout::LayoutEnvironment {
        scrollbars_hidden: runtime.scrollbars_hidden(),
    });
    document_stack.push(document);
    let source = source_view::NativeLayoutSourceView::with_paint_resources(
        runtime,
        root,
        request.requests_paint(),
    );
    let source_ready = profile_started.map(|_| std::time::Instant::now());
    let mut styles = style_resolver::NativeLayoutStyleResolver::new(
        runtime,
        root,
        document,
        request.viewport,
        environment_override,
    );
    let styles_ready = profile_started.map(|_| std::time::Instant::now());
    let result = {
        let mut frames = NativeEmbeddedFrameRenderer {
            runtime,
            reason: request.reason,
            capture_paint: request.requests_paint(),
            include_backgrounds: request.includes_backgrounds(),
            environment_override,
            document_stack,
            embedded_document_services,
        };
        if services.has_stylesheet_web_fonts() {
            if let Some(resources) = styles.stylesheet_resources(root) {
                services.with_stylesheet_font_declarations(
                    resources.web_fonts().iter().filter_map(|resource| {
                        let font = resource.web_font()?;
                        Some(moli_layout::WebFontDeclaration {
                            slot: font.registration_slot(),
                            source_key: font.resource_key(),
                            face: font.face(),
                        })
                    }),
                    |services| {
                        build_layout_pass_with_embedded_frames(
                            &source,
                            &mut styles,
                            services,
                            request,
                            &mut frames,
                        )
                    },
                )
            } else {
                Err(moli_layout::LayoutError::source_contract(
                    "layout fonts",
                    "missing current stylesheet resource manifest",
                ))
            }
        } else {
            build_layout_pass_with_embedded_frames(
                &source,
                &mut styles,
                services,
                request,
                &mut frames,
            )
        }
    };
    if let (Some(started), Some(source_ready), Some(styles_ready)) =
        (profile_started, source_ready, styles_ready)
    {
        let finished = std::time::Instant::now();
        tracing::info!(
            target: "moli_cpu_profile",
            stage = "layout_native_adapter",
            document = document.index_u32(),
            root = ?root,
            reason = ?request.reason,
            total_us = finished.duration_since(started).as_micros(),
            source_setup_us = source_ready.duration_since(started).as_micros(),
            style_setup_us = styles_ready.duration_since(source_ready).as_micros(),
            backend_us = finished.duration_since(styles_ready).as_micros(),
            backend_reported_us = result.as_ref().map(|pass| pass.metrics.elapsed.as_micros()).ok(),
            success = result.is_ok(),
        );
    }
    styles.trace_profile(document);
    document_stack.pop();
    result
}

struct NativeEmbeddedFrameRenderer<'a> {
    runtime: &'a JsContextHost,
    reason: moli_layout::LayoutFlushReason,
    capture_paint: bool,
    include_backgrounds: bool,
    environment_override: Option<crate::style_engine::StyloStyleEnvironment>,
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
            capture.base_background_color = moli_layout::PaintColor::TRANSPARENT;
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
            self.environment_override,
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
    let source = source_view::NativeLayoutSourceView::new(runtime, root);
    let mut styles =
        style_resolver::NativeLayoutStyleResolver::new(runtime, root, document, viewport, None);
    moli_layout::build_layout_world(&source, &mut styles).map(|world| world.normalized_tree())
}
