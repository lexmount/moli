//! Display-wide vision emulation in linear RGB, matching SVG filter color space.

use crate::{MAX_TRANSIENT_RASTER_BYTES, PaintError, RasterImage};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VisionDeficiency {
    #[default]
    None,
    BlurredVision,
    ReducedContrast,
    Achromatopsia,
    Deuteranopia,
    Protanopia,
    Tritanopia,
}

impl VisionDeficiency {
    pub fn apply(self, raster: &mut RasterImage, device_scale: f32) -> Result<(), PaintError> {
        if self == Self::None {
            return Ok(());
        }
        let expected = (raster.width as usize)
            .checked_mul(raster.height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(PaintError::BufferLengthOverflow {
                width: raster.width,
                height: raster.height,
            })?;
        if raster.rgba.len() != expected {
            return Err(PaintError::UnexpectedBufferLength {
                expected,
                actual: raster.rgba.len(),
            });
        }
        if self == Self::BlurredVision {
            return blur(raster, device_scale);
        }
        // Machado et al. (2009), full-severity matrices rounded as in Blink's
        // core/css/vision_deficiency.cc. Alpha is not affected by color emulation.
        let matrix = match self {
            Self::Achromatopsia => [[0.213, 0.715, 0.072]; 3],
            Self::Deuteranopia => [
                [0.367, 0.861, -0.228],
                [0.280, 0.673, 0.047],
                [-0.012, 0.043, 0.969],
            ],
            Self::Protanopia => [
                [0.152, 1.053, -0.205],
                [0.115, 0.786, 0.099],
                [-0.004, -0.048, 1.052],
            ],
            Self::Tritanopia => [
                [1.256, -0.077, -0.179],
                [-0.078, 0.931, 0.148],
                [0.005, 0.691, 0.304],
            ],
            Self::ReducedContrast => [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            Self::None | Self::BlurredVision => unreachable!(),
        };
        let offset = if self == Self::ReducedContrast {
            0.5
        } else {
            0.0
        };
        for pixel in raster.rgba.chunks_exact_mut(4) {
            let rgb = [pixel[0], pixel[1], pixel[2]].map(srgb_to_linear);
            for (channel, row) in pixel[..3].iter_mut().zip(matrix) {
                let value = row
                    .into_iter()
                    .zip(rgb)
                    .map(|(weight, value)| weight * value)
                    .sum::<f32>()
                    + offset;
                *channel = linear_to_srgb(value);
            }
        }
        Ok(())
    }
}

fn srgb_to_linear(value: u8) -> f32 {
    let value = f32::from(value) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let value = if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (value * 255.0).round() as u8
}

// Filter in premultiplied linear RGB so transparent pixels cannot bleed their
// hidden color into visible neighbors. Charge the source, intermediate and
// destination float images against the same budget as paint filters.
fn blur(raster: &mut RasterImage, device_scale: f32) -> Result<(), PaintError> {
    if !(device_scale * 2.0).is_normal() || device_scale <= 0.0 {
        return Err(PaintError::InvalidCaptureDeviceScale { device_scale });
    }
    let padding = (6.0 * device_scale).ceil() as u32;
    let (width, height) = raster
        .width
        .checked_add(padding.saturating_mul(2))
        .zip(raster.height.checked_add(padding.saturating_mul(2)))
        .ok_or(PaintError::BufferLengthOverflow {
            width: raster.width,
            height: raster.height,
        })?;
    let required_bytes = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(48)
        .saturating_add(raster.rgba.len());
    if required_bytes > MAX_TRANSIENT_RASTER_BYTES {
        return Err(PaintError::TransientRasterBudgetExceeded {
            required_bytes,
            max_bytes: MAX_TRANSIENT_RASTER_BYTES,
        });
    }
    if raster.width == 0 || raster.height == 0 {
        return Ok(());
    }
    let image = image::Rgba32FImage::from_fn(width, height, |x, y| {
        if x < padding || y < padding || x >= padding + raster.width || y >= padding + raster.height
        {
            return image::Rgba([0.0; 4]);
        }
        let (x, y) = (x - padding, y - padding);
        let offset = ((y as usize) * raster.width as usize + x as usize) * 4;
        let pixel = &raster.rgba[offset..offset + 4];
        let alpha = f32::from(pixel[3]) / 255.0;
        image::Rgba([
            srgb_to_linear(pixel[0]) * alpha,
            srgb_to_linear(pixel[1]) * alpha,
            srgb_to_linear(pixel[2]) * alpha,
            alpha,
        ])
    });
    let blurred = image::imageops::blur(&image, 2.0 * device_scale);
    for (index, output) in raster.rgba.chunks_exact_mut(4).enumerate() {
        let input = blurred.get_pixel(
            index as u32 % raster.width + padding,
            index as u32 / raster.width + padding,
        );
        let alpha = input[3];
        for (out, value) in output[..3].iter_mut().zip(input.0) {
            *out = linear_to_srgb(if alpha > 0.0 { value / alpha } else { 0.0 });
        }
        output[3] = (alpha * 255.0).round() as u8;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_filters_preserve_alpha_and_neutral_colors() {
        for filter in [
            VisionDeficiency::Achromatopsia,
            VisionDeficiency::Deuteranopia,
            VisionDeficiency::Protanopia,
            VisionDeficiency::Tritanopia,
        ] {
            let mut raster = RasterImage {
                width: 3,
                height: 1,
                rgba: vec![0, 0, 0, 255, 128, 128, 128, 64, 255, 255, 255, 0],
            };
            filter.apply(&mut raster, 1.0).unwrap();
            assert_eq!(
                raster.rgba,
                [0, 0, 0, 255, 128, 128, 128, 64, 255, 255, 255, 0]
            );
        }
    }

    #[test]
    fn blur_does_not_expose_transparent_pixel_colors() {
        let mut raster = RasterImage {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 0, 0, 0, 255, 255],
        };
        VisionDeficiency::BlurredVision
            .apply(&mut raster, 1.0)
            .unwrap();
        for pixel in raster.rgba.chunks_exact(4) {
            assert_eq!(&pixel[..3], &[0, 0, 255]);
            assert!(pixel[3] > 0 && pixel[3] < 255);
        }
    }

    #[test]
    fn invalid_buffer_is_rejected_without_mutation() {
        let mut raster = RasterImage {
            width: 2,
            height: 1,
            rgba: vec![1, 2, 3, 4],
        };
        assert!(matches!(
            VisionDeficiency::BlurredVision.apply(&mut raster, 1.0),
            Err(PaintError::UnexpectedBufferLength { .. })
        ));
        assert_eq!(raster.rgba, [1, 2, 3, 4]);
    }
}
