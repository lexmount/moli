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
        capture_overlay: None,
        capture_discovery: None,
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

pub(crate) fn build_native_capture_layout_pass(
    runtime: &JsContextHost,
    root: DomHandle,
    services: &mut DocumentLayoutServices,
    embedded_document_services: &mut HashMap<DomHandle, DocumentLayoutServices>,
    request: LayoutPassRequest,
    environment_override: Option<crate::style_engine::StyloStyleEnvironment>,
    resources: crate::capture_layout_resources::CaptureLayoutPass<'_>,
) -> Result<LayoutPassResult<DomHandle>, moli_layout::LayoutError> {
    let mut tree = NativeLayoutPassTree {
        document_stack: Vec::new(),
        working_sets: None,
        capture_overlay: Some(resources.overlay),
        capture_discovery: Some(resources.discovery),
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
    capture_overlay: Option<&'a crate::capture_layout_resources::CaptureLayoutOverlay>,
    capture_discovery: Option<&'a mut crate::capture_layout_resources::CaptureLayoutDiscovery>,
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
    let capture_identity = tree
        .capture_overlay
        .map(|_| {
            runtime
                .capture_layout_identity_for_document(document)
                .filter(|identity| identity.root == root)
                .ok_or_else(|| {
                    moli_layout::LayoutError::source_contract(
                        "capture member",
                        "recursive capture Document has no current exact owner",
                    )
                })
        })
        .transpose()?;
    let source = source_view::NativeLayoutSourceView::with_paint_resources(
        runtime,
        root,
        request.requests_paint(),
    );
    let source = match (tree.capture_overlay, capture_identity) {
        (Some(overlay), Some(identity)) => source.with_capture_images(overlay, identity),
        _ => source,
    };
    let source_ready = profile_started.map(|_| std::time::Instant::now());
    let mut styles = style_resolver::NativeLayoutStyleResolver::new(
        runtime,
        root,
        document,
        request.viewport,
        environment_override,
    );
    // Capture discovery must inspect the active manifest even when no live
    // CSS font has been downloaded. Use this pass's exact child viewport.
    let mut capture_services =
        if let (Some(overlay), Some(identity)) = (tree.capture_overlay, capture_identity) {
            let fonts = styles.stylesheet_resources(root).ok_or_else(|| {
                moli_layout::LayoutError::source_contract(
                    "capture fonts",
                    "missing current stylesheet resource manifest",
                )
            })?;
            let active = fonts
                .web_fonts()
                .iter()
                .filter_map(|resource| resource.web_font().map(|font| font.registration_slot()))
                .collect::<std::collections::HashSet<_>>();
            let temporary = overlay
                .font_services_for_document(identity, services, |slot| active.contains(slot));
            // Keep selected registrations alive across unrelated resource
            // awaits; a later live reconciliation may legitimately prune them.
            let available_fonts =
                crate::capture_layout_resources::CaptureAvailableFonts::from_selected(&temporary);
            let available_font_slots = active
                .into_iter()
                .filter(|slot| temporary.web_font_decoded_bytes(slot).is_some())
                .map(str::to_owned)
                .collect();
            if let Some(discovery) = tree.capture_discovery.as_deref_mut() {
                discovery
                    .members
                    .push(crate::capture_layout_resources::CaptureLayoutMember {
                        identity,
                        viewport: request.viewport,
                        fonts,
                        available_font_slots,
                        available_fonts,
                    });
            }
            Some(temporary)
        } else {
            None
        };
    let styles_ready = profile_started.map(|_| std::time::Instant::now());
    let result = {
        let mut frames = NativeEmbeddedFrameRenderer {
            runtime,
            reason: request.reason,
            capture_paint: request.requests_paint(),
            include_backgrounds: request.includes_backgrounds(),
            environment_override,
            frame_coverage: request.frame_coverage(),
            tree,
            embedded_document_services,
        };
        if let Some(services) = capture_services.as_mut() {
            build_layout_pass_with_working_set(
                &source,
                &mut styles,
                services,
                request,
                &mut frames,
                None,
            )
        } else if services.has_stylesheet_web_fonts() {
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
    environment_override: Option<crate::style_engine::StyloStyleEnvironment>,
    frame_coverage: moli_layout::LayoutFrameCoverage,
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
        .with_frame_coverage(self.frame_coverage);
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
