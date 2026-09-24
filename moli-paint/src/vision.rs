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
// hidden color into visible neighbors. Process horizontal bands with enough
// transparent overlap for the Gaussian kernel instead of retaining full-size
// float images for the entire capture.
fn blur(raster: &mut RasterImage, device_scale: f32) -> Result<(), PaintError> {
    if !(device_scale * 2.0).is_normal() || device_scale <= 0.0 {
        return Err(PaintError::InvalidCaptureDeviceScale { device_scale });
    }
    // Four sigma covers image::blur's finite kernel on both sides of a band.
    let padding = (8.0 * device_scale).ceil() as u32;
    let width = raster.width.checked_add(padding.saturating_mul(2)).ok_or(
        PaintError::BufferLengthOverflow {
            width: raster.width,
            height: raster.height,
        },
    )?;
    if raster.width == 0 || raster.height == 0 {
        return Ok(());
    }
    // The filter holds its input, two work images, and output simultaneously.
    let float_row_bytes = (width as usize).saturating_mul(64);
    let raster_bytes = raster.rgba.len().saturating_mul(2);
    let max_rows = (MAX_TRANSIENT_RASTER_BYTES.saturating_sub(raster_bytes) / float_row_bytes)
        .saturating_sub(padding as usize * 2);
    if max_rows == 0 {
        return Err(PaintError::TransientRasterBudgetExceeded {
            required_bytes: float_row_bytes
                .saturating_mul(padding as usize * 2 + 1)
                .saturating_add(raster_bytes),
            max_bytes: MAX_TRANSIENT_RASTER_BYTES,
        });
    }
    let band_rows = max_rows.min(256).min(raster.height as usize) as u32;
    let mut result = vec![0; raster.rgba.len()];
    for start_y in (0..raster.height).step_by(band_rows as usize) {
        let rows = band_rows.min(raster.height - start_y);
        let image = image::Rgba32FImage::from_fn(width, rows + padding * 2, |x, y| {
            let source_y = start_y.checked_add(y).and_then(|y| y.checked_sub(padding));
            let Some(source_y) = source_y.filter(|&y| y < raster.height) else {
                return image::Rgba([0.0; 4]);
            };
            let Some(source_x) = x.checked_sub(padding).filter(|&x| x < raster.width) else {
                return image::Rgba([0.0; 4]);
            };
            let offset = ((source_y as usize) * raster.width as usize + source_x as usize) * 4;
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
        for y in 0..rows {
            let offset = ((start_y + y) as usize * raster.width as usize) * 4;
            let row = &mut result[offset..offset + raster.width as usize * 4];
            for (x, output) in row.chunks_exact_mut(4).enumerate() {
                let input = blurred.get_pixel(x as u32 + padding, y + padding);
                let alpha = input[3];
                for (out, value) in output[..3].iter_mut().zip(input.0) {
                    *out = linear_to_srgb(if alpha > 0.0 { value / alpha } else { 0.0 });
                }
                output[3] = (alpha * 255.0).round() as u8;
            }
        }
    }
    raster.rgba = result;
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

    #[test]
    fn blurred_vision_accepts_4k_capture() {
        let mut raster = RasterImage {
            width: 3840,
            height: 2160,
            rgba: vec![255; 3840 * 2160 * 4],
        };
        VisionDeficiency::BlurredVision
            .apply(&mut raster, 1.0)
            .unwrap();
        for y in [255, 256, 511, 512, 1080] {
            assert_eq!(&raster.rgba[(y * 3840 + 1920) * 4..][..4], &[255; 4]);
        }
    }
}
