mod inline_svg;
mod source_view;
mod style_resolver;
mod working_sets;

use std::collections::HashMap;

use moli_layout::{
    DocumentLayoutServices, EmbeddedFrameRenderer, LayoutPassRequest, LayoutPassResult,
    LayoutViewport, build_layout_pass_with_working_set,
};

pub(crate) use working_sets::{
    NativeLayoutWorkingSetOwner, NativeLayoutWorkingSetPass, NativeLayoutWorkingSets,
};

use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

pub(crate) fn current_native_stylesheet_resources(
    runtime: &JsContextHost,
    root: DomHandle,
) -> Option<crate::style_engine::StylesheetResourceSnapshot> {
    let mut reads = crate::native_bridge::element::StyleObservation::new(runtime);
    reads.read(root).stylesheet_resource_snapshot()
}

pub(crate) fn build_native_layout_pass(
    runtime: &JsContextHost,
    root: DomHandle,
    services: &mut DocumentLayoutServices,
    embedded_document_services: &mut HashMap<DomHandle, DocumentLayoutServices>,
    request: LayoutPassRequest,
    environment_override: Option<crate::style_engine::StyloStyleEnvironment>,
    working_sets: Option<&mut NativeLayoutWorkingSetPass>,
) -> Result<LayoutPassResult<DomHandle>, moli_layout::LayoutError> {
    let mut tree = NativeLayoutPassTree {
        document_stack: Vec::new(),
        working_sets,
    };
    build_native_layout_pass_recursive(
        runtime,
        root,
        services,
        embedded_document_services,
        request,
        environment_override,
        &mut tree,
    )
}

struct NativeLayoutPassTree<'a> {
    document_stack: Vec<DomHandle>,
    working_sets: Option<&'a mut NativeLayoutWorkingSetPass>,
}

fn build_native_layout_pass_recursive(
    runtime: &JsContextHost,
    root: DomHandle,
    services: &mut DocumentLayoutServices,
    embedded_document_services: &mut HashMap<DomHandle, DocumentLayoutServices>,
    request: LayoutPassRequest,
    environment_override: Option<crate::style_engine::StyloStyleEnvironment>,
    tree: &mut NativeLayoutPassTree<'_>,
) -> Result<LayoutPassResult<DomHandle>, moli_layout::LayoutError> {
    let profile_started = moli_trace::cpu_profile_enabled().then(std::time::Instant::now);
    let document = runtime
        .dom_host()
        .owner_document_handle(root)
        .unwrap_or_else(|| runtime.document_handle());
    let request = request.with_environment(moli_layout::LayoutEnvironment {
        scrollbars_hidden: runtime.scrollbars_hidden(),
    });
    tree.document_stack.push(document);
    let mut working_set = tree
        .working_sets
        .as_deref_mut()
        .and_then(|working_sets| working_sets.take_document(document, root));
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
            frame_coverage: request.frame_coverage(),
            text_preparation_reuse: request.text_preparation_reuse(),
            environment_override,
            tree,
            embedded_document_services,
        };
        if services.has_stylesheet_web_fonts() {
            if let Some(resources) = styles.stylesheet_resources(root) {
                let active = resources
                    .web_fonts()
                    .iter()
                    .filter_map(|resource| resource.web_font().map(|font| font.slot()))
                    .collect::<std::collections::HashSet<_>>();
                services.with_active_stylesheet_fonts(
                    |slot| active.contains(slot),
                    |services| {
                        build_layout_pass_with_working_set(
                            &source,
                            &mut styles,
                            services,
                            request,
                            &mut frames,
                            working_set.as_mut().map(|entry| &mut entry.working_set),
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
            build_layout_pass_with_working_set(
                &source,
                &mut styles,
                services,
                request,
                &mut frames,
                working_set.as_mut().map(|entry| &mut entry.working_set),
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
    tree.document_stack.pop();
    if result.is_ok()
        && let Some(working_set) = working_set
        && let Some(working_sets) = tree.working_sets.as_deref_mut()
    {
        working_sets.complete_document(working_set);
    }
    result
}

struct NativeEmbeddedFrameRenderer<'a, 'working> {
    runtime: &'a JsContextHost,
    reason: moli_layout::LayoutFlushReason,
    capture_paint: bool,
    include_backgrounds: bool,
    frame_coverage: moli_layout::LayoutFrameCoverage,
    text_preparation_reuse: bool,
    environment_override: Option<crate::style_engine::StyloStyleEnvironment>,
    tree: &'a mut NativeLayoutPassTree<'working>,
    embedded_document_services: &'a mut HashMap<DomHandle, DocumentLayoutServices>,
}

impl EmbeddedFrameRenderer<DomHandle> for NativeEmbeddedFrameRenderer<'_, '_> {
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
        if self.tree.document_stack.len() >= MAX_EMBEDDED_DOCUMENT_DEPTH
            || self.tree.document_stack.contains(&document)
        {
            if self.frame_coverage == moli_layout::LayoutFrameCoverage::AllLayoutBoxes {
                return Err(moli_layout::LayoutError::source_contract(
                    "embedded frame coverage",
                    "required Document exceeds the recursive frame depth or repeats an ancestor",
                ));
            }
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
        // A zero viewport has observable descendant geometry but no paint
        // surface. Preserve the recursive coverage without allocating one.
        let request = if self.capture_paint && viewport.css_width > 0 && viewport.css_height > 0 {
            let mut capture = moli_layout::PaintCaptureRequest::viewport();
            capture.include_backgrounds = self.include_backgrounds;
            capture.base_background_color = moli_layout::PaintColor::TRANSPARENT;
            LayoutPassRequest::with_capture(viewport, self.reason, capture)
        } else {
            LayoutPassRequest::new(viewport, self.reason)
        }
        .with_frame_coverage(self.frame_coverage)
        .with_text_preparation_reuse(self.text_preparation_reuse);
        let result = build_native_layout_pass_recursive(
            self.runtime,
            root,
            &mut services,
            self.embedded_document_services,
            request,
            self.environment_override,
            self.tree,
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
