use font8x8::{BASIC_FONTS, UnicodeFonts};

use crate::rect::paint_rect;
use crate::types::surface_matches_len;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextBaseline {
    Top,
    Hanging,
    Middle,
    #[default]
    Alphabetic,
    Ideographic,
    Bottom,
}

impl TextBaseline {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "top" => Self::Top,
            "hanging" => Self::Hanging,
            "middle" => Self::Middle,
            "alphabetic" => Self::Alphabetic,
            "ideographic" => Self::Ideographic,
            "bottom" => Self::Bottom,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Hanging => "hanging",
            Self::Middle => "middle",
            Self::Alphabetic => "alphabetic",
            Self::Ideographic => "ideographic",
            Self::Bottom => "bottom",
        }
    }

    // The bitmap font uses an eight-row em box with a one-row descent.
    fn top_offset(self, scale: u32) -> f64 {
        let baseline_row = match self {
            Self::Top => 0,
            Self::Hanging => 1,
            Self::Middle => 4,
            Self::Alphabetic => 7,
            Self::Ideographic | Self::Bottom => 8,
        };
        -f64::from(baseline_row) * f64::from(scale)
    }
}

pub fn measure_text_width(text: &str, font: &str) -> f64 {
    let scale = text_scale(font);
    let glyph_advance = 8u32.saturating_mul(scale) + scale;
    text.chars().count() as f64 * glyph_advance as f64
}

pub fn measure_text_vertical_bounds(text: &str, font: &str, baseline: TextBaseline) -> (f64, f64) {
    let mut first_row = 8;
    let mut end_row = 0;
    for glyph in text.chars().filter_map(|ch| BASIC_FONTS.get(ch)) {
        for (row, bits) in glyph.into_iter().enumerate() {
            if bits != 0 {
                first_row = first_row.min(row);
                end_row = end_row.max(row + 1);
            }
        }
    }
    if first_row >= end_row {
        return (0.0, 0.0);
    }
    let scale = text_scale(font);
    let top = baseline.top_offset(scale);
    (
        -(top + first_row as f64 * f64::from(scale)),
        top + end_row as f64 * f64::from(scale),
    )
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
    draw_text_with_baseline(
        pixels,
        canvas_width,
        canvas_height,
        text,
        x,
        y,
        font,
        TextBaseline::Alphabetic,
        rgba,
    );
}

pub fn draw_text_with_baseline(
    pixels: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    text: &str,
    x: f64,
    y: f64,
    font: &str,
    baseline: TextBaseline,
    rgba: [u8; 4],
) {
    if !surface_matches_len(pixels, canvas_width, canvas_height) {
        return;
    }
    let scale = text_scale(font);
    let mut cursor_x = x.round() as i32;
    let top = (y.round() + baseline.top_offset(scale)) as i32;
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
            let pixel_x = origin_x + (col * scale) as i32;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_text_renders_asymmetric_glyphs_in_reading_orientation() {
        let mut pixels = [0; 8 * 8 * 4];
        draw_text_with_baseline(
            &mut pixels,
            8,
            8,
            "F",
            0.0,
            0.0,
            "8px sans-serif",
            TextBaseline::Top,
            [255; 4],
        );
        let bottom_row = &pixels[6 * 8 * 4..7 * 8 * 4];
        assert_eq!(
            bottom_row.as_chunks::<4>().0,
            &[
                [255; 4], [255; 4], [255; 4], [255; 4], [0; 4], [0; 4], [0; 4], [0; 4],
            ]
        );
    }
}
