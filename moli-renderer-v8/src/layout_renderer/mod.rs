mod container_feedback;
mod inline_svg;
mod screen_styles;
#[cfg(test)]
pub(crate) use inline_svg::parse_count as svg_parse_count;
#[cfg(feature = "devtools")]
pub(crate) use inline_svg::{SvgGeometry, prepare_svg_paint};
pub(crate) use inline_svg::{
    decode_geometry as decode_frozen_svg_geometry, pending_svg_geometry, prepare_svg_geometry,
};
pub(crate) use screen_styles::screen_targets_allow_layout_reuse;
mod source_view;
#[cfg(feature = "devtools")]
pub(crate) use source_view::native_replaced_image;
pub(crate) use source_view::native_scroll_offset;
mod style_resolver;

use std::{cell::RefCell, collections::HashMap};

use moli_layout::{
    DocumentLayoutServices, EmbeddedFrameRenderer, LayoutPassRequest, LayoutPassResult,
    LayoutViewport, RetainedLayoutWorld, finish_measured_layout_pass, measure_prepared_layout_pass,
    prepare_layout_pass_reusing,
};
use moli_page_types::ViewportSurface;
use moli_selector::QueryContainerSizes;

use crate::{
    document_runtime::DomHandle,
    native_bridge::{CssImagePaintSnapshot, PageScriptHost},
    style_engine::ScreenStyleCoverage,
};
use container_feedback::ContainerQueryFeedback;

pub(crate) fn layout_viewport_for_surface(surface: ViewportSurface) -> LayoutViewport {
    let (width, height) = surface.css_layout_size();
    let viewport = LayoutViewport::new(width, height, surface.device_pixel_ratio as f32);
    if surface.mobile_viewport().is_some() {
        viewport.with_mobile_frame_size(surface.inner_width, surface.inner_height)
    } else {
        viewport
    }
}

/// Measured sizes are provisional until a Realtime screen pass publishes.
/// Captures and failed passes restore the previous tables for CSSOM reads.
pub(crate) struct LayoutPassFeedback<'a> {
    host: &'a PageScriptHost,
    previous: HashMap<DomHandle, QueryContainerSizes>,
    screen_style_coverage: Option<HashMap<DomHandle, ScreenStyleCoverage>>,
}

impl LayoutPassFeedback<'_> {
    pub(crate) fn commit(mut self) -> HashMap<DomHandle, ScreenStyleCoverage> {
        self.previous.clear();
        self.screen_style_coverage.take().unwrap_or_default()
    }
}

impl Drop for LayoutPassFeedback<'_> {
    fn drop(&mut self) {
        for (document, sizes) in self.previous.drain() {
            let _ = self.host.replace_layout_container_sizes(document, sizes);
        }
    }
}

/// Layout consumers choose how SVG and style inputs survive the pass.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum LayoutInputRetention {
    Transient,
    Frozen,
    Realtime,
}

pub(crate) fn build_native_layout_pass<'a>(
    runtime: &'a PageScriptHost,
    root: DomHandle,
    services: &mut DocumentLayoutServices,
    embedded_document_services: &mut HashMap<DomHandle, DocumentLayoutServices>,
    request: LayoutPassRequest,
    environment_override: Option<crate::style_engine::StyloStyleEnvironment>,
    inputs: LayoutInputRetention,
    retained: Option<Box<RetainedLayoutWorld<DomHandle>>>,
) -> Result<(LayoutPassResult<DomHandle>, LayoutPassFeedback<'a>), moli_layout::LayoutError> {
    let mut feedback = LayoutPassFeedback {
        host: runtime,
        previous: HashMap::new(),
        screen_style_coverage: (inputs == LayoutInputRetention::Realtime).then(HashMap::new),
    };
    let css_images = request
        .requests_paint()
        .then(|| runtime.css_image_paint_snapshot());
    let pass = build_native_layout_pass_recursive(
        runtime,
        root,
        services,
        embedded_document_services,
        request,
        css_images.as_ref(),
        environment_override,
        &mut Vec::new(),
        &mut feedback.previous,
        &mut feedback.screen_style_coverage,
        inputs == LayoutInputRetention::Frozen,
        retained,
    )?;
    Ok((pass, feedback))
}

fn build_native_layout_pass_recursive(
    runtime: &PageScriptHost,
    root: DomHandle,
    services: &mut DocumentLayoutServices,
    embedded_document_services: &mut HashMap<DomHandle, DocumentLayoutServices>,
    request: LayoutPassRequest,
    css_images: Option<&CssImagePaintSnapshot>,
    environment_override: Option<crate::style_engine::StyloStyleEnvironment>,
    document_stack: &mut Vec<DomHandle>,
    original_container_sizes: &mut HashMap<DomHandle, QueryContainerSizes>,
    screen_style_coverage: &mut Option<HashMap<DomHandle, ScreenStyleCoverage>>,
    freeze_svg_inputs: bool,
    mut retained: Option<Box<RetainedLayoutWorld<DomHandle>>>,
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
        request.requests_paint() || request.retains_screen_paint(),
        css_images,
    );
    let source_ready = profile_started.map(|_| std::time::Instant::now());
    let mut styles_ready = None;
    let collect_screen_styles = screen_style_coverage.is_some();
    let mut document_coverage = collect_screen_styles.then(ScreenStyleCoverage::default);
    let mut retained_frames = retained
        .as_deref_mut()
        .map_or_else(HashMap::new, RetainedLayoutWorld::take_embedded_layouts);
    let result = (|| {
        let mut prepare = || {
            style_resolver::with_layout_styles(
                runtime,
                document,
                request.viewport,
                environment_override,
                document_coverage.as_mut(),
                |styles| {
                    if profile_started.is_some() && styles_ready.is_none() {
                        styles_ready = Some(std::time::Instant::now());
                    }
                    let changes = if retained.is_some() && !source.has_text_selection() {
                        runtime.retained_box_tree_changes(document)
                    } else {
                        None
                    };
                    if collect_screen_styles {
                        runtime.begin_screen_style_layout(document);
                    }
                    prepare_layout_pass_reusing(
                        &source,
                        styles,
                        request,
                        retained.take(),
                        changes.as_deref(),
                    )
                },
            )
        };
        let prepared = prepare()?;
        let mut frames = NativeEmbeddedFrameRenderer {
            runtime,
            reason: request.reason,
            css_images,
            environment_override,
            frame_coverage: request.frame_coverage(),
            retain_scroll_projection: request.retains_scroll_projection(),
            retain_screen_paint: request.retains_screen_paint(),
            freeze_svg_inputs,
            retain_layout_world: request.retains_layout_world(),
            retained_frames: &mut retained_frames,
            document_stack,
            embedded_document_services,
            original_container_sizes,
            screen_style_coverage,
        };
        let finish = |services: &mut DocumentLayoutServices| {
            let mut measured = measure_prepared_layout_pass(prepared, services)?;
            let mut feedback = ContainerQueryFeedback::default();
            let mut previous_scrollbars = None;
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
                let Some(previous) = runtime.replace_layout_container_sizes(document, sizes) else {
                    break;
                };
                let froze_scrollbars = match previous_scrollbars.take() {
                    Some(scrollbars) => feedback.observe(previous, scrollbars)?,
                    None => {
                        frames
                            .original_container_sizes
                            .entry(document)
                            .or_insert(previous);
                        false
                    }
                };
                // The size table returned on the next swap describes this
                // measurement. Start a fresh checkpoint after changing gutters.
                previous_scrollbars = (!froze_scrollbars).then(|| measured.auto_scrollbars());
                feedback.reserve_rebuild(measured.box_count())?;
                measured =
                    measured.remeasure(services, &feedback.frozen_scrollbars, &mut prepare)?;
            }
            let freeze_input = |node, style: &moli_layout::ResolvedLayoutStyle, size| {
                source.svg_input(node, style, size)
            };
            let paint_geometry = RefCell::new(Vec::new());
            let publishes_geometry = freeze_svg_inputs || collect_screen_styles;
            let svg_image =
                |_,
                 _,
                 node,
                 style: &moli_layout::ResolvedLayoutStyle,
                 size,
                 input: Option<&moli_layout::InlineSvgInput<DomHandle>>| {
                    let decode =
                        |input| inline_svg::decode_paint(node, input, size, publishes_geometry);
                    let decoded = if let Some(input) = input {
                        decode(input)
                    } else {
                        source
                            .svg_input(node, style, size)
                            .as_ref()
                            .and_then(decode)
                    };
                    let (image, objects) = decoded.map_or_else(
                        || (None, Vec::new()),
                        |(image, objects)| (Some(image), objects),
                    );
                    // A screen geometry query can reuse the scene parsed for paint.
                    if publishes_geometry {
                        paint_geometry.borrow_mut().push((node, objects));
                    }
                    image
                };
            let mut pass = finish_measured_layout_pass(
                measured,
                &source,
                &mut frames,
                moli_layout::InlineSvgConsumers {
                    freeze: freeze_svg_inputs.then_some(&freeze_input),
                    paint: Some(&svg_image),
                },
            )?;
            let geometry = paint_geometry.into_inner();
            if !geometry.is_empty() {
                pass.tree.append_svg_geometry(root, geometry)?;
            }
            Ok(pass)
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
    if result.is_ok()
        && let (Some(coverage), Some(document_coverage)) =
            (screen_style_coverage.as_mut(), document_coverage)
    {
        coverage.insert(document, document_coverage);
    }
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
    css_images: Option<&'a CssImagePaintSnapshot>,
    environment_override: Option<crate::style_engine::StyloStyleEnvironment>,
    frame_coverage: moli_layout::LayoutFrameCoverage,
    retain_scroll_projection: bool,
    retain_screen_paint: bool,
    freeze_svg_inputs: bool,
    retain_layout_world: bool,
    retained_frames: &'a mut HashMap<
        DomHandle,
        (
            moli_layout::LayoutBoxId,
            Box<RetainedLayoutWorld<DomHandle>>,
        ),
    >,
    document_stack: &'a mut Vec<DomHandle>,
    embedded_document_services: &'a mut HashMap<DomHandle, DocumentLayoutServices>,
    original_container_sizes: &'a mut HashMap<DomHandle, QueryContainerSizes>,
    screen_style_coverage: &'a mut Option<HashMap<DomHandle, ScreenStyleCoverage>>,
}

impl EmbeddedFrameRenderer<DomHandle> for NativeEmbeddedFrameRenderer<'_> {
    fn render_embedded_frame(
        &mut self,
        frame: DomHandle,
        viewport: LayoutViewport,
        paint: Option<moli_layout::EmbeddedFramePaintRequest>,
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
        let request = if let Some(paint) = paint
            && viewport.css_width > 0
            && viewport.css_height > 0
        {
            let capture = moli_layout::PaintCaptureRequest {
                include_backgrounds: paint.include_backgrounds,
                base_background_color: moli_layout::PaintColor::TRANSPARENT,
                ..moli_layout::PaintCaptureRequest::viewport()
            };
            LayoutPassRequest::with_capture(viewport, self.reason, capture)
                .with_paint_cull_rect(paint.cull_rect)
        } else {
            LayoutPassRequest::new(viewport, self.reason)
        }
        .with_frame_coverage(self.frame_coverage)
        .with_scroll_projection(self.retain_scroll_projection)
        .with_screen_paint(
            self.retain_screen_paint && viewport.css_width > 0 && viewport.css_height > 0,
        );
        let request = if self.retain_layout_world {
            request.with_retained_layout_world()
        } else {
            request
        };
        let retained = self
            .retained_frames
            .remove(&frame)
            .map(|(_, layout)| layout);
        let result = build_native_layout_pass_recursive(
            self.runtime,
            root,
            &mut services,
            self.embedded_document_services,
            request,
            self.css_images,
            self.environment_override,
            self.document_stack,
            self.original_container_sizes,
            self.screen_style_coverage,
            self.freeze_svg_inputs,
            retained,
        );
        self.embedded_document_services.insert(document, services);
        let output = result?;
        Ok(Some(output.into_embedded_snapshot()))
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
    style_resolver::with_layout_styles(runtime, document, viewport, None, None, |styles| {
        moli_layout::build_layout_world(&source, styles).map(|world| world.normalized_tree())
    })
}
