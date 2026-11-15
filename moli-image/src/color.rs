//! Image colors retain extended sRGB until a consumer chooses its output gamut.

use ::color::ColorSpace;
use moxcms::{ColorProfile, DataColorSpace, Layout, RenderingIntent, TransformOptions};

use crate::{RgbaImage, RgbaImageError, budget::MAX_DECODED_COLOR_BYTES};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RasterColorPrecision {
    #[default]
    Unorm8,
    Float32,
}

impl RasterColorPrecision {
    pub(crate) fn retained_bytes_per_pixel(self) -> usize {
        match self {
            Self::Unorm8 => 4,
            // Layout, full precision colors and up to 16-bit unconverted samples.
            Self::Float32 => 4 + 4 * size_of::<f32>() + 4 * size_of::<u16>(),
        }
    }
}

/// Straight-alpha extended sRGB, without clipping to an RGBA8 layout surface.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorManagedRasterImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<f32>,
    pub unconverted: Option<UnconvertedRasterPixels>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum UnconvertedRasterPixels {
    Unorm8(Vec<u8>),
    Unorm16(Vec<u16>),
}

#[derive(Debug, thiserror::Error)]
pub enum ColorDecodeError {
    #[error(transparent)]
    InvalidOutput(#[from] RgbaImageError),
    #[error("unable to allocate decoded image colors")]
    Allocation,
    #[error("image color transformation failed: {0}")]
    Transformation(#[from] moxcms::CmsError),
}

pub(crate) enum ColorSamples {
    U8(Vec<u8>),
    U16(Vec<u16>),
}

impl ColorSamples {
    fn len(&self) -> usize {
        match self {
            Self::U8(values) => values.len(),
            Self::U16(values) => values.len(),
        }
    }

    fn sample(&self, index: usize) -> f32 {
        match self {
            Self::U8(values) => f32::from(values[index]) / 255.0,
            Self::U16(values) => f32::from(values[index]) / 65535.0,
        }
    }
}

pub(crate) struct RawColorImage {
    pub width: u32,
    pub height: u32,
    pub samples: ColorSamples,
    pub layout: Layout,
    pub profile: Option<ColorProfile>,
    pub cmyk: bool,
}

pub(crate) type DecodedColorPixels = (RgbaImage, Option<ColorManagedRasterImage>);

impl RawColorImage {
    pub(crate) fn finish(
        self,
        convert_profile: bool,
    ) -> Result<DecodedColorPixels, ColorDecodeError> {
        let length = crate::rgba::checked_rgba_len(self.width, self.height)?;
        let count = length / 4;
        let channels = if self.cmyk {
            4
        } else {
            match self.layout {
                Layout::Rgb => 3,
                Layout::Rgba => 4,
                Layout::Gray => 1,
                Layout::GrayAlpha => 2,
                _ => unreachable!("only decoded RGB, gray and CMYK images are admitted"),
            }
        };
        if self.samples.len() != count * channels {
            return Err(RgbaImageError::InvalidBufferLength {
                expected: count * channels,
                actual: self.samples.len(),
            }
            .into());
        }
        let transform = self
            .profile
            .as_ref()
            .filter(|profile| {
                convert_profile
                    && match profile.color_space {
                        DataColorSpace::Cmyk => self.cmyk,
                        DataColorSpace::Rgb => !self.cmyk && channels >= 3,
                        DataColorSpace::Gray => channels <= 2,
                        _ => false,
                    }
            })
            .and_then(|profile| {
                // Unsupported or malformed profiles do not make valid pixels fail
                // to load. The untagged fallback still handles CMYK and gray.
                // ICC parametric TRCs extend their linear toe to negative
                // inputs. Canvas uses the sign-preserving CSS transfer curve.
                // Keep CMS output linear, then use the native CSS color space
                // conversion without clipping wide-gamut negative components.
                let mut destination = ColorProfile::new_srgb();
                let linear = moxcms::ToneReprCurve::Parametric(vec![1.0, 1.0, 0.0, 1.0, 0.0]);
                destination.red_trc = Some(linear.clone());
                destination.green_trc = Some(linear.clone());
                destination.blue_trc = Some(linear);
                destination.cicp = None;
                profile
                    .create_transform_f32(
                        self.layout,
                        &destination,
                        Layout::Rgba,
                        TransformOptions {
                            rendering_intent: RenderingIntent::RelativeColorimetric,
                            prefer_fixed_point: false,
                            allow_extended_range_rgb_xyz: true,
                            ..Default::default()
                        },
                    )
                    .ok()
            });
        if transform.is_none()
            && let ColorSamples::U8(samples) = self.samples
        {
            // Ordinary RGBA keeps its existing zero-copy path. Untagged RGB
            // and gray also need no floating-point CMS scratch buffers.
            let rgba = if self.cmyk {
                samples
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|ink| {
                        let black = 255 - u16::from(ink[3]);
                        let color =
                            |sample: u8| (((255 - u16::from(sample)) * black + 127) / 255) as u8;
                        [color(ink[0]), color(ink[1]), color(ink[2]), 255]
                    })
                    .collect()
            } else {
                match self.layout {
                    Layout::Rgba => samples,
                    Layout::Rgb => samples
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .flat_map(|p| [p[0], p[1], p[2], 255])
                        .collect(),
                    Layout::Gray => samples.into_iter().flat_map(|v| [v, v, v, 255]).collect(),
                    Layout::GrayAlpha => samples
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .flat_map(|p| [p[0], p[0], p[0], p[1]])
                        .collect(),
                    _ => unreachable!("decoded image layout"),
                }
            };
            return Ok((RgbaImage::try_new(self.width, self.height, rgba)?, None));
        }
        let retain_precision = transform.is_some() || matches!(self.samples, ColorSamples::U16(_));
        let precision = if retain_precision {
            RasterColorPrecision::Float32
        } else {
            RasterColorPrecision::Unorm8
        };
        let retained = count * precision.retained_bytes_per_pixel();
        if retained > MAX_DECODED_COLOR_BYTES {
            return Err(RgbaImageError::BufferBudgetExceeded {
                width: self.width,
                height: self.height,
                required: retained,
                max: MAX_DECODED_COLOR_BYTES,
            }
            .into());
        }
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(length)
            .map_err(|_| ColorDecodeError::Allocation)?;
        let mut managed = retain_precision.then(Vec::new);
        if let Some(values) = &mut managed {
            values
                .try_reserve_exact(length)
                .map_err(|_| ColorDecodeError::Allocation)?;
        }
        let mut unconverted = if transform.is_some() {
            if matches!(self.samples, ColorSamples::U16(_)) {
                Some(UnconvertedRasterPixels::Unorm16(Vec::new()))
            } else {
                Some(UnconvertedRasterPixels::Unorm8(Vec::new()))
            }
        } else {
            None
        };
        match &mut unconverted {
            Some(UnconvertedRasterPixels::Unorm8(values)) => values
                .try_reserve_exact(length)
                .map_err(|_| ColorDecodeError::Allocation)?,
            Some(UnconvertedRasterPixels::Unorm16(values)) => values
                .try_reserve_exact(length)
                .map_err(|_| ColorDecodeError::Allocation)?,
            None => {}
        }
        // CMS transforms have no inter-pixel state. Fixed scratch buffers keep
        // a very wide image from allocating another full floating-point row.
        let mut input = [0.0_f32; 4096];
        let mut output = [0.0_f32; 4096];
        for first in (0..count).step_by(1024) {
            let pixels = (count - first).min(1024);
            for (index, sample) in input[..pixels * channels].iter_mut().enumerate() {
                *sample = self.samples.sample(first * channels + index);
            }
            if let Some(unconverted) = &mut unconverted {
                for source in input[..pixels * channels].chunks_exact(channels) {
                    let alpha = if !self.cmyk && self.layout.has_alpha() {
                        source[channels - 1]
                    } else {
                        1.0
                    };
                    let raw = if self.cmyk {
                        let black = 1.0 - source[3];
                        [
                            (1.0 - source[0]) * black,
                            (1.0 - source[1]) * black,
                            (1.0 - source[2]) * black,
                            alpha,
                        ]
                    } else if channels <= 2 {
                        [source[0], source[0], source[0], alpha]
                    } else {
                        [source[0], source[1], source[2], alpha]
                    };
                    match unconverted {
                        UnconvertedRasterPixels::Unorm8(values) => {
                            values.extend(raw.map(|v| (v * 255.0).round() as u8))
                        }
                        UnconvertedRasterPixels::Unorm16(values) => {
                            values.extend(raw.map(|v| (v * 65535.0).round() as u16))
                        }
                    }
                }
            }
            if let Some(transform) = &transform {
                transform.transform(&input[..pixels * channels], &mut output[..pixels * 4])?;
                for pixel in output[..pixels * 4].chunks_exact_mut(4) {
                    let encoded = ::color::Srgb::from_linear_srgb([pixel[0], pixel[1], pixel[2]]);
                    pixel[..3].copy_from_slice(&encoded);
                }
            } else {
                for (source, target) in input[..pixels * channels]
                    .chunks_exact(channels)
                    .zip(output[..pixels * 4].chunks_exact_mut(4))
                {
                    if self.cmyk {
                        let black = 1.0 - source[3];
                        target.copy_from_slice(&[
                            (1.0 - source[0]) * black,
                            (1.0 - source[1]) * black,
                            (1.0 - source[2]) * black,
                            1.0,
                        ]);
                    } else if channels <= 2 {
                        target[..3].fill(source[0]);
                    } else {
                        target[..3].copy_from_slice(&source[..3]);
                    }
                }
            }
            for (source, target) in input[..pixels * channels]
                .chunks_exact(channels)
                .zip(output[..pixels * 4].chunks_exact_mut(4))
            {
                // A CMYK K sample is ink, never an alpha channel.
                target[3] = if !self.cmyk && self.layout.has_alpha() {
                    source[channels - 1]
                } else {
                    1.0
                };
                rgba.extend(
                    target
                        .iter()
                        .map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8),
                );
            }
            if let Some(values) = &mut managed {
                values.extend_from_slice(&output[..pixels * 4]);
            }
        }
        let image = RgbaImage::try_new(self.width, self.height, rgba)?;
        Ok((
            image,
            managed.map(|rgba| ColorManagedRasterImage {
                width: self.width,
                height: self.height,
                rgba,
                unconverted,
            }),
        ))
    }
}
