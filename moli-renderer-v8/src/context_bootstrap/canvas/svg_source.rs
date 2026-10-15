//! Immutable external SVG image snapshots, resolved at the consumer's viewport.

use super::pixels::CanvasImage;
use std::sync::Arc;

pub(super) struct SvgSource {
    pub(super) image: Arc<moli_image::SvgImage>,
    pub(super) origin_clean: bool,
}

impl SvgSource {
    pub(super) fn render_for_draw(
        &self,
        source: [f64; 4],
        destination: [f64; 4],
        transform: moli_layout::LayoutTransform2D,
    ) -> Result<Option<CanvasImage>, String> {
        use super::path::coordinates::{NativeAffine, Scalar};
        let (width, height) = self.dimensions();
        if destination[2] == 0.0
            || destination[3] == 0.0
            || !super::image_paint::source_rect_intersects_image(source, [width, height])
            || NativeAffine::inverse(kurbo::Affine::new(transform.coefficients)).is_none()
        {
            return Ok(None);
        }
        let [a, b, c, d, _, _] = transform.coefficients;
        let x = Scalar::new(width) * Scalar::new(destination[2]) / Scalar::new(source[2]);
        let y = Scalar::new(height) * Scalar::new(destination[3]) / Scalar::new(source[3]);
        let raster_width = (x * Scalar::new(a))
            .lossy()
            .hypot((x * Scalar::new(b)).lossy())
            .ceil()
            .max(1.0);
        let raster_height = (y * Scalar::new(c))
            .lossy()
            .hypot((y * Scalar::new(d)).lossy())
            .ceil()
            .max(1.0);
        if !raster_width.is_finite()
            || !raster_height.is_finite()
            || raster_width > f64::from(u32::MAX)
            || raster_height > f64::from(u32::MAX)
        {
            return Err("SVG raster dimensions exceed the supported image surface.".to_owned());
        }
        self.render_region(
            [width, height],
            [0.0, 0.0, width, height],
            raster_width as u32,
            raster_height as u32,
        )
        .map(Some)
    }

    pub(super) fn has_natural_dimensions(&self) -> bool {
        let metadata = self.image.metadata();
        metadata.intrinsic_width.is_some() && metadata.intrinsic_height.is_some()
    }

    pub(super) fn dimensions(&self) -> (f64, f64) {
        let metadata = self.image.metadata();
        (
            f64::from(metadata.concrete_width),
            f64::from(metadata.concrete_height),
        )
    }

    pub(super) fn with_default_size(self, width: f32, height: f32) -> Result<Self, String> {
        let metadata = self
            .image
            .metadata()
            .with_default_object_size(width, height);
        self.with_viewport(metadata.concrete_width, metadata.concrete_height)
    }

    pub(super) fn with_viewport(self, width: f32, height: f32) -> Result<Self, String> {
        let size = self.image.tree().size();
        let metadata = self.image.metadata();
        if size.width() == width
            && size.height() == height
            && metadata.concrete_width == width
            && metadata.concrete_height == height
        {
            return Ok(self);
        }
        let image = self
            .image
            .with_viewport(width, height)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            image: Arc::new(image),
            origin_clean: self.origin_clean,
        })
    }

    pub(super) fn render_region(
        &self,
        [image_width, image_height]: [f64; 2],
        [x, y, width, height]: [f64; 4],
        output_width: u32,
        output_height: u32,
    ) -> Result<CanvasImage, String> {
        let sx = f64::from(output_width) / width;
        let sy = f64::from(output_height) / height;
        let size = self.image.tree().size();
        let image = moli_paint::raster_svg_image(
            &self.image,
            output_width,
            output_height,
            [
                sx * image_width / f64::from(size.width()),
                0.0,
                0.0,
                sy * image_height / f64::from(size.height()),
                -x * sx,
                -y * sy,
            ],
        )
        .map_err(|error| error.to_string())?;
        CanvasImage::rgba8(image.width, image.height, image.rgba)
            .map(|image| image.with_origin_clean(self.origin_clean))
            .ok_or_else(|| "Unable to allocate SVG image pixels.".to_owned())
    }

    pub(super) fn rasterize(
        self,
        default_width: f32,
        default_height: f32,
    ) -> Result<CanvasImage, String> {
        let source = self.with_default_size(default_width, default_height)?;
        let (width, height) = source.dimensions();
        source.render_region(
            [width, height],
            [0.0, 0.0, width, height],
            width.round() as u32,
            height.round() as u32,
        )
    }
}
