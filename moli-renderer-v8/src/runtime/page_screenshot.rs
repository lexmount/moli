use moli_layout::{
    LayoutRect, LayoutTransform2D, PaintCaptureRegion, PaintCaptureRequest, PaintCaptureSurface,
    PaintViewport,
};
use moli_page_types::{LayoutPolicy, ViewportSurface};
use std::sync::Arc;
use std::time::Instant;

use super::{
    PageVm, RendererCaptureScreencastFrameReply, RendererCaptureScreenshotReply,
    RendererCapturedScreencastFrame, RendererCapturedScreenshot, RendererDocumentLifecycleIdentity,
};

#[derive(Default)]
struct PaintFragmentProfile {
    push_layer: usize,
    push_clip: usize,
    pop_layer: usize,
    fill: usize,
    stroke: usize,
    border: usize,
    box_shadow: usize,
    text_decoration: usize,
    text_shadow: usize,
    glyph_run: usize,
    glyph: usize,
    image: usize,
    svg_image: usize,
}

impl PaintFragmentProfile {
    fn from_snapshot(snapshot: &moli_layout::PaintSnapshot) -> Self {
        let mut profile = Self::default();
        for fragment in &snapshot.fragments {
            match fragment {
                moli_layout::PaintFragment::PushLayer { .. } => profile.push_layer += 1,
                moli_layout::PaintFragment::PushClip { .. } => profile.push_clip += 1,
                moli_layout::PaintFragment::PopLayer => profile.pop_layer += 1,
                moli_layout::PaintFragment::Fill { .. } => profile.fill += 1,
                moli_layout::PaintFragment::Stroke(_) => profile.stroke += 1,
                moli_layout::PaintFragment::Border { .. } => profile.border += 1,
                moli_layout::PaintFragment::BoxShadow(_) => profile.box_shadow += 1,
                moli_layout::PaintFragment::TextDecoration(_) => profile.text_decoration += 1,
                moli_layout::PaintFragment::TextShadow(shadow) => {
                    profile.text_shadow += 1;
                    profile.glyph += shadow.run.glyphs.len();
                }
                moli_layout::PaintFragment::GlyphRun(run) => {
                    profile.glyph_run += 1;
                    profile.glyph += run.glyphs.len();
                }
                moli_layout::PaintFragment::Image(_) => profile.image += 1,
                moli_layout::PaintFragment::SvgImage(_) => profile.svg_image += 1,
            }
        }
        profile
    }
}

/// Opaque identity for the renderer state that can affect one viewport frame.
///
/// The token retains generation metadata only. It never owns layout, paint,
/// raster, or encoded-image data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererVisualStateToken(Arc<RendererVisualState>);

#[derive(Debug, PartialEq, Eq)]
struct RendererVisualState {
    document: RendererDocumentLifecycleIdentity,
    dom_generation: u64,
    style_generations: Vec<(u32, u64, u64, u64)>,
    interaction_generation: u64,
    resource_generation: u64,
    viewport_width: u32,
    viewport_height: u32,
    device_pixel_ratio_bits: u32,
    base_background_color: [u8; 4],
}

impl RendererVisualStateToken {
    pub(crate) fn new(
        document: RendererDocumentLifecycleIdentity,
        dom_generation: u64,
        style_generations: Vec<(crate::document_runtime::DomHandle, u64, u64, u64)>,
        interaction_generation: u64,
        resource_generation: u64,
        viewport: PaintViewport,
        base_background_color: [u8; 4],
    ) -> Self {
        Self(Arc::new(RendererVisualState {
            document,
            dom_generation,
            style_generations: style_generations
                .into_iter()
                .map(|(document, source, computed, context)| {
                    (document.index_u32(), source, computed, context)
                })
                .collect(),
            interaction_generation,
            resource_generation,
            viewport_width: viewport.css_width,
            viewport_height: viewport.css_height,
            device_pixel_ratio_bits: viewport.device_pixel_ratio.to_bits(),
            base_background_color,
        }))
    }

    fn has_same_resource_generation(&self, other: &Self) -> bool {
        self.0.resource_generation == other.0.resource_generation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererScreenshotFormat {
    Png,
    Jpeg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererScreenshotPurpose {
    Screenshot,
    Print { print_background: bool },
}

/// A CDP page-coordinate clip. Validation remains at the renderer boundary so
/// every protocol frontend shares the same finite/range checks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RendererScreenshotClip {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RendererScreenshotRegion {
    Viewport,
    FullDocument,
    /// A document-coordinate clip of the live viewport compositor surface.
    /// Root viewport controls remain present, matching CDP when
    /// `captureBeyondViewport` is false.
    ViewportClip(RendererScreenshotClip),
    PageClip(RendererScreenshotClip),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RendererCaptureScreenshotRequest {
    pub purpose: RendererScreenshotPurpose,
    pub format: RendererScreenshotFormat,
    pub quality: u8,
    pub region: RendererScreenshotRegion,
    /// Straight-alpha sRGB bytes in RGBA order, beneath author backgrounds.
    pub base_background_color: [u8; 4],
    pub optimize_for_speed: bool,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
}

/// One viewport screencast poll. Unlike an explicit screenshot, the caller
/// may supply the last emitted visual state and receive `Unchanged` without a
/// layout or paint pass.
#[derive(Clone, Debug, PartialEq)]
pub struct RendererCaptureScreencastFrameRequest {
    pub format: RendererScreenshotFormat,
    pub quality: u8,
    /// Straight-alpha sRGB bytes in RGBA order, beneath author backgrounds.
    pub base_background_color: [u8; 4],
    pub optimize_for_speed: bool,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    pub known_visual_state: Option<RendererVisualStateToken>,
}

impl RendererCaptureScreenshotRequest {
    pub fn viewport_png() -> Self {
        Self {
            purpose: RendererScreenshotPurpose::Screenshot,
            format: RendererScreenshotFormat::Png,
            quality: 100,
            region: RendererScreenshotRegion::Viewport,
            base_background_color: [255; 4],
            optimize_for_speed: false,
            max_width: None,
            max_height: None,
        }
    }
}

impl PageVm {
    /// Captures an explicit screenshot or print raster. These demands always
    /// execute a fresh paint pass and never carry screencast state.
    pub(super) fn capture_screenshot(
        &mut self,
        request: RendererCaptureScreenshotRequest,
    ) -> anyhow::Result<RendererCaptureScreenshotReply> {
        let barrier = match request.purpose {
            RendererScreenshotPurpose::Screenshot => moli_action_window::ActionBarrier::Screenshot,
            RendererScreenshotPurpose::Print { .. } => moli_action_window::ActionBarrier::Explicit,
        };
        self.flush_page_action_window(barrier)?;
        let paint_capture =
            request.paint_capture_request(self.viewport_surface.unwrap_or_default())?;
        let restore_media = if matches!(request.purpose, RendererScreenshotPurpose::Print { .. })
            && self.emulated_media.media.is_none()
        {
            let previous = self.emulated_media.clone();
            let mut print = previous.clone();
            print.media = Some("print".to_owned());
            self.set_emulated_media(&print);
            Some(previous)
        } else {
            None
        };
        let result = self.capture_image(
            request.format,
            request.quality,
            request.optimize_for_speed,
            paint_capture,
            match request.purpose {
                RendererScreenshotPurpose::Screenshot => moli_layout::LayoutFlushReason::Screenshot,
                RendererScreenshotPurpose::Print { .. } => moli_layout::LayoutFlushReason::Print,
            },
        );
        if let Some(previous) = restore_media {
            self.set_emulated_media(&previous);
        }
        result.map(RendererImageCaptureOutcome::into_screenshot_reply)
    }

    /// Polls one screencast subscription. Only this request accepts a known
    /// visual state and only this reply can report `Unchanged`.
    pub(super) fn capture_screencast_frame(
        &mut self,
        request: RendererCaptureScreencastFrameRequest,
    ) -> anyhow::Result<RendererCaptureScreencastFrameReply> {
        self.capture_screencast_frame_with_before_layout(request, || {})
    }

    #[cfg(test)]
    pub(super) fn capture_screencast_frame_with_before_layout_hook(
        &mut self,
        request: RendererCaptureScreencastFrameRequest,
        before_layout: impl FnOnce(),
    ) -> anyhow::Result<RendererCaptureScreencastFrameReply> {
        self.capture_screencast_frame_with_before_layout(request, before_layout)
    }

    fn capture_screencast_frame_with_before_layout(
        &mut self,
        request: RendererCaptureScreencastFrameRequest,
        before_layout: impl FnOnce(),
    ) -> anyhow::Result<RendererCaptureScreencastFrameReply> {
        self.flush_page_action_window(moli_action_window::ActionBarrier::Screencast)?;
        if self.layout_policy == LayoutPolicy::Mock {
            return Ok(RendererCaptureScreencastFrameReply::LayoutDisabled);
        }
        let surface = self.viewport_surface.unwrap_or_default();
        let viewport = PaintViewport::new(
            surface.inner_width,
            surface.inner_height,
            surface.device_pixel_ratio as f32,
        );
        let visual_state_before = self.vm().visual_state_token(
            self.document_lifecycle.identity(),
            viewport,
            request.base_background_color,
        );
        if request.known_visual_state.as_ref() == Some(&visual_state_before) {
            return Ok(RendererCaptureScreencastFrameReply::Unchanged);
        }
        before_layout();
        let paint_capture = PaintCaptureRequest {
            region: paint_viewport_region(surface, ViewportCapture::Widget)?,
            include_backgrounds: true,
            include_viewport_controls: true,
            base_background_color: paint_background_color(request.base_background_color),
            max_width: request.max_width,
            max_height: request.max_height,
        };
        let image = match self.capture_image(
            request.format,
            request.quality,
            request.optimize_for_speed,
            paint_capture,
            moli_layout::LayoutFlushReason::Screencast,
        )? {
            RendererImageCaptureOutcome::Captured(image) => image,
            RendererImageCaptureOutcome::LayoutDisabled => {
                return Ok(RendererCaptureScreencastFrameReply::LayoutDisabled);
            }
            RendererImageCaptureOutcome::NoDocument => {
                return Ok(RendererCaptureScreencastFrameReply::NoDocument);
            }
        };
        let visual_state_after = self.vm().visual_state_token(
            self.document_lifecycle.identity(),
            viewport,
            request.base_background_color,
        );
        let visual_state =
            visual_state_for_captured_screencast_frame(visual_state_before, visual_state_after);
        Ok(RendererCaptureScreencastFrameReply::Captured(
            RendererCapturedScreencastFrame {
                viewport_size: surface.visible_size(),
                image,
                visual_state,
            },
        ))
    }

    fn capture_image(
        &mut self,
        format: RendererScreenshotFormat,
        quality: u8,
        optimize_for_speed: bool,
        paint_capture: PaintCaptureRequest,
        reason: moli_layout::LayoutFlushReason,
    ) -> anyhow::Result<RendererImageCaptureOutcome> {
        let profile_enabled = moli_trace::cpu_profile_enabled();
        let total_started = profile_enabled.then(Instant::now);
        if self.layout_policy == LayoutPolicy::Mock {
            return Ok(RendererImageCaptureOutcome::LayoutDisabled);
        }
        let surface = self.viewport_surface.unwrap_or_default();
        let viewport = PaintViewport::new(
            surface.inner_width,
            surface.inner_height,
            surface.device_pixel_ratio as f32,
        );
        let layout_started = profile_enabled.then(Instant::now);
        let Some(snapshot) =
            self.vm_mut()
                .paint_layout_snapshot_with_capture(viewport, reason, paint_capture)?
        else {
            return Ok(RendererImageCaptureOutcome::NoDocument);
        };
        let layout_us = layout_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();

        if profile_enabled {
            let profile = PaintFragmentProfile::from_snapshot(&snapshot);
            tracing::info!(
                target: "moli_cpu_profile",
                stage = "paint_fragment_profile",
                push_layer = profile.push_layer,
                push_clip = profile.push_clip,
                pop_layer = profile.pop_layer,
                fill = profile.fill,
                stroke = profile.stroke,
                border = profile.border,
                box_shadow = profile.box_shadow,
                text_decoration = profile.text_decoration,
                text_shadow = profile.text_shadow,
                glyph_run = profile.glyph_run,
                glyph = profile.glyph,
                image = profile.image,
                svg_image = profile.svg_image,
            );
        }

        let raster_started = profile_enabled.then(Instant::now);
        let mut raster = moli_paint::raster_snapshot(&snapshot)?;
        let raster_us = raster_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        let encode_started = profile_enabled.then(Instant::now);
        let (mime_type, width, height, bytes) = match format {
            RendererScreenshotFormat::Png => {
                let encoded = moli_image::encode_png_with_options(
                    &raster,
                    moli_image::PngEncodeOptions { optimize_for_speed },
                )?;
                ("image/png", encoded.width, encoded.height, encoded.bytes)
            }
            RendererScreenshotFormat::Jpeg => {
                // Chrome captures JPEG against black. The encoder discards alpha,
                // so composite our straight-alpha raster before handing it over.
                for pixel in raster.rgba.chunks_exact_mut(4) {
                    let alpha = u16::from(pixel[3]);
                    if alpha != 255 {
                        for channel in &mut pixel[..3] {
                            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
                        }
                        pixel[3] = 255;
                    }
                }
                let encoded = moli_image::encode_jpeg(&raster, quality)?;
                ("image/jpeg", encoded.width, encoded.height, encoded.bytes)
            }
        };
        let encode_us = encode_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        if let Some(started) = total_started {
            tracing::info!(
                target: "moli_cpu_profile",
                stage = "image_capture",
                reason = ?reason,
                width,
                height,
                encoded_bytes = bytes.len(),
                layout_us,
                raster_us,
                encode_us,
                total_us = started.elapsed().as_micros(),
            );
        }
        Ok(RendererImageCaptureOutcome::Captured(
            RendererCapturedScreenshot {
                mime_type: mime_type.to_owned(),
                width,
                height,
                bytes: bytes.into(),
            },
        ))
    }
}

enum RendererImageCaptureOutcome {
    Captured(RendererCapturedScreenshot),
    LayoutDisabled,
    NoDocument,
}

impl RendererImageCaptureOutcome {
    fn into_screenshot_reply(self) -> RendererCaptureScreenshotReply {
        match self {
            Self::Captured(image) => RendererCaptureScreenshotReply::Captured(image),
            Self::LayoutDisabled => RendererCaptureScreenshotReply::LayoutDisabled,
            Self::NoDocument => RendererCaptureScreenshotReply::NoDocument,
        }
    }
}

fn visual_state_for_captured_screencast_frame(
    before: RendererVisualStateToken,
    after: RendererVisualStateToken,
) -> RendererVisualStateToken {
    // Resource decoders can publish from another task while layout samples
    // immutable resources. Keep the older token in that race so the next poll
    // cannot mistake a potentially stale frame for the new resource state.
    // Other changes are renderer-internal world preparation represented by
    // the completed fresh frame.
    if before.has_same_resource_generation(&after) {
        after
    } else {
        before
    }
}

impl RendererCaptureScreenshotRequest {
    fn paint_capture_request(
        &self,
        surface: ViewportSurface,
    ) -> anyhow::Result<PaintCaptureRequest> {
        let include_viewport_controls = matches!(
            self.region,
            RendererScreenshotRegion::Viewport | RendererScreenshotRegion::ViewportClip(_)
        );
        let region = match self.region {
            RendererScreenshotRegion::Viewport => {
                paint_viewport_region(surface, ViewportCapture::Surface)?
            }
            RendererScreenshotRegion::FullDocument => moli_layout::PaintCaptureRegion::FullDocument,
            RendererScreenshotRegion::ViewportClip(clip)
            | RendererScreenshotRegion::PageClip(clip) => {
                moli_layout::PaintCaptureRegion::PageClip {
                    rect: LayoutRect::new(
                        finite_f32("clip x", clip.x)?,
                        finite_f32("clip y", clip.y)?,
                        finite_f32("clip width", clip.width)?,
                        finite_f32("clip height", clip.height)?,
                    ),
                    scale: finite_f32("clip scale", clip.scale)?,
                }
            }
        };
        Ok(PaintCaptureRequest {
            region,
            include_backgrounds: match self.purpose {
                RendererScreenshotPurpose::Print { print_background } => print_background,
                RendererScreenshotPurpose::Screenshot => true,
            },
            include_viewport_controls,
            base_background_color: paint_background_color(self.base_background_color),
            max_width: self.max_width,
            max_height: self.max_height,
        })
    }
}

fn paint_background_color(rgba: [u8; 4]) -> moli_layout::PaintColor {
    let [r, g, b, a] = rgba.map(|channel| f32::from(channel) / 255.0);
    moli_layout::PaintColor::new(r, g, b, a)
}

fn finite_f32(label: &str, value: f64) -> anyhow::Result<f32> {
    if !value.is_finite() || value < f64::from(f32::MIN) || value > f64::from(f32::MAX) {
        anyhow::bail!("{label} must be a finite CSS-pixel value");
    }
    Ok(value as f32)
}

enum ViewportCapture {
    /// Screenshots restore the emulated DPR, regardless of the preview scale.
    Surface,
    /// Screencasts capture the actual widget, including the preview scale.
    Widget,
}

fn paint_viewport_region(
    surface: ViewportSurface,
    capture: ViewportCapture,
) -> anyhow::Result<PaintCaptureRegion> {
    let Some(view) = surface.emulated_view else {
        return Ok(PaintCaptureRegion::Viewport);
    };
    let (scale, width, height) = match capture {
        ViewportCapture::Surface => {
            let ratio = surface.device_pixel_ratio / view.native_device_pixel_ratio;
            (
                ratio,
                f64::from(surface.inner_width) * ratio,
                f64::from(surface.inner_height) * ratio,
            )
        }
        ViewportCapture::Widget => (view.scale, f64::from(view.width), f64::from(view.height)),
    };
    let (transform, scroll_scale) = if let Some(viewport) = view.viewport {
        // Blink applies the preview scale first, then the page-coordinate
        // viewport offset and scale. Resolve scroll compensation during layout.
        (
            LayoutTransform2D::new([
                viewport.scale * scale,
                0.0,
                0.0,
                viewport.scale * scale,
                -viewport.x * viewport.scale,
                -viewport.y * viewport.scale,
            ]),
            viewport.scale,
        )
    } else {
        (LayoutTransform2D::scale(scale, scale), 0.0)
    };
    Ok(PaintCaptureRegion::TransformedViewport {
        surface: PaintCaptureSurface::new(
            finite_f32("capture width", width)?,
            finite_f32("capture height", height)?,
            finite_f32("native device-pixel ratio", view.native_device_pixel_ratio)?,
        ),
        transform,
        scroll_scale,
    })
}
