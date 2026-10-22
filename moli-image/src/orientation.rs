//! Normalize decoded samples before producing any consumer's color planes.

use image::metadata::Orientation;

use crate::ColorDecodeError;

pub(crate) fn from_exif(exif: Option<&[u8]>) -> Orientation {
    exif.and_then(Orientation::from_exif_chunk)
        .unwrap_or(Orientation::NoTransforms)
}

pub(crate) fn dimensions(width: u32, height: u32, orientation: Orientation) -> (u32, u32) {
    match orientation {
        Orientation::Rotate90
        | Orientation::Rotate270
        | Orientation::Rotate90FlipH
        | Orientation::Rotate270FlipH => (height, width),
        _ => (width, height),
    }
}

/// The caller validates the dimensions and sample count before transforming.
/// Flips retain the existing allocation; axis exchanges reserve one bounded
/// replacement buffer. Integer samples are copied without precision changes.
pub(crate) fn apply<T: Copy>(
    samples: &mut Vec<T>,
    width: u32,
    height: u32,
    channels: usize,
    orientation: Orientation,
) -> Result<(), ColorDecodeError> {
    if orientation == Orientation::NoTransforms || samples.is_empty() {
        return Ok(());
    }
    let (width, height) = (width as usize, height as usize);
    if matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    ) {
        let mut output = Vec::new();
        output
            .try_reserve_exact(samples.len())
            .map_err(|_| ColorDecodeError::Allocation)?;
        for y in 0..width {
            for x in 0..height {
                let (source_x, source_y) = match orientation {
                    Orientation::Rotate90 => (y, height - 1 - x),
                    Orientation::Rotate270 => (width - 1 - y, x),
                    Orientation::Rotate90FlipH => (y, x),
                    Orientation::Rotate270FlipH => (width - 1 - y, height - 1 - x),
                    _ => unreachable!("axis-exchanging orientation"),
                };
                let start = (source_y * width + source_x) * channels;
                output.extend_from_slice(&samples[start..start + channels]);
            }
        }
        *samples = output;
    } else {
        let flip_x = matches!(
            orientation,
            Orientation::FlipHorizontal | Orientation::Rotate180
        );
        let flip_y = matches!(
            orientation,
            Orientation::FlipVertical | Orientation::Rotate180
        );
        for y in 0..height {
            for x in 0..width {
                let target_x = if flip_x { width - 1 - x } else { x };
                let target_y = if flip_y { height - 1 - y } else { y };
                let source = (y * width + x) * channels;
                let target = (target_y * width + target_x) * channels;
                if source < target {
                    for channel in 0..channels {
                        samples.swap(source + channel, target + channel);
                    }
                }
            }
        }
    }
    Ok(())
}
