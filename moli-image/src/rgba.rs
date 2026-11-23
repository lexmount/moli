use crate::MAX_DECODED_RGBA_BYTES;

/// A conservative support radius for the Gaussian kernel, in standard deviations.
pub const GAUSSIAN_BLUR_OUTSET: f32 = 4.0;
const MAX_BLUR_WORKING_BYTES: usize = 2 * MAX_DECODED_RGBA_BYTES;

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

    pub fn gaussian_blur(&mut self, sigma: f32) -> Result<(), RgbaImageError> {
        self.validate()?;
        validate_gaussian_blur(self.width, self.height, sigma)?;
        if self.rgba.is_empty() || sigma < f32::MIN_POSITIVE {
            return Ok(());
        }
        let image =
            image::RgbaImage::from_raw(self.width, self.height, std::mem::take(&mut self.rgba))
                .expect("validated RGBA dimensions");
        // The dynamically typed RGBA8 path filters bytes directly. The generic
        // blur API expands the whole input and destination to f32.
        self.rgba = image::DynamicImage::ImageRgba8(image)
            .blur(sigma)
            .into_rgba8()
            .into_raw();
        Ok(())
    }
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
    let radius = (f64::from(sigma) * f64::from(GAUSSIAN_BLUR_OUTSET)).ceil() as usize;
    let kernel = radius.saturating_mul(2).saturating_add(1);
    let row = (width as usize).saturating_mul(4);
    let neighborhood = row.saturating_mul(kernel);
    // image 0.25 uses a ring of rows for short kernels. Large kernels can
    // allocate an intermediate image as well as the padded boundary rows.
    let intermediate = if kernel < 55 { 0 } else { bytes };
    let required = bytes
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
        // Typical kernels use rows of RGBA8 scratch even on a large image.
        assert!(validate_gaussian_blur(4096, 8192, 2.0).is_ok());
        assert!(matches!(
            validate_gaussian_blur(4096, 8192, 64.0),
            Err(RgbaImageError::BlurBudgetExceeded { .. })
        ));
        assert_eq!(
            validate_gaussian_blur(1, 1, f32::INFINITY),
            Err(RgbaImageError::InvalidBlurSigma),
        );
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
