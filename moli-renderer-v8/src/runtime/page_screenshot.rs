//! Screenshot and screencast requests, replies and pixel capture.

use moli_layout::{
    LayoutRect, LayoutTransform2D, PaintCaptureRegion, PaintCaptureRequest, PaintCaptureSurface,
    PaintViewport,
};
use moli_page_types::{LayoutPolicy, ViewportSurface};
use std::sync::Arc;
use std::time::Instant;

use super::{PageVm, RendererDocumentLifecycleIdentity};

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
    style_generations: Vec<(u32, u64, u64)>,
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
        style_generations: Vec<(crate::document_runtime::DomHandle, u64, u64)>,
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
                .map(|(document, source, input)| (document.index_u32(), source, input))
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
    /// Retain the pre-encoding raster for the optional MP4 sink.
    pub capture_video: bool,
}

/// Raw pixels from the same raster pass as a CDP screencast frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererCapturedVideoFrame {
    pub image: moli_image::RgbaImage,
    pub captured_at: Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererCapturedScreenshot {
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub bytes: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererCaptureScreenshotReply {
    Captured(RendererCapturedScreenshot),
    LayoutDisabled,
    NoDocument,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererCapturedScreencastFrame {
    pub video_frame: Option<RendererCapturedVideoFrame>,
    /// Native viewport dimensions before the encoded frame is fitted to limits.
    pub viewport_size: (u32, u32),
    pub image: RendererCapturedScreenshot,
    pub visual_state: RendererVisualStateToken,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererCaptureScreencastFrameReply {
    Captured(RendererCapturedScreencastFrame),
    Unchanged,
    LayoutDisabled,
    NoDocument,
}

impl RendererCaptureScreenshotRequest {
    pub fn viewport_png() -> Self {
        Self {
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
    /// Captures an explicit screenshot. These demands always execute a fresh
    /// paint pass and never carry screencast state.
    pub(super) fn capture_screenshot(
        &mut self,
        request: RendererCaptureScreenshotRequest,
    ) -> anyhow::Result<RendererCaptureScreenshotReply> {
        self.flush_page_action_window(moli_action_window::ActionBarrier::Screenshot)?;
        let paint_capture = request
            .paint_capture_request(self.document_settings.viewport_surface.unwrap_or_default())?;
        self.capture_image(
            request.format,
            request.quality,
            request.optimize_for_speed,
            paint_capture,
            moli_layout::LayoutFlushReason::Screenshot,
        )
        .map(RendererImageCaptureOutcome::into_screenshot_reply)
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
        if self.layout_configuration.policy == LayoutPolicy::Mock {
            return Ok(RendererCaptureScreencastFrameReply::LayoutDisabled);
        }
        let surface = self.document_settings.viewport_surface.unwrap_or_default();
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
        let total_started = moli_trace::cpu_profile_enabled().then(Instant::now);
        let mut raster =
            match self.capture_raster(paint_capture, moli_layout::LayoutFlushReason::Screencast)? {
                RendererRasterCaptureOutcome::Captured(raster) => raster,
                RendererRasterCaptureOutcome::LayoutDisabled => {
                    return Ok(RendererCaptureScreencastFrameReply::LayoutDisabled);
                }
                RendererRasterCaptureOutcome::NoDocument => {
                    return Ok(RendererCaptureScreencastFrameReply::NoDocument);
                }
            };
        let image = Self::encode_raster_capture(
            &mut raster,
            request.format,
            request.quality,
            request.optimize_for_speed,
            request.capture_video,
            moli_layout::LayoutFlushReason::Screencast,
            total_started,
        )?;
        let video_frame = request.capture_video.then_some(RendererCapturedVideoFrame {
            image: raster.image,
            captured_at: raster.captured_at,
        });
        let visual_state_after = self.vm().visual_state_token(
            self.document_lifecycle.identity(),
            viewport,
            request.base_background_color,
        );
        let visual_state =
            visual_state_for_captured_screencast_frame(visual_state_before, visual_state_after);
        Ok(RendererCaptureScreencastFrameReply::Captured(
            RendererCapturedScreencastFrame {
                video_frame,
                viewport_size: surface.visible_size(),
                image,
                visual_state,
            },
        ))
    }

    fn capture_raster(
        &mut self,
        paint_capture: PaintCaptureRequest,
        reason: moli_layout::LayoutFlushReason,
    ) -> anyhow::Result<RendererRasterCaptureOutcome> {
        let profile_enabled = moli_trace::cpu_profile_enabled();
        if self.layout_configuration.policy == LayoutPolicy::Mock {
            return Ok(RendererRasterCaptureOutcome::LayoutDisabled);
        }
        let surface = self.document_settings.viewport_surface.unwrap_or_default();
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
            return Ok(RendererRasterCaptureOutcome::NoDocument);
        };
        let captured_at = Instant::now();
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
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let raster_us = raster_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        Ok(RendererRasterCaptureOutcome::Captured(CapturedRaster {
            image: raster,
            captured_at,
            layout_us,
            raster_us,
        }))
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
        let mut captured = match self.capture_raster(paint_capture, reason)? {
            RendererRasterCaptureOutcome::Captured(captured) => captured,
            RendererRasterCaptureOutcome::LayoutDisabled => {
                return Ok(RendererImageCaptureOutcome::LayoutDisabled);
            }
            RendererRasterCaptureOutcome::NoDocument => {
                return Ok(RendererImageCaptureOutcome::NoDocument);
            }
        };
        Self::encode_raster_capture(
            &mut captured,
            format,
            quality,
            optimize_for_speed,
            false,
            reason,
            total_started,
        )
        .map(RendererImageCaptureOutcome::Captured)
    }

    fn encode_raster_capture(
        captured: &mut CapturedRaster,
        format: RendererScreenshotFormat,
        quality: u8,
        optimize_for_speed: bool,
        preserve_raw: bool,
        reason: moli_layout::LayoutFlushReason,
        total_started: Option<Instant>,
    ) -> anyhow::Result<RendererCapturedScreenshot> {
        let profile_enabled = moli_trace::cpu_profile_enabled();
        let layout_us = captured.layout_us;
        let raster_us = captured.raster_us;
        let raster = &mut captured.image;
        let encode_started = profile_enabled.then(Instant::now);
        let (mime_type, width, height, bytes) = match format {
            RendererScreenshotFormat::Png => {
                let encoded = moli_image::encode_png_with_options(
                    raster,
                    moli_image::PngEncodeOptions { optimize_for_speed },
                )?;
                ("image/png", encoded.width, encoded.height, encoded.bytes)
            }
            RendererScreenshotFormat::Jpeg => {
                // The video sink needs the original straight-alpha pixels.
                // Clone only when JPEG's black compositing would change them.
                let mut composite = (preserve_raw
                    && raster
                        .rgba
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|pixel| pixel[3] != 255))
                .then(|| raster.clone());
                let raster = composite.as_mut().unwrap_or(raster);
                // Chrome captures JPEG against black. The encoder discards alpha,
                // so composite our straight-alpha raster before handing it over.
                for pixel in raster.rgba.as_chunks_mut::<4>().0 {
                    let alpha = u16::from(pixel[3]);
                    if alpha != 255 {
                        for channel in &mut pixel[..3] {
                            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
                        }
                        pixel[3] = 255;
                    }
                }
                let encoded = moli_image::encode_jpeg(raster, quality)?;
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
        Ok(RendererCapturedScreenshot {
            mime_type: mime_type.to_owned(),
            width,
            height,
            bytes: bytes.into(),
        })
    }
}

struct CapturedRaster {
    image: moli_image::RgbaImage,
    captured_at: Instant,
    layout_us: u128,
    raster_us: u128,
}

enum RendererRasterCaptureOutcome {
    Captured(CapturedRaster),
    LayoutDisabled,
    NoDocument,
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
            include_backgrounds: true,
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
