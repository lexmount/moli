use font8x8::{BASIC_FONTS, UnicodeFonts};

use crate::rect::{paint_rect, paint_rect_premul};
use crate::types::surface_matches_len;

pub fn measure_text_width(text: &str, font: &str) -> f64 {
    let scale = text_scale(font);
    let glyph_advance = 8u32.saturating_mul(scale) + scale;
    text.chars().count() as f64 * glyph_advance as f64
}

pub fn draw_text(
    pixels: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    text: &str,
    x: f64,
    y: f64,
    font: &str,
    rgba: [u8; 4],
) {
    if !surface_matches_len(pixels, canvas_width, canvas_height) {
        return;
    }
    let scale = text_scale(font);
    let glyph_height = (8 * scale) as i32;
    let mut cursor_x = x.round() as i32;
    let top = y.round() as i32 - glyph_height + scale as i32;
    for ch in text.chars() {
        if let Some(glyph) = BASIC_FONTS.get(ch) {
            draw_glyph(
                pixels,
                canvas_width,
                canvas_height,
                glyph,
                cursor_x,
                top,
                scale,
                rgba,
            );
        }
        cursor_x += (8 * scale + scale) as i32;
    }
}

fn parse_font_size_px(font: &str) -> u32 {
    font.split_whitespace()
        .find_map(|part| part.strip_suffix("px"))
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(10)
}

fn text_scale(font: &str) -> u32 {
    (parse_font_size_px(font) / 8).max(1)
}

fn draw_glyph(
    pixels: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    glyph: [u8; 8],
    origin_x: i32,
    origin_y: i32,
    scale: u32,
    rgba: [u8; 4],
) {
    for (row, bits) in glyph.into_iter().enumerate() {
        for col in 0..8 {
            if (bits & (1 << col)) == 0 {
                continue;
            }
            let pixel_x = origin_x + ((7 - col) as u32 * scale) as i32;
            let pixel_y = origin_y + (row as u32 * scale) as i32;
            paint_rect(
                pixels,
                canvas_width,
                canvas_height,
                (
                    pixel_x,
                    pixel_y,
                    pixel_x + scale as i32,
                    pixel_y + scale as i32,
                ),
                rgba,
            );
        }
    }
}

/// Like [`draw_text`] but works directly on a premultiplied RGBA8 surface,
/// compositing each glyph with source-over blending in premultiplied space.
/// This avoids the full-surface unpremultiply/premultiply round-trip.
pub fn draw_text_premul(
    pixels: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    text: &str,
    x: f64,
    y: f64,
    font: &str,
    rgba: [u8; 4],
) {
    if !surface_matches_len(pixels, canvas_width, canvas_height) {
        return;
    }
    let scale = text_scale(font);
    let glyph_height = (8 * scale) as i32;
    let mut cursor_x = x.round() as i32;
    let top = y.round() as i32 - glyph_height + scale as i32;
    for ch in text.chars() {
        if let Some(glyph) = BASIC_FONTS.get(ch) {
            draw_glyph_premul(
                pixels,
                canvas_width,
                canvas_height,
                glyph,
                cursor_x,
                top,
                scale,
                rgba,
            );
        }
        cursor_x += (8 * scale + scale) as i32;
    }
}

fn draw_glyph_premul(
    pixels: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    glyph: [u8; 8],
    origin_x: i32,
    origin_y: i32,
    scale: u32,
    rgba: [u8; 4],
) {
    for (row, bits) in glyph.into_iter().enumerate() {
        for col in 0..8 {
            if (bits & (1 << col)) == 0 {
                continue;
            }
            let pixel_x = origin_x + ((7 - col) as u32 * scale) as i32;
            let pixel_y = origin_y + (row as u32 * scale) as i32;
            paint_rect_premul(
                pixels,
                canvas_width,
                canvas_height,
                (
                    pixel_x,
                    pixel_y,
                    pixel_x + scale as i32,
                    pixel_y + scale as i32,
                ),
                rgba,
            );
        }
    }
}
