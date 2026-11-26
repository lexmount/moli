//! Screenshot and screencast requests, replies and pixel capture.

use crate::layout_renderer::layout_viewport_for_surface;
use crate::native_bridge::LayoutPublicationRequest;
use moli_action_window::ActionBarrier;
use moli_layout::{
    LayoutFlushReason, LayoutRect, LayoutTransform2D, PaintCaptureRegion, PaintCaptureRequest,
    PaintCaptureSurface, PaintViewport,
};
use moli_page_types::{LayoutPolicy, ViewportSurface};
use std::sync::Arc;
use std::time::Instant;

#[cfg(feature = "devtools")]
use super::RendererDevToolsPageReply;
use super::{PageVm, RendererDocumentLifecycleIdentity};
#[cfg(feature = "devtools")]
use crate::raster_worker::RendererRasterStream;

mod vision;

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
                moli_layout::PaintFragment::Tiles(tiles) => match tiles.source {
                    moli_layout::PaintTileSource::Image(..) => profile.image += 1,
                    moli_layout::PaintTileSource::Svg(..) => profile.svg_image += 1,
                    moli_layout::PaintTileSource::Brush(_) => profile.fill += 1,
                },
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
pub struct RendererVisualStateToken {
    state: Arc<RendererVisualState>,
    overlay_generation: u64,
    post_layout_work_deferred: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct RendererVisualState {
    document: RendererDocumentLifecycleIdentity,
    dom_generation: u64,
    style_generations: Vec<(u32, u64, u64)>,
    interaction_generation: u64,
    resource_generation: u64,
    viewport_width: u32,
    viewport_height: u32,
    mobile_frame_size: Option<(u32, u32)>,
    device_pixel_ratio_bits: u32,
    base_background_color: [u8; 4],
}

impl RendererVisualStateToken {
    #[cfg(feature = "devtools")]
    pub(crate) fn new(
        document: RendererDocumentLifecycleIdentity,
        dom_generation: u64,
        style_generations: Vec<(crate::document_runtime::DomHandle, u64, u64)>,
        interaction_generation: u64,
        resource_generation: u64,
        viewport: PaintViewport,
        base_background_color: [u8; 4],
    ) -> Self {
        Self {
            state: Arc::new(RendererVisualState {
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
                mobile_frame_size: viewport.mobile_frame_size(),
                device_pixel_ratio_bits: viewport.device_pixel_ratio.to_bits(),
                base_background_color,
            }),
            overlay_generation: 0,
            post_layout_work_deferred: false,
        }
    }

    #[cfg(feature = "devtools")]
    fn with_overlay_generation(mut self, generation: u64) -> Self {
        self.overlay_generation = generation;
        self
    }

    #[cfg(feature = "devtools")]
    fn has_same_resource_generation(&self, other: &Self) -> bool {
        self.state.resource_generation == other.state.resource_generation
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
    /// Capture or export using the print style environment.
    Print {
        print_background: bool,
    },
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
    #[cfg(feature = "devtools")]
    pub paint_session: Option<super::RendererScreencastPaintSession>,
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

#[derive(Clone, Debug, PartialEq)]
pub struct RendererCapturedScreencastFrame {
    /// Native viewport dimensions before the encoded frame is fitted to limits.
    pub viewport_size: (u32, u32),
    pub page_scale_factor: f64,
    pub image: RendererCapturedScreenshot,
    pub visual_state: RendererVisualStateToken,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RendererCaptureScreencastFrameReply {
    Captured(RendererCapturedScreencastFrame),
    Unchanged,
    LayoutDisabled,
    NoDocument,
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
    /// Captures an explicit screenshot, borrowing screen paint when it is
    /// already retained by a continuous consumer.
    pub(super) fn capture_screenshot(
        &mut self,
        request: RendererCaptureScreenshotRequest,
    ) -> anyhow::Result<RendererCaptureScreenshotReply> {
        let (barrier, reason) = match request.purpose {
            RendererScreenshotPurpose::Screenshot => {
                (ActionBarrier::Screenshot, LayoutFlushReason::Screenshot)
            }
            RendererScreenshotPurpose::Print { .. } => {
                (ActionBarrier::Explicit, LayoutFlushReason::Print)
            }
        };
        if !self.vm().is_debugger_paused() {
            self.flush_page_action_window(barrier)?;
        }
        let paint_capture =
            request.paint_capture_request(self.vm().viewport_surface().unwrap_or_default())?;
        if self.layout_configuration.policy == LayoutPolicy::Mock {
            return Ok(RendererCaptureScreenshotReply::LayoutDisabled);
        }
        let capture = self.prepare_image_capture(
            paint_capture,
            reason,
            #[cfg(feature = "devtools")]
            None,
        )?;
        match capture {
            Some(capture) => Ok(RendererCaptureScreenshotReply::Captured(capture.encode(
                request.format,
                request.quality,
                request.optimize_for_speed,
            )?)),
            None => Ok(RendererCaptureScreenshotReply::NoDocument),
        }
    }

    /// The owner produces an immutable snapshot. Realtime capture can then
    /// rasterize and encode it without holding the page execution lane.
    #[cfg(feature = "devtools")]
    pub(super) fn dispatch_screencast_frame(
        &mut self,
        request: RendererCaptureScreencastFrameRequest,
    ) -> anyhow::Result<RendererDevToolsPageReply> {
        self.prepare_screencast_frame(request, || {})
    }

    #[cfg(all(feature = "devtools", test))]
    pub(super) fn capture_screencast_frame(
        &mut self,
        request: RendererCaptureScreencastFrameRequest,
    ) -> anyhow::Result<RendererCaptureScreencastFrameReply> {
        self.capture_screencast_frame_with_before_layout_hook(request, || {})
    }

    #[cfg(feature = "devtools")]
    #[cfg(test)]
    pub(super) fn capture_screencast_frame_with_before_layout_hook(
        &mut self,
        request: RendererCaptureScreencastFrameRequest,
        before_layout: impl FnOnce(),
    ) -> anyhow::Result<RendererCaptureScreencastFrameReply> {
        match self.prepare_screencast_frame(request, before_layout)? {
            RendererDevToolsPageReply::CaptureScreencastFrame(reply) => Ok(reply),
            RendererDevToolsPageReply::ScreencastFrameRasterJob(job) => job.finish(),
            _ => unreachable!("screencast preparation only produces a screencast reply"),
        }
    }

    #[cfg(feature = "devtools")]
    fn prepare_screencast_frame(
        &mut self,
        request: RendererCaptureScreencastFrameRequest,
        before_layout: impl FnOnce(),
    ) -> anyhow::Result<RendererDevToolsPageReply> {
        use RendererCaptureScreencastFrameReply::{LayoutDisabled, NoDocument, Unchanged};
        use RendererDevToolsPageReply::CaptureScreencastFrame;
        let debugger_paused = self.vm().is_debugger_paused();
        if !debugger_paused {
            self.flush_page_action_window(ActionBarrier::Screencast)?;
        }
        if self.layout_configuration.policy == LayoutPolicy::Mock {
            return Ok(CaptureScreencastFrame(LayoutDisabled));
        }
        let surface = self.vm().viewport_surface().unwrap_or_default();
        let viewport = layout_viewport_for_surface(surface);
        let visual_state_before = self
            .vm()
            .visual_state_token(
                self.document_lifecycle.identity(),
                viewport,
                request.base_background_color,
            )
            .with_overlay_generation(self.devtools.overlay.generation());
        // A paused capture skips resource discovery and other post-layout
        // work. Retry that work on the first screencast poll after resuming.
        if request.known_visual_state.as_ref().is_some_and(|known| {
            (debugger_paused || !known.post_layout_work_deferred)
                && known.state == visual_state_before.state
                && known.overlay_generation == visual_state_before.overlay_generation
        }) {
            return Ok(CaptureScreencastFrame(Unchanged));
        }
        before_layout();
        let paint_capture = PaintCaptureRequest {
            region: paint_viewport_region(surface, ViewportCapture::Widget)?,
            include_backgrounds: true,
            include_viewport_controls: true,
            base_background_color: paint_background_color(request.base_background_color),
            clip_filter_outset: 0.0,
            max_width: request.max_width,
            max_height: request.max_height,
        };
        // The fresh capture path defers resource admission while paused and
        // revisits deferred work on the first resumed poll.
        let paint_session = request.paint_session.as_ref().filter(|_| {
            !debugger_paused
                && !request
                    .known_visual_state
                    .as_ref()
                    .is_some_and(|known| known.post_layout_work_deferred)
        });
        let Some(capture) = self.prepare_image_capture(
            paint_capture,
            LayoutFlushReason::Screencast,
            paint_session,
        )?
        else {
            return Ok(CaptureScreencastFrame(NoDocument));
        };
        let visual_state_after = self
            .vm()
            .visual_state_token(
                self.document_lifecycle.identity(),
                viewport,
                request.base_background_color,
            )
            .with_overlay_generation(self.devtools.overlay.generation());
        let mut visual_state =
            visual_state_for_rendered_frame(visual_state_before, visual_state_after);
        visual_state.post_layout_work_deferred = debugger_paused;
        let realtime = self.layout_configuration.policy.is_realtime();
        let job = RendererScreencastFrameRasterJob {
            capture,
            viewport_size: surface.visible_size(),
            page_scale_factor: surface
                .mobile_viewport()
                .map_or(1.0, |viewport| viewport.scale),
            visual_state,
            format: request.format,
            quality: request.quality,
            optimize_for_speed: request.optimize_for_speed,
            stream: request
                .paint_session
                .as_ref()
                .filter(|_| realtime)
                .map(super::RendererScreencastPaintSession::raster_stream),
        };
        if realtime {
            Ok(RendererDevToolsPageReply::ScreencastFrameRasterJob(
                Box::new(job),
            ))
        } else {
            job.finish().map(CaptureScreencastFrame)
        }
    }

    fn prepare_image_capture(
        &mut self,
        mut paint_capture: PaintCaptureRequest,
        reason: LayoutFlushReason,
        #[cfg(feature = "devtools")] paint_session: Option<&super::RendererScreencastPaintSession>,
    ) -> anyhow::Result<Option<PreparedImageCapture>> {
        let profile_enabled = moli_trace::cpu_profile_enabled();
        let total_started = profile_enabled.then(Instant::now);
        use moli_page_types::VisionDeficiency;
        let is_print = reason == LayoutFlushReason::Print;
        let deficiency = if is_print {
            VisionDeficiency::None
        } else {
            self.document_settings.emulated_media.vision_deficiency
        };
        if deficiency == VisionDeficiency::BlurredVision {
            paint_capture.clip_filter_outset =
                vision::BLURRED_VISION_SIGMA_CSS_PX * moli_image::GAUSSIAN_BLUR_OUTSET;
        }
        let surface = self.vm().viewport_surface().unwrap_or_default();
        let (viewport, publication, environment_override) = if is_print {
            let (width, height) = surface.css_layout_size();
            (
                PaintViewport::new(width, height, surface.device_pixel_ratio as f32),
                LayoutPublicationRequest::Transient,
                self.print_style_environment_override(),
            )
        } else {
            (
                layout_viewport_for_surface(surface),
                LayoutPublicationRequest::Screen,
                None,
            )
        };
        let layout_started = profile_enabled.then(Instant::now);
        #[cfg(feature = "devtools")]
        let reusable = if paint_session.is_some()
            || (reason == LayoutFlushReason::Screenshot
                && self.layout_configuration.policy.is_realtime()
                && !self.vm().is_debugger_paused())
        {
            self.vm_mut().paint_current_screen_layout(
                paint_session,
                viewport,
                paint_capture,
                reason,
            )?
        } else {
            None
        };
        #[cfg(not(feature = "devtools"))]
        let reusable = None;
        let snapshot = match reusable {
            Some(snapshot) => Some(snapshot),
            None => self.vm_mut().paint_layout_snapshot_with_capture(
                viewport,
                reason,
                paint_capture,
                publication,
                environment_override,
            )?,
        };
        let Some(snapshot) = snapshot else {
            return Ok(None);
        };
        let layout_us = layout_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        #[cfg(feature = "devtools")]
        let snapshot = if is_print {
            snapshot
        } else {
            self.paint_overlay_highlight(snapshot)
        };

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

        Ok(Some(PreparedImageCapture {
            snapshot,
            deficiency,
            reason,
            total_started,
            layout_us,
        }))
    }
}

/// Owned pixels work. The snapshot contains no live layout or JavaScript state.
struct PreparedImageCapture {
    snapshot: moli_layout::PaintSnapshot,
    deficiency: moli_page_types::VisionDeficiency,
    reason: moli_layout::LayoutFlushReason,
    total_started: Option<Instant>,
    layout_us: u128,
}

impl PreparedImageCapture {
    fn encode(
        self,
        format: RendererScreenshotFormat,
        quality: u8,
        optimize_for_speed: bool,
    ) -> anyhow::Result<RendererCapturedScreenshot> {
        let Self {
            snapshot,
            deficiency,
            reason,
            total_started,
            layout_us,
        } = self;
        use moli_page_types::VisionDeficiency;
        let profile_enabled = total_started.is_some();
        let raster_started = profile_enabled.then(Instant::now);
        let raster_scale = snapshot
            .surface
            .viewport_to_raster_scale(snapshot.viewport_to_surface)
            as f32;
        if deficiency == VisionDeficiency::BlurredVision {
            let layout = moli_paint::raster_buffer_layout(
                snapshot.surface.css_width,
                snapshot.surface.css_height,
                snapshot.surface.device_scale,
            )?;
            moli_image::validate_gaussian_blur(
                layout.width,
                layout.height,
                vision::BLURRED_VISION_SIGMA_CSS_PX * raster_scale,
            )?;
        }
        let mut raster = moli_paint::raster_snapshot(&snapshot)?;
        vision::apply(&mut raster, deficiency, raster_scale)?;
        if let Some(crop) = snapshot.raster_crop {
            crop_capture(&mut raster, crop)?;
        }
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
                for pixel in raster.rgba.as_chunks_mut::<4>().0 {
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
        Ok(RendererCapturedScreenshot {
            mime_type: mime_type.to_owned(),
            width,
            height,
            bytes: bytes.into(),
        })
    }
}

/// Pixel encoding work for a single pending Realtime screencast capture.
#[cfg(feature = "devtools")]
pub struct RendererScreencastFrameRasterJob {
    capture: PreparedImageCapture,
    viewport_size: (u32, u32),
    page_scale_factor: f64,
    visual_state: RendererVisualStateToken,
    format: RendererScreenshotFormat,
    quality: u8,
    optimize_for_speed: bool,
    stream: Option<std::sync::Weak<RendererRasterStream>>,
}

#[cfg(feature = "devtools")]
impl RendererScreencastFrameRasterJob {
    pub(super) async fn finish_on_worker(
        mut self,
    ) -> anyhow::Result<RendererCaptureScreencastFrameReply> {
        if let Some(stream) = self.stream.take() {
            let pending = stream
                .upgrade()
                .ok_or_else(|| anyhow::anyhow!("screencast subscription has closed"))?
                .submit(move || self.finish())?;
            pending.await?
        } else {
            // Native callers can capture one frame without a CDP subscription.
            let stream = RendererRasterStream::default();
            stream.submit(move || self.finish())?.await?
        }
    }

    pub(super) fn finish(self) -> anyhow::Result<RendererCaptureScreencastFrameReply> {
        let image = self
            .capture
            .encode(self.format, self.quality, self.optimize_for_speed)?;
        Ok(RendererCaptureScreencastFrameReply::Captured(
            RendererCapturedScreencastFrame {
                viewport_size: self.viewport_size,
                page_scale_factor: self.page_scale_factor,
                visual_state: self.visual_state,
                image,
            },
        ))
    }
}

#[cfg(feature = "devtools")]
pub(super) fn visual_state_for_rendered_frame(
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
                RendererScreenshotPurpose::Screenshot => true,
                RendererScreenshotPurpose::Print { print_background } => print_background,
            },
            include_viewport_controls,
            base_background_color: paint_background_color(self.base_background_color),
            clip_filter_outset: 0.0,
            max_width: self.max_width,
            max_height: self.max_height,
        })
    }
}

fn crop_capture(
    raster: &mut moli_image::RgbaImage,
    crop: moli_layout::PaintCaptureCrop,
) -> anyhow::Result<()> {
    let layout = moli_paint::raster_buffer_layout(
        crop.surface.css_width,
        crop.surface.css_height,
        crop.surface.device_scale,
    )?;
    anyhow::ensure!(
        layout.width <= raster.width.saturating_sub(crop.x)
            && layout.height <= raster.height.saturating_sub(crop.y),
        "requested output is outside its padded capture",
    );
    let source_stride = raster.width as usize * 4;
    let output_stride = layout.width as usize * 4;
    for row in 0..layout.height as usize {
        let start = (row + crop.y as usize) * source_stride + crop.x as usize * 4;
        raster
            .rgba
            .copy_within(start..start + output_stride, row * output_stride);
    }
    raster.rgba.truncate(layout.byte_len);
    raster.width = layout.width;
    raster.height = layout.height;
    Ok(())
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
    #[cfg(feature = "devtools")]
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
            let (width, height) = view.device_size;
            (ratio, f64::from(width) * ratio, f64::from(height) * ratio)
        }
        #[cfg(feature = "devtools")]
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
        let scale = scale * view.mobile_viewport.map_or(1.0, |viewport| viewport.scale);
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

#[cfg(feature = "devtools")]
pub(super) mod overlay;
