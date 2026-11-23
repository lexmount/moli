use crate::MAX_DECODED_RGBA_BYTES;

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
        let image =
            image::RgbaImage::from_raw(self.width, self.height, std::mem::take(&mut self.rgba))
                .expect("validated RGBA dimensions");
        self.rgba = image::imageops::blur(&image, sigma).into_raw();
        Ok(())
    }
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
