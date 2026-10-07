//! CSS filter backend adaptation.
//!
//! Conversion starts from Blitz d788124a's AnyRender filter mapping. Blur and
//! drop-shadow execute directly in the pinned Vello CPU 0.2 backend. The other
//! CSS color-only functions are applied to an isolated one-shot RGBA layer so
//! they share one complete matrix implementation instead of depending on the
//! backend's currently partial single-node filter coverage.

use anyrender::{Filter, filters::FilterEffect};
use moli_layout::PaintFilter;

pub(super) fn backend_filter(filter: &PaintFilter) -> Option<Filter> {
    let effect = match *filter {
        PaintFilter::Blur(radius) => FilterEffect::blur(finite_nonnegative(radius)),
        PaintFilter::DropShadow {
            offset,
            blur_radius,
            color,
        } => FilterEffect::drop_shadow(
            finite_or_zero(offset.x),
            finite_or_zero(offset.y),
            finite_nonnegative(blur_radius),
            super::to_backend_color(color),
        ),
        PaintFilter::Brightness(_)
        | PaintFilter::Contrast(_)
        | PaintFilter::Grayscale(_)
        | PaintFilter::HueRotate(_)
        | PaintFilter::Invert(_)
        | PaintFilter::Opacity(_)
        | PaintFilter::Saturate(_)
        | PaintFilter::Sepia(_) => return None,
    };
    Some(Filter::single(effect))
}

pub(super) fn is_software_color_filter(filter: &PaintFilter) -> bool {
    backend_filter(filter).is_none()
}

pub(super) fn apply_software_color_filter(rgba: &mut [u8], filter: &PaintFilter) {
    use anyrender::filters::color_transformation::ColorMatrix;

    let matrix = match *filter {
        PaintFilter::Brightness(amount) => rgb_linear_matrix(amount, 0.0),
        PaintFilter::Contrast(amount) => rgb_linear_matrix(amount, -(0.5 * amount) + 0.5),
        PaintFilter::Grayscale(amount) => ColorMatrix::grayscale(amount),
        PaintFilter::HueRotate(angle) => ColorMatrix::hue_rotate(angle),
        PaintFilter::Invert(amount) => rgb_linear_matrix(1.0 - 2.0 * amount, amount),
        PaintFilter::Opacity(amount) => ColorMatrix([
            1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0,
            0.0, amount, 0.0,
        ]),
        PaintFilter::Saturate(amount) => ColorMatrix::saturate(amount),
        PaintFilter::Sepia(amount) => ColorMatrix::sepia(amount),
        PaintFilter::Blur(_) | PaintFilter::DropShadow { .. } => return,
    };
    apply_color_matrix(rgba, &matrix.0);
}

fn apply_color_matrix(rgba: &mut [u8], matrix: &[f32; 20]) {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let input = [
            f32::from(pixel[0]) / 255.0,
            f32::from(pixel[1]) / 255.0,
            f32::from(pixel[2]) / 255.0,
            f32::from(pixel[3]) / 255.0,
        ];
        for (channel, output_channel) in pixel.iter_mut().enumerate() {
            let row = channel * 5;
            let output = matrix[row] * input[0]
                + matrix[row + 1] * input[1]
                + matrix[row + 2] * input[2]
                + matrix[row + 3] * input[3]
                + matrix[row + 4];
            *output_channel = (finite_or_zero(output).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
}

fn rgb_linear_matrix(
    slope: f32,
    intercept: f32,
) -> anyrender::filters::color_transformation::ColorMatrix {
    anyrender::filters::color_transformation::ColorMatrix([
        slope, 0.0, 0.0, 0.0, intercept, 0.0, slope, 0.0, 0.0, intercept, 0.0, 0.0, slope, 0.0,
        intercept, 0.0, 0.0, 0.0, 1.0, 0.0,
    ])
}

fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn finite_nonnegative(value: f32) -> f32 {
    finite_or_zero(value).max(0.0)
}
