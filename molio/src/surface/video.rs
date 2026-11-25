use gpui::{Bounds, ContentMask, Pixels, PlanarVideoSurface, Window, point, size};
use moli_core::page::ViewportVideoLayer;

/// Paints the native resources immediately below the backing texture.
pub(super) fn paint_layers(
    layers: &[ViewportVideoLayer],
    bounds: Bounds<Pixels>,
    width: u32,
    height: u32,
    window: &mut Window,
) {
    let x = bounds.size.width / width as f32;
    let y = bounds.size.height / height as f32;
    for layer in layers {
        let frame = layer.source.latest();
        let mask = ContentMask {
            bounds: Bounds::new(
                point(
                    bounds.origin.x + x * layer.clip.x,
                    bounds.origin.y + y * layer.clip.y,
                ),
                size(x * layer.clip.width, y * layer.clip.height),
            ),
            fade_out: Default::default(),
        };
        let destination = Bounds::new(
            point(
                bounds.origin.x + x * layer.destination.x,
                bounds.origin.y + y * layer.destination.y,
            ),
            size(x * layer.destination.width, y * layer.destination.height),
        );
        window.with_content_mask(Some(mask), |window| {
            window.paint_surface(
                destination,
                PlanarVideoSurface {
                    buffer: frame.buffer(),
                    visible_rect: frame.visible_rect(),
                    color_conversion: frame.conversion(),
                    chroma_offset: frame.chroma_offset(),
                    nearest: layer.nearest,
                    lease: Some(frame),
                },
            );
        });
    }
}
