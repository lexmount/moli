use moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE;
use moli_layout::{LayoutRect, PaintCaptureRequest, PaintViewport};
use moli_page_types::LayoutPolicy;

use super::{PageVm, RendererCaptureScreenshotReply, RendererCapturedScreenshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererScreenshotFormat {
    Png,
    Jpeg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererScreenshotPurpose {
    Screenshot,
    Screencast,
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
    /// Captures the current committed document at one renderer-owner command
    /// turn. The pass-local world, Stylo values, Taffy cache, and paint
    /// resources do not escape; the remaining owned geometry projection is
    /// published as the Document's latest observable layout snapshot.
    pub(super) fn capture_screenshot(
        &mut self,
        request: RendererCaptureScreenshotRequest,
    ) -> anyhow::Result<RendererCaptureScreenshotReply> {
        let barrier = match request.purpose {
            RendererScreenshotPurpose::Screenshot => moli_action_window::ActionBarrier::Screenshot,
            RendererScreenshotPurpose::Screencast => moli_action_window::ActionBarrier::Screencast,
            RendererScreenshotPurpose::Print { .. } => moli_action_window::ActionBarrier::Explicit,
        };
        self.flush_page_action_window(barrier)?;
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
        let result = self.capture_screenshot_inner(request);
        if let Some(previous) = restore_media {
            self.set_emulated_media(&previous);
        }
        result
    }

    fn capture_screenshot_inner(
        &mut self,
        request: RendererCaptureScreenshotRequest,
    ) -> anyhow::Result<RendererCaptureScreenshotReply> {
        if self.layout_policy == LayoutPolicy::Mock {
            return Ok(RendererCaptureScreenshotReply::LayoutDisabled);
        }

        let surface = self
            .viewport_surface
            .unwrap_or_else(default_viewport_surface);
        let viewport = PaintViewport::new(
            surface.inner_width,
            surface.inner_height,
            surface.device_pixel_ratio as f32,
        );
        let paint_capture = request.paint_capture_request()?;
        let layout_reason = match request.purpose {
            RendererScreenshotPurpose::Screenshot | RendererScreenshotPurpose::Print { .. } => {
                moli_layout::LayoutFlushReason::Screenshot
            }
            RendererScreenshotPurpose::Screencast => moli_layout::LayoutFlushReason::Screencast,
        };
        let Some(snapshot) = self.vm_mut().paint_layout_snapshot_with_capture(
            viewport,
            layout_reason,
            paint_capture,
        )?
        else {
            return Ok(RendererCaptureScreenshotReply::NoDocument);
        };

        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let (mime_type, width, height, bytes) = match request.format {
            RendererScreenshotFormat::Png => {
                let encoded = moli_image::encode_png_with_options(
                    &raster,
                    moli_image::PngEncodeOptions {
                        optimize_for_speed: request.optimize_for_speed,
                    },
                )?;
                ("image/png", encoded.width, encoded.height, encoded.bytes)
            }
            RendererScreenshotFormat::Jpeg => {
                let encoded = moli_image::encode_jpeg(&raster, request.quality)?;
                ("image/jpeg", encoded.width, encoded.height, encoded.bytes)
            }
        };
        Ok(RendererCaptureScreenshotReply::Captured(
            RendererCapturedScreenshot {
                mime_type: mime_type.to_owned(),
                width,
                height,
                bytes: bytes.into(),
            },
        ))
    }
}

impl RendererCaptureScreenshotRequest {
    fn paint_capture_request(&self) -> anyhow::Result<PaintCaptureRequest> {
        let region = match self.region {
            RendererScreenshotRegion::Viewport => moli_layout::PaintCaptureRegion::Viewport,
            RendererScreenshotRegion::FullDocument => moli_layout::PaintCaptureRegion::FullDocument,
            RendererScreenshotRegion::PageClip(clip) => moli_layout::PaintCaptureRegion::PageClip {
                rect: LayoutRect::new(
                    finite_f32("clip x", clip.x)?,
                    finite_f32("clip y", clip.y)?,
                    finite_f32("clip width", clip.width)?,
                    finite_f32("clip height", clip.height)?,
                ),
                scale: finite_f32("clip scale", clip.scale)?,
            },
        };
        Ok(PaintCaptureRequest {
            region,
            include_backgrounds: match self.purpose {
                RendererScreenshotPurpose::Print { print_background } => print_background,
                RendererScreenshotPurpose::Screenshot | RendererScreenshotPurpose::Screencast => {
                    true
                }
            },
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
