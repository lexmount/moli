use moli_image::{RgbaImage, decode_srgb8, encode_srgb8};
use moli_page_types::VisionDeficiency;

pub(super) const BLURRED_VISION_SIGMA_CSS_PX: f32 = 2.0;

pub(super) fn apply(
    raster: &mut RgbaImage,
    deficiency: VisionDeficiency,
    raster_scale: f32,
) -> Result<(), moli_image::RgbaImageError> {
    let (matrix, offset) = match deficiency {
        VisionDeficiency::None => return Ok(()),
        VisionDeficiency::BlurredVision => {
            return raster.gaussian_blur(BLURRED_VISION_SIGMA_CSS_PX * raster_scale);
        }
        VisionDeficiency::ReducedContrast => {
            ([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], 0.5)
        }
        VisionDeficiency::Achromatopsia => ([[0.213, 0.715, 0.072]; 3], 0.0),
        // Machado, Oliveira and Fernandes (2009), severity 1.0, as in Blink.
        VisionDeficiency::Deuteranopia => (
            [
                [0.367, 0.861, -0.228],
                [0.280, 0.673, 0.047],
                [-0.012, 0.043, 0.969],
            ],
            0.0,
        ),
        VisionDeficiency::Protanopia => (
            [
                [0.152, 1.053, -0.205],
                [0.115, 0.786, 0.099],
                [-0.004, -0.048, 1.052],
            ],
            0.0,
        ),
        VisionDeficiency::Tritanopia => (
            [
                [1.256, -0.077, -0.179],
                [-0.078, 0.931, 0.148],
                [0.005, 0.691, 0.304],
            ],
            0.0,
        ),
    };
    let linear: [f32; 256] = std::array::from_fn(|byte| decode_srgb8(byte as u8));
    for pixel in raster.rgba.as_chunks_mut::<4>().0 {
        let [r, g, b] = [pixel[0], pixel[1], pixel[2]].map(|byte| linear[usize::from(byte)]);
        for (channel, [red, green, blue]) in pixel[..3].iter_mut().zip(matrix) {
            *channel = encode_srgb8(red * r + green * g + blue * b + offset);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protanopia_projects_linear_rgb_and_keeps_alpha() {
        let mut raster = RgbaImage::try_new(1, 1, vec![255, 0, 0, 128]).unwrap();
        apply(&mut raster, VisionDeficiency::Protanopia, 1.0).unwrap();
        assert_eq!(raster.rgba, [109, 95, 0, 128]);
    }
}
