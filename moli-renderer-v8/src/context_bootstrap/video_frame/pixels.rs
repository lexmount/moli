//! CPU pixel export and display geometry; raw plane copies bypass conversion.

use super::{PixelFormat, Rect, VideoFrameInfo};
use crate::context_bootstrap::video_color_space::{
    VideoColorPrimaries as Primaries, VideoMatrixCoefficients as Matrix,
    VideoTransferCharacteristics as Transfer,
};
use crate::webidl;

#[derive(Clone, Copy, Default, webidl::WebIdlEnum)]
pub(super) enum RgbColorSpace {
    #[default]
    Srgb,
    #[webidl(token = "display-p3")]
    DisplayP3,
}

fn allocate(scope: &mut v8::PinScope<'_, '_>, size: usize) -> Option<Vec<u8>> {
    if size > moli_canvas::MAX_RGBA8_BYTE_LENGTH {
        webidl::throw_dom_exception(
            scope,
            "InvalidStateError",
            "The frame exceeds the CPU raster allocation limit.",
        );
        return None;
    }
    let mut bytes = Vec::new();
    if bytes.try_reserve_exact(size).is_err() {
        webidl::throw_dom_exception(
            scope,
            "InvalidStateError",
            "The frame pixels could not be allocated.",
        );
        return None;
    }
    bytes.resize(size, 0);
    Some(bytes)
}

pub(super) fn convert(
    scope: &mut v8::PinScope<'_, '_>,
    info: &VideoFrameInfo,
    data: &[u8],
    rect: Rect,
    format: PixelFormat,
    destination: RgbColorSpace,
) -> Option<Vec<u8>> {
    if matches!(
        info.color_space.transfer,
        Some(Transfer::Pq | Transfer::Hlg)
    ) {
        webidl::throw_dom_exception(
            scope,
            "NotSupportedError",
            "HDR tone mapping is not available.",
        );
        return None;
    }
    let size = (rect.width as usize)
        .checked_mul(rect.height as usize)?
        .checked_mul(4)?;
    let mut output = allocate(scope, size)?;
    let specs = info.format.planes();
    let depth = info.format.depth();
    let max = ((1u32 << depth) - 1) as f64;
    for y in 0..rect.height {
        for x in 0..rect.width {
            let sx = x + rect.x;
            let sy = y + rect.y;
            let sample = |plane: usize, component: usize| {
                let spec = specs[plane];
                let layout = info.layout[plane];
                let pos = layout.offset as usize
                    + (sy / spec.height) as usize * layout.stride as usize
                    + (sx / spec.width) as usize * spec.bytes as usize
                    + component;
                if depth == 8 {
                    data[pos] as f64
                } else {
                    u16::from_le_bytes([data[pos], data[pos + 1]]) as f64
                }
            };
            let (mut rgb, alpha) = if info.format.rgb() {
                let offset = info.layout[0].offset as usize
                    + sy as usize * info.layout[0].stride as usize
                    + sx as usize * 4;
                let p = &data[offset..offset + 4];
                let rgb = if matches!(info.format, PixelFormat::Bgra | PixelFormat::Bgrx) {
                    [p[2], p[1], p[0]]
                } else {
                    [p[0], p[1], p[2]]
                };
                (
                    rgb.map(|v| v as f64 / 255.0),
                    if info.format.has_alpha() { p[3] } else { 255 },
                )
            } else {
                let (kr, kb) = match info.color_space.matrix {
                    Some(Matrix::Bt470Bg | Matrix::Smpte170M) => (0.299, 0.114),
                    Some(Matrix::Bt2020Ncl) => (0.2627, 0.0593),
                    _ => (0.2126, 0.0722),
                };
                let factor = (1u32 << (depth - 8)) as f64;
                let full = info.color_space.full_range.unwrap_or(false);
                let luma = if full {
                    sample(0, 0) / max
                } else {
                    (sample(0, 0) - 16.0 * factor) / (219.0 * factor)
                };
                let chroma = if info.format == PixelFormat::NV12 {
                    [sample(1, 0), sample(1, 1)]
                } else {
                    [sample(1, 0), sample(2, 0)]
                };
                let [u, v] =
                    chroma.map(|v| (v - 128.0 * factor) / if full { max } else { 224.0 * factor });
                let r = luma + 2.0 * (1.0 - kr) * v;
                let b = luma + 2.0 * (1.0 - kb) * u;
                let g = (luma - kr * r - kb * b) / (1.0 - kr - kb);
                (
                    [r, g, b],
                    if info.format.has_alpha() {
                        (sample(3, 0) / max * 255.0).round().clamp(0.0, 255.0) as u8
                    } else {
                        255
                    },
                )
            };
            let identity_color = info.format.rgb()
                && matches!(destination, RgbColorSpace::Srgb)
                && matches!(info.color_space.primaries, None | Some(Primaries::Bt709))
                && matches!(
                    info.color_space.transfer,
                    None | Some(Transfer::Iec61966_2_1)
                );
            if !identity_color {
                let transfer = info.color_space.transfer.unwrap_or(if info.format.rgb() {
                    Transfer::Iec61966_2_1
                } else {
                    Transfer::Bt709
                });
                let linear = rgb.map(|v| match transfer {
                    Transfer::Linear => v,
                    Transfer::Iec61966_2_1 => {
                        if v <= 0.04045 {
                            v / 12.92
                        } else {
                            ((v + 0.055) / 1.055).powf(2.4)
                        }
                    }
                    _ => {
                        if v < 0.081 {
                            v / 4.5
                        } else {
                            ((v + 0.099) / 1.099).powf(1.0 / 0.45)
                        }
                    }
                });
                // D65 matrices from CSS Color's linear-light RGB conversion.
                let to_xyz = match info.color_space.primaries.unwrap_or(Primaries::Bt709) {
                    Primaries::Bt2020 => [
                        [0.636958, 0.144617, 0.168881],
                        [0.262700, 0.677998, 0.059302],
                        [0.0, 0.028073, 1.060985],
                    ],
                    Primaries::Smpte432 => [
                        [0.486571, 0.265668, 0.198217],
                        [0.228975, 0.691739, 0.079287],
                        [0.0, 0.045113, 1.043944],
                    ],
                    Primaries::Smpte170M => [
                        [0.393521, 0.365258, 0.191677],
                        [0.212376, 0.701060, 0.086564],
                        [0.018739, 0.111934, 0.958385],
                    ],
                    Primaries::Bt470Bg => [
                        [0.430619, 0.341541, 0.178309],
                        [0.222037, 0.706638, 0.071325],
                        [0.020185, 0.129551, 0.939094],
                    ],
                    Primaries::Bt709 => [
                        [0.412391, 0.357584, 0.180481],
                        [0.212639, 0.715169, 0.072192],
                        [0.019331, 0.119195, 0.950532],
                    ],
                };
                let xyz = multiply(to_xyz, linear);
                let from_xyz = match destination {
                    RgbColorSpace::Srgb => [
                        [3.240970, -1.537383, -0.498611],
                        [-0.969244, 1.875968, 0.041555],
                        [0.055630, -0.203977, 1.056972],
                    ],
                    RgbColorSpace::DisplayP3 => [
                        [2.493497, -0.931384, -0.402711],
                        [-0.829489, 1.762664, 0.023625],
                        [0.035846, -0.076172, 0.956885],
                    ],
                };
                rgb = multiply(from_xyz, xyz).map(|v| {
                    if v <= 0.0031308 {
                        12.92 * v
                    } else {
                        1.055 * v.powf(1.0 / 2.4) - 0.055
                    }
                });
            }
            let mut p = rgb.map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8);
            if matches!(format, PixelFormat::Bgra | PixelFormat::Bgrx) {
                p.swap(0, 2);
            }
            let offset = (y as usize * rect.width as usize + x as usize) * 4;
            output[offset..offset + 3].copy_from_slice(&p);
            output[offset + 3] = if format.has_alpha() { alpha } else { 255 };
        }
    }
    Some(output)
}

fn multiply(matrix: [[f64; 3]; 3], values: [f64; 3]) -> [f64; 3] {
    matrix.map(|row| row.iter().zip(values).map(|(a, b)| a * b).sum())
}

pub(super) fn render(
    scope: &mut v8::PinScope<'_, '_>,
    info: &VideoFrameInfo,
    pixels: Vec<u8>,
) -> Option<(Vec<u8>, u32, u32)> {
    let (w, h) = (info.visible.width, info.visible.height);
    let (rw, rh) = if info.rotation.is_multiple_of(180) {
        (w, h)
    } else {
        (h, w)
    };
    let mut rotated = allocate(scope, pixels.len())?;
    for y in 0..h {
        for x in 0..w {
            let (mut dx, dy) = match info.rotation {
                90 => (h - 1 - y, x),
                180 => (w - 1 - x, h - 1 - y),
                270 => (y, w - 1 - x),
                _ => (x, y),
            };
            if info.flip {
                dx = rw - 1 - dx;
            }
            let from = (y as usize * w as usize + x as usize) * 4;
            let to = (dy as usize * rw as usize + dx as usize) * 4;
            rotated[to..to + 4].copy_from_slice(&pixels[from..from + 4]);
        }
    }
    if (rw, rh) == (info.display_width, info.display_height) {
        return Some((rotated, rw, rh));
    }
    let size = (info.display_width as usize)
        .checked_mul(info.display_height as usize)?
        .checked_mul(4)?;
    let mut scaled = allocate(scope, size)?;
    moli_canvas::blit_draw_image_filtered(
        &mut scaled,
        info.display_width,
        info.display_height,
        &rotated,
        rw,
        rh,
        moli_canvas::DrawImageBlit::new(
            0.0,
            0.0,
            rw as f64,
            rh as f64,
            0.0,
            0.0,
            info.display_width as f64,
            info.display_height as f64,
        )?,
        moli_canvas::ScaleFilter::Bilinear,
    );
    Some((scaled, info.display_width, info.display_height))
}
