use std::sync::Arc;

use moli_image::{ImagePixels, VideoFrameSource};
use moli_layout::{
    PaintBlendMode, PaintColor, PaintCompositeMode, PaintFragment, PaintImage, PaintImageId,
    PaintImageSampling, PaintRect, PaintShape, PaintSnapshot, PaintTileSource, PaintTransform2D,
    pixel_snap_paint_rect,
};

/// A video resource paired with the geometry of the published viewport scene.
/// Both rectangles are in device pixels relative to the viewport origin.
#[derive(Clone, Debug)]
pub struct ViewportVideoLayer {
    pub destination: PaintRect,
    pub clip: PaintRect,
    pub nearest: bool,
    pub source: VideoFrameSource,
}

/// The viewport's raster backing and independently refreshed video layers.
/// Screenshot and PDF consumers continue to use the original paint snapshot.
pub struct ViewportComposition {
    pub raster: Arc<PaintSnapshot>,
    pub layers: Vec<ViewportVideoLayer>,
    /// Accepted sources include culled videos that do not need a window redraw.
    pub sources: Vec<VideoFrameSource>,
}

impl ViewportComposition {
    pub fn new(snapshot: &Arc<PaintSnapshot>, video_sources: &[VideoFrameSource]) -> Self {
        let mut composition = Self {
            raster: Arc::clone(snapshot),
            layers: Vec::new(),
            sources: video_sources.to_vec(),
        };
        if video_sources.is_empty() {
            return composition;
        }
        let scale = snapshot.surface.device_scale;
        let viewport = PaintRect::new(
            0.,
            0.,
            snapshot.surface.css_width,
            snapshot.surface.css_height,
        );
        let can_separate = snapshot.raster_crop.is_none()
            && !snapshot.fragments.iter().any(|fragment| {
                // A backdrop-dependent operation can read pixels from the
                // video even when the operation is outside its paint scope.
                matches!(
                    fragment,
                    PaintFragment::PushLayer { blend_mode, composite, .. }
                        if *blend_mode != PaintBlendMode::Normal
                            || *composite != PaintCompositeMode::SrcOver
                )
            });
        let mut scopes = Vec::new();
        let mut candidates = Vec::new();
        let mut software = Vec::new();
        for (index, fragment) in snapshot.fragments.iter().enumerate() {
            match fragment {
                PaintFragment::PushClip { shape, transform } => {
                    scopes.push(match shape {
                        PaintShape::Rect(rect) => axis_aligned_rect(*rect, *transform),
                        _ => None,
                    });
                }
                PaintFragment::PushLayer { .. } => scopes.push(None),
                PaintFragment::PopLayer => {
                    scopes.pop();
                }
                PaintFragment::Image(image) => {
                    let Some(source) = video_source(snapshot, image.image) else {
                        continue;
                    };
                    let placement = can_separate
                        .then(|| placement(image, scale, viewport, &scopes))
                        .flatten();
                    if let Some(placement) = placement {
                        candidates.push((index, source.clone(), placement, image.sampling));
                    } else {
                        software.push(source.clone());
                    }
                }
                PaintFragment::Tiles(tiles) => {
                    if let PaintTileSource::Image(id, _) = tiles.source
                        && let Some(source) = video_source(snapshot, id)
                    {
                        software.push(source.clone());
                    }
                }
                _ => {}
            }
        }
        // A source may be drawn more than once. Its software consumers must
        // continue receiving every resource update if any use is ineligible.
        composition
            .sources
            .retain(|source| !software.contains(source));
        candidates.retain(|(_, source, _, _)| composition.sources.contains(source));
        if candidates.is_empty() {
            return composition;
        }

        let raster = Arc::make_mut(&mut composition.raster);
        raster.fragments.clear();
        let mut candidates = candidates.into_iter().peekable();
        let mut scopes = Vec::new();
        for (index, fragment) in snapshot.fragments.iter().enumerate() {
            if let Some((_, source, placement, sampling)) =
                candidates.next_if(|(candidate, ..)| *candidate == index)
            {
                if let Some((destination, clip)) = placement {
                    composition.layers.push(ViewportVideoLayer {
                        destination: scale_rect(destination, scale),
                        clip: scale_rect(clip, scale),
                        nearest: sampling == PaintImageSampling::Nearest,
                        source,
                    });
                    punch_video_hole(&mut raster.fragments, &scopes, clip);
                }
                continue;
            }
            match fragment {
                PaintFragment::PushLayer { .. } | PaintFragment::PushClip { .. } => {
                    scopes.push(fragment);
                }
                PaintFragment::PopLayer => {
                    scopes.pop();
                }
                _ => {}
            }
            raster.fragments.push(fragment.clone());
        }
        composition
    }

    /// Pins the newest pixels after retiring a previous native subscription.
    /// A window can switch back to raster while a video is paused or seeking.
    pub fn refresh_software_frames(&mut self) {
        let updates: Vec<_> = self
            .raster
            .images
            .iter()
            .enumerate()
            .filter_map(|(index, resource)| {
                let ImagePixels::Video {
                    frame,
                    source: Some(source),
                } = &resource.image
                else {
                    return None;
                };
                if self.sources.contains(source) {
                    return None;
                }
                let latest = source.latest();
                (!Arc::ptr_eq(&latest, frame)).then_some((index, latest))
            })
            .collect();
        if !updates.is_empty() {
            let raster = Arc::make_mut(&mut self.raster);
            for (index, latest) in updates {
                let ImagePixels::Video { frame, .. } = &mut raster.images[index].image else {
                    unreachable!("video frame changed kind");
                };
                *frame = latest;
            }
        }
    }
}

fn video_source(snapshot: &PaintSnapshot, id: PaintImageId) -> Option<&VideoFrameSource> {
    match &snapshot.image(id)?.image {
        ImagePixels::Video { source, .. } => source.as_ref(),
        ImagePixels::Rgba(_) => None,
    }
}

/// Outer None means raster fallback; inner None means fully clipped.
fn placement(
    image: &PaintImage,
    scale: f32,
    viewport: PaintRect,
    scopes: &[Option<PaintRect>],
) -> Option<Option<(PaintRect, PaintRect)>> {
    let destination =
        axis_aligned_rect(pixel_snap_paint_rect(image.destination)?, image.transform)?;
    let mut clip = intersect(viewport, destination);
    for scope in scopes {
        clip = intersect(clip, (*scope)?);
    }
    if clip.width <= 0. || clip.height <= 0. {
        return Some(None);
    }
    // A shared edge would otherwise be antialiased twice, once when cutting
    // the raster and again when the window compositor draws the video.
    let aligned = [Some(destination), Some(clip)]
        .into_iter()
        .chain(scopes.iter().copied())
        .flatten()
        .all(|rect| {
            [rect.x, rect.y, rect.right(), rect.bottom()]
                .into_iter()
                .all(|edge| (edge * scale).fract() == 0.)
        });
    aligned.then_some(Some((destination, clip)))
}

fn axis_aligned_rect(rect: PaintRect, transform: PaintTransform2D) -> Option<PaintRect> {
    let [a, b, c, d, _, _] = transform.coefficients;
    if b != 0. || c != 0. || a <= 0. || d <= 0. || !transform.is_finite() {
        return None;
    }
    let rect = transform.map_rect(rect).bounding_rect();
    [rect.x, rect.y, rect.right(), rect.bottom()]
        .into_iter()
        .all(f32::is_finite)
        .then_some(rect)
}

fn intersect(a: PaintRect, b: PaintRect) -> PaintRect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    PaintRect::new(
        x,
        y,
        a.right().min(b.right()) - x,
        a.bottom().min(b.bottom()) - y,
    )
}

fn scale_rect(rect: PaintRect, scale: f32) -> PaintRect {
    PaintRect::new(
        rect.x * scale,
        rect.y * scale,
        rect.width * scale,
        rect.height * scale,
    )
}

fn punch_video_hole(
    fragments: &mut Vec<PaintFragment>,
    scopes: &[&PaintFragment],
    clip: PaintRect,
) {
    // The opaque video replaces what has already been painted in this region.
    // Finish clip groups before erasing so their isolation cannot hide the
    // destination. Later paint remains above the video in the raster backing.
    fragments.extend(scopes.iter().map(|_| PaintFragment::PopLayer));
    fragments.push(PaintFragment::PushLayer {
        opacity: 1.,
        blend_mode: PaintBlendMode::Normal,
        composite: PaintCompositeMode::DestOut,
        clip: PaintShape::Rect(clip),
        transform: PaintTransform2D::IDENTITY,
        filter: None,
    });
    fragments.push(PaintFragment::solid_rect(clip, PaintColor::WHITE));
    fragments.push(PaintFragment::PopLayer);
    fragments.extend(scopes.iter().map(|fragment| (*fragment).clone()));
}

#[cfg(test)]
mod tests {
    use moli_layout::PaintViewport;

    use super::*;

    #[test]
    fn viewport_video_retains_sampling_rect_through_scroll_clips() {
        let mut snapshot = PaintSnapshot::new(PaintViewport::new(640, 480, 2.), PaintColor::WHITE);
        let mut image = PaintImage {
            image: snapshot.add_image(Arc::new(
                moli_image::RgbaImage::try_new(1, 1, vec![0; 4]).unwrap(),
            )),
            destination: PaintRect::new(0., 0., 560., 400.),
            sampling: PaintImageSampling::Linear,
            transform: PaintTransform2D::translation(0., -150.),
        };
        let viewport = PaintRect::new(0., 0., 640., 480.);
        let scroller = PaintRect::new(0., 0., 560., 360.);
        assert_eq!(
            placement(&image, 2., viewport, &[Some(scroller)]),
            Some(Some((
                PaintRect::new(0., -150., 560., 400.),
                PaintRect::new(0., 0., 560., 250.),
            )))
        );
        assert_eq!(placement(&image, 2., viewport, &[None]), None);
        let fractional_scroller = PaintRect::new(0., 0., 560., 360.25);
        assert_eq!(
            placement(&image, 2., viewport, &[Some(fractional_scroller)]),
            None
        );
        image.transform = PaintTransform2D::translation(0., -400.);
        assert_eq!(
            placement(&image, 2., viewport, &[Some(scroller)]),
            Some(None)
        );
    }

    #[test]
    fn viewport_video_hole_keeps_later_translucent_paint() {
        let viewport = PaintRect::new(0., 0., 12., 12.);
        let mut snapshot = PaintSnapshot::new(PaintViewport::new(12, 12, 1.), PaintColor::WHITE);
        let clip = PaintFragment::push_clip(viewport);
        snapshot.push_fragment(clip.clone());
        snapshot.push_fragment(PaintFragment::solid_rect(
            viewport,
            PaintColor::new(1., 0., 0., 1.),
        ));
        punch_video_hole(
            &mut snapshot.fragments,
            &[&clip],
            PaintRect::new(2., 2., 8., 8.),
        );
        snapshot.push_fragment(PaintFragment::solid_rect(
            PaintRect::new(4., 4., 4., 4.),
            PaintColor::new(0., 1., 0., 0.5),
        ));
        snapshot.push_fragment(PaintFragment::PopLayer);

        let raster = crate::raster_snapshot(&snapshot).unwrap();
        let pixel = |x: usize, y: usize| &raster.rgba[(y * 12 + x) * 4..][..4];
        assert_eq!(pixel(1, 1), [255, 0, 0, 255]);
        assert_eq!(pixel(3, 3), [0, 0, 0, 0]);
        assert_eq!(pixel(5, 5), [0, 255, 0, 128]);
    }
}
