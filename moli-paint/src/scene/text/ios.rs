// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{ffi::c_void, ptr::NonNull};

use moli_layout::{PaintGlyphRun, PaintPoint};
use objc2_core_foundation::CFString;
use objc2_core_graphics::{CGPath, CGPathElement, CGPathElementType};
use objc2_core_text::CTFont;
use peniko::kurbo::{Affine, BezPath};

pub(super) fn outline(
    font_blob_id: u64,
    run: &PaintGlyphRun,
    offset: PaintPoint,
    glyph_transform: Option<Affine>,
) -> Option<BezPath> {
    let name = moli_system_fonts::native_outline_font_name(font_blob_id)?;
    let font = unsafe {
        CTFont::with_name(
            &CFString::from_str(name),
            f64::from(run.font_size),
            std::ptr::null(),
        )
    };
    let transform = glyph_transform.unwrap_or(Affine::IDENTITY) * Affine::FLIP_Y;
    let mut run_path = BezPath::new();
    for glyph in &run.glyphs {
        let Ok(id) = u16::try_from(glyph.id) else {
            continue;
        };
        let Some(path) = (unsafe { font.path_for_glyph(id, std::ptr::null()) }) else {
            continue;
        };
        let mut glyph_path = BezPath::new();
        // Core Graphics calls the callback synchronously with valid path elements.
        unsafe {
            CGPath::apply(
                Some(&path),
                (&mut glyph_path as *mut BezPath).cast(),
                Some(add_element),
            )
        };
        let translate =
            Affine::translate((f64::from(glyph.x + offset.x), f64::from(glyph.y + offset.y)));
        glyph_path.apply_affine(translate * transform);
        run_path.extend(glyph_path);
    }
    Some(run_path)
}

unsafe extern "C-unwind" fn add_element(context: *mut c_void, element: NonNull<CGPathElement>) {
    // The caller passes a mutable BezPath, and Core Graphics provides the
    // number of points specified by the element type below.
    let path = unsafe { &mut *context.cast::<BezPath>() };
    let element = unsafe { element.as_ref() };
    let point = |index| {
        let point = unsafe { *element.points.as_ptr().add(index) };
        (point.x, point.y)
    };
    match element.r#type {
        CGPathElementType::MoveToPoint => path.move_to(point(0)),
        CGPathElementType::AddLineToPoint => path.line_to(point(0)),
        CGPathElementType::AddQuadCurveToPoint => path.quad_to(point(0), point(1)),
        CGPathElementType::AddCurveToPoint => path.curve_to(point(0), point(1), point(2)),
        CGPathElementType::CloseSubpath => path.close_path(),
        _ => {}
    }
}
