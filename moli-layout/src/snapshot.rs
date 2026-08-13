use parley::FontData;

use crate::PaintCaptureSurface;

/// Paint-facing compatibility name for the canonical layout viewport.
pub type PaintViewport = crate::LayoutViewport;

/// Paint-facing compatibility name for a canonical CSS-pixel extent.
pub type PaintSize = crate::LayoutSize;

/// A straight-alpha sRGB color.
///
/// Components normally use the inclusive `0.0..=1.0` range. The paint
/// backend clamps non-finite and out-of-range values at its trust boundary so
/// malformed input cannot reach the rasterizer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintColor {
    /// Red component.
    pub red: f32,
    /// Green component.
    pub green: f32,
    /// Blue component.
    pub blue: f32,
    /// Alpha component.
    pub alpha: f32,
}

impl PaintColor {
    /// Fully transparent black.
    pub const TRANSPARENT: Self = Self::new(0.0, 0.0, 0.0, 0.0);
    /// Opaque white.
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0, 1.0);
    /// Opaque black.
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0, 1.0);

    /// Creates a straight-alpha sRGB color.
    pub const fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    /// Returns the four straight-alpha sRGB components in RGBA order.
    pub const fn components(self) -> [f32; 4] {
        [self.red, self.green, self.blue, self.alpha]
    }
}

impl Default for PaintColor {
    fn default() -> Self {
        Self::BLACK
    }
}

/// Paint-facing compatibility name for a canonical CSS-pixel rectangle.
pub type PaintRect = crate::LayoutRect;

/// Snap one paint-space extent to whole layout pixels while preserving a
/// non-trivial nonzero size.
///
/// This is Blink's `SnapSizeToPixel` contract over Moli's shared 1/64 layout
/// grid. The returned location and size must be used together: the size is
/// sensitive to the fractional part of the location.
pub fn pixel_snap_paint_axis(location: f32, size: f32) -> (f32, f32) {
    pixel_snap_paint_axis_impl(location, size, false)
}

/// Snap one paint-space extent to whole layout pixels, allowing a thin extent
/// to collapse to zero.
///
/// Border inner edges use this variant, matching Blink's
/// `SnapSizeToPixelAllowingZero` behavior.
pub fn pixel_snap_paint_axis_allowing_zero(location: f32, size: f32) -> (f32, f32) {
    pixel_snap_paint_axis_impl(location, size, true)
}

fn pixel_snap_paint_axis_impl(location: f32, size: f32, allow_zero: bool) -> (f32, f32) {
    let location = f64::from(location);
    let size = f64::from(size);
    let fraction = location % 1.0;
    let snapped_location = round_layout_pixel(location);
    let mut snapped_size = round_layout_pixel(fraction + size) - round_layout_pixel(fraction);
    let layout_unit_epsilon = 1.0 / f64::from(crate::LAYOUT_SUBPIXELS_PER_CSS_PIXEL);
    if !allow_zero && snapped_size == 0.0 && size.abs() > 4.0 * layout_unit_epsilon {
        snapped_size = size.signum();
    }
    (snapped_location as f32, snapped_size as f32)
}

pub(crate) fn round_layout_pixel(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// Pixel-snap a positive finite rectangle in pre-transform paint space.
pub fn pixel_snap_paint_rect(rect: PaintRect) -> Option<PaintRect> {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
        || rect.width <= 0.0
        || rect.height <= 0.0
    {
        return None;
    }
    let (x, width) = pixel_snap_paint_axis(rect.x, rect.width);
    let (y, height) = pixel_snap_paint_axis(rect.y, rect.height);
    (width > 0.0 && height > 0.0).then(|| PaintRect::new(x, y, width, height))
}

/// Paint-facing compatibility name for a canonical CSS-pixel point.
pub type PaintPoint = crate::LayoutPoint;

/// Paint-facing compatibility name for an owned CSS 2D affine transform.
pub type PaintTransform2D = crate::LayoutTransform2D;

/// One command in an owned backend-neutral Bézier path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaintPathElement {
    MoveTo(PaintPoint),
    LineTo(PaintPoint),
    QuadTo(PaintPoint, PaintPoint),
    CubicTo(PaintPoint, PaintPoint, PaintPoint),
    Close,
}

/// One owned Bézier path and its conservative local bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintPath {
    pub elements: Vec<PaintPathElement>,
    pub bounds: PaintRect,
}

/// Cap style for an owned stroked path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaintLineCap {
    #[default]
    Butt,
    Round,
    Square,
}

/// Join style for an owned stroked path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaintLineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// Backend-neutral stroke data. Dash lengths are in local CSS pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintStroke {
    pub path: PaintPath,
    pub color: PaintColor,
    pub width: f32,
    pub join: PaintLineJoin,
    pub start_cap: PaintLineCap,
    pub end_cap: PaintLineCap,
    pub miter_limit: f32,
    pub dash_pattern: Vec<f32>,
    pub dash_offset: f32,
    pub transform: PaintTransform2D,
}

/// Four physical edge sizes in CSS pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PaintEdgeSizes {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl PaintEdgeSizes {
    /// Creates physical edge sizes in top/right/bottom/left order.
    pub const fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    /// Returns whether at least one edge has positive extent.
    pub fn has_positive_edge(self) -> bool {
        [self.top, self.right, self.bottom, self.left]
            .into_iter()
            .any(|value| value.is_finite() && value > 0.0)
    }
}

/// Straight-alpha sRGB colors for four physical border edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintBorderColors {
    pub top: PaintColor,
    pub right: PaintColor,
    pub bottom: PaintColor,
    pub left: PaintColor,
}

impl PaintBorderColors {
    /// Uses the same color for every physical edge.
    pub const fn all(color: PaintColor) -> Self {
        Self {
            top: color,
            right: color,
            bottom: color,
            left: color,
        }
    }

    /// Returns whether at least one edge can affect output.
    pub fn has_visible_edge(self) -> bool {
        [self.top, self.right, self.bottom, self.left]
            .into_iter()
            .any(|color| color.alpha.is_finite() && color.alpha > 0.0)
    }
}

impl Default for PaintBorderColors {
    fn default() -> Self {
        Self::all(PaintColor::TRANSPARENT)
    }
}

/// One elliptical corner radius in CSS pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PaintCornerRadius {
    pub x: f32,
    pub y: f32,
}

impl PaintCornerRadius {
    pub const ZERO: Self = Self::new(0.0, 0.0);

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Elliptical radii for the four physical corners of a box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PaintCornerRadii {
    pub top_left: PaintCornerRadius,
    pub top_right: PaintCornerRadius,
    pub bottom_right: PaintCornerRadius,
    pub bottom_left: PaintCornerRadius,
}

impl PaintCornerRadii {
    pub const ZERO: Self = Self::all(PaintCornerRadius::ZERO);

    pub const fn all(radius: PaintCornerRadius) -> Self {
        Self {
            top_left: radius,
            top_right: radius,
            bottom_right: radius,
            bottom_left: radius,
        }
    }

    pub fn is_zero(self) -> bool {
        [
            self.top_left,
            self.top_right,
            self.bottom_right,
            self.bottom_left,
        ]
        .into_iter()
        .all(|radius| radius.x <= 0.0 || radius.y <= 0.0)
    }
}

/// A backend-neutral shape embedded in an owned paint command.
#[derive(Clone, Debug, PartialEq)]
pub enum PaintShape {
    Rect(PaintRect),
    RoundedRect {
        rect: PaintRect,
        radii: PaintCornerRadii,
    },
    Path(PaintPath),
}

impl PaintShape {
    pub const fn rect(rect: PaintRect) -> Self {
        Self::Rect(rect)
    }

    pub const fn rounded_rect(rect: PaintRect, radii: PaintCornerRadii) -> Self {
        Self::RoundedRect { rect, radii }
    }

    pub const fn bounds(&self) -> PaintRect {
        match self {
            Self::Rect(rect) | Self::RoundedRect { rect, .. } => *rect,
            Self::Path(path) => path.bounds,
        }
    }
}

/// How a gradient continues outside its first and last stops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaintGradientExtend {
    #[default]
    Pad,
    Repeat,
    Reflect,
}

/// Color space used to interpolate an owned gradient's resolved stops.
///
/// The variants are limited to color spaces that the raster backend can
/// represent exactly. CSS interpolation methods outside this set are sampled
/// into sRGB stops while the DOM-neutral snapshot is built.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaintGradientColorSpace {
    #[default]
    Srgb,
    LinearSrgb,
    Hsl,
    Hwb,
    Lab,
    Lch,
    Oklab,
    Oklch,
    DisplayP3,
    A98Rgb,
    ProphotoRgb,
    Rec2020,
    XyzD50,
    XyzD65,
}

/// Direction used when interpolating hue in a cylindrical color space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaintGradientHueDirection {
    #[default]
    Shorter,
    Longer,
    Increasing,
    Decreasing,
}

/// Backend-neutral interpolation contract for one owned gradient.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaintGradientInterpolation {
    pub color_space: PaintGradientColorSpace,
    pub hue_direction: PaintGradientHueDirection,
}

/// One resolved gradient stop. Offsets are normalized to the gradient line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintGradientStop {
    pub offset: f32,
    pub color: PaintColor,
}

/// An owned, resolved linear gradient in the primitive's local coordinate space.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintLinearGradient {
    pub start: PaintPoint,
    pub end: PaintPoint,
    pub stops: Vec<PaintGradientStop>,
    pub extend: PaintGradientExtend,
    pub interpolation: PaintGradientInterpolation,
}

/// An owned, resolved two-circle radial gradient.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintRadialGradient {
    pub start_center: PaintPoint,
    pub start_radius: f32,
    pub end_center: PaintPoint,
    pub end_radius: f32,
    pub stops: Vec<PaintGradientStop>,
    pub extend: PaintGradientExtend,
    pub interpolation: PaintGradientInterpolation,
    /// Optional local transform used for elliptical CSS radial gradients.
    pub transform: PaintTransform2D,
}

/// An owned, resolved conic gradient.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintConicGradient {
    pub center: PaintPoint,
    pub start_angle_radians: f32,
    pub end_angle_radians: f32,
    pub stops: Vec<PaintGradientStop>,
    pub extend: PaintGradientExtend,
    pub interpolation: PaintGradientInterpolation,
    /// Local rotation and center translation for CSS conic coordinates.
    pub transform: PaintTransform2D,
}

/// Backend-neutral brush data. Image brushes deliberately remain a Phase 9 concern.
#[derive(Clone, Debug, PartialEq)]
pub enum PaintBrush {
    Solid(PaintColor),
    LinearGradient(PaintLinearGradient),
    RadialGradient(PaintRadialGradient),
    ConicGradient(PaintConicGradient),
}

/// CSS border/outline line style after cascade.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaintBorderStyle {
    None,
    Hidden,
    #[default]
    Solid,
    Dotted,
    Dashed,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
}

/// Physical border styles for the four box edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaintBorderStyles {
    pub top: PaintBorderStyle,
    pub right: PaintBorderStyle,
    pub bottom: PaintBorderStyle,
    pub left: PaintBorderStyle,
}

impl PaintBorderStyles {
    pub const fn all(style: PaintBorderStyle) -> Self {
        Self {
            top: style,
            right: style,
            bottom: style,
            left: style,
        }
    }
}

impl Default for PaintBorderStyles {
    fn default() -> Self {
        Self::all(PaintBorderStyle::Solid)
    }
}

/// CSS blend mode used when compositing a stacking-context layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaintBlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    PlusLighter,
}

/// Porter-Duff operator for an isolated owned layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaintCompositeMode {
    #[default]
    SrcOver,
    DestIn,
    SrcOut,
    SrcIn,
    Xor,
}

/// One CSS filter function after computed-value resolution.
///
/// A CSS filter list is serialized as nested single-filter layers so the CPU
/// backend never silently truncates AnyRender's filter graph to its first node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaintFilter {
    Blur(f32),
    Brightness(f32),
    Contrast(f32),
    Grayscale(f32),
    HueRotate(f32),
    Invert(f32),
    Opacity(f32),
    Saturate(f32),
    Sepia(f32),
    DropShadow {
        offset: PaintPoint,
        blur_radius: f32,
        color: PaintColor,
    },
}

/// One resolved CSS box shadow.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintBoxShadow {
    pub rect: PaintRect,
    pub radii: PaintCornerRadii,
    pub color: PaintColor,
    pub offset: PaintPoint,
    pub blur_radius: f32,
    pub spread_radius: f32,
    pub inset: bool,
    pub transform: PaintTransform2D,
}

/// Dense snapshot-local font identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PaintFontId(u32);

impl PaintFontId {
    /// Creates an identifier from a resource-table index.
    pub fn from_index(index: usize) -> Self {
        Self(u32::try_from(index).expect("one snapshot exceeded the u32 font resource limit"))
    }

    /// Returns the dense resource-table index.
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// One snapshot-owned, shareable font resource.
///
/// `FontData` holds the immutable font blob through an internal `Arc`; cloning
/// a snapshot does not duplicate font bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintFontResource {
    /// Immutable font blob and collection index.
    pub font: FontData,
}

/// Dense snapshot-local image identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PaintImageId(u32);

impl PaintImageId {
    pub fn from_index(index: usize) -> Self {
        Self(u32::try_from(index).expect("one snapshot exceeded the u32 image resource limit"))
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// One immutable decoded image shared with the renderer resource owner.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintImageResource {
    pub image: std::sync::Arc<moli_image::RgbaImage>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaintImageSampling {
    Nearest,
    #[default]
    Linear,
}

/// One resolved image destination in local CSS pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintImage {
    pub image: PaintImageId,
    pub destination: PaintRect,
    pub sampling: PaintImageSampling,
    pub transform: PaintTransform2D,
}

/// Dense snapshot-local SVG image identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PaintSvgImageId(u32);

impl PaintSvgImageId {
    pub fn from_index(index: usize) -> Self {
        Self(u32::try_from(index).expect("one snapshot exceeded the u32 SVG resource limit"))
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// One parsed immutable SVG tree shared with the renderer resource owner.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintSvgImageResource {
    pub image: std::sync::Arc<moli_image::SvgImage>,
}

/// One resolved SVG destination in local CSS pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintSvgImage {
    pub image: PaintSvgImageId,
    pub destination: PaintRect,
    pub transform: PaintTransform2D,
}

/// One positioned glyph in CSS-pixel coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintGlyph {
    /// Font-specific glyph identifier.
    pub id: u32,
    /// Horizontal baseline position in CSS pixels.
    pub x: f32,
    /// Vertical baseline position in CSS pixels.
    pub y: f32,
}

/// Minimal owned output of Parley shaping needed by the paint backend.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintGlyphRun {
    /// Snapshot-local font resource.
    pub font: PaintFontId,
    /// Font size in CSS pixels.
    pub font_size: f32,
    /// Font variation coordinates in normalized font units.
    pub normalized_coords: Vec<i16>,
    /// Glyph fill color.
    pub color: PaintColor,
    /// Optional synthetic italic skew.
    pub glyph_skew_radians: Option<f32>,
    /// Synthetic weight expansion in CSS pixels, or zero for a real face.
    pub glyph_embolden: PaintPoint,
    /// Fully positioned glyphs.
    pub glyphs: Vec<PaintGlyph>,
    /// Maps the glyph positions from their layout coordinate space to viewport CSS pixels.
    pub transform: PaintTransform2D,
}

/// CSS line style for an owned text-decoration command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaintTextDecorationStyle {
    #[default]
    Solid,
    Double,
    Dotted,
    Dashed,
    Wavy,
}

/// One resolved decoration segment for a shaped style run.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintTextDecoration {
    /// Local horizontal start in CSS pixels.
    pub x: f32,
    /// Local vertical center of the decoration stroke.
    pub y: f32,
    /// Segment advance in CSS pixels.
    pub width: f32,
    /// Used stroke thickness in CSS pixels.
    pub thickness: f32,
    pub color: PaintColor,
    pub style: PaintTextDecorationStyle,
    pub transform: PaintTransform2D,
}

/// One resolved text shadow applied to one owned glyph run.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintTextShadow {
    pub run: PaintGlyphRun,
    pub color: PaintColor,
    pub offset: PaintPoint,
    /// CSS shadow blur radius in CSS pixels.
    pub blur_radius: f32,
}

impl PaintGlyphRun {
    /// Copies positioned glyphs into capture-surface CSS coordinates.
    pub fn glyphs_in_surface(&self) -> Vec<PaintGlyph> {
        self.glyphs
            .iter()
            .map(|glyph| {
                let point = self
                    .transform
                    .map_point(crate::LayoutPoint::new(glyph.x, glyph.y));
                PaintGlyph {
                    id: glyph.id,
                    x: point.x,
                    y: point.y,
                }
            })
            .collect()
    }
}

/// One owned paint command emitted by layout.
///
/// Push commands and [`PaintFragment::PopLayer`] form a single well-nested
/// compositor stack. Geometry and brushes are source-free, and every primitive
/// carries the exact transform needed to map its local CSS coordinates to the
/// viewport. The raster backend therefore never needs the live layout world.
#[derive(Clone, Debug, PartialEq)]
pub enum PaintFragment {
    /// Begins an isolated stacking-context layer.
    PushLayer {
        opacity: f32,
        blend_mode: PaintBlendMode,
        composite: PaintCompositeMode,
        clip: PaintShape,
        transform: PaintTransform2D,
        filter: Option<PaintFilter>,
    },
    /// Begins a clip-only layer.
    PushClip {
        shape: PaintShape,
        transform: PaintTransform2D,
    },
    /// Ends the most recently opened layer or clip.
    PopLayer,
    /// Fills one shape with a solid color or CSS gradient.
    Fill {
        shape: PaintShape,
        brush: PaintBrush,
        transform: PaintTransform2D,
    },
    /// Strokes one owned path.
    Stroke(PaintStroke),
    /// Four physical styled border edges around one border box.
    Border {
        rect: PaintRect,
        widths: PaintEdgeSizes,
        colors: PaintBorderColors,
        styles: PaintBorderStyles,
        radii: PaintCornerRadii,
        transform: PaintTransform2D,
    },
    /// One outset or inset CSS box shadow.
    BoxShadow(PaintBoxShadow),
    /// One line segment belonging to underline/overline/line-through paint.
    TextDecoration(PaintTextDecoration),
    /// One glyph shadow, painted before its source glyph run.
    TextShadow(PaintTextShadow),
    /// One shaped, positioned glyph run backed by a snapshot font resource.
    GlyphRun(PaintGlyphRun),
    /// One decoded raster image backed by a snapshot-local resource table.
    Image(PaintImage),
    /// One parsed vector image backed by a snapshot-local SVG resource table.
    SvgImage(PaintSvgImage),
}

impl PaintFragment {
    /// Creates a solid rectangle fragment.
    pub const fn solid_rect(rect: PaintRect, color: PaintColor) -> Self {
        Self::Fill {
            shape: PaintShape::Rect(rect),
            brush: PaintBrush::Solid(color),
            transform: PaintTransform2D::IDENTITY,
        }
    }

    /// Creates a transformed solid rounded rectangle fragment.
    pub const fn solid_rounded_rect(
        rect: PaintRect,
        radii: PaintCornerRadii,
        color: PaintColor,
        transform: PaintTransform2D,
    ) -> Self {
        if radii.top_left.x <= 0.0
            && radii.top_left.y <= 0.0
            && radii.top_right.x <= 0.0
            && radii.top_right.y <= 0.0
            && radii.bottom_right.x <= 0.0
            && radii.bottom_right.y <= 0.0
            && radii.bottom_left.x <= 0.0
            && radii.bottom_left.y <= 0.0
        {
            return Self::Fill {
                shape: PaintShape::Rect(rect),
                brush: PaintBrush::Solid(color),
                transform,
            };
        }
        Self::Fill {
            shape: PaintShape::RoundedRect { rect, radii },
            brush: PaintBrush::Solid(color),
            transform,
        }
    }

    /// Creates a physical border fragment.
    pub const fn border(
        rect: PaintRect,
        widths: PaintEdgeSizes,
        colors: PaintBorderColors,
    ) -> Self {
        Self::Border {
            rect,
            widths,
            colors,
            styles: PaintBorderStyles::all(PaintBorderStyle::Solid),
            radii: PaintCornerRadii::ZERO,
            transform: PaintTransform2D::IDENTITY,
        }
    }

    /// Begins a rectangular clip scope.
    pub const fn push_clip(rect: PaintRect) -> Self {
        Self::PushClip {
            shape: PaintShape::Rect(rect),
            transform: PaintTransform2D::IDENTITY,
        }
    }

    /// Returns a solid fill's local bounds, color, and local-to-surface transform.
    pub fn solid_fill(&self) -> Option<(PaintRect, PaintColor, PaintTransform2D)> {
        let Self::Fill {
            shape,
            brush: PaintBrush::Solid(color),
            transform,
        } = self
        else {
            return None;
        };
        Some((shape.bounds(), *color, *transform))
    }

    /// Returns a solid fill's axis-aligned capture-surface bounds and color.
    pub fn solid_fill_in_surface(&self) -> Option<(PaintRect, PaintColor)> {
        self.solid_fill()
            .map(|(rect, color, transform)| (transform.map_rect(rect).bounding_rect(), color))
    }
}

/// Severity attached to a diagnostic emitted while building a snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintDiagnosticSeverity {
    /// Informational context that did not change output.
    Info,
    /// A supported fallback changed the requested output.
    Warning,
}

/// An owned diagnostic carried with a paint snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaintDiagnostic {
    /// Stable machine-readable identifier.
    pub code: String,
    /// Human-readable detail.
    pub message: String,
    /// Diagnostic severity.
    pub severity: PaintDiagnosticSeverity,
}

impl PaintDiagnostic {
    /// Creates an owned diagnostic.
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        severity: PaintDiagnosticSeverity,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            severity,
        }
    }
}

/// Fully owned input to a paint backend.
///
/// No source-tree identifier, computed-style handle, or renderer borrow is
/// retained here. Layout is responsible for projecting those values into this
/// representation before returning.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintSnapshot {
    /// Live CSS layout viewport used to build this snapshot.
    pub viewport: PaintViewport,
    /// Short-lived raster surface selected for this capture.
    pub surface: PaintCaptureSurface,
    /// Maps projection viewport coordinates into capture-surface CSS pixels.
    pub viewport_to_surface: PaintTransform2D,
    /// Color composited over the whole output before any fragments.
    pub canvas_color: PaintColor,
    /// Basic document overflow extent for this layout demand.
    ///
    /// This is independent of device pixel ratio and excludes fixed-position
    /// subtrees. Per-node scroll geometry remains a later layout-output concern.
    pub content_size: PaintSize,
    /// Paint operations in back-to-front order.
    pub fragments: Vec<PaintFragment>,
    /// Immutable resources referenced by glyph fragments.
    pub fonts: Vec<PaintFontResource>,
    /// Immutable resources referenced by image fragments.
    pub images: Vec<PaintImageResource>,
    /// Immutable vector resources referenced by SVG image fragments.
    pub svg_images: Vec<PaintSvgImageResource>,
    /// Non-fatal projection diagnostics.
    pub diagnostics: Vec<PaintDiagnostic>,
}

impl PaintSnapshot {
    /// Creates an empty snapshot for a viewport and canvas color.
    pub fn new(viewport: PaintViewport, canvas_color: PaintColor) -> Self {
        Self {
            viewport,
            surface: PaintCaptureSurface::for_viewport(viewport),
            viewport_to_surface: PaintTransform2D::IDENTITY,
            canvas_color,
            content_size: PaintSize::new(viewport.css_width as f32, viewport.css_height as f32),
            fragments: Vec::new(),
            fonts: Vec::new(),
            images: Vec::new(),
            svg_images: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Appends a paint fragment.
    pub fn push_fragment(&mut self, fragment: PaintFragment) {
        self.fragments.push(fragment);
    }

    /// Interns a font by resource identity and collection index.
    pub fn intern_font(&mut self, font: &FontData) -> PaintFontId {
        if let Some(index) = self
            .fonts
            .iter()
            .position(|resource| resource.font == *font)
        {
            return PaintFontId::from_index(index);
        }
        let id = PaintFontId::from_index(self.fonts.len());
        self.fonts.push(PaintFontResource { font: font.clone() });
        id
    }

    /// Resolves a snapshot-local font identifier without consulting live state.
    pub fn font(&self, id: PaintFontId) -> Option<&PaintFontResource> {
        self.fonts.get(id.index())
    }

    /// Adds one renderer-owned immutable image without copying its pixels.
    ///
    /// Image projection currently emits one image fragment per resource
    /// reference, so an O(n) interning scan would cost CPU without eliminating
    /// table entries. Callers that emit repeated fragments should reuse the
    /// returned identifier themselves.
    pub fn add_image(&mut self, image: std::sync::Arc<moli_image::RgbaImage>) -> PaintImageId {
        let id = PaintImageId::from_index(self.images.len());
        self.images.push(PaintImageResource { image });
        id
    }

    pub fn image(&self, id: PaintImageId) -> Option<&PaintImageResource> {
        self.images.get(id.index())
    }

    /// Adds one renderer-owned immutable SVG tree without rasterizing it.
    pub fn add_svg_image(
        &mut self,
        image: std::sync::Arc<moli_image::SvgImage>,
    ) -> PaintSvgImageId {
        let id = PaintSvgImageId::from_index(self.svg_images.len());
        self.svg_images.push(PaintSvgImageResource { image });
        id
    }

    pub fn svg_image(&self, id: PaintSvgImageId) -> Option<&PaintSvgImageResource> {
        self.svg_images.get(id.index())
    }

    /// Appends an owned diagnostic.
    pub fn push_diagnostic(&mut self, diagnostic: PaintDiagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// Consumes one child-document snapshot into this snapshot's resource and
    /// command stream.
    ///
    /// `local_to_surface` maps the child viewport origin into the parent's
    /// capture surface. The explicit clip is the iframe's exact used content
    /// box; it can differ fractionally from the integer child layout viewport.
    /// Snapshot-local resource identifiers are remapped while immutable font,
    /// raster, and SVG allocations remain shared.
    pub fn append_embedded_snapshot(
        &mut self,
        child: PaintSnapshot,
        clip: PaintRect,
        local_to_surface: PaintTransform2D,
    ) {
        if !clip.width.is_finite()
            || !clip.height.is_finite()
            || clip.width <= 0.0
            || clip.height <= 0.0
            || !local_to_surface.is_finite()
        {
            return;
        }

        let font_ids = child
            .fonts
            .iter()
            .map(|resource| self.intern_font(&resource.font))
            .collect::<Vec<_>>();
        let image_ids = child
            .images
            .iter()
            .map(|resource| self.add_image(resource.image.clone()))
            .collect::<Vec<_>>();
        let svg_image_ids = child
            .svg_images
            .iter()
            .map(|resource| self.add_svg_image(resource.image.clone()))
            .collect::<Vec<_>>();

        self.push_fragment(PaintFragment::PushClip {
            shape: PaintShape::Rect(clip),
            transform: local_to_surface,
        });
        self.push_fragment(PaintFragment::Fill {
            shape: PaintShape::Rect(clip),
            brush: PaintBrush::Solid(child.canvas_color),
            transform: local_to_surface,
        });
        for fragment in child.fragments {
            if let Some(fragment) = rebase_embedded_fragment(
                fragment,
                local_to_surface,
                &font_ids,
                &image_ids,
                &svg_image_ids,
            ) {
                self.push_fragment(fragment);
            }
        }
        self.push_fragment(PaintFragment::PopLayer);
        for diagnostic in child.diagnostics {
            if !self.diagnostics.contains(&diagnostic) {
                self.push_diagnostic(diagnostic);
            }
        }
    }
}

fn rebase_embedded_fragment(
    fragment: PaintFragment,
    parent: PaintTransform2D,
    font_ids: &[PaintFontId],
    image_ids: &[PaintImageId],
    svg_image_ids: &[PaintSvgImageId],
) -> Option<PaintFragment> {
    Some(match fragment {
        PaintFragment::PushLayer {
            opacity,
            blend_mode,
            composite,
            clip,
            transform,
            filter,
        } => PaintFragment::PushLayer {
            opacity,
            blend_mode,
            composite,
            clip,
            transform: parent.concatenate(transform),
            filter,
        },
        PaintFragment::PushClip { shape, transform } => PaintFragment::PushClip {
            shape,
            transform: parent.concatenate(transform),
        },
        PaintFragment::PopLayer => PaintFragment::PopLayer,
        PaintFragment::Fill {
            shape,
            brush,
            transform,
        } => PaintFragment::Fill {
            shape,
            brush,
            transform: parent.concatenate(transform),
        },
        PaintFragment::Stroke(mut stroke) => {
            stroke.transform = parent.concatenate(stroke.transform);
            PaintFragment::Stroke(stroke)
        }
        PaintFragment::Border {
            rect,
            widths,
            colors,
            styles,
            radii,
            transform,
        } => PaintFragment::Border {
            rect,
            widths,
            colors,
            styles,
            radii,
            transform: parent.concatenate(transform),
        },
        PaintFragment::BoxShadow(mut shadow) => {
            shadow.transform = parent.concatenate(shadow.transform);
            PaintFragment::BoxShadow(shadow)
        }
        PaintFragment::TextDecoration(mut decoration) => {
            decoration.transform = parent.concatenate(decoration.transform);
            PaintFragment::TextDecoration(decoration)
        }
        PaintFragment::TextShadow(mut shadow) => {
            shadow.run.font = *font_ids.get(shadow.run.font.index())?;
            shadow.run.transform = parent.concatenate(shadow.run.transform);
            PaintFragment::TextShadow(shadow)
        }
        PaintFragment::GlyphRun(mut run) => {
            run.font = *font_ids.get(run.font.index())?;
            run.transform = parent.concatenate(run.transform);
            PaintFragment::GlyphRun(run)
        }
        PaintFragment::Image(mut image) => {
            image.image = *image_ids.get(image.image.index())?;
            image.transform = parent.concatenate(image.transform);
            PaintFragment::Image(image)
        }
        PaintFragment::SvgImage(mut image) => {
            image.image = *svg_image_ids.get(image.image.index())?;
            image.transform = parent.concatenate(image.transform);
            PaintFragment::SvgImage(image)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_source_free<T: Send + Sync + 'static>() {}

    #[test]
    fn paint_pixel_snapping_preserves_blink_layout_unit_semantics() {
        assert_eq!(pixel_snap_paint_axis(0.0, 3.0 / 64.0), (0.0, 0.0));
        assert_eq!(pixel_snap_paint_axis(0.0, 5.0 / 64.0), (0.0, 1.0));
        assert_eq!(pixel_snap_paint_axis(0.5, 1.5), (1.0, 1.0));
        assert_eq!(pixel_snap_paint_axis_allowing_zero(0.5, 0.5), (1.0, 0.0));
        assert_eq!(
            pixel_snap_paint_rect(PaintRect::new(151.09375, 24.0, 90.0, 90.0)),
            Some(PaintRect::new(151.0, 24.0, 90.0, 90.0))
        );
    }

    #[test]
    fn snapshot_is_owned_and_source_free() {
        assert_source_free::<PaintSnapshot>();

        let source_code = String::from("unsupported-gradient");
        let source_message = String::from("fell back to a solid color");
        let mut snapshot = PaintSnapshot::new(PaintViewport::new(800, 600, 1.0), PaintColor::WHITE);
        snapshot.push_fragment(PaintFragment::solid_rect(
            PaintRect::new(10.0, 20.0, 30.0, 40.0),
            PaintColor::new(1.0, 0.0, 0.0, 0.5),
        ));
        snapshot.push_diagnostic(PaintDiagnostic::new(
            source_code,
            source_message,
            PaintDiagnosticSeverity::Warning,
        ));

        let moved = snapshot;
        assert_eq!(moved.viewport, PaintViewport::new(800, 600, 1.0));
        assert_eq!(moved.fragments.len(), 1);
        assert_eq!(moved.diagnostics[0].code, "unsupported-gradient");
    }

    #[test]
    fn embedded_snapshot_is_clipped_and_rebased_into_parent_surface() {
        let mut parent = PaintSnapshot::new(PaintViewport::new(300, 200, 1.0), PaintColor::WHITE);
        let mut child = PaintSnapshot::new(
            PaintViewport::new(100, 50, 1.0),
            PaintColor::new(0.1, 0.2, 0.3, 1.0),
        );
        child.push_fragment(PaintFragment::Fill {
            shape: PaintShape::Rect(PaintRect::new(0.0, 0.0, 10.0, 5.0)),
            brush: PaintBrush::Solid(PaintColor::BLACK),
            transform: PaintTransform2D::translation(1.0, 2.0),
        });

        parent.append_embedded_snapshot(
            child,
            PaintRect::new(0.0, 0.0, 100.0, 50.0),
            PaintTransform2D::translation(20.0, 30.0),
        );

        assert_eq!(parent.fragments.len(), 4);
        assert!(matches!(
            parent.fragments[0],
            PaintFragment::PushClip {
                shape: PaintShape::Rect(PaintRect {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 50.0,
                }),
                transform,
            } if transform == PaintTransform2D::translation(20.0, 30.0)
        ));
        assert!(matches!(
            parent.fragments[1],
            PaintFragment::Fill {
                brush: PaintBrush::Solid(PaintColor {
                    red: 0.1,
                    green: 0.2,
                    blue: 0.3,
                    alpha: 1.0,
                }),
                ..
            }
        ));
        let PaintFragment::Fill { transform, .. } = parent.fragments[2] else {
            panic!("child fill was not retained");
        };
        assert_eq!(transform, PaintTransform2D::translation(21.0, 32.0));
        assert_eq!(parent.fragments[3], PaintFragment::PopLayer);
    }
}
