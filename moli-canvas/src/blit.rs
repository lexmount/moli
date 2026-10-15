use crate::pixel::{copy_rgba8_rect, scale_rgba8};
use crate::types::{
    DrawImageBlit, ImageDataRect, Rgba8Rect, ScaleFilter, byte_len, surface_matches_len,
};

pub fn extract_image_data(
    pixels: &[u8],
    canvas_width: u32,
    canvas_height: u32,
    rect: ImageDataRect,
) -> Vec<u8> {
    let (width, height) = (rect.width, rect.height);
    let mut out = vec![0; byte_len(width, height).unwrap_or(0)];
    if !surface_matches_len(pixels, canvas_width, canvas_height) {
        return out;
    }
    let Some((source_rect, dest_x, dest_y)) =
        clipped_source_rect_for_extract(rect, canvas_width, canvas_height)
    else {
        return out;
    };
    let _ = copy_rgba8_rect(
        pixels,
        canvas_width,
        canvas_height,
        source_rect,
        &mut out,
        width,
        height,
        dest_x,
        dest_y,
    );
    out
}

fn clipped_source_rect_for_extract(
    rect: ImageDataRect,
    canvas_width: u32,
    canvas_height: u32,
) -> Option<(Rgba8Rect, u32, u32)> {
    let left = rect.x;
    let top = rect.y;
    let right = left + i64::from(rect.width);
    let bottom = top + i64::from(rect.height);
    let copy_left = left.max(0).min(canvas_width as i64);
    let copy_top = top.max(0).min(canvas_height as i64);
    let copy_right = right.max(0).min(canvas_width as i64);
    let copy_bottom = bottom.max(0).min(canvas_height as i64);
    if copy_left >= copy_right || copy_top >= copy_bottom {
        return None;
    }
    let source_rect = Rgba8Rect::new(
        copy_left as u32,
        copy_top as u32,
        (copy_right - copy_left) as u32,
        (copy_bottom - copy_top) as u32,
    )?;
    Some((
        source_rect,
        (copy_left - left) as u32,
        (copy_top - top) as u32,
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn blit_image_data(
    pixels: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    source: &[u8],
    source_width: u32,
    source_height: u32,
    dx: i32,
    dy: i32,
    dirty_rect: ImageDataRect,
) {
    if !surface_matches_len(pixels, canvas_width, canvas_height)
        || !surface_matches_len(source, source_width, source_height)
        || dirty_rect.width == 0
        || dirty_rect.height == 0
    {
        return;
    }
    let Some((source_rect, dest_x, dest_y)) = clipped_blit_rect(
        source_width,
        source_height,
        canvas_width,
        canvas_height,
        dx,
        dy,
        dirty_rect,
    ) else {
        return;
    };
    let _ = copy_rgba8_rect(
        source,
        source_width,
        source_height,
        source_rect,
        pixels,
        canvas_width,
        canvas_height,
        dest_x,
        dest_y,
    );
}

fn clipped_blit_rect(
    source_width: u32,
    source_height: u32,
    dest_width: u32,
    dest_height: u32,
    dest_x: i32,
    dest_y: i32,
    dirty_rect: ImageDataRect,
) -> Option<(Rgba8Rect, u32, u32)> {
    // Intersect in source coordinates. A source pixel (x, y) is always written
    // to (dest_x + x, dest_y + y), even when the dirty rectangle is clipped.
    let dest_x = i64::from(dest_x);
    let dest_y = i64::from(dest_y);
    let source_left = dirty_rect.x.max(0).max(-dest_x);
    let source_top = dirty_rect.y.max(0).max(-dest_y);
    let source_right = (dirty_rect.x + i64::from(dirty_rect.width))
        .min(i64::from(source_width))
        .min(i64::from(dest_width) - dest_x);
    let source_bottom = (dirty_rect.y + i64::from(dirty_rect.height))
        .min(i64::from(source_height))
        .min(i64::from(dest_height) - dest_y);
    if source_left >= source_right || source_top >= source_bottom {
        return None;
    }
    let source_rect = Rgba8Rect::new(
        source_left as u32,
        source_top as u32,
        (source_right - source_left) as u32,
        (source_bottom - source_top) as u32,
    )?;
    Some((
        source_rect,
        (dest_x + source_left) as u32,
        (dest_y + source_top) as u32,
    ))
}

pub fn blit_draw_image(
    pixels: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    source: &[u8],
    source_width: u32,
    source_height: u32,
    blit: DrawImageBlit,
) {
    blit_draw_image_filtered(
        pixels,
        canvas_width,
        canvas_height,
        source,
        source_width,
        source_height,
        blit,
        ScaleFilter::Nearest,
    );
}

pub fn blit_draw_image_filtered(
    pixels: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    source: &[u8],
    source_width: u32,
    source_height: u32,
    blit: DrawImageBlit,
    filter: ScaleFilter,
) {
    if !surface_matches_len(pixels, canvas_width, canvas_height)
        || !surface_matches_len(source, source_width, source_height)
    {
        return;
    }
    let dest_left = blit.dest_x.floor() as i32;
    let dest_top = blit.dest_y.floor() as i32;
    let dest_right = (blit.dest_x + blit.dest_width).ceil() as i32;
    let dest_bottom = (blit.dest_y + blit.dest_height).ceil() as i32;
    if dest_left >= dest_right || dest_top >= dest_bottom {
        return;
    }

    let start_x = dest_left.max(0).min(canvas_width as i32);
    let start_y = dest_top.max(0).min(canvas_height as i32);
    let end_x = dest_right.max(0).min(canvas_width as i32);
    let end_y = dest_bottom.max(0).min(canvas_height as i32);
    if start_x >= end_x || start_y >= end_y {
        return;
    }

    let visible_width = (end_x - start_x) as u32;
    let visible_height = (end_y - start_y) as u32;
    let Some(scaled) = scale_rgba8(
        source,
        source_width,
        source_height,
        blit,
        start_x,
        start_y,
        visible_width,
        visible_height,
        filter,
    ) else {
        return;
    };
    let Some(source_rect) = Rgba8Rect::new(0, 0, visible_width, visible_height) else {
        return;
    };
    let _ = copy_rgba8_rect(
        &scaled,
        visible_width,
        visible_height,
        source_rect,
        pixels,
        canvas_width,
        canvas_height,
        start_x as u32,
        start_y as u32,
    );
}

/// Like [`blit_draw_image_filtered`] but operates directly on a premultiplied
/// RGBA8 destination surface. The source is expected to be straight-alpha
/// (e.g. from a canvas snapshot). Scaled pixels are unpremultiplied and
/// composited with source-over blending in premultiplied space, avoiding the
/// full-surface unpremultiply/premultiply round-trip.
#[allow(clippy::too_many_arguments)]
pub fn blit_draw_image_filtered_premul(
    pixels: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    source: &[u8],
    source_width: u32,
    source_height: u32,
    blit: DrawImageBlit,
    filter: ScaleFilter,
) {
    if !surface_matches_len(pixels, canvas_width, canvas_height)
        || !surface_matches_len(source, source_width, source_height)
    {
        return;
    }
    let dest_left = blit.dest_x.floor() as i32;
    let dest_top = blit.dest_y.floor() as i32;
    let dest_right = (blit.dest_x + blit.dest_width).ceil() as i32;
    let dest_bottom = (blit.dest_y + blit.dest_height).ceil() as i32;
    if dest_left >= dest_right || dest_top >= dest_bottom {
        return;
    }

    let start_x = dest_left.max(0).min(canvas_width as i32);
    let start_y = dest_top.max(0).min(canvas_height as i32);
    let end_x = dest_right.max(0).min(canvas_width as i32);
    let end_y = dest_bottom.max(0).min(canvas_height as i32);
    if start_x >= end_x || start_y >= end_y {
        return;
    }

    let visible_width = (end_x - start_x) as u32;
    let visible_height = (end_y - start_y) as u32;
    let Some(scaled) = scale_rgba8(
        source,
        source_width,
        source_height,
        blit,
        start_x,
        start_y,
        visible_width,
        visible_height,
        filter,
    ) else {
        return;
    };

    let row_stride = canvas_width as usize * 4;
    for sy in 0..visible_height as usize {
        let dy = start_y as u32 + sy as u32;
        if dy >= canvas_height {
            break;
        }
        for sx in 0..visible_width as usize {
            let dx = start_x as u32 + sx as u32;
            if dx >= canvas_width {
                break;
            }
            let si = (sy * visible_width as usize + sx) * 4;
            let sa = scaled[si + 3] as u32;
            if sa == 0 {
                continue;
            }
            let di = dy as usize * row_stride + dx as usize * 4;
            let spr = (scaled[si] as u32 * sa + 128) / 255;
            let spg = (scaled[si + 1] as u32 * sa + 128) / 255;
            let spb = (scaled[si + 2] as u32 * sa + 128) / 255;
            let inv_sa = 255 - sa;
            let d = &mut pixels[di..di + 4];
            d[0] = ((spr * 255 + d[0] as u32 * inv_sa + 128) / 255).min(255) as u8;
            d[1] = ((spg * 255 + d[1] as u32 * inv_sa + 128) / 255).min(255) as u8;
            d[2] = ((spb * 255 + d[2] as u32 * inv_sa + 128) / 255).min(255) as u8;
            d[3] = ((sa * 255 + d[3] as u32 * inv_sa + 128) / 255).min(255) as u8;
        }
    }
}

/// Copies straight-alpha source pixels onto a premultiplied RGBA8 surface as a
/// **raw overwrite** (matching the `putImageData` contract, not a source-over
/// composite). Each destination pixel becomes the premultiplied form of the
/// source straight-alpha value, replacing prior content exactly and ignoring the
/// ordinary paint state / composite operation. This avoids the full-surface
/// unpremultiply/premultiply round-trip that [`blit_image_data`] requires.
#[allow(clippy::too_many_arguments)]
pub fn blit_image_data_premul(
    pixels: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    source: &[u8],
    source_width: u32,
    source_height: u32,
    dx: i32,
    dy: i32,
    dirty_x: i32,
    dirty_y: i32,
    dirty_width: i32,
    dirty_height: i32,
) {
    if !surface_matches_len(pixels, canvas_width, canvas_height)
        || !surface_matches_len(source, source_width, source_height)
        || dirty_width == 0
        || dirty_height == 0
    {
        return;
    }
    let Some((source_rect, dest_x, dest_y)) = clipped_blit_rect(
        source_width,
        source_height,
        canvas_width,
        canvas_height,
        dx,
        dy,
        ImageDataRect::new(dirty_x, dirty_y, dirty_width, dirty_height),
    ) else {
        return;
    };

    let src_row_stride = source_width as usize * 4;
    let dst_row_stride = canvas_width as usize * 4;
    let copy_w = source_rect.width as usize;

    for row in 0..source_rect.height as usize {
        let src_y = source_rect.y as usize + row;
        let dst_y = dest_y as usize + row;
        if dst_y >= canvas_height as usize {
            break;
        }
        let src_start = src_y * src_row_stride + source_rect.x as usize * 4;
        let dst_start = dst_y * dst_row_stride + dest_x as usize * 4;
        for col in 0..copy_w {
            let si = src_start + col * 4;
            let di = dst_start + col * 4;
            if di + 4 > pixels.len() {
                break;
            }
            let sa = source[si + 3] as u32;
            // Raw overwrite: premultiply the straight source channel into the
            // destination, replacing it without reading/compositing over it.
            if sa == 0 {
                pixels[di] = 0;
                pixels[di + 1] = 0;
                pixels[di + 2] = 0;
                pixels[di + 3] = 0;
                continue;
            }
            pixels[di] = ((source[si] as u32 * sa + 128) / 255) as u8;
            pixels[di + 1] = ((source[si + 1] as u32 * sa + 128) / 255) as u8;
            pixels[di + 2] = ((source[si + 2] as u32 * sa + 128) / 255) as u8;
            pixels[di + 3] = sa as u8;
        }
    }
}
