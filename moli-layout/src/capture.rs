use crate::{LayoutError, LayoutPoint, LayoutRect, LayoutSize, LayoutTransform2D, LayoutViewport};

/// Exclusive CSS-pixel limit for either side of an automatic full-document
/// capture, matching Chromium's `Page.captureScreenshot` boundary.
pub const FULL_DOCUMENT_CAPTURE_CSS_DIMENSION_LIMIT: f32 = (128 * 1024) as f32;

/// Which CSS-pixel region one paint-enabled layout demand should capture.
///
/// Page clips use document coordinates, matching CDP `Page.Viewport`. They do
/// not replace the layout viewport or change any containing block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaintCaptureRegion {
    /// The current visual viewport.
    Viewport,
    /// The complete document content extent computed by this layout pass.
    FullDocument,
    /// One explicit page-coordinate rectangle.
    PageClip { rect: LayoutRect, scale: f32 },
}

/// Output constraints for one short-lived screenshot or screencast capture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintCaptureRequest {
    pub region: PaintCaptureRegion,
    /// Whether CSS background colors and images are included in paint.
    pub include_backgrounds: bool,
    /// Whether compositor-owned viewport controls are included in this capture.
    ///
    /// A bounded CDP clip can still capture the live viewport surface, while a
    /// capture-beyond-viewport clip paints page content only. Keep that
    /// distinction outside [`PaintCaptureRegion`], because both use the same
    /// document-coordinate raster rectangle.
    pub include_viewport_controls: bool,
    /// Maximum encoded image width in device pixels.
    pub max_width: Option<u32>,
    /// Maximum encoded image height in device pixels.
    pub max_height: Option<u32>,
}

impl PaintCaptureRequest {
    /// Captures the current viewport at its live device-pixel ratio.
    pub const fn viewport() -> Self {
        Self {
            region: PaintCaptureRegion::Viewport,
            include_backgrounds: true,
            include_viewport_controls: true,
            max_width: None,
            max_height: None,
        }
    }

    /// Captures the current viewport while fitting within optional device-pixel limits.
    pub const fn viewport_with_limits(max_width: Option<u32>, max_height: Option<u32>) -> Self {
        Self {
            region: PaintCaptureRegion::Viewport,
            include_backgrounds: true,
            include_viewport_controls: true,
            max_width,
            max_height,
        }
    }

    /// Captures the complete document extent without changing its layout viewport.
    pub const fn full_document() -> Self {
        Self {
            region: PaintCaptureRegion::FullDocument,
            include_backgrounds: true,
            include_viewport_controls: false,
            max_width: None,
            max_height: None,
        }
    }

    /// Captures one document-coordinate rectangle at the requested clip scale.
    pub const fn page_clip(rect: LayoutRect, scale: f32) -> Self {
        Self {
            region: PaintCaptureRegion::PageClip { rect, scale },
            include_backgrounds: true,
            include_viewport_controls: false,
            max_width: None,
            max_height: None,
        }
    }
}

/// Exact CSS and device scale of the one-shot raster surface.
///
/// This is deliberately distinct from [`LayoutViewport`]. A full-page or
/// clipped capture changes this surface while layout continues to use the
/// browser's live viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintCaptureSurface {
    pub css_width: f32,
    pub css_height: f32,
    pub device_scale: f32,
}

impl PaintCaptureSurface {
    pub const fn new(css_width: f32, css_height: f32, device_scale: f32) -> Self {
        Self {
            css_width,
            css_height,
            device_scale,
        }
    }

    pub const fn for_viewport(viewport: LayoutViewport) -> Self {
        Self::new(
            viewport.css_width as f32,
            viewport.css_height as f32,
            viewport.device_pixel_ratio,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ResolvedPaintCapture {
    /// Captured rectangle in the projection's viewport coordinate system.
    pub(crate) viewport_rect: LayoutRect,
    /// Translation applied after local-to-viewport transforms.
    pub(crate) viewport_to_surface: LayoutTransform2D,
    pub(crate) surface: PaintCaptureSurface,
    pub(crate) include_backgrounds: bool,
    /// Viewport captures include the root viewport's native controls. Full
    /// document and page-clip captures paint page content only, matching
    /// Chromium's compositor capture behavior.
    pub(crate) paint_root_scrollbars: bool,
}

impl PaintCaptureRequest {
    pub(crate) fn resolve(
        self,
        viewport: LayoutViewport,
        viewport_scroll: LayoutPoint,
        content_size: LayoutSize,
    ) -> Result<ResolvedPaintCapture, LayoutError> {
        validate_viewport(viewport)?;
        let paint_root_scrollbars = self.include_viewport_controls;
        let (viewport_rect, capture_scale) = match self.region {
            PaintCaptureRegion::Viewport => (
                LayoutRect::new(
                    0.0,
                    0.0,
                    viewport.css_width as f32,
                    viewport.css_height as f32,
                ),
                1.0,
            ),
            PaintCaptureRegion::FullDocument => {
                validate_full_document_extent(content_size.width, content_size.height)?;
                (
                    LayoutRect::new(
                        -viewport_scroll.x,
                        -viewport_scroll.y,
                        content_size.width,
                        content_size.height,
                    ),
                    1.0,
                )
            }
            PaintCaptureRegion::PageClip { rect, scale } => {
                validate_clip(rect, scale)?;
                (
                    LayoutRect::new(
                        rect.x - viewport_scroll.x,
                        rect.y - viewport_scroll.y,
                        rect.width,
                        rect.height,
                    ),
                    scale,
                )
            }
        };

        let mut device_scale = viewport.device_pixel_ratio * capture_scale;
        if let Some(max_width) = self.max_width {
            if max_width == 0 {
                return Err(invalid_capture("maximum width must be greater than zero"));
            }
            device_scale = device_scale.min(max_width as f32 / viewport_rect.width);
        }
        if let Some(max_height) = self.max_height {
            if max_height == 0 {
                return Err(invalid_capture("maximum height must be greater than zero"));
            }
            device_scale = device_scale.min(max_height as f32 / viewport_rect.height);
        }
        if !device_scale.is_finite() || device_scale <= 0.0 {
            return Err(invalid_capture(
                "resolved capture device scale must be finite and greater than zero",
            ));
        }

        Ok(ResolvedPaintCapture {
            viewport_rect,
            viewport_to_surface: LayoutTransform2D::translation(-viewport_rect.x, -viewport_rect.y),
            surface: PaintCaptureSurface::new(
                viewport_rect.width,
                viewport_rect.height,
                device_scale,
            ),
            include_backgrounds: self.include_backgrounds,
            paint_root_scrollbars,
        })
    }
}

fn validate_viewport(viewport: LayoutViewport) -> Result<(), LayoutError> {
    validate_extent(
        "layout viewport",
        viewport.css_width as f32,
        viewport.css_height as f32,
    )?;
    if !viewport.device_pixel_ratio.is_finite() || viewport.device_pixel_ratio <= 0.0 {
        return Err(invalid_capture(
            "layout viewport device-pixel ratio must be finite and greater than zero",
        ));
    }
    Ok(())
}

fn validate_clip(rect: LayoutRect, scale: f32) -> Result<(), LayoutError> {
    if !rect.x.is_finite() || !rect.y.is_finite() {
        return Err(invalid_capture("clip origin must be finite"));
    }
    validate_extent("clip", rect.width, rect.height)?;
    if !scale.is_finite() || scale <= 0.0 {
        return Err(invalid_capture(
            "clip scale must be finite and greater than zero",
        ));
    }
    Ok(())
}

fn validate_full_document_extent(width: f32, height: f32) -> Result<(), LayoutError> {
    validate_extent("document content", width, height)?;
    if width >= FULL_DOCUMENT_CAPTURE_CSS_DIMENSION_LIMIT
        || height >= FULL_DOCUMENT_CAPTURE_CSS_DIMENSION_LIMIT
    {
        return Err(invalid_capture(format!(
            "document content width and height must each be less than {} CSS pixels, got {width}x{height}",
            FULL_DOCUMENT_CAPTURE_CSS_DIMENSION_LIMIT
        )));
    }
    Ok(())
}

fn validate_extent(label: &str, width: f32, height: f32) -> Result<(), LayoutError> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(invalid_capture(format!(
            "{label} dimensions must be finite and greater than zero"
        )));
    }
    Ok(())
}

fn invalid_capture(detail: impl Into<String>) -> LayoutError {
    LayoutError::InvalidPaintCapture {
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_document_capture_keeps_layout_viewport_and_offsets_scroll() {
        let capture = PaintCaptureRequest::full_document()
            .resolve(
                LayoutViewport::new(800, 600, 2.0),
                LayoutPoint::new(10.0, 25.0),
                LayoutSize::new(900.0, 1400.0),
            )
            .expect("full document capture");

        assert_eq!(
            capture.viewport_rect,
            LayoutRect::new(-10.0, -25.0, 900.0, 1400.0)
        );
        assert_eq!(
            capture.surface,
            PaintCaptureSurface::new(900.0, 1400.0, 2.0)
        );
        assert_eq!(
            capture
                .viewport_to_surface
                .map_point(LayoutPoint::new(-10.0, -25.0)),
            LayoutPoint::ZERO
        );
        assert!(!capture.paint_root_scrollbars);
    }

    #[test]
    fn page_clip_combines_dpr_clip_scale_and_device_limits() {
        let capture = PaintCaptureRequest {
            region: PaintCaptureRegion::PageClip {
                rect: LayoutRect::new(120.0, 250.0, 200.0, 100.0),
                scale: 1.5,
            },
            include_backgrounds: true,
            include_viewport_controls: false,
            max_width: Some(500),
            max_height: Some(200),
        }
        .resolve(
            LayoutViewport::new(800, 600, 2.0),
            LayoutPoint::new(20.0, 50.0),
            LayoutSize::new(800.0, 1200.0),
        )
        .expect("page clip");

        assert_eq!(
            capture.viewport_rect,
            LayoutRect::new(100.0, 200.0, 200.0, 100.0)
        );
        assert_eq!(capture.surface, PaintCaptureSurface::new(200.0, 100.0, 2.0));
        assert!(!capture.paint_root_scrollbars);
    }

    #[test]
    fn viewport_capture_includes_root_scrollbars() {
        let capture = PaintCaptureRequest::viewport()
            .resolve(
                LayoutViewport::new(800, 600, 1.0),
                LayoutPoint::ZERO,
                LayoutSize::new(1200.0, 900.0),
            )
            .expect("viewport capture");

        assert!(capture.paint_root_scrollbars);
    }

    #[test]
    fn invalid_capture_dimensions_are_rejected_before_paint_projection() {
        let error = PaintCaptureRequest::page_clip(LayoutRect::new(0.0, 0.0, 0.0, 10.0), 1.0)
            .resolve(
                LayoutViewport::new(800, 600, 1.0),
                LayoutPoint::ZERO,
                LayoutSize::new(800.0, 600.0),
            )
            .expect_err("zero-width clip");
        assert!(matches!(error, LayoutError::InvalidPaintCapture { .. }));
    }

    #[test]
    fn full_document_capture_uses_an_exclusive_128k_css_dimension_limit() {
        let below_limit = FULL_DOCUMENT_CAPTURE_CSS_DIMENSION_LIMIT - 1.0;
        PaintCaptureRequest::full_document()
            .resolve(
                LayoutViewport::new(800, 600, 1.0),
                LayoutPoint::ZERO,
                LayoutSize::new(below_limit, below_limit),
            )
            .expect("dimensions immediately below 128K CSS pixels");

        for content_size in [
            LayoutSize::new(FULL_DOCUMENT_CAPTURE_CSS_DIMENSION_LIMIT, 1.0),
            LayoutSize::new(1.0, FULL_DOCUMENT_CAPTURE_CSS_DIMENSION_LIMIT),
        ] {
            let error = PaintCaptureRequest::full_document()
                .resolve(
                    LayoutViewport::new(800, 600, 1.0),
                    LayoutPoint::ZERO,
                    content_size,
                )
                .expect_err("128K CSS pixels is outside the exclusive limit");
            assert!(
                error.to_string().contains(
                    "document content width and height must each be less than 131072 CSS pixels"
                ),
                "{error}"
            );
        }
    }

    #[test]
    fn explicit_page_clip_bypasses_the_full_document_dimension_limit() {
        let clip = PaintCaptureRequest::page_clip(
            LayoutRect::new(0.0, 0.0, FULL_DOCUMENT_CAPTURE_CSS_DIMENSION_LIMIT, 1.0),
            1.0,
        )
        .resolve(
            LayoutViewport::new(800, 600, 1.0),
            LayoutPoint::ZERO,
            LayoutSize::new(800.0, 600.0),
        )
        .expect("explicit clips are validated by the raster backend");

        assert_eq!(
            clip.surface.css_width,
            FULL_DOCUMENT_CAPTURE_CSS_DIMENSION_LIMIT
        );
    }
}
