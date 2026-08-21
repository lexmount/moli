use moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE;
use moli_layout::{LayoutRect, PaintCaptureRequest, PaintViewport};
use moli_page_types::LayoutPolicy;
use std::sync::Arc;

use super::{
    PageVm, RendererCaptureScreencastFrameReply, RendererCaptureScreenshotReply,
    RendererCapturedScreencastFrame, RendererCapturedScreenshot, RendererDocumentLifecycleIdentity,
};

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
}

impl RendererVisualStateToken {
    pub(crate) fn new(
        document: RendererDocumentLifecycleIdentity,
        dom_generation: u64,
        style_generations: Vec<(crate::document_runtime::DomHandle, u64, u64, u64)>,
        interaction_generation: u64,
        resource_generation: u64,
        viewport: PaintViewport,
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
        let paint_capture = request.paint_capture_request()?;
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
            moli_layout::LayoutFlushReason::Screenshot,
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
        let surface = self
            .viewport_surface
            .unwrap_or_else(default_viewport_surface);
        let viewport = PaintViewport::new(
            surface.inner_width,
            surface.inner_height,
            surface.device_pixel_ratio as f32,
        );
        let visual_state_before = self
            .vm()
            .visual_state_token(self.document_lifecycle.identity(), viewport);
        if request.known_visual_state.as_ref() == Some(&visual_state_before) {
            return Ok(RendererCaptureScreencastFrameReply::Unchanged);
        }
        before_layout();
        let paint_capture = PaintCaptureRequest {
            region: moli_layout::PaintCaptureRegion::Viewport,
            include_backgrounds: true,
            include_viewport_controls: true,
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
        let visual_state_after = self
            .vm()
            .visual_state_token(self.document_lifecycle.identity(), viewport);
        let visual_state =
            visual_state_for_captured_screencast_frame(visual_state_before, visual_state_after);
        Ok(RendererCaptureScreencastFrameReply::Captured(
            RendererCapturedScreencastFrame {
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
        if self.layout_policy == LayoutPolicy::Mock {
            return Ok(RendererImageCaptureOutcome::LayoutDisabled);
        }
        let surface = self
            .viewport_surface
            .unwrap_or_else(default_viewport_surface);
        let viewport = PaintViewport::new(
            surface.inner_width,
            surface.inner_height,
            surface.device_pixel_ratio as f32,
        );
        let Some(snapshot) =
            self.vm_mut()
                .paint_layout_snapshot_with_capture(viewport, reason, paint_capture)?
        else {
            return Ok(RendererImageCaptureOutcome::NoDocument);
        };

        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let (mime_type, width, height, bytes) = match format {
            RendererScreenshotFormat::Png => {
                let encoded = moli_image::encode_png_with_options(
                    &raster,
                    moli_image::PngEncodeOptions { optimize_for_speed },
                )?;
                ("image/png", encoded.width, encoded.height, encoded.bytes)
            }
            RendererScreenshotFormat::Jpeg => {
                let encoded = moli_image::encode_jpeg(&raster, quality)?;
                ("image/jpeg", encoded.width, encoded.height, encoded.bytes)
            }
        };
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
    fn paint_capture_request(&self) -> anyhow::Result<PaintCaptureRequest> {
        let include_viewport_controls = matches!(
            self.region,
            RendererScreenshotRegion::Viewport | RendererScreenshotRegion::ViewportClip(_)
        );
        let region = match self.region {
            RendererScreenshotRegion::Viewport => moli_layout::PaintCaptureRegion::Viewport,
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
            max_width: self.max_width,
            max_height: self.max_height,
        })
    }
}

fn finite_f32(label: &str, value: f64) -> anyhow::Result<f32> {
    if !value.is_finite() || value < f64::from(f32::MIN) || value > f64::from(f32::MAX) {
        anyhow::bail!("{label} must be a finite CSS-pixel value");
    }
    Ok(value as f32)
}

fn default_viewport_surface() -> crate::protocol_types::ViewportSurface {
    fn dimension(value: f64) -> u32 {
        debug_assert!(value.is_finite() && value >= 0.0 && value <= f64::from(u32::MAX));
        value as u32
    }

    crate::protocol_types::ViewportSurface {
        inner_width: dimension(DEFAULT_WINDOW_SURFACE_PROFILE.inner_width),
        inner_height: dimension(DEFAULT_WINDOW_SURFACE_PROFILE.inner_height),
        outer_width: dimension(DEFAULT_WINDOW_SURFACE_PROFILE.inner_width),
        outer_height: dimension(DEFAULT_WINDOW_SURFACE_PROFILE.inner_height),
        device_pixel_ratio: DEFAULT_WINDOW_SURFACE_PROFILE.device_pixel_ratio,
        screen_width: dimension(DEFAULT_WINDOW_SURFACE_PROFILE.screen_width),
        screen_height: dimension(DEFAULT_WINDOW_SURFACE_PROFILE.screen_height),
        screen_avail_width: dimension(DEFAULT_WINDOW_SURFACE_PROFILE.screen_avail_width),
        screen_avail_height: dimension(DEFAULT_WINDOW_SURFACE_PROFILE.screen_avail_height),
    }
}
