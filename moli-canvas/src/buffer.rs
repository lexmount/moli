//! Owned Canvas pixels. Precision is retained through copies, resampling and
//! compositing; conversion to display or encoded bytes happens at the consumer.

use crate::{
    CanvasRect, DrawImageBlit, ImageDataRect, MAX_RGBA8_BYTE_LENGTH, Rgba8Rect, ScaleFilter,
};
use half::f16;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PixelFormat {
    #[default]
    Unorm8,
    Float16,
}

impl PixelFormat {
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Unorm8 => 4,
            Self::Float16 => 8,
        }
    }

    pub fn byte_len(self, width: u32, height: u32) -> Option<usize> {
        (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(self.bytes_per_pixel())
            .filter(|length| *length <= MAX_RGBA8_BYTE_LENGTH)
    }

    fn read(self, bytes: &[u8]) -> [f32; 4] {
        std::array::from_fn(|index| match self {
            Self::Unorm8 => f32::from(bytes[index]) / 255.0,
            Self::Float16 => f16::from_ne_bytes([bytes[index * 2], bytes[index * 2 + 1]]).to_f32(),
        })
    }

    fn write(self, bytes: &mut [u8], channels: [f32; 4]) {
        match self {
            Self::Unorm8 => {
                for (byte, value) in bytes.iter_mut().zip(channels) {
                    *byte = (value * 255.0).round_ties_even().clamp(0.0, 255.0) as u8;
                }
            }
            Self::Float16 => {
                for (pair, value) in bytes.chunks_exact_mut(2).zip(channels) {
                    pair.copy_from_slice(&f16::from_f32(value).to_ne_bytes());
                }
            }
        }
    }

    fn quantize(self, channels: [f32; 4]) -> [f32; 4] {
        let mut bytes = [0; 8];
        self.write(&mut bytes[..self.bytes_per_pixel()], channels);
        self.read(&bytes)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AlphaMode {
    #[default]
    Straight,
    Premultiplied,
}

impl AlphaMode {
    pub fn convert_pixel(self, mut channels: [f32; 4], destination: Self) -> [f32; 4] {
        if self != destination {
            let alpha = channels[3];
            for channel in &mut channels[..3] {
                *channel = match destination {
                    Self::Premultiplied => *channel * alpha,
                    Self::Straight if alpha == 0.0 => 0.0,
                    Self::Straight => *channel / alpha,
                };
            }
        }
        channels
    }
}

/// A validated row-major image with an explicit alpha representation. Color
/// coordinates belong to the producer; no color-space conversion is implicit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PixelBuffer {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    format: PixelFormat,
    alpha_mode: AlphaMode,
}

impl PixelBuffer {
    pub fn new(width: u32, height: u32, format: PixelFormat, bytes: Vec<u8>) -> Option<Self> {
        Self::new_with_alpha_mode(width, height, format, AlphaMode::Straight, bytes)
    }

    pub fn new_with_alpha_mode(
        width: u32,
        height: u32,
        format: PixelFormat,
        alpha_mode: AlphaMode,
        bytes: Vec<u8>,
    ) -> Option<Self> {
        (format.byte_len(width, height)? == bytes.len()).then_some(Self {
            bytes,
            width,
            height,
            format,
            alpha_mode,
        })
    }

    pub fn blank(width: u32, height: u32, format: PixelFormat) -> Option<Self> {
        Self::blank_with_alpha_mode(width, height, format, AlphaMode::Straight)
    }

    pub fn blank_with_alpha_mode(
        width: u32,
        height: u32,
        format: PixelFormat,
        alpha_mode: AlphaMode,
    ) -> Option<Self> {
        let length = format.byte_len(width, height)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).ok()?;
        bytes.resize(length, 0);
        Self::new_with_alpha_mode(width, height, format, alpha_mode, bytes)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn format(&self) -> PixelFormat {
        self.format
    }

    pub fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Read native channels without changing their alpha representation.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[f32; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = self.offset(x, y);
        Some(self.format.read(&self.bytes[offset..]))
    }

    fn offset(&self, x: u32, y: u32) -> usize {
        (y as usize * self.width as usize + x as usize) * self.format.bytes_per_pixel()
    }

    fn set_pixel(&mut self, x: u32, y: u32, channels: [f32; 4]) {
        let offset = self.offset(x, y);
        self.format.write(
            &mut self.bytes[offset..offset + self.format.bytes_per_pixel()],
            channels,
        );
    }

    fn copy_rect(&mut self, source: &Self, rect: Rgba8Rect, x: u32, y: u32) {
        debug_assert!(rect.fits_within(source.width, source.height));
        debug_assert!((Rgba8Rect { x, y, ..rect }).fits_within(self.width, self.height));
        if source.format == self.format && source.alpha_mode == self.alpha_mode {
            let length = rect.width as usize * self.format.bytes_per_pixel();
            for row in 0..rect.height {
                let src = source.offset(rect.x, rect.y + row);
                let dst = self.offset(x, y + row);
                self.bytes[dst..dst + length].copy_from_slice(&source.bytes[src..src + length]);
            }
        } else {
            for row in 0..rect.height {
                for column in 0..rect.width {
                    self.set_pixel(
                        x + column,
                        y + row,
                        source.alpha_mode.convert_pixel(
                            source.pixel(rect.x + column, rect.y + row).unwrap(),
                            self.alpha_mode,
                        ),
                    );
                }
            }
        }
    }

    pub fn extract(&self, rect: ImageDataRect) -> Option<Self> {
        let mut output =
            Self::blank_with_alpha_mode(rect.width(), rect.height(), self.format, self.alpha_mode)?;
        if let Some((source_rect, x, y)) =
            crate::blit::clipped_source_rect_for_extract(rect, self.width, self.height)
        {
            output.copy_rect(self, source_rect, x, y);
        }
        Some(output)
    }

    pub fn put_image_data(&mut self, source: &Self, x: i32, y: i32, dirty: ImageDataRect) {
        if let Some((rect, x, y)) = crate::blit::clipped_blit_rect(
            source.width,
            source.height,
            self.width,
            self.height,
            x,
            y,
            dirty,
        ) {
            self.copy_rect(source, rect, x, y);
        }
    }

    /// Copy/resample pixels, interpolating in premultiplied coordinates.
    pub fn blit(&mut self, source: &Self, blit: DrawImageBlit, filter: ScaleFilter) {
        self.sample_into(source, blit, filter, false, 1.0);
    }

    pub fn draw_image(
        &mut self,
        source: &Self,
        blit: DrawImageBlit,
        filter: ScaleFilter,
        alpha: f32,
    ) {
        if alpha == 0.0 {
            return;
        }
        self.sample_into(source, blit, filter, true, alpha);
    }

    fn sample_into(
        &mut self,
        source: &Self,
        blit: DrawImageBlit,
        filter: ScaleFilter,
        composite: bool,
        alpha: f32,
    ) {
        let left = blit.dest_x.floor().max(0.0).min(f64::from(self.width)) as u32;
        let top = blit.dest_y.floor().max(0.0).min(f64::from(self.height)) as u32;
        let right = (blit.dest_x + blit.dest_width)
            .ceil()
            .max(0.0)
            .min(f64::from(self.width)) as u32;
        let bottom = (blit.dest_y + blit.dest_height)
            .ceil()
            .max(0.0)
            .min(f64::from(self.height)) as u32;
        for y in top..bottom {
            let sy = blit.source_y
                + (f64::from(y) - blit.dest_y) * blit.source_height / blit.dest_height;
            if sy < 0.0 || sy >= f64::from(source.height) {
                continue;
            }
            for x in left..right {
                let sx = blit.source_x
                    + (f64::from(x) - blit.dest_x) * blit.source_width / blit.dest_width;
                if sx < 0.0 || sx >= f64::from(source.width) {
                    continue;
                }
                if !composite && filter == ScaleFilter::Nearest {
                    self.copy_rect(
                        source,
                        Rgba8Rect::new(sx.floor() as u32, sy.floor() as u32, 1, 1).unwrap(),
                        x,
                        y,
                    );
                    continue;
                }
                let mut pixel = match filter {
                    ScaleFilter::Nearest => source.alpha_mode.convert_pixel(
                        source.pixel(sx.floor() as u32, sy.floor() as u32).unwrap(),
                        AlphaMode::Premultiplied,
                    ),
                    ScaleFilter::Bilinear => source.sample_bilinear(sx, sy),
                };
                if composite {
                    for channel in &mut pixel {
                        *channel *= alpha;
                    }
                    if self.format == PixelFormat::Unorm8 {
                        pixel = self.format.quantize(pixel);
                    }
                    if pixel == [0.0; 4] {
                        continue;
                    }
                    let destination = self
                        .alpha_mode
                        .convert_pixel(self.pixel(x, y).unwrap(), AlphaMode::Premultiplied);
                    pixel = source_over(destination, pixel);
                }
                self.set_pixel(
                    x,
                    y,
                    AlphaMode::Premultiplied.convert_pixel(pixel, self.alpha_mode),
                );
            }
        }
    }

    fn sample_bilinear(&self, x: f64, y: f64) -> [f32; 4] {
        let x0 = x.floor() as u32;
        let y0 = y.floor() as u32;
        let x1 = x0.saturating_add(1).min(self.width - 1);
        let y1 = y0.saturating_add(1).min(self.height - 1);
        let fx = (x - f64::from(x0)) as f32;
        let fy = (y - f64::from(y0)) as f32;
        // Interpolation uses premultiplied color to keep transparent pixels
        // from bleeding their hidden RGB into the result.
        let mut output = [0.0; 4];
        for (sx, sy, weight) in [
            (x0, y0, (1.0 - fx) * (1.0 - fy)),
            (x1, y0, fx * (1.0 - fy)),
            (x0, y1, (1.0 - fx) * fy),
            (x1, y1, fx * fy),
        ] {
            if weight == 0.0 {
                continue;
            }
            let pixel = self
                .alpha_mode
                .convert_pixel(self.pixel(sx, sy).unwrap(), AlphaMode::Premultiplied);
            for channel in 0..4 {
                output[channel] += pixel[channel] * weight;
            }
        }
        output
    }

    pub fn flip_y(&mut self) {
        let row_len = self.width as usize * self.format.bytes_per_pixel();
        for row in 0..self.height as usize / 2 {
            let bottom = (self.height as usize - row - 1) * row_len;
            let (top_rows, bottom_rows) = self.bytes.split_at_mut(bottom);
            top_rows[row * row_len..(row + 1) * row_len]
                .swap_with_slice(&mut bottom_rows[..row_len]);
        }
    }

    pub fn premultiply(&mut self) {
        if self.alpha_mode == AlphaMode::Premultiplied {
            return;
        }
        self.alpha_mode = AlphaMode::Premultiplied;
        if self.format == PixelFormat::Unorm8 {
            let _ = crate::premultiply_rgba8_in_place(&mut self.bytes);
            return;
        }
        self.map_alpha(|channel, alpha| channel * alpha);
    }

    pub fn unpremultiply(&mut self) {
        if self.alpha_mode == AlphaMode::Straight {
            return;
        }
        self.alpha_mode = AlphaMode::Straight;
        if self.format == PixelFormat::Unorm8 {
            for pixel in self.bytes.chunks_exact_mut(4) {
                let alpha = u32::from(pixel[3]);
                for channel in &mut pixel[..3] {
                    *channel = (u32::from(*channel) * 255 + alpha / 2)
                        .checked_div(alpha)
                        .unwrap_or(0)
                        .min(255) as u8;
                }
            }
            return;
        }
        self.map_alpha(|channel, alpha| if alpha == 0.0 { 0.0 } else { channel / alpha });
    }

    fn map_alpha(&mut self, map: impl Fn(f32, f32) -> f32) {
        for pixel in self.bytes.chunks_exact_mut(self.format.bytes_per_pixel()) {
            let mut channels = self.format.read(pixel);
            for index in 0..3 {
                channels[index] = map(channels[index], channels[3]);
            }
            self.format.write(pixel, channels);
        }
    }

    pub fn force_opaque(&mut self) {
        for pixel in self.bytes.chunks_exact_mut(self.format.bytes_per_pixel()) {
            match self.format {
                PixelFormat::Unorm8 => pixel[3] = 255,
                PixelFormat::Float16 => pixel[6..8].copy_from_slice(&f16::ONE.to_ne_bytes()),
            }
        }
    }

    pub fn paint_rect(&mut self, (left, top, right, bottom): CanvasRect, color: [f32; 4]) {
        let color = AlphaMode::Straight.convert_pixel(color, self.alpha_mode);
        let left = i64::from(left).max(0).min(i64::from(self.width)) as u32;
        let top = i64::from(top).max(0).min(i64::from(self.height)) as u32;
        let right = i64::from(right).max(0).min(i64::from(self.width)) as u32;
        let bottom = i64::from(bottom).max(0).min(i64::from(self.height)) as u32;
        for y in top..bottom {
            for x in left..right {
                self.set_pixel(x, y, color);
            }
        }
    }

    /// Apply an RGBA8 coverage mask without quantizing the source color or
    /// unrelated destination pixels. Used with the path rasterizer's geometry.
    pub fn composite_solid(&mut self, coverage: &[u8], color: [f32; 4]) -> Option<()> {
        if crate::byte_len(self.width, self.height)? != coverage.len() {
            return None;
        }
        if color[3] == 0.0 {
            return Some(());
        }
        for (destination, mask) in self
            .bytes
            .chunks_exact_mut(self.format.bytes_per_pixel())
            .zip(coverage.chunks_exact(4))
        {
            if mask[3] == 0 {
                continue;
            }
            let mut source = color;
            source[3] *= f32::from(mask[3]) / 255.0;
            let mut source = AlphaMode::Straight.convert_pixel(source, AlphaMode::Premultiplied);
            if self.format == PixelFormat::Unorm8 {
                // The 8-bit raster source uses the same channel precision as
                // its destination. Float16 drawing keeps the full paint value.
                source = self.format.quantize(source);
            }
            let target = self
                .alpha_mode
                .convert_pixel(self.format.read(destination), AlphaMode::Premultiplied);
            let output = source_over(target, source);
            self.format.write(
                destination,
                AlphaMode::Premultiplied.convert_pixel(output, self.alpha_mode),
            );
        }
        Some(())
    }
}

fn source_over(destination: [f32; 4], source: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|index| source[index] + destination[index] * (1.0 - source[3]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn half_image(width: u32, channels: &[f32]) -> PixelBuffer {
        PixelBuffer::new(
            width,
            channels.len() as u32 / (width * 4),
            PixelFormat::Float16,
            channels
                .iter()
                .flat_map(|value| f16::from_f32(*value).to_ne_bytes())
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn format_limits_validate_actual_storage_before_allocation() {
        assert_eq!(
            PixelFormat::Unorm8.byte_len(23_170, 23_170),
            Some(2_147_395_600)
        );
        assert_eq!(PixelFormat::Unorm8.byte_len(23_171, 23_171), None);
        assert_eq!(
            PixelFormat::Float16.byte_len(16_383, 16_383),
            Some(2_147_221_512)
        );
        assert_eq!(PixelFormat::Float16.byte_len(16_384, 16_384), None);
        assert!(PixelBuffer::new(1, 1, PixelFormat::Float16, vec![0; 4]).is_none());
        assert!(PixelBuffer::blank(u32::MAX, u32::MAX, PixelFormat::Float16).is_none());
    }

    #[test]
    fn clipped_copies_keep_hdr_and_every_half_bit_pattern() {
        let bytes = [
            0x8000u16, 0x0001, 0x7e21, 0x3c00, 0x4000, 0xbc00, 0x7bff, 0x3800,
        ]
        .into_iter()
        .flat_map(u16::to_ne_bytes)
        .collect::<Vec<_>>();
        let source = PixelBuffer::new(2, 1, PixelFormat::Float16, bytes.clone()).unwrap();
        let extracted = source.extract(ImageDataRect::new(-1, 0, 3, 1)).unwrap();
        assert_eq!(&extracted.bytes()[..8], &[0; 8]);
        assert_eq!(&extracted.bytes()[8..], bytes);
        let mut output = PixelBuffer::blank(3, 2, PixelFormat::Float16).unwrap();
        output.put_image_data(&source, 1, 1, ImageDataRect::new(0, 0, 2, 1));
        assert_eq!(&output.bytes()[32..], bytes);
        output.put_image_data(
            &source,
            -1,
            0,
            ImageDataRect::new(i32::MIN, i32::MIN, i32::MIN, i32::MIN),
        );
        assert_eq!(&output.bytes()[..8], &[0; 8]);
        output.flip_y();
        assert_eq!(&output.bytes()[8..24], bytes);
    }

    #[test]
    fn resampling_and_premultiplication_preserve_hdr_coordinates() {
        let source = half_image(2, &[2.0, -1.0, 0.5, 1.0, -2.0, 3.0, 0.25, 1.0]);
        let blit = DrawImageBlit::new(0.0, 0.0, 2.0, 1.0, 0.0, 0.0, 4.0, 1.0).unwrap();
        let mut output = PixelBuffer::blank(4, 1, PixelFormat::Float16).unwrap();
        output.blit(&source, blit, ScaleFilter::Nearest);
        assert_eq!(output.pixel(0, 0), Some([2.0, -1.0, 0.5, 1.0]));
        assert_eq!(output.pixel(3, 0), Some([-2.0, 3.0, 0.25, 1.0]));
        output.blit(&source, blit, ScaleFilter::Bilinear);
        assert_eq!(output.pixel(1, 0), Some([0.0, 1.0, 0.375, 1.0]));
        let mut alpha = half_image(2, &[2.0, -1.0, 0.5, 0.5, 2.0, -1.0, 0.5, 0.0]);
        alpha.premultiply();
        assert_eq!(alpha.pixel(0, 0), Some([1.0, -0.5, 0.25, 0.5]));
        alpha.unpremultiply();
        assert_eq!(alpha.pixel(0, 0), Some([2.0, -1.0, 0.5, 0.5]));
        assert_eq!(alpha.pixel(1, 0), Some([0.0; 4]));
    }

    #[test]
    fn alpha_weighted_filtering_does_not_leak_transparent_rgb() {
        let source = half_image(2, &[2.0, -1.0, 0.5, 1.0, -2.0, 3.0, 0.25, 0.0]);
        let mut output = PixelBuffer::blank(4, 1, PixelFormat::Float16).unwrap();
        output.blit(
            &source,
            DrawImageBlit::new(0.0, 0.0, 2.0, 1.0, 0.0, 0.0, 4.0, 1.0).unwrap(),
            ScaleFilter::Bilinear,
        );
        assert_eq!(output.pixel(1, 0), Some([2.0, -1.0, 0.5, 0.5]));
    }

    #[test]
    fn drawing_and_clearing_do_not_requantize_uncovered_hdr_pixels() {
        let mut image = half_image(2, &[2.0, -1.0, 0.5, 1.0, 3.0, -2.0, 0.25, 1.0]);
        let untouched = image.bytes()[8..].to_vec();
        image
            .composite_solid(&[0, 0, 0, 255, 0, 0, 0, 0], [4.0, -3.0, 0.25, 0.5])
            .unwrap();
        assert_eq!(image.pixel(0, 0), Some([3.0, -2.0, 0.375, 1.0]));
        assert_eq!(&image.bytes()[8..], untouched);
        image.paint_rect((0, 0, 1, 1), [0.0; 4]);
        assert_eq!(image.pixel(0, 0), Some([0.0; 4]));
        assert_eq!(&image.bytes()[8..], untouched);
    }

    #[test]
    fn native_premultiplied_copies_preserve_half_bits_without_reconversion() {
        let bytes = [0x0672u16, 0xbc00, 0x7e21, 0x2e66]
            .into_iter()
            .flat_map(u16::to_ne_bytes)
            .collect::<Vec<_>>();
        let mut source = PixelBuffer::new_with_alpha_mode(
            1,
            1,
            PixelFormat::Float16,
            AlphaMode::Premultiplied,
            bytes.clone(),
        )
        .unwrap();
        source.premultiply();
        assert_eq!(source.bytes(), bytes);
        let mut copy = source.extract(ImageDataRect::new(0, 0, 1, 1)).unwrap();
        copy.premultiply();
        assert_eq!(copy.bytes(), bytes);
        let mut output = PixelBuffer::blank_with_alpha_mode(
            2,
            1,
            PixelFormat::Float16,
            AlphaMode::Premultiplied,
        )
        .unwrap();
        output.blit(
            &copy,
            DrawImageBlit::new(0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 2.0, 1.0).unwrap(),
            ScaleFilter::Nearest,
        );
        assert_eq!(output.bytes(), bytes.repeat(2));
        output.unpremultiply();
        let straight = output.bytes().to_vec();
        output.unpremultiply();
        assert_eq!(output.bytes(), straight);
    }

    #[test]
    fn partial_writes_do_not_roundtrip_uncovered_premultiplied_channels() {
        let mut image = half_image(2, &[2.0, -1.0, 0.5, 0.5, 0.000984, -1.25, 2.5, 0.1]);
        image.premultiply();
        let original = image.bytes()[8..].to_vec();
        let source = half_image(1, &[4.0, -2.0, 0.25, 0.25]);
        image.put_image_data(&source, 0, 0, ImageDataRect::new(0, 0, 1, 1));
        assert_eq!(image.pixel(0, 0), Some([1.0, -0.5, 0.0625, 0.25]));
        assert_eq!(&image.bytes()[8..], original);
        image
            .composite_solid(&[0, 0, 0, 255, 0, 0, 0, 0], [2.0, -1.0, 0.5, 0.5])
            .unwrap();
        assert_eq!(&image.bytes()[8..], original);
        image.paint_rect((0, 0, 1, 1), [0.0; 4]);
        assert_eq!(image.pixel(0, 0), Some([0.0; 4]));
        assert_eq!(&image.bytes()[8..], original);
    }

    #[test]
    fn source_over_preserves_additive_premultiplied_color_at_zero_alpha() {
        let mut source = half_image(1, &[1.0, -1.0, 0.25, 0.0]);
        source.alpha_mode = AlphaMode::Premultiplied;
        let mut target = half_image(1, &[0.5, -0.25, 0.125, 0.5]);
        target.alpha_mode = AlphaMode::Premultiplied;
        target.draw_image(
            &source,
            DrawImageBlit::new(0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0).unwrap(),
            ScaleFilter::Nearest,
            0.5,
        );
        assert_eq!(target.pixel(0, 0), Some([1.0, -0.75, 0.25, 0.5]));
    }

    #[test]
    fn rgba8_bilinear_filtering_excludes_hidden_transparent_rgb() {
        let source = PixelBuffer::new(
            2,
            1,
            PixelFormat::Unorm8,
            vec![255, 0, 0, 255, 0, 0, 255, 0],
        )
        .unwrap();
        for alpha_mode in [AlphaMode::Straight, AlphaMode::Premultiplied] {
            let mut target =
                PixelBuffer::blank_with_alpha_mode(4, 1, PixelFormat::Unorm8, alpha_mode).unwrap();
            target.blit(
                &source,
                DrawImageBlit::new(0.0, 0.0, 2.0, 1.0, 0.0, 0.0, 4.0, 1.0).unwrap(),
                ScaleFilter::Bilinear,
            );
            target.unpremultiply();
            assert_eq!(&target.bytes()[4..8], &[255, 0, 0, 128]);
        }
    }

    #[test]
    fn painting_uses_normalized_bounds_without_clearing_adjacent_pixels() {
        for format in [PixelFormat::Unorm8, PixelFormat::Float16] {
            let mut target =
                PixelBuffer::blank_with_alpha_mode(3, 2, format, AlphaMode::Premultiplied).unwrap();
            target.paint_rect((0, 0, 3, 2), [1.0, 0.0, 0.0, 0.5]);
            let untouched = target.bytes()[..format.bytes_per_pixel()].to_vec();
            target.paint_rect((1, 0, 2, 2), [0.0; 4]);
            for row in 0..2 {
                assert_eq!(target.pixel(1, row), Some([0.0; 4]));
                for column in [0, 2] {
                    let start = target.offset(column, row);
                    assert_eq!(
                        &target.bytes()[start..start + format.bytes_per_pixel()],
                        untouched
                    );
                }
            }
        }
    }

    #[test]
    fn raster_source_alpha_uses_the_destination_channel_precision() {
        let mut byte =
            PixelBuffer::blank_with_alpha_mode(1, 1, PixelFormat::Unorm8, AlphaMode::Premultiplied)
                .unwrap();
        byte.paint_rect((0, 0, 1, 1), [0.0, 0.0, 1.0, 1.0]);
        byte.composite_solid(&[255; 4], [1.0, 0.0, 0.0, 0.5])
            .unwrap();
        assert_eq!(byte.bytes(), &[128, 0, 127, 255]);
        let mut half = half_image(1, &[0.0, 0.0, 1.0, 1.0]);
        half.premultiply();
        half.composite_solid(&[255; 4], [1.0, 0.0, 0.0, 0.5])
            .unwrap();
        assert_eq!(half.pixel(0, 0), Some([0.5, 0.0, 0.5, 1.0]));
    }
}
