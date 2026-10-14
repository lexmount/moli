//! Immutable clipping snapshots and their shared, dimension-specific masks.

use super::{path::Canvas2dPathState, path2d::CanvasFillRule, state::canvas_clipping_state};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
struct ClipPath {
    path: Canvas2dPathState,
    rule: moli_layout::PaintFillRule,
}

#[derive(Default)]
struct ClipData {
    paths: Vec<Rc<ClipPath>>,
    mask: RefCell<Option<Rc<moli_paint::RasterImage>>>,
}

#[derive(Clone, Default)]
pub(super) struct CanvasClipRegion(Rc<ClipData>);

impl CanvasClipRegion {
    pub(super) fn intersect(
        &mut self,
        path: Canvas2dPathState,
        rule: CanvasFillRule,
    ) -> Result<(), moli_paint::PaintError> {
        if self.0.paths.len() >= moli_paint::MAX_PAINT_LAYER_DEPTH {
            return Err(moli_paint::PaintError::PaintLayerDepthBudgetExceeded {
                depth: self.0.paths.len() + 1,
                max_depth: moli_paint::MAX_PAINT_LAYER_DEPTH,
            });
        }
        let mut paths = self.0.paths.clone();
        paths.push(Rc::new(ClipPath {
            path,
            rule: rule.paint_rule(),
        }));
        self.0 = Rc::new(ClipData {
            paths,
            mask: RefCell::default(),
        });
        Ok(())
    }

    pub(super) fn mask(
        &self,
        width: u32,
        height: u32,
    ) -> Result<Option<Rc<moli_paint::RasterImage>>, moli_paint::PaintError> {
        if self.0.paths.is_empty() || width == 0 || height == 0 {
            return Ok(None);
        }
        if let Some(mask) = self.0.mask.borrow().as_ref()
            && mask.width == width
            && mask.height == height
        {
            return Ok(Some(mask.clone()));
        }
        let clips = self
            .0
            .paths
            .iter()
            .map(|clip| {
                Ok(moli_paint::CanvasClip {
                    path: clip.path.clip_paint_path(width, height)?,
                    fill_rule: clip.rule,
                })
            })
            .collect::<Result<Vec<_>, moli_paint::PaintError>>()?;
        let mask = Rc::new(moli_paint::raster_canvas_clip_mask(width, height, &clips)?);
        *self.0.mask.borrow_mut() = Some(mask.clone());
        Ok(Some(mask))
    }
}

pub(super) fn region<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> CanvasClipRegion {
    canvas_clipping_state(scope, context).borrow().clone()
}

pub(super) fn throw_paint_error(scope: &mut v8::PinScope<'_, '_>, error: &moli_paint::PaintError) {
    let message = crate::util::v8_string(scope, &format!("Canvas drawing failed: {error}"))
        .unwrap_or_else(|| v8::String::empty(scope));
    let exception = v8::Exception::range_error(scope, message);
    scope.throw_exception(exception);
}

/// All drawing consumers share the same final clip. putImageData deliberately
/// uses the raw bitmap updater because pixel uploads do not use drawing state.
pub(super) fn draw_with_clip<'s, F>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    canvas: v8::Local<'s, v8::Object>,
    draw: F,
) where
    F: FnOnce(&mut super::pixels::CanvasImage, Option<&[u8]>) -> Result<(), moli_paint::PaintError>,
{
    let clipping = region(scope, context);
    let result = super::backing_store::try_with_canvas_image_mut(scope, canvas, |image| {
        let mask = clipping.mask(image.pixels.width(), image.pixels.height())?;
        draw(image, mask.as_ref().map(|mask| mask.rgba.as_slice()))
    });
    if let Err(error) = result {
        throw_paint_error(scope, &error);
    }
}

pub(super) fn clip_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated clip receiver");
    let Some((explicit, rule)) = super::path2d::clip_arguments(scope, &args) else {
        return;
    };
    // Author conversions can reset the context or change either path. Read
    // geometry and the current transform only after every conversion succeeds.
    let context_path = super::state::canvas_path_state(scope, context);
    let transform = if explicit.is_some() {
        context_path.borrow().transform()
    } else {
        moli_layout::LayoutTransform2D::IDENTITY
    };
    let source = explicit
        .map(|path| super::state::canvas_path_state(scope, path))
        .unwrap_or(context_path);
    let snapshot = source.borrow().transformed_snapshot(transform);
    let result = canvas_clipping_state(scope, context)
        .borrow_mut()
        .intersect(snapshot, rule);
    if let Err(error) = result {
        throw_paint_error(scope, &error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_clipping_preserves_large_finite_rectangles_and_diagonal_sides() {
        let mut path = Canvas2dPathState::default();
        path.set_transform(f64::MAX, 0.0, 0.0, f64::MAX, 0.0, 0.0);
        path.rect(-10.0, -10.0, 20.0, 20.0);
        let mut clip = CanvasClipRegion::default();
        clip.intersect(path, CanvasFillRule::Nonzero).unwrap();
        let mask = clip.mask(8, 8).unwrap().unwrap();
        assert!(mask.rgba.chunks_exact(4).all(|p| p[3] == 255));

        let mut path = Canvas2dPathState::default();
        path.set_transform(1e200, 0.0, 0.0, 1e200, 0.0, 0.0);
        path.move_to(-10.0, -10.0);
        path.line_to(10.0, 10.0);
        path.line_to(-10.0, 10.0);
        path.close_path();
        let mut clip = CanvasClipRegion::default();
        clip.intersect(path, CanvasFillRule::Nonzero).unwrap();
        let mask = clip.mask(8, 8).unwrap().unwrap();
        assert_eq!(mask.rgba[(7 * 8) * 4 + 3], 255);
        assert_eq!(mask.rgba[7 * 4 + 3], 0);

        let mut path = Canvas2dPathState::default();
        path.move_to(-1e200, -2e200);
        path.line_to(1e200, 2e200);
        path.line_to(-1e200, 2e200);
        path.close_path();
        let mut clip = CanvasClipRegion::default();
        clip.intersect(path, CanvasFillRule::Nonzero).unwrap();
        let mask = clip.mask(8, 8).unwrap().unwrap();
        // Clamping each coordinate independently would change this slope.
        assert_eq!(mask.rgba[(5 * 8 + 3) * 4 + 3], 0);
        assert_eq!(mask.rgba[(7 * 8) * 4 + 3], 255);
    }

    #[test]
    fn native_clip_curves_retain_pixel_sized_terms_beside_enormous_controls() {
        let mut path = Canvas2dPathState::default();
        path.move_to(-1e200, -1e200);
        path.quadratic_curve_to(1e200, 1e200, 8.0, 0.0);
        path.close_path();
        let mut clip = CanvasClipRegion::default();
        clip.intersect(path, CanvasFillRule::Nonzero).unwrap();
        let mask = clip.mask(8, 8).unwrap().unwrap();
        // At the near-origin crossing t tends to 1/3 and x-y tends to 8/9.
        // Rounded subdivision would erase the endpoint's eight-pixel offset
        // and incorrectly turn these diagonal samples half-opaque.
        assert!(mask.rgba[(8 + 1) * 4 + 3] < 8);
        assert_eq!(mask.rgba[(8 + 7) * 4 + 3], 255);
        assert_eq!(mask.rgba[(7 * 8) * 4 + 3], 0);
    }

    #[test]
    fn native_clip_precision_is_bounded_before_allocating_unbounded_integers() {
        let mut path = Canvas2dPathState::default();
        path.rect(-1.0, -1.0, 2.0, 2.0);
        for _ in 0..20 {
            let mut next = Canvas2dPathState::default();
            next.add_path(&path, [f64::MAX, 0.0, 0.0, f64::MAX, 0.0, 0.0]);
            path = next;
        }
        let mut clip = CanvasClipRegion::default();
        clip.intersect(path, CanvasFillRule::Nonzero).unwrap();
        assert!(matches!(
            clip.mask(8, 8),
            Err(moli_paint::PaintError::CanvasClipCoordinateBudgetExceeded { .. })
        ));
    }
}
