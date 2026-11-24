use crate::MAX_DECODED_RGBA_BYTES;

/// A conservative support radius for the Gaussian kernel, in standard deviations.
pub const GAUSSIAN_BLUR_OUTSET: f32 = 4.0;
const MAX_BLUR_WORKING_BYTES: usize = 2 * MAX_DECODED_RGBA_BYTES;
const BLUR_TILE_EDGE: usize = 512;
const FLOAT_PIXEL_BYTES: usize = std::mem::size_of::<[f32; 4]>();

/// An owned, row-major, straight-alpha RGBA8 image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RgbaImageError {
    #[error("RGBA byte length overflows the host address space for {width}x{height}")]
    BufferLengthOverflow { width: u32, height: u32 },
    #[error(
        "RGBA image {width}x{height} requires {required} bytes, exceeding the {max}-byte decoded image budget"
    )]
    BufferBudgetExceeded {
        width: u32,
        height: u32,
        required: usize,
        max: usize,
    },
    #[error("RGBA input contains {actual} bytes, expected {expected}")]
    InvalidBufferLength { expected: usize, actual: usize },
    #[error("Gaussian blur requires a finite, non-negative sigma")]
    InvalidBlurSigma,
    #[error("Gaussian blur requires {required} working bytes, exceeding the {max}-byte budget")]
    BlurBudgetExceeded { required: usize, max: usize },
}

impl RgbaImage {
    pub fn try_new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, RgbaImageError> {
        let expected = checked_rgba_len(width, height)?;
        if rgba.len() != expected {
            return Err(RgbaImageError::InvalidBufferLength {
                expected,
                actual: rgba.len(),
            });
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    pub fn validate(&self) -> Result<(), RgbaImageError> {
        let expected = checked_rgba_len(self.width, self.height)?;
        if self.rgba.len() != expected {
            return Err(RgbaImageError::InvalidBufferLength {
                expected,
                actual: self.rgba.len(),
            });
        }
        Ok(())
    }

    /// Returns the exact retained RGBA payload size.
    pub fn byte_len(&self) -> usize {
        self.rgba.len()
    }

    /// Blurs straight-alpha sRGB in linear premultiplied space.
    pub fn gaussian_blur(&mut self, sigma: f32) -> Result<(), RgbaImageError> {
        self.validate()?;
        validate_gaussian_blur(self.width, self.height, sigma)?;
        if self.rgba.is_empty() || sigma < f32::MIN_POSITIVE {
            return Ok(());
        }
        let width = self.width as usize;
        let height = self.height as usize;
        let radius = blur_radius(sigma);
        let linear: [f32; 256] = std::array::from_fn(|byte| decode_srgb8(byte as u8));
        let mut output = vec![0; self.rgba.len()];
        // Each tile includes the original neighboring pixels required by the
        // kernel. Only the tile interior is copied to the final RGBA8 image.
        for top in (0..height).step_by(BLUR_TILE_EDGE) {
            let bottom = (top + BLUR_TILE_EDGE).min(height);
            let input_top = top.saturating_sub(radius);
            let input_bottom = bottom.saturating_add(radius).min(height);
            for left in (0..width).step_by(BLUR_TILE_EDGE) {
                let right = (left + BLUR_TILE_EDGE).min(width);
                let input_left = left.saturating_sub(radius);
                let input_right = right.saturating_add(radius).min(width);
                let input_width = input_right - input_left;
                let input_height = input_bottom - input_top;
                let mut pixels = Vec::with_capacity(input_width * input_height * 4);
                for row in input_top..input_bottom {
                    let start = (row * width + input_left) * 4;
                    let end = start + input_width * 4;
                    for &[red, green, blue, alpha] in self.rgba[start..end].as_chunks::<4>().0 {
                        let alpha = f32::from(alpha) / 255.0;
                        pixels.extend(
                            [red, green, blue].map(|byte| linear[usize::from(byte)] * alpha),
                        );
                        pixels.push(alpha);
                    }
                }
                let image =
                    image::Rgba32FImage::from_raw(input_width as u32, input_height as u32, pixels)
                        .expect("validated blur tile dimensions");
                let filtered = image::DynamicImage::ImageRgba32F(image)
                    .blur(sigma)
                    .into_rgba32f()
                    .into_raw();
                let row_bytes = (right - left) * 4;
                for row in top..bottom {
                    let source = ((row - input_top) * input_width + left - input_left) * 4;
                    let destination = (row * width + left) * 4;
                    for (&[red, green, blue, alpha], pixel) in filtered[source..source + row_bytes]
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(
                            output[destination..destination + row_bytes]
                                .as_chunks_mut::<4>()
                                .0,
                        )
                    {
                        pixel[3] = (alpha * 255.0).round() as u8;
                        if pixel[3] != 0 {
                            pixel[..3].copy_from_slice(
                                &[red, green, blue].map(|channel| encode_srgb8(channel / alpha)),
                            );
                        }
                    }
                }
            }
        }
        self.rgba = output;
        Ok(())
    }
}

pub fn decode_srgb8(byte: u8) -> f32 {
    let value = f32::from(byte) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

pub fn encode_srgb8(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let encoded = if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

fn blur_radius(sigma: f32) -> usize {
    (f64::from(sigma) * f64::from(GAUSSIAN_BLUR_OUTSET)).ceil() as usize
}

/// Checks destination and filter work buffers before entering the image library.
pub fn validate_gaussian_blur(width: u32, height: u32, sigma: f32) -> Result<(), RgbaImageError> {
    let bytes = checked_rgba_len(width, height)?;
    if !sigma.is_finite() || sigma < 0.0 {
        return Err(RgbaImageError::InvalidBlurSigma);
    }
    if bytes == 0 || sigma < f32::MIN_POSITIVE {
        return Ok(());
    }
    let radius = blur_radius(sigma);
    let kernel = radius.saturating_mul(2).saturating_add(1);
    let tile_edge = BLUR_TILE_EDGE.saturating_add(radius.saturating_mul(2));
    let row = (width as usize)
        .min(tile_edge)
        .saturating_mul(FLOAT_PIXEL_BYTES);
    let tile = row.saturating_mul((height as usize).min(tile_edge));
    let neighborhood = row.saturating_mul(kernel);
    // image 0.25 uses a ring of rows for short kernels. Large kernels can
    // allocate an intermediate image as well as the padded boundary rows.
    let intermediate = if kernel < 55 { 0 } else { tile };
    let required = bytes
        .saturating_add(tile.saturating_mul(2))
        .saturating_add(intermediate)
        .saturating_add(neighborhood)
        .saturating_add(row)
        // Coefficients, packed row padding, and the column's slice references.
        .saturating_add(kernel.saturating_mul(64));
    if required > MAX_BLUR_WORKING_BYTES {
        return Err(RgbaImageError::BlurBudgetExceeded {
            required,
            max: MAX_BLUR_WORKING_BYTES,
        });
    }
    Ok(())
}

impl AsRef<[u8]> for RgbaImage {
    fn as_ref(&self) -> &[u8] {
        &self.rgba
    }
}

pub(crate) fn checked_rgba_len(width: u32, height: u32) -> Result<usize, RgbaImageError> {
    let required = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(RgbaImageError::BufferLengthOverflow { width, height })?;
    if required > MAX_DECODED_RGBA_BYTES {
        return Err(RgbaImageError::BufferBudgetExceeded {
            width,
            height,
            required,
            max: MAX_DECODED_RGBA_BYTES,
        });
    }
    Ok(required)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_blur_working_memory_before_allocating_an_image() {
        // Typical kernels keep the floating-point work restricted to tiles.
        assert!(validate_gaussian_blur(4096, 8192, 2.0).is_ok());
        assert!(matches!(
            validate_gaussian_blur(4096, 8192, 512.0),
            Err(RgbaImageError::BlurBudgetExceeded { .. })
        ));
        assert_eq!(
            validate_gaussian_blur(1, 1, f32::INFINITY),
            Err(RgbaImageError::InvalidBlurSigma),
        );
    }

    #[test]
    fn blur_preserves_flat_dark_colors_and_alpha() {
        for color in [[6, 7, 8, 255], [8, 29, 96, 1], [19, 7, 29, 64]] {
            let pixels = color.repeat(9 * 17);
            let mut image = RgbaImage::try_new(9, 17, pixels.clone()).unwrap();
            image.gaussian_blur(2.0).unwrap();
            assert_eq!(image.rgba, pixels);
        }
    }

    #[test]
    fn blur_samples_transparent_neighbors_across_tile_edges() {
        fn edge_row(width: usize, edge: usize) -> RgbaImage {
            let rgba = [240, 20, 10, 0]
                .repeat(edge)
                .into_iter()
                .chain([8, 29, 96, 128].repeat(width - edge))
                .collect();
            RgbaImage::try_new(width as u32, 1, rgba).unwrap()
        }
        let mut tiled = edge_row(BLUR_TILE_EDGE + 16, BLUR_TILE_EDGE);
        let mut reference = edge_row(32, 16);
        tiled.gaussian_blur(2.0).unwrap();
        reference.gaussian_blur(2.0).unwrap();
        assert_eq!(&tiled.rgba[(BLUR_TILE_EDGE - 16) * 4..], reference.rgba);
        let [red, green, blue, alpha] = tiled.rgba.as_chunks::<4>().0[BLUR_TILE_EDGE];
        assert_eq!([red, green, blue], [8, 29, 96]);
        assert!((1..128).contains(&alpha));
    }

    #[test]
    fn validates_length_and_budget_without_allocating_the_claimed_surface() {
        assert!(matches!(
            RgbaImage::try_new(2, 2, vec![0; 15]),
            Err(RgbaImageError::InvalidBufferLength {
                expected: 16,
                actual: 15
            })
        ));
        assert!(matches!(
            RgbaImage::try_new(65_535, 65_535, Vec::new()),
            Err(RgbaImageError::BufferBudgetExceeded { .. })
        ));
    }
}
