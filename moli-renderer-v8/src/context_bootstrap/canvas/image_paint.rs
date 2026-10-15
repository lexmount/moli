//! Native drawImage rectangle mapping, transformed coverage and image sampling.

use super::path::{
    Canvas2dPathState,
    coordinates::{NativeAffine, Point, Scalar},
};
use moli_canvas::{AlphaMode, PixelBuffer, PixelSource, ScaleFilter};
use moli_layout::{LayoutTransform2D, PaintFillRule};

#[derive(Clone, Copy)]
struct ImageRect {
    origin: Point,
    width: Scalar,
    height: Scalar,
}

impl ImageRect {
    fn new([x, y, width, height]: [f64; 4]) -> Option<Self> {
        if width == 0.0 || height == 0.0 {
            return None;
        }
        let mut origin = Point::new(x, y);
        let mut width = Scalar::new(width);
        let mut height = Scalar::new(height);
        if width < Scalar::ZERO {
            origin.x = origin.x + width;
            width = -width;
        }
        if height < Scalar::ZERO {
            origin.y = origin.y + height;
            height = -height;
        }
        Some(Self {
            origin,
            width,
            height,
        })
    }

    fn transformed_corners(self, matrix: kurbo::Affine, offset: Point) -> [Point; 4] {
        let origin = self.origin.transform(matrix);
        let [a, b, c, d, _, _] = matrix.as_coeffs().map(Scalar::new);
        let origin = Point {
            x: origin.x + a * offset.x + c * offset.y,
            y: origin.y + b * offset.x + d * offset.y,
        };
        let across = Point {
            x: a * self.width,
            y: b * self.width,
        };
        let down = Point {
            x: c * self.height,
            y: d * self.height,
        };
        [
            origin,
            Point {
                x: origin.x + across.x,
                y: origin.y + across.y,
            },
            Point {
                x: origin.x + across.x + down.x,
                y: origin.y + across.y + down.y,
            },
            Point {
                x: origin.x + down.x,
                y: origin.y + down.y,
            },
        ]
    }
}

pub(super) struct CanvasImagePaint<'a> {
    pixels: &'a PixelBuffer,
    path: Canvas2dPathState,
    inverse: NativeAffine,
    source: ImageRect,
    destination: ImageRect,
    filter: ScaleFilter,
    alpha: f32,
}

impl<'a> CanvasImagePaint<'a> {
    pub(super) fn new(
        pixels: &'a PixelBuffer,
        source: [f64; 4],
        destination: [f64; 4],
        transform: LayoutTransform2D,
        filter: ScaleFilter,
        alpha: f32,
    ) -> Option<Self> {
        let matrix = kurbo::Affine::new(transform.coefficients);
        let inverse = NativeAffine::inverse(matrix)?;
        let source = ImageRect::new(source)?;
        let destination = ImageRect::new(destination)?;
        let left = if source.origin.x < Scalar::ZERO {
            Scalar::ZERO
        } else {
            source.origin.x
        };
        let top = if source.origin.y < Scalar::ZERO {
            Scalar::ZERO
        } else {
            source.origin.y
        };
        let image_width = Scalar::new(f64::from(pixels.width()));
        let image_height = Scalar::new(f64::from(pixels.height()));
        let source_width = source.width - (left - source.origin.x);
        let source_height = source.height - (top - source.origin.y);
        let width = if source_width > image_width - left {
            image_width - left
        } else {
            source_width
        };
        let height = if source_height > image_height - top {
            image_height - top
        } else {
            source_height
        };
        if width <= Scalar::ZERO || height <= Scalar::ZERO {
            return None;
        }
        // Clip source and destination proportionally before applying the CTM.
        // Native scalars retain finite-input sums and products beyond f64.
        let clipped = ImageRect {
            origin: destination.origin,
            width: width * destination.width / source.width,
            height: height * destination.height / source.height,
        };
        let offset = Point {
            x: (left - source.origin.x) * destination.width / source.width,
            y: (top - source.origin.y) * destination.height / source.height,
        };
        let path =
            Canvas2dPathState::from_closed_polygon(clipped.transformed_corners(matrix, offset));
        Some(Self {
            pixels,
            path,
            inverse,
            source,
            destination,
            filter,
            alpha,
        })
    }

    pub(super) fn path(&self) -> &Canvas2dPathState {
        &self.path
    }

    pub(super) fn coverage(
        &self,
        width: u32,
        height: u32,
    ) -> Result<moli_paint::RasterImage, moli_paint::PaintError> {
        moli_paint::raster_canvas_clip_mask(
            width,
            height,
            &[moli_paint::CanvasClip {
                path: self.path.clip_paint_path(width, height)?,
                fill_rule: PaintFillRule::NonZero,
            }],
        )
    }
}

impl PixelSource for CanvasImagePaint<'_> {
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn sample(&self, x: f64, y: f64) -> [f32; 4] {
        if self.alpha == 0.0 {
            return [0.0; 4];
        }
        let local = self
            .inverse
            .map_relative(Point::new(x + 0.5, y + 0.5), self.destination.origin);
        let x = self.source.origin.x + local.x * self.source.width / self.destination.width;
        let y = self.source.origin.y + local.y * self.source.height / self.destination.height;
        self.pixels
            .sample_texel(x.lossy(), y.lossy(), self.filter)
            .map(|channel| channel * self.alpha)
    }
}
