//! One-shot CSS box construction, numeric layout, and owned paint projection.
//!
//! The crate knows Stylo and Taffy but never knows Moli's live DOM or V8
//! runtime. Renderer-owned adapters lend a canonical source view and resolve
//! styles; all source/style borrows and per-pass caches are gone before the
//! returned [`FrozenLayoutTree`] or [`PaintSnapshot`] crosses into a consumer.

/// Number of fixed layout subpixels in one CSS pixel, matching Blink's
/// `LayoutUnit` precision.
pub const LAYOUT_SUBPIXELS_PER_CSS_PIXEL: f32 = 64.0;

mod builder;
mod capture;
mod containment;
mod error;
mod form;
mod gradient;
mod inline;
mod intrinsic;
mod layout_tree;
mod list;
mod normalize;
mod normalize_source;
mod paint;
mod pass;
mod positioned;
mod projection;
mod replaced;
mod scrollbar;
mod snapshot;
mod source;
mod stacking;
mod style;
mod stylo_to_parley;
mod system_fonts;
mod table;
mod taffy_tree;
mod text;
mod world;

pub use builder::build_layout_world;
pub use capture::{
    FULL_DOCUMENT_CAPTURE_CSS_DIMENSION_LIMIT, PaintCaptureRegion, PaintCaptureRequest,
    PaintCaptureSurface,
};
pub use error::LayoutError;
pub use layout_tree::{
    FrozenCoordinateSpace, FrozenLayoutBox, FrozenLayoutTree, GeometryProvider, LayoutAnswers,
    LayoutBoxGeometry, LayoutBoxModel, LayoutCaretPosition, LayoutClipChainId, LayoutClipNode,
    LayoutCoordinateSpaceId, LayoutDocumentMetrics, LayoutElementMetrics, LayoutFlushReason,
    LayoutFragment, LayoutFragmentBoxModel, LayoutFragmentId, LayoutFragmentKind, LayoutHit,
    LayoutIntersectionGeometry, LayoutNodeOutput, LayoutOutputBoxId, LayoutPassMetrics,
    LayoutPassResult, LayoutPoint, LayoutQuad, LayoutQuery, LayoutQueryAnswer, LayoutQueryBatch,
    LayoutRect, LayoutScrollContainerMetrics, LayoutScrollExtent, LayoutScrollIntoViewGeometry,
    LayoutSize, LayoutTransform2D, LayoutTreeRetentionMetrics, LayoutViewport,
    MAX_RETAINED_LAYOUT_BOXES, MAX_RETAINED_LAYOUT_FRAGMENTS, MAX_RETAINED_LAYOUT_TREE_BYTES,
};
pub use normalize::{NormalizedBoxNode, NormalizedBoxTree, NormalizedFormattingContext};
pub use normalize_source::{
    NormalizedLayoutSourceNode, NormalizedLayoutSourceTree, normalize_layout_source,
};
pub use pass::{
    EmbeddedFrameRenderer, LayoutPassRequest, ScreenshotLayoutRequest, build_layout_pass,
    build_layout_pass_with_embedded_frames, build_screenshot_snapshot,
};
pub use scrollbar::{
    LayoutScrollbarAxis, LayoutScrollbarColors, LayoutScrollbarGeometry, LayoutScrollbarGutter,
    LayoutScrollbarHit, LayoutScrollbarPart, LayoutScrollbarWidth,
};
pub use snapshot::{
    PaintBlendMode, PaintBorderColors, PaintBorderStyle, PaintBorderStyles, PaintBoxShadow,
    PaintBrush, PaintColor, PaintCompositeMode, PaintConicGradient, PaintCornerRadii,
    PaintCornerRadius, PaintDiagnostic, PaintDiagnosticSeverity, PaintEdgeSizes, PaintFilter,
    PaintFontId, PaintFontResource, PaintFragment, PaintGlyph, PaintGlyphRun,
    PaintGradientColorSpace, PaintGradientExtend, PaintGradientHueDirection,
    PaintGradientInterpolation, PaintGradientStop, PaintImage, PaintImageId, PaintImageResource,
    PaintImageSampling, PaintLineCap, PaintLineJoin, PaintLinearGradient, PaintPath,
    PaintPathElement, PaintPoint, PaintRadialGradient, PaintRect, PaintShape, PaintSize,
    PaintSnapshot, PaintStroke, PaintSvgImage, PaintSvgImageId, PaintSvgImageResource,
    PaintTextDecoration, PaintTextDecorationStyle, PaintTextShadow, PaintTransform2D,
    PaintViewport, pixel_snap_paint_axis, pixel_snap_paint_axis_allowing_zero,
    pixel_snap_paint_rect,
};
pub use source::{
    LayoutElementCategory, LayoutElementMetadata, LayoutElementSemantics, LayoutFormControlData,
    LayoutFormControlKind, LayoutImageResource, LayoutInputControlKind, LayoutListData,
    LayoutListRole, LayoutNamespace, LayoutPseudo, LayoutReplacedKind, LayoutSource,
    LayoutSourceKind, LayoutStyleResolver, LayoutTableData, LayoutTableRole, LayoutTextSelection,
    ReplacedMetrics,
};
pub use style::{
    LayoutDisplay, LayoutInlineAlignment, LayoutListMarkerPosition, LayoutListMarkerType,
    LayoutPosition, ResolvedLayoutStyle,
};
pub use text::{
    DocumentLayoutServices, SystemFontPolicy, WebFontFace, WebFontRegistration,
    WebFontRegistrationError, WebFontRegistrationOutcome, WebFontStyle, WebFontUnicodeRange,
};
pub use world::{
    LayoutAnonymousReason, LayoutBox, LayoutBoxId, LayoutBoxKind, LayoutCapabilityDiagnostic,
    LayoutWorld,
};
