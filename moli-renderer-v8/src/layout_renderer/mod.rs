mod inline_svg;
#[cfg(test)]
pub(crate) use inline_svg::parse_count as svg_parse_count;
mod source_view;
pub(crate) use source_view::native_scroll_offset;
mod style_resolver;

use std::collections::HashMap;

use moli_layout::{
    AutoScrollbars, DocumentLayoutServices, EmbeddedFrameRenderer, LayoutPassRequest,
    LayoutPassResult, LayoutViewport, finish_measured_layout_pass, measure_prepared_layout_pass,
    prepare_layout_pass,
};
use moli_selector::QueryContainerSizes;

use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};

/// Measured sizes are provisional until a Realtime screen pass publishes.
/// Captures and failed passes restore the previous tables for CSSOM reads.
pub(crate) struct ContainerSizeFeedback<'a> {
    host: &'a PageScriptHost,
    previous: HashMap<DomHandle, QueryContainerSizes>,
}

impl ContainerSizeFeedback<'_> {
    pub(crate) fn commit(mut self) {
        self.previous.clear();
    }
}

impl Drop for ContainerSizeFeedback<'_> {
    fn drop(&mut self) {
        for (document, sizes) in self.previous.drain() {
            let _ = self.host.replace_layout_container_sizes(document, sizes);
        }
    }
}

pub(crate) fn build_native_layout_pass<'a>(
    runtime: &'a PageScriptHost,
    root: DomHandle,
    services: &mut DocumentLayoutServices,
    embedded_document_services: &mut HashMap<DomHandle, DocumentLayoutServices>,
    request: LayoutPassRequest,
    environment_override: Option<crate::style_engine::StyloStyleEnvironment>,
) -> Result<(LayoutPassResult<DomHandle>, ContainerSizeFeedback<'a>), moli_layout::LayoutError> {
    let mut feedback = ContainerSizeFeedback {
        host: runtime,
        previous: HashMap::new(),
    };
    let pass = build_native_layout_pass_recursive(
        runtime,
        root,
        services,
        embedded_document_services,
        request,
        environment_override,
        &mut Vec::new(),
        &mut feedback.previous,
    )?;
    Ok((pass, feedback))
}

fn build_native_layout_pass_recursive(
    runtime: &PageScriptHost,
    root: DomHandle,
    services: &mut DocumentLayoutServices,
    embedded_document_services: &mut HashMap<DomHandle, DocumentLayoutServices>,
    request: LayoutPassRequest,
    environment_override: Option<crate::style_engine::StyloStyleEnvironment>,
    document_stack: &mut Vec<DomHandle>,
    original_container_sizes: &mut HashMap<DomHandle, QueryContainerSizes>,
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
        style_resolver::layout_style_read_scope(
            runtime,
            document,
            request.viewport,
            environment_override,
        ),
        request.requests_paint(),
    );
    let source_ready = profile_started.map(|_| std::time::Instant::now());
    let mut styles_ready = None;
    let result = (|| {
        let mut prepare = || {
            style_resolver::with_layout_styles(
                runtime,
                document,
                request.viewport,
                environment_override,
                |styles| {
                    if profile_started.is_some() && styles_ready.is_none() {
                        styles_ready = Some(std::time::Instant::now());
                    }
                    prepare_layout_pass(&source, styles, request)
                },
            )
        };
        let prepared = prepare()?;
        let mut frames = NativeEmbeddedFrameRenderer {
            runtime,
            reason: request.reason,
            capture_paint: request.requests_paint(),
            include_backgrounds: request.includes_backgrounds(),
            environment_override,
            frame_coverage: request.frame_coverage(),
            retain_scroll_projection: request.retains_scroll_projection(),
            document_stack,
            embedded_document_services,
            original_container_sizes,
        };
        let finish = |services: &mut DocumentLayoutServices| {
            let mut measured = measure_prepared_layout_pass(prepared, services)?;
            let mut iterations = 0usize;
            let mut largest_container_count = 0;
            let mut previous_scrollbars = None;
            let mut scrollbar_transitions =
                Vec::<(QueryContainerSizes, AutoScrollbars<DomHandle>)>::new();
            let mut frozen_scrollbars = AutoScrollbars::default();
            let mut changed_scrollbar_axes = AutoScrollbars::default();
            let mut last_scrollbar_progress = 0;
            loop {
                let sizes: QueryContainerSizes = measured
                    .query_container_sizes()
                    .map(|(handle, [width, height])| {
                        (
                            handle,
                            euclid::default::Size2D::new(
                                width.map(app_units::Au::from_f32_px),
                                height.map(app_units::Au::from_f32_px),
                            ),
                        )
                    })
                    .collect();
                largest_container_count = largest_container_count.max(sizes.len());
                let repeated = scrollbar_transitions
                    .iter()
                    .rposition(|(earlier, _)| earlier == &sizes);
                let Some(previous) = runtime.replace_layout_container_sizes(document, sizes) else {
                    break;
                };
                largest_container_count = largest_container_count.max(previous.len());
                let scrollbars = measured.auto_scrollbars();
                if let Some((earlier, _)) = &previous_scrollbars
                    && changed_scrollbar_axes.extend_changed_axes(earlier, &scrollbars)
                {
                    last_scrollbar_progress = iterations;
                }
                // Keep only the size tables measured at a change in scrollbar
                // visibility. An exact repeat identifies a feedback cycle, and
                // only the gutters that toggled within it need to be frozen.
                let changed = previous_scrollbars
                    .as_ref()
                    .is_none_or(|(earlier, _)| earlier != &scrollbars);
                match previous_scrollbars.replace((scrollbars, changed)) {
                    Some((earlier, true)) => scrollbar_transitions.push((previous, earlier)),
                    Some((_, false)) => {}
                    None => {
                        frames
                            .original_container_sizes
                            .entry(document)
                            .or_insert(previous);
                    }
                }
                if let Some(start) = repeated
                    && let Some((current, record_current)) = &mut previous_scrollbars
                {
                    for (_, earlier) in scrollbar_transitions.drain(start..) {
                        if frozen_scrollbars.extend_changed_axes(current, &earlier) {
                            last_scrollbar_progress = iterations;
                        }
                    }
                    scrollbar_transitions.clear();
                    // The next pass starts a new history with the frozen gutters.
                    *record_current = false;
                }
                // Each new scrollbar axis or frozen gutter can start another
                // propagation through the container chain. Observing the same
                // axis toggle again grants no additional rounds.
                let budget = largest_container_count.saturating_mul(2).saturating_add(1);
                if iterations.saturating_sub(last_scrollbar_progress) >= budget {
                    return Err(
                        moli_layout::LayoutError::ContainerQuerySizesDidNotConverge { iterations },
                    );
                }
                iterations += 1;
                measured = measured.remeasure(services, &frozen_scrollbars, &mut prepare)?;
            }
            finish_measured_layout_pass(measured, &source, &mut frames)
        };
        if services.has_stylesheet_web_fonts() {
            let resources = runtime
                .stylesheet_resource_snapshot_for_document(document)
                .ok_or_else(|| {
                    moli_layout::LayoutError::source_contract(
                        "layout fonts",
                        "missing current stylesheet resource manifest",
                    )
                })?;
            services.with_stylesheet_font_declarations(
                resources.web_fonts().iter().filter_map(|resource| {
                    let font = resource.web_font()?;
                    Some(moli_layout::WebFontDeclaration {
                        slot: font.registration_slot(),
                        source_key: font.resource_key(),
                        face: font.face(),
                    })
                }),
                finish,
            )
        } else {
            finish(services)
        }
    })();
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
    document_stack.pop();
    result
}

struct NativeEmbeddedFrameRenderer<'a> {
    runtime: &'a PageScriptHost,
    reason: moli_layout::LayoutFlushReason,
    capture_paint: bool,
    include_backgrounds: bool,
    environment_override: Option<crate::style_engine::StyloStyleEnvironment>,
    frame_coverage: moli_layout::LayoutFrameCoverage,
    retain_scroll_projection: bool,
    document_stack: &'a mut Vec<DomHandle>,
    embedded_document_services: &'a mut HashMap<DomHandle, DocumentLayoutServices>,
    original_container_sizes: &'a mut HashMap<DomHandle, QueryContainerSizes>,
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
        .with_scroll_projection(self.retain_scroll_projection);
        let result = build_native_layout_pass_recursive(
            self.runtime,
            root,
            &mut services,
            self.embedded_document_services,
            request,
            self.environment_override,
            self.document_stack,
            self.original_container_sizes,
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
    runtime: &PageScriptHost,
    root: DomHandle,
) -> Result<moli_layout::NormalizedBoxTree, moli_layout::LayoutError> {
    let document = runtime
        .dom_host()
        .owner_document_handle(root)
        .unwrap_or_else(|| runtime.document_handle());
    let viewport = runtime.layout_viewport_for_document(document);
    let source = source_view::NativeLayoutSourceView::new(runtime, root);
    style_resolver::with_layout_styles(runtime, document, viewport, None, |styles| {
        moli_layout::build_layout_world(&source, styles).map(|world| world.normalized_tree())
    })
}
