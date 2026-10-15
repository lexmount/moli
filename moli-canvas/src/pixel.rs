use crate::types::{DrawImageBlit, Rgba8Rect, ScaleFilter, byte_len, surface_matches_len};

#[inline]
pub fn multiply_u8_color(a: u8, b: u8) -> u8 {
    let c = a as u32 * b as u32 + 128;
    ((c + (c >> 8)) >> 8) as u8
}

pub fn premultiply_rgba8_in_place(pixels: &mut [u8]) -> Option<bool> {
    if !pixels.len().is_multiple_of(4) {
        return None;
    }
    let mut opaque = true;
    for rgba in pixels.chunks_mut(4) {
        rgba[0] = multiply_u8_color(rgba[0], rgba[3]);
        rgba[1] = multiply_u8_color(rgba[1], rgba[3]);
        rgba[2] = multiply_u8_color(rgba[2], rgba[3]);
        opaque &= rgba[3] == u8::MAX;
    }
    Some(opaque)
}

/// Converts premultiplied RGBA8 to straight (non-premultiplied) RGBA8 in place.
///
/// This is the inverse of [`premultiply_rgba8_in_place`] and is used at every
/// boundary that publishes canvas pixels to straight-alpha consumers (region
/// readback, `ImageData`, PNG encoding, page snapshots). Transparent pixels keep
/// arbitrary RGB and are normalized to transparent black so repeats stay stable.
/// Returns `None` when the byte length is not a multiple of four.
pub fn unpremultiply_rgba8_in_place(pixels: &mut [u8]) -> Option<bool> {
    if !pixels.len().is_multiple_of(4) {
        return None;
    }
    let mut opaque = true;
    for rgba in pixels.chunks_mut(4) {
        let alpha = u32::from(rgba[3]);
        if alpha == 0 {
            rgba[0] = 0;
            rgba[1] = 0;
            rgba[2] = 0;
            opaque = false;
            continue;
        }
        opaque &= alpha == u32::from(u8::MAX);
        if alpha == u32::from(u8::MAX) {
            continue;
        }
        for value in rgba[..3].iter_mut() {
            *value = ((u32::from(*value) * 255 + alpha / 2) / alpha) as u8;
        }
    }
    Some(opaque)
}

pub fn flip_y_rgba8_in_place(pixels: &mut [u8], width: u32, height: u32) -> Option<()> {
    if !surface_matches_len(pixels, width, height) {
        return None;
    }
    let row_len = width as usize * 4;
    let half_height = height as usize / 2;
    let (top_half, bottom_half) = pixels.split_at_mut(pixels.len() - row_len * half_height);
    for row in 0..half_height {
        let top = &mut top_half[row * row_len..][..row_len];
        let bottom = &mut bottom_half[(half_height - row - 1) * row_len..][..row_len];
        top.swap_with_slice(bottom);
    }
    Some(())
}

pub fn copy_rgba8_rect(
    source: &[u8],
    source_width: u32,
    source_height: u32,
    source_rect: Rgba8Rect,
    dest: &mut [u8],
    dest_width: u32,
    dest_height: u32,
    dest_x: u32,
    dest_y: u32,
) -> Option<()> {
    if !surface_matches_len(source, source_width, source_height)
        || !surface_matches_len(dest, dest_width, dest_height)
        || !source_rect.fits_within(source_width, source_height)
        || !(Rgba8Rect {
            x: dest_x,
            y: dest_y,
            width: source_rect.width,
            height: source_rect.height,
        })
        .fits_within(dest_width, dest_height)
    {
        return None;
    }

    if source_width == dest_width
        && source_height == dest_height
        && source_rect.x == dest_x
        && source_rect.y == dest_y
        && source_rect.width == source_width
        && source_rect.height == source_height
    {
        dest.copy_from_slice(source);
        return Some(());
    }

    let source_row_len = source_width as usize * 4;
    let dest_row_len = dest_width as usize * 4;
    let source_col_offset = source_rect.x as usize * 4;
    let dest_col_offset = dest_x as usize * 4;
    let row_copy_len = source_rect.width as usize * 4;
    let source_first_row = source_rect.y as usize * source_row_len;
    let dest_first_row = dest_y as usize * dest_row_len;

    for row in 0..source_rect.height as usize {
        let source_start = source_first_row + row * source_row_len + source_col_offset;
        let dest_start = dest_first_row + row * dest_row_len + dest_col_offset;
        dest[dest_start..dest_start + row_copy_len]
            .copy_from_slice(&source[source_start..source_start + row_copy_len]);
    }
    Some(())
}

#[allow(clippy::too_many_arguments)]
pub fn scale_rgba8_nearest(
    source: &[u8],
    source_width: u32,
    source_height: u32,
    blit: DrawImageBlit,
    target_left: i32,
    target_top: i32,
    target_width: u32,
    target_height: u32,
) -> Option<Vec<u8>> {
    if !surface_matches_len(source, source_width, source_height) {
        return None;
    }
    let mut scaled = vec![0; byte_len(target_width, target_height)?];
    let scale_x = blit.source_width / blit.dest_width;
    let scale_y = blit.source_height / blit.dest_height;
    for target_y in 0..target_height {
        let dest_y = target_top + target_y as i32;
        let local_y = dest_y as f64 - blit.dest_y;
        let src_y = (blit.source_y + local_y * scale_y).floor() as i32;
        if src_y < 0 || src_y >= source_height as i32 {
            continue;
        }
        for target_x in 0..target_width {
            let dest_x = target_left + target_x as i32;
            let local_x = dest_x as f64 - blit.dest_x;
            let src_x = (blit.source_x + local_x * scale_x).floor() as i32;
            if src_x < 0 || src_x >= source_width as i32 {
                continue;
            }
            let source_index = (((src_y as u32) * source_width + src_x as u32) * 4) as usize;
            let target_index = ((target_y * target_width + target_x) * 4) as usize;
            scaled[target_index..target_index + 4]
                .copy_from_slice(&source[source_index..source_index + 4]);
        }
    }
    Some(scaled)
}

#[allow(clippy::too_many_arguments)]
pub fn scale_rgba8(
    source: &[u8],
    source_width: u32,
    source_height: u32,
    blit: DrawImageBlit,
    target_left: i32,
    target_top: i32,
    target_width: u32,
    target_height: u32,
    filter: ScaleFilter,
) -> Option<Vec<u8>> {
    match filter {
        ScaleFilter::Nearest => scale_rgba8_nearest(
            source,
            source_width,
            source_height,
            blit,
            target_left,
            target_top,
            target_width,
            target_height,
        ),
        ScaleFilter::Bilinear => scale_rgba8_bilinear(
            source,
            source_width,
            source_height,
            blit,
            target_left,
            target_top,
            target_width,
            target_height,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn scale_rgba8_bilinear(
    source: &[u8],
    source_width: u32,
    source_height: u32,
    blit: DrawImageBlit,
    target_left: i32,
    target_top: i32,
    target_width: u32,
    target_height: u32,
) -> Option<Vec<u8>> {
    if !surface_matches_len(source, source_width, source_height) {
        return None;
    }
    let mut scaled = vec![0; byte_len(target_width, target_height)?];
    let scale_x = blit.source_width / blit.dest_width;
    let scale_y = blit.source_height / blit.dest_height;
    for target_y in 0..target_height {
        let dest_y = target_top + target_y as i32;
        let local_y = dest_y as f64 - blit.dest_y;
        let src_y = blit.source_y + local_y * scale_y;
        if src_y < 0.0 || src_y >= source_height as f64 {
            continue;
        }
        for target_x in 0..target_width {
            let dest_x = target_left + target_x as i32;
            let local_x = dest_x as f64 - blit.dest_x;
            let src_x = blit.source_x + local_x * scale_x;
            if src_x < 0.0 || src_x >= source_width as f64 {
                continue;
            }
            let rgba = sample_rgba8_bilinear(source, source_width, source_height, src_x, src_y);
            let target_index = ((target_y * target_width + target_x) * 4) as usize;
            scaled[target_index..target_index + 4].copy_from_slice(&rgba);
        }
    }
    Some(scaled)
}

fn sample_rgba8_bilinear(
    source: &[u8],
    source_width: u32,
    source_height: u32,
    x: f64,
    y: f64,
) -> [u8; 4] {
    let x0 = x.floor().max(0.0).min((source_width - 1) as f64) as u32;
    let y0 = y.floor().max(0.0).min((source_height - 1) as f64) as u32;
    let x1 = x0.saturating_add(1).min(source_width - 1);
    let y1 = y0.saturating_add(1).min(source_height - 1);
    let fx = (x - x0 as f64).clamp(0.0, 1.0);
    let fy = (y - y0 as f64).clamp(0.0, 1.0);
    let top_left = rgba8_pixel(source, source_width, x0, y0);
    let top_right = rgba8_pixel(source, source_width, x1, y0);
    let bottom_left = rgba8_pixel(source, source_width, x0, y1);
    let bottom_right = rgba8_pixel(source, source_width, x1, y1);
    let mut out = [0; 4];
    for channel in 0..4 {
        let top = lerp(top_left[channel] as f64, top_right[channel] as f64, fx);
        let bottom = lerp(
            bottom_left[channel] as f64,
            bottom_right[channel] as f64,
            fx,
        );
        out[channel] = lerp(top, bottom, fy).round().clamp(0.0, 255.0) as u8;
    }
    out
}

fn rgba8_pixel(source: &[u8], source_width: u32, x: u32, y: u32) -> [u8; 4] {
    let index = ((y * source_width + x) * 4) as usize;
    [
        source[index],
        source[index + 1],
        source[index + 2],
        source[index + 3],
    ]
}

fn lerp(start: f64, end: f64, amount: f64) -> f64 {
    start + (end - start) * amount
}
