//! Canvas drawing-model state and shadows, before the final bitmap clip.

use super::{context2d, style_state::CanvasPaint};
use moli_canvas::{CompositeOperation, DrawImageBlit, PixelBuffer, PixelSource, ScaleFilter};
use moli_layout::{PaintColor, PaintFragment, PaintSnapshot, PaintViewport};
use moli_paint::PaintError;

// HTML explicitly permits an implementation limit on the Gaussian sigma.
// Four standard deviations retain more than 99.99% of the kernel's mass.
const MAX_SHADOW_SIGMA: f64 = 64.0;

struct ShadowStyle {
    color: [f32; 4],
    offset: [f64; 2],
    sigma: f64,
}

pub(super) struct CanvasDrawingState {
    pub(super) operation: CompositeOperation,
    shadow: Option<ShadowStyle>,
    opaque: bool,
}

pub(super) enum ShadowSource<'a> {
    Geometry {
        fragment: &'a PaintFragment,
        paint: &'a CanvasPaint,
    },
    Text {
        text: &'a str,
        font: &'a str,
        x: f64,
        y: f64,
        paint: &'a CanvasPaint,
    },
    Image {
        pixels: &'a PixelBuffer,
        blit: DrawImageBlit,
        filter: ScaleFilter,
        alpha: f32,
    },
}

impl CanvasDrawingState {
    pub(super) fn for_context<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        context: v8::Local<'s, v8::Object>,
    ) -> Self {
        let context = moli_webapi_declare::web_api_object_target(scope, context)
            .expect("validated Canvas drawing receiver");
        let operation = context2d::context_composite_operation(scope, context);
        let offset = [
            context2d::context_number_slot(
                scope,
                context,
                super::CANVAS_CONTEXT_SHADOW_OFFSET_X_SLOT,
            )
            .unwrap_or(0.0),
            context2d::context_number_slot(
                scope,
                context,
                super::CANVAS_CONTEXT_SHADOW_OFFSET_Y_SLOT,
            )
            .unwrap_or(0.0),
        ];
        let blur =
            context2d::context_number_slot(scope, context, super::CANVAS_CONTEXT_SHADOW_BLUR_SLOT)
                .unwrap_or(0.0);
        let shadow = if blur == 0.0 && offset == [0.0; 2] {
            None
        } else {
            let color = super::style_state::context_style_channels(
                scope,
                context,
                super::CANVAS_CONTEXT_SHADOW_COLOR_SLOT,
            );
            (color[3] != 0.0).then_some(ShadowStyle {
                color,
                offset,
                sigma: (blur / 2.0).min(MAX_SHADOW_SIGMA),
            })
        };
        let opaque = shadow.is_some()
            && !super::settings::Canvas2dSettings::for_context(scope, context).alpha;
        Self {
            operation,
            shadow,
            opaque,
        }
    }

    pub(super) fn composite_shadow(
        &self,
        target: &mut PixelBuffer,
        clip: Option<&[u8]>,
        source: ShadowSource<'_>,
    ) -> Result<(), PaintError> {
        let Some(shadow) = &self.shadow else {
            return Ok(());
        };
        let width = target.width();
        let height = target.height();
        if width == 0 || height == 0 {
            return Ok(());
        }
        let region = ShadowRegion::new(width, height, shadow)?;
        let alpha = source.alpha(&region)?;
        let alpha = region.blur(alpha, shadow.sigma);
        let shadow = ShadowPixels {
            width,
            alpha,
            color: shadow.color,
        };
        target
            .composite_pixels(clip, self.operation, &shadow)
            .expect("shadow and clip use the destination dimensions");
        // An alpha:false output becomes opaque after each compositing step,
        // including the intermediate shadow, before the source is composited.
        if self.opaque {
            target.force_opaque();
        }
        Ok(())
    }
}

struct ShadowRegion {
    width: u32,
    height: u32,
    output_width: u32,
    output_height: u32,
    origin: [f64; 2],
    radius: usize,
}

impl ShadowRegion {
    fn new(width: u32, height: u32, shadow: &ShadowStyle) -> Result<Self, PaintError> {
        let radius = (shadow.sigma * 4.0).ceil() as u32;
        let overflow = PaintError::BufferLengthOverflow { width, height };
        let expanded_width = width
            .checked_add(radius * 2)
            .ok_or_else(|| overflow.clone())?;
        let expanded_height = height
            .checked_add(radius * 2)
            .ok_or_else(|| overflow.clone())?;
        // Coverage, raster work, float32 alpha, both convolution passes and the
        // final mask fit this conservative transient budget.
        let required_bytes = (expanded_width as usize)
            .checked_mul(expanded_height as usize)
            .and_then(|pixels| pixels.checked_mul(24))
            .ok_or(overflow)?;
        if required_bytes > moli_paint::MAX_TRANSIENT_RASTER_BYTES {
            return Err(PaintError::TransientRasterBudgetExceeded {
                required_bytes,
                max_bytes: moli_paint::MAX_TRANSIENT_RASTER_BYTES,
            });
        }
        Ok(Self {
            width: expanded_width,
            height: expanded_height,
            output_width: width,
            output_height: height,
            origin: [
                -shadow.offset[0] - f64::from(radius),
                -shadow.offset[1] - f64::from(radius),
            ],
            radius: radius as usize,
        })
    }

    fn blur(&self, alpha: Vec<f32>, sigma: f64) -> Vec<f32> {
        if self.radius == 0 {
            return alpha;
        }
        let width = self.output_width as usize;
        let height = self.output_height as usize;
        let first = alpha[0];
        if alpha.iter().all(|value| *value == first) {
            return vec![first; width * height];
        }
        let mut output = vec![0.0; width * height];
        let mut kernel: Vec<f64> = (0..self.radius * 2 + 1)
            .map(|index| {
                let distance = index as f64 - self.radius as f64;
                (-0.5 * (distance / sigma).powi(2)).exp()
            })
            .collect();
        let sum: f64 = kernel.iter().sum();
        for weight in &mut kernel {
            *weight /= sum;
        }
        let mut horizontal = vec![0.0; width * self.height as usize];
        // Only retain the columns that can contribute to the output bitmap.
        // Padding supplies the source on both sides, before applying any clip.
        for (source, destination) in alpha
            .chunks_exact(self.width as usize)
            .zip(horizontal.chunks_exact_mut(width))
        {
            if source.iter().all(|value| *value == 0.0) {
                continue;
            }
            for (x, value) in destination.iter_mut().enumerate() {
                *value = source[x..x + kernel.len()]
                    .iter()
                    .zip(&kernel)
                    .map(|(alpha, weight)| f64::from(*alpha) * weight)
                    .sum::<f64>() as f32;
            }
        }
        for y in 0..height {
            for x in 0..width {
                output[y * width + x] = kernel
                    .iter()
                    .enumerate()
                    .map(|(offset, weight)| {
                        f64::from(horizontal[(y + offset) * width + x]) * weight
                    })
                    .sum::<f64>() as f32;
            }
        }
        output
    }
}

impl ShadowSource<'_> {
    fn alpha(&self, region: &ShadowRegion) -> Result<Vec<f32>, PaintError> {
        let width = region.width;
        let height = region.height;
        let (coverage, paint) = match self {
            Self::Geometry { fragment, paint } => {
                let mut fragment = (*fragment).clone();
                let transform = match &mut fragment {
                    PaintFragment::Fill { transform, .. } => transform,
                    PaintFragment::Stroke(stroke) => &mut stroke.transform,
                    _ => unreachable!("Canvas geometry contains fills or strokes"),
                };
                transform.coefficients[4] -= region.origin[0];
                transform.coefficients[5] -= region.origin[1];
                let mut snapshot = PaintSnapshot::new(
                    PaintViewport::new(width, height, 1.0),
                    PaintColor::new(0.0, 0.0, 0.0, 0.0),
                );
                snapshot.push_fragment(fragment);
                (moli_paint::raster_snapshot(&snapshot)?.rgba, *paint)
            }
            Self::Text {
                text,
                font,
                x,
                y,
                paint,
            } => {
                let mut coverage = vec![
                    0;
                    moli_canvas::byte_len(width, height)
                        .expect("validated shadow dimensions")
                ];
                moli_canvas::draw_text(
                    &mut coverage,
                    width,
                    height,
                    text,
                    x - region.origin[0],
                    y - region.origin[1],
                    font,
                    [255; 4],
                );
                (coverage, *paint)
            }
            Self::Image {
                pixels,
                blit,
                filter,
                alpha,
            } => {
                let mut blit = *blit;
                blit.dest_x -= region.origin[0];
                blit.dest_y -= region.origin[1];
                return Ok((0..height)
                    .flat_map(|y| (0..width).map(move |x| (x, y)))
                    .map(|(x, y)| {
                        (pixels.sample_image(blit, *filter, f64::from(x), f64::from(y))[3] * alpha)
                            .clamp(0.0, 1.0)
                    })
                    .collect());
            }
        };
        Ok(coverage
            .chunks_exact(4)
            .enumerate()
            .map(|(index, pixel)| {
                if pixel[3] == 0 {
                    return 0.0;
                }
                let x = (index % width as usize) as f64 + region.origin[0];
                let y = (index / width as usize) as f64 + region.origin[1];
                paint.sample(x, y)[3].clamp(0.0, 1.0) * (f32::from(pixel[3]) / 255.0)
            })
            .collect())
    }
}

struct ShadowPixels {
    width: u32,
    alpha: Vec<f32>,
    color: [f32; 4],
}

impl PixelSource for ShadowPixels {
    fn sample(&self, x: f64, y: f64) -> [f32; 4] {
        let mut color = self.color;
        color[3] *= self.alpha[y as usize * self.width as usize + x as usize];
        color
    }
}
