// SPDX-License-Identifier: MIT OR Apache-2.0
//
// The Stylo-to-Taffy projection uses the standalone `stylo_taffy` crate from
// DioxusLabs/blitz commit d788124ab881f9bb537cb452ec1d837604a374a8.

use std::sync::Arc;

use style::{
    Atom,
    color::ColorSpace,
    computed_values::{
        content_visibility::T as StyloContentVisibility, flex_direction::T as StyloFlexDirection,
        isolation::T as StyloIsolation, mix_blend_mode::T as StyloMixBlendMode,
        visibility::T as StyloVisibility,
    },
    properties::ComputedValues,
    properties::generated::longhands::position::computed_value::T as StyloPosition,
    properties::generated::longhands::{
        direction::computed_value::T as StyloDirection,
        text_orientation::computed_value::T as StyloTextOrientation,
        unicode_bidi::computed_value::T as StyloUnicodeBidi,
        writing_mode::computed_value::T as StyloWritingMode,
    },
    servo_arc::Arc as ServoArc,
    values::{
        computed::{
            AlignmentBaseline, BorderStyle as StyloBorderStyle, Content, ContentItem, Float,
            OutlineStyle as StyloOutlineStyle, Overflow, basic_shape::ClipPath as StyloClipPath,
            length::CSSPixelLength,
        },
        generics::{
            box_::{
                BaselineShift as GenericBaselineShift, BaselineShiftKeyword,
                GenericContainIntrinsicSize, Perspective as GenericPerspective,
            },
            flex::GenericFlexBasis,
            grid::GenericGridTemplateComponent,
            image::GenericImage,
            length::{GenericMaxSize, GenericSize},
            position::PreferredRatio,
            transform::{Rotate, Scale, Translate},
        },
        specified::box_::{ContainerType, DisplayInside, DisplayOutside},
        specified::{
            TextAlignKeyword, WillChangeBits,
            align::{AlignFlags, ContentDistribution},
            text::{TextTransform, TextTransformCase},
        },
    },
};
use taffy::{
    BoxSizing, Display as TaffyDisplay, FontBaseline, LogicalSize, Position as TaffyPosition,
    ResolvedAspectRatio, Size, SizeContainment, Style,
};

use crate::{
    LayoutElementContent, LayoutPoint, LayoutRect, LayoutSize, LayoutTransform2D, PaintBlendMode,
    PaintBorderColors, PaintBorderStyle, PaintBorderStyles, PaintBoxShadow, PaintColor,
    PaintCornerRadii, PaintCornerRadius, PaintEdgeSizes, PaintFragment,
};

/// Marker families implemented by the Phase 4 list formatter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutListMarkerType {
    None,
    Decimal,
    LowerAlpha,
    UpperAlpha,
    Disc,
    Circle,
    Square,
    DisclosureOpen,
    DisclosureClosed,
    String(Arc<str>),
    Symbols(Vec<Arc<str>>),
    Fallback,
}

fn stylo_blend_mode(mode: StyloMixBlendMode) -> PaintBlendMode {
    match mode {
        StyloMixBlendMode::Normal => PaintBlendMode::Normal,
        StyloMixBlendMode::Multiply => PaintBlendMode::Multiply,
        StyloMixBlendMode::Screen => PaintBlendMode::Screen,
        StyloMixBlendMode::Overlay => PaintBlendMode::Overlay,
        StyloMixBlendMode::Darken => PaintBlendMode::Darken,
        StyloMixBlendMode::Lighten => PaintBlendMode::Lighten,
        StyloMixBlendMode::ColorDodge => PaintBlendMode::ColorDodge,
        StyloMixBlendMode::ColorBurn => PaintBlendMode::ColorBurn,
        StyloMixBlendMode::HardLight => PaintBlendMode::HardLight,
        StyloMixBlendMode::SoftLight => PaintBlendMode::SoftLight,
        StyloMixBlendMode::Difference => PaintBlendMode::Difference,
        StyloMixBlendMode::Exclusion => PaintBlendMode::Exclusion,
        StyloMixBlendMode::Hue => PaintBlendMode::Hue,
        StyloMixBlendMode::Saturation => PaintBlendMode::Saturation,
        StyloMixBlendMode::Color => PaintBlendMode::Color,
        StyloMixBlendMode::Luminosity => PaintBlendMode::Luminosity,
        StyloMixBlendMode::PlusLighter => PaintBlendMode::PlusLighter,
    }
}

/// Splits one physical `contain-intrinsic-*` component into the authored
/// fallback and the stateful `auto` selector.
fn contain_intrinsic_component(
    value: &style::values::computed::ContainIntrinsicSize,
) -> (Option<f32>, bool) {
    match value {
        GenericContainIntrinsicSize::Length(length) => (Some(length.px()), false),
        GenericContainIntrinsicSize::AutoLength(length) => (Some(length.px()), true),
        GenericContainIntrinsicSize::None => (None, false),
        GenericContainIntrinsicSize::AutoNone => (None, true),
    }
}

/// Last content-box size recorded for an element with an `auto`
/// `contain-intrinsic-*` component.
///
/// Values are logical CSS pixels. The browser owns their lifetime and passes
/// them into a new layout epoch; layout converts them back into the element's
/// current physical writing mode and effective zoom.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutLastRememberedSize {
    pub inline_size: Option<f32>,
    pub block_size: Option<f32>,
}

impl LayoutLastRememberedSize {
    pub const fn is_empty(self) -> bool {
        self.inline_size.is_none() && self.block_size.is_none()
    }
}

/// Axis and writing-mode policy used by the browser's intrinsic-size observer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutLastRememberedSizePolicy {
    auto_inline_size: bool,
    auto_block_size: bool,
    horizontal_writing_mode: bool,
}

impl LayoutLastRememberedSizePolicy {
    pub const fn records_inline_size(self) -> bool {
        self.auto_inline_size
    }

    pub const fn records_block_size(self) -> bool {
        self.auto_block_size
    }

    pub const fn records_any_size(self) -> bool {
        self.auto_inline_size || self.auto_block_size
    }

    /// Converts an unzoomed physical content box into the logical axes that
    /// remain meaningful if the element later changes writing mode.
    pub fn observe(self, physical_size: LayoutSize) -> LayoutLastRememberedSize {
        let (inline_size, block_size) = if self.horizontal_writing_mode {
            (physical_size.width, physical_size.height)
        } else {
            (physical_size.height, physical_size.width)
        };
        LayoutLastRememberedSize {
            inline_size: self.auto_inline_size.then_some(inline_size),
            block_size: self.auto_block_size.then_some(block_size),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LayoutContentVisibility {
    #[default]
    Visible,
    Hidden,
    Auto,
}

/// Computed `visibility` retained without collapsing CSS's table-specific
/// `collapse` value into ordinary paint suppression.
///
/// `hidden` and `collapse` both suppress ordinary paint. Table layout also
/// consumes `collapse` as a track-level used value, so that distinction must
/// survive the Stylo projection boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LayoutVisibility {
    #[default]
    Visible,
    Hidden,
    Collapse,
}

/// One style's complete path from authored containment to the used value
/// consumed by Taffy. Browser state is injected once per layout epoch; all
/// writing-mode and zoom projection remains owned by this object.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LayoutSizeContainmentState {
    authored_axes: LogicalSize<bool>,
    intrinsic_fallback: Size<Option<f32>>,
    intrinsic_auto_axes: Size<bool>,
    content_visibility: LayoutContentVisibility,
    contents_skipped: bool,
    remembered_size: LayoutLastRememberedSize,
    used: SizeContainment,
}

impl Default for LayoutSizeContainmentState {
    fn default() -> Self {
        Self {
            authored_axes: LogicalSize {
                inline_size: false,
                block_size: false,
            },
            intrinsic_fallback: Size::NONE,
            intrinsic_auto_axes: Size {
                width: false,
                height: false,
            },
            content_visibility: LayoutContentVisibility::Visible,
            contents_skipped: false,
            remembered_size: LayoutLastRememberedSize::default(),
            used: SizeContainment::NONE,
        }
    }
}

impl LayoutSizeContainmentState {
    fn new(
        authored_axes: LogicalSize<bool>,
        intrinsic_fallback: Size<Option<f32>>,
        intrinsic_auto_axes: Size<bool>,
        content_visibility: LayoutContentVisibility,
        writing_mode: taffy::WritingMode,
        effective_zoom: f32,
    ) -> Self {
        let mut state = Self {
            authored_axes,
            intrinsic_fallback,
            intrinsic_auto_axes,
            content_visibility,
            contents_skipped: content_visibility == LayoutContentVisibility::Hidden,
            remembered_size: LayoutLastRememberedSize::default(),
            used: SizeContainment::NONE,
        };
        state.recompute(writing_mode, effective_zoom);
        state
    }

    fn resolve_browser_state(
        &mut self,
        auto_contents_skipped: bool,
        remembered_size: LayoutLastRememberedSize,
        writing_mode: taffy::WritingMode,
        effective_zoom: f32,
    ) {
        self.contents_skipped = match self.content_visibility {
            LayoutContentVisibility::Visible => false,
            LayoutContentVisibility::Hidden => true,
            LayoutContentVisibility::Auto => auto_contents_skipped,
        };
        self.remembered_size = remembered_size;
        self.recompute(writing_mode, effective_zoom);
    }

    fn recompute(&mut self, writing_mode: taffy::WritingMode, effective_zoom: f32) {
        let logical_axes = LogicalSize {
            inline_size: self.authored_axes.inline_size || self.contents_skipped,
            block_size: self.authored_axes.block_size || self.contents_skipped,
        };
        let mut intrinsic_content_size = self.intrinsic_fallback;
        if self.contents_skipped {
            let to_layout_space = |size: Option<f32>| {
                size.filter(|size| size.is_finite())
                    .map(|size| (size.max(0.0) * effective_zoom).max(0.0))
                    .filter(|size| size.is_finite())
            };
            let remembered = writing_mode.to_physical(LogicalSize {
                inline_size: to_layout_space(self.remembered_size.inline_size),
                block_size: to_layout_space(self.remembered_size.block_size),
            });
            if self.intrinsic_auto_axes.width && remembered.width.is_some() {
                intrinsic_content_size.width = remembered.width;
            }
            if self.intrinsic_auto_axes.height && remembered.height.is_some() {
                intrinsic_content_size.height = remembered.height;
            }
        }
        self.used = SizeContainment::new(
            writing_mode.to_physical(logical_axes),
            intrinsic_content_size,
        );
    }

    fn observer_policy(self, writing_mode: taffy::WritingMode) -> LayoutLastRememberedSizePolicy {
        let auto_axes = writing_mode.to_logical(self.intrinsic_auto_axes);
        LayoutLastRememberedSizePolicy {
            auto_inline_size: auto_axes.inline_size,
            auto_block_size: auto_axes.block_size,
            horizontal_writing_mode: writing_mode.is_horizontal(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutListMarkerPosition {
    Inside,
    #[default]
    Outside,
}

/// CSS whitespace processing mode retained before Parley shaping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum InlineWhiteSpaceCollapse {
    #[default]
    Collapse,
    Preserve,
    PreserveBreaks,
    BreakSpaces,
}

/// Case transform applied while producing an IFC's shared logical text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum InlineTextTransform {
    #[default]
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum InlineDirection {
    #[default]
    Ltr,
    Rtl,
}

/// Glyph orientation selected inside a vertical inline formatting context.
///
/// The shaping backend still owns glyph forms. Layout retains the full CSS
/// value because it also selects the IFC's dominant baseline: `sideways`
/// uses the alphabetic baseline while `mixed` and `upright` use the central
/// baseline in vertical flow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum InlineTextOrientation {
    #[default]
    Mixed,
    Upright,
    Sideways,
}

impl InlineTextOrientation {
    const fn font_baseline(self, writing_mode: taffy::WritingMode) -> FontBaseline {
        match writing_mode {
            taffy::WritingMode::VerticalRl | taffy::WritingMode::VerticalLr
                if !matches!(self, Self::Sideways) =>
            {
                FontBaseline::Central
            }
            taffy::WritingMode::HorizontalTb
            | taffy::WritingMode::VerticalRl
            | taffy::WritingMode::VerticalLr
            | taffy::WritingMode::SidewaysRl
            | taffy::WritingMode::SidewaysLr => FontBaseline::Alphabetic,
        }
    }
}

impl InlineDirection {
    const fn from_taffy(direction: taffy::Direction) -> Self {
        match direction {
            taffy::Direction::Ltr => Self::Ltr,
            taffy::Direction::Rtl => Self::Rtl,
        }
    }

    const fn to_taffy(self) -> taffy::Direction {
        match self {
            Self::Ltr => taffy::Direction::Ltr,
            Self::Rtl => taffy::Direction::Rtl,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum InlineUnicodeBidi {
    #[default]
    Normal,
    Embed,
    Isolate,
    BidiOverride,
    IsolateOverride,
    Plaintext,
}

/// Alignment of an inline-level box relative to its parent inline box or line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutInlineAlignment {
    #[default]
    Baseline,
    TextTop,
    Middle,
    TextBottom,
    Top,
    Bottom,
}

/// The two independent components of the CSS `vertical-align` shorthand.
/// Positive `baseline_shift` values raise content in CSS coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct InlineVerticalAlign {
    pub(crate) kind: LayoutInlineAlignment,
    pub(crate) baseline_shift: f32,
}

/// The two independent components retained from CSS `aspect-ratio`.
///
/// Taffy's public style currently stores only the numeric ratio. Keeping the
/// `auto` component here is essential for replaced elements: `1 / 1` replaces
/// an image's natural ratio, while `auto 1 / 1` uses that natural ratio and
/// only falls back to 1:1 when no natural ratio exists.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum PreferredAspectRatio {
    #[default]
    Auto,
    Ratio(f32),
    AutoAndRatio(f32),
}

impl PreferredAspectRatio {
    fn from_components(auto: bool, ratio: Option<f32>) -> Self {
        match (auto, usable_aspect_ratio(ratio)) {
            (_, None) => Self::Auto,
            (false, Some(ratio)) => Self::Ratio(ratio),
            (true, Some(ratio)) => Self::AutoAndRatio(ratio),
        }
    }

    fn from_taffy(ratio: Option<f32>) -> Self {
        Self::from_components(false, ratio)
    }

    fn numeric_ratio(self) -> Option<f32> {
        match self {
            Self::Auto => None,
            Self::Ratio(ratio) | Self::AutoAndRatio(ratio) => Some(ratio),
        }
    }

    fn resolve(
        self,
        natural_ratio: Option<f32>,
        authored_box_sizing: BoxSizing,
    ) -> ResolvedAspectRatio {
        let natural_ratio = usable_aspect_ratio(natural_ratio);
        match self {
            Self::Ratio(ratio) => ResolvedAspectRatio {
                ratio: Some(ratio),
                box_sizing: authored_box_sizing,
            },
            Self::Auto => ResolvedAspectRatio {
                ratio: natural_ratio,
                box_sizing: BoxSizing::ContentBox,
            },
            // Blink's BoxSizingForAspectRatio() uses content-box for the
            // combined `auto <ratio>` value, including when its ratio is used
            // as the fallback because no natural ratio is available.
            Self::AutoAndRatio(fallback) => ResolvedAspectRatio {
                ratio: natural_ratio.or(Some(fallback)),
                box_sizing: BoxSizing::ContentBox,
            },
        }
    }
}

/// CSS display classification retained across box construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutDisplay {
    None,
    Contents,
    Block,
    FlowRoot,
    Inline,
    InlineBlock,
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
    BlockListItem,
    InlineListItem,
    Table,
    InlineTable,
    TableCaption,
    TableRowGroup,
    TableHeaderGroup,
    TableFooterGroup,
    TableColumnGroup,
    TableColumn,
    TableRow,
    TableCell,
}

impl LayoutDisplay {
    pub const fn debug_name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Contents => "contents",
            Self::Block => "block",
            Self::FlowRoot => "flow-root",
            Self::Inline => "inline",
            Self::InlineBlock => "inline-block",
            Self::Flex => "flex",
            Self::InlineFlex => "inline-flex",
            Self::Grid => "grid",
            Self::InlineGrid => "inline-grid",
            Self::BlockListItem => "block-list-item",
            Self::InlineListItem => "inline-list-item",
            Self::Table => "table",
            Self::InlineTable => "inline-table",
            Self::TableCaption => "table-caption",
            Self::TableRowGroup => "table-row-group",
            Self::TableHeaderGroup => "table-header-group",
            Self::TableFooterGroup => "table-footer-group",
            Self::TableColumnGroup => "table-column-group",
            Self::TableColumn => "table-column",
            Self::TableRow => "table-row",
            Self::TableCell => "table-cell",
        }
    }

    pub const fn is_inline_level(self) -> bool {
        matches!(
            self,
            Self::Inline
                | Self::InlineBlock
                | Self::InlineFlex
                | Self::InlineGrid
                | Self::InlineListItem
                | Self::InlineTable
        )
    }

    pub(crate) const fn is_inline_flow(self) -> bool {
        matches!(self, Self::Inline | Self::InlineListItem)
    }

    pub(crate) const fn is_flex_container(self) -> bool {
        matches!(self, Self::Flex | Self::InlineFlex)
    }

    pub(crate) const fn is_grid_container(self) -> bool {
        matches!(self, Self::Grid | Self::InlineGrid)
    }

    pub(crate) const fn is_list_item(self) -> bool {
        matches!(self, Self::BlockListItem | Self::InlineListItem)
    }

    pub(crate) const fn is_table(self) -> bool {
        matches!(
            self,
            Self::Table
                | Self::InlineTable
                | Self::TableCaption
                | Self::TableRowGroup
                | Self::TableHeaderGroup
                | Self::TableFooterGroup
                | Self::TableColumnGroup
                | Self::TableColumn
                | Self::TableRow
                | Self::TableCell
        )
    }
}

/// Exact CSS positioning mode retained beside Taffy's reduced two-state model.
///
/// Taffy 0.12 represents both CSS `static` and `relative` as
/// [`taffy::Position::Relative`], and both `absolute` and `fixed` as
/// [`taffy::Position::Absolute`]. Layout must retain the browser-level value so
/// containing-block selection does not accidentally use the direct box parent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutPosition {
    #[default]
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

impl LayoutPosition {
    pub const fn debug_name(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Relative => "relative",
            Self::Absolute => "absolute",
            Self::Fixed => "fixed",
            Self::Sticky => "sticky",
        }
    }

    pub(crate) const fn is_positioned(self) -> bool {
        !matches!(self, Self::Static)
    }

    pub(crate) const fn is_absolute(self) -> bool {
        matches!(self, Self::Absolute)
    }

    pub(crate) const fn is_fixed(self) -> bool {
        matches!(self, Self::Fixed)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GeneratedContent {
    Normal,
    None,
    Items {
        text: Arc<str>,
        has_unsupported_items: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ResolvedLayoutTransform {
    pub(crate) transform: LayoutTransform2D,
    pub(crate) has_unsupported_3d: bool,
    pub(crate) establishes_property_space: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TableLayoutPreference {
    #[default]
    Automatic,
    Fixed,
}

impl ResolvedLayoutTransform {
    pub(crate) const IDENTITY: Self = Self {
        transform: LayoutTransform2D::IDENTITY,
        has_unsupported_3d: false,
        establishes_property_space: false,
    };
}

/// Owned style input for one pass-local layout box.
///
/// `computed` deliberately stays alive beside the converted Taffy style. A
/// Taffy calc value can contain a pointer into the Stylo value, so dropping the
/// `ComputedValues` before the world would make otherwise-owned Taffy data
/// invalid.
#[derive(Clone)]
pub struct ResolvedLayoutStyle {
    pub(crate) computed: Option<ServoArc<ComputedValues>>,
    pub(crate) taffy: Style<Atom>,
    preferred_aspect_ratio: PreferredAspectRatio,
    display: LayoutDisplay,
    background_color: PaintColor,
    border_colors: PaintBorderColors,
    generated_content: GeneratedContent,
    font_size: f32,
    line_height: f32,
    /// Blink expands `line-height: normal` using every font actually selected
    /// during shaping. Explicit line heights keep using the primary strut.
    include_used_font_metrics: bool,
    text_color: PaintColor,
    white_space_collapse: InlineWhiteSpaceCollapse,
    text_transform: InlineTextTransform,
    text_align: parley::Alignment,
    direction: InlineDirection,
    writing_mode: taffy::WritingMode,
    text_orientation: InlineTextOrientation,
    unicode_bidi: InlineUnicodeBidi,
    vertical_align: InlineVerticalAlign,
    text_projection_deferred: bool,
    overflow_clips: bool,
    out_of_flow: bool,
    position: LayoutPosition,
    sticky_inset: taffy::Rect<taffy::LengthPercentageAuto>,
    establishes_transform_containing_block: bool,
    synthetic_transform: Option<LayoutTransform2D>,
    visibility: LayoutVisibility,
    pointer_events: bool,
    order: i32,
    anchor_sizing_deferred: bool,
    grid_template_mode_deferred: bool,
    table_layout: TableLayoutPreference,
    explicit_z_index: Option<i32>,
    opacity: f32,
    blend_mode: PaintBlendMode,
    has_filter_effect: bool,
    has_filter_containing_block_trigger: bool,
    has_clip_path: bool,
    has_mask: bool,
    isolation: bool,
    size_containment: LayoutSizeContainmentState,
    style_containment: bool,
    layout_containment: bool,
    paint_containment: bool,
    will_change_containment: bool,
    will_change_position: bool,
    will_change_stacking_context: bool,
    list_marker_type: LayoutListMarkerType,
    list_marker_position: LayoutListMarkerPosition,
}

impl std::fmt::Debug for ResolvedLayoutStyle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedLayoutStyle")
            .field("has_computed_values", &self.computed.is_some())
            .field("display", &self.display)
            .field("preferred_aspect_ratio", &self.preferred_aspect_ratio)
            .field("background_color", &self.background_color)
            .field("border_colors", &self.border_colors)
            .field("generated_content", &self.generated_content)
            .field("font_size", &self.font_size)
            .field("line_height", &self.line_height)
            .field("text_color", &self.text_color)
            .field("white_space_collapse", &self.white_space_collapse)
            .field("text_transform", &self.text_transform)
            .field("text_align", &self.text_align)
            .field("direction", &self.direction)
            .field("writing_mode", &self.writing_mode)
            .field("text_orientation", &self.text_orientation)
            .field("unicode_bidi", &self.unicode_bidi)
            .field("vertical_align", &self.vertical_align)
            .field("text_projection_deferred", &self.text_projection_deferred)
            .field("overflow_clips", &self.overflow_clips)
            .field("out_of_flow", &self.out_of_flow)
            .field("position", &self.position)
            .field(
                "establishes_transform_containing_block",
                &self.establishes_transform_containing_block,
            )
            .field(
                "has_synthetic_transform",
                &self.synthetic_transform.is_some(),
            )
            .field("visibility", &self.visibility)
            .field("pointer_events", &self.pointer_events)
            .field("order", &self.order)
            .field("anchor_sizing_deferred", &self.anchor_sizing_deferred)
            .field(
                "grid_template_mode_deferred",
                &self.grid_template_mode_deferred,
            )
            .field("table_layout", &self.table_layout)
            .field("explicit_z_index", &self.explicit_z_index)
            .field("opacity", &self.opacity)
            .field("blend_mode", &self.blend_mode)
            .field("has_filter_effect", &self.has_filter_effect)
            .field(
                "has_filter_containing_block_trigger",
                &self.has_filter_containing_block_trigger,
            )
            .field("has_clip_path", &self.has_clip_path)
            .field("has_mask", &self.has_mask)
            .field("isolation", &self.isolation)
            .field("size_containment", &self.size_containment)
            .field("style_containment", &self.style_containment)
            .field("layout_containment", &self.layout_containment)
            .field("paint_containment", &self.paint_containment)
            .field("will_change_containment", &self.will_change_containment)
            .field("will_change_position", &self.will_change_position)
            .field(
                "will_change_stacking_context",
                &self.will_change_stacking_context,
            )
            .field("list_marker_type", &self.list_marker_type)
            .field("list_marker_position", &self.list_marker_position)
            .finish_non_exhaustive()
    }
}

impl ResolvedLayoutStyle {
    /// Converts one retained Stylo style while preserving its allocation for
    /// the full lifetime of the pass-local Taffy projection.
    pub fn from_stylo(computed: ServoArc<ComputedValues>) -> Self {
        let display = classify_display(&computed);
        let background_color = stylo_background_color(&computed);
        let border_colors = stylo_border_colors(&computed);
        let generated_content = stylo_generated_content(&computed);
        let (font_size, line_height) = stylo_font_metrics(&computed);
        let include_used_font_metrics = matches!(
            computed.clone_line_height(),
            style::values::computed::font::LineHeight::Normal
        );
        let text_color = stylo_text_color(&computed);
        let white_space_collapse = match computed.clone_white_space_collapse() {
            style::computed_values::white_space_collapse::T::Collapse => {
                InlineWhiteSpaceCollapse::Collapse
            }
            style::computed_values::white_space_collapse::T::Preserve => {
                InlineWhiteSpaceCollapse::Preserve
            }
            style::computed_values::white_space_collapse::T::PreserveBreaks => {
                InlineWhiteSpaceCollapse::PreserveBreaks
            }
            style::computed_values::white_space_collapse::T::BreakSpaces => {
                InlineWhiteSpaceCollapse::BreakSpaces
            }
        };
        let text_transform_value = computed.clone_text_transform();
        let text_transform = match text_transform_value.case() {
            TextTransformCase::None => InlineTextTransform::None,
            TextTransformCase::Uppercase => InlineTextTransform::Uppercase,
            TextTransformCase::Lowercase => InlineTextTransform::Lowercase,
            TextTransformCase::Capitalize => InlineTextTransform::Capitalize,
        };
        let text_align = match computed.clone_text_align() {
            TextAlignKeyword::Start => parley::Alignment::Start,
            TextAlignKeyword::End => parley::Alignment::End,
            TextAlignKeyword::Left | TextAlignKeyword::MozLeft => parley::Alignment::Left,
            TextAlignKeyword::Right | TextAlignKeyword::MozRight => parley::Alignment::Right,
            TextAlignKeyword::Center | TextAlignKeyword::MozCenter => parley::Alignment::Center,
            TextAlignKeyword::Justify => parley::Alignment::Justify,
        };
        let direction = match computed.clone_direction() {
            StyloDirection::Ltr => InlineDirection::Ltr,
            StyloDirection::Rtl => InlineDirection::Rtl,
        };
        let writing_mode = match computed.clone_writing_mode() {
            StyloWritingMode::HorizontalTb => taffy::WritingMode::HorizontalTb,
            StyloWritingMode::VerticalRl => taffy::WritingMode::VerticalRl,
            StyloWritingMode::VerticalLr => taffy::WritingMode::VerticalLr,
            StyloWritingMode::SidewaysRl => taffy::WritingMode::SidewaysRl,
            StyloWritingMode::SidewaysLr => taffy::WritingMode::SidewaysLr,
        };
        let text_orientation = match computed.clone_text_orientation() {
            StyloTextOrientation::Mixed => InlineTextOrientation::Mixed,
            StyloTextOrientation::Upright => InlineTextOrientation::Upright,
            StyloTextOrientation::Sideways => InlineTextOrientation::Sideways,
        };
        let unicode_bidi = match computed.clone_unicode_bidi() {
            StyloUnicodeBidi::Normal => InlineUnicodeBidi::Normal,
            StyloUnicodeBidi::Embed => InlineUnicodeBidi::Embed,
            StyloUnicodeBidi::Isolate => InlineUnicodeBidi::Isolate,
            StyloUnicodeBidi::BidiOverride => InlineUnicodeBidi::BidiOverride,
            StyloUnicodeBidi::IsolateOverride => InlineUnicodeBidi::IsolateOverride,
            StyloUnicodeBidi::Plaintext => InlineUnicodeBidi::Plaintext,
        };
        let (vertical_align, vertical_align_deferred) =
            stylo_vertical_align(&computed, font_size, line_height);
        let text_projection_deferred = text_transform_value.intersects(TextTransform::FULL_WIDTH)
            || text_transform_value.intersects(TextTransform::FULL_SIZE_KANA)
            || vertical_align_deferred;
        let overflow_clips = stylo_overflow_clips(&computed);
        let stylo_position = computed.clone_position();
        let position = match stylo_position {
            StyloPosition::Static => LayoutPosition::Static,
            StyloPosition::Relative => LayoutPosition::Relative,
            StyloPosition::Absolute => LayoutPosition::Absolute,
            StyloPosition::Fixed => LayoutPosition::Fixed,
            StyloPosition::Sticky => LayoutPosition::Sticky,
        };
        let position_style = computed.get_position();
        let grid_template_mode_deferred = [
            &position_style.grid_template_rows,
            &position_style.grid_template_columns,
        ]
        .into_iter()
        .any(|template| {
            matches!(
                template,
                GenericGridTemplateComponent::Subgrid(_) | GenericGridTemplateComponent::Masonry
            )
        });
        let table_layout =
            if computed.clone_table_layout() == style::computed_values::table_layout::T::Fixed {
                TableLayoutPreference::Fixed
            } else {
                TableLayoutPreference::Automatic
            };
        let out_of_flow = !matches!(
            stylo_position,
            StyloPosition::Static | StyloPosition::Relative | StyloPosition::Sticky
        ) || computed.clone_float() != Float::None;
        let z_index = computed.clone_z_index();
        let explicit_z_index = (!z_index.is_auto()).then(|| z_index.integer_or(0));
        let opacity = computed.clone_opacity().clamp(0.0, 1.0);
        let blend_mode = stylo_blend_mode(computed.clone_mix_blend_mode());
        let effects = computed.get_effects();
        let has_filter_effect =
            !effects.filter.0.is_empty() || !effects.backdrop_filter.0.is_empty();
        let has_clip_path = !matches!(computed.clone_clip_path(), StyloClipPath::None);
        let has_mask = computed
            .get_svg()
            .mask_image
            .0
            .iter()
            .any(|image| !matches!(image, GenericImage::None));
        let isolation = computed.clone_isolation() == StyloIsolation::Isolate;
        let contain = computed.clone_contain();
        let stylo_content_visibility = computed.clone_content_visibility();
        let content_visibility = match stylo_content_visibility {
            StyloContentVisibility::Visible => LayoutContentVisibility::Visible,
            StyloContentVisibility::Hidden => LayoutContentVisibility::Hidden,
            StyloContentVisibility::Auto => LayoutContentVisibility::Auto,
        };
        // Blink's EffectiveContainment folds authored containment,
        // container-type, and content-visibility:hidden into the size axes.
        // `content-visibility:auto` only adds size containment while the
        // browser's display-lock policy actively skips the subtree.
        let container_type = computed.clone_container_type();
        let authored_size_containment_axes = LogicalSize {
            inline_size: contain.contains(style::values::computed::Contain::INLINE_SIZE)
                || container_type.intersects(ContainerType::INLINE_SIZE)
                || container_type.intersects(ContainerType::SIZE),
            block_size: contain.contains(style::values::computed::Contain::BLOCK_SIZE)
                || container_type.intersects(ContainerType::SIZE),
        };
        let (contain_intrinsic_width_fallback, contain_intrinsic_width_auto) =
            contain_intrinsic_component(&computed.clone_contain_intrinsic_width());
        let (contain_intrinsic_height_fallback, contain_intrinsic_height_auto) =
            contain_intrinsic_component(&computed.clone_contain_intrinsic_height());
        let contain_intrinsic_fallback = Size {
            width: contain_intrinsic_width_fallback,
            height: contain_intrinsic_height_fallback,
        };
        let contain_intrinsic_auto_axes = Size {
            width: contain_intrinsic_width_auto,
            height: contain_intrinsic_height_auto,
        };
        let size_containment = LayoutSizeContainmentState::new(
            authored_size_containment_axes,
            contain_intrinsic_fallback,
            contain_intrinsic_auto_axes,
            content_visibility,
            writing_mode,
            computed.effective_zoom.value(),
        );
        let style_containment = contain.contains(style::values::computed::Contain::STYLE);
        let mut layout_containment = contain.contains(style::values::computed::Contain::LAYOUT);
        let mut paint_containment = contain.contains(style::values::computed::Contain::PAINT);
        if stylo_content_visibility != StyloContentVisibility::Visible {
            // Blink folds `content-visibility:auto/hidden` into effective
            // layout and paint containment before asking the LayoutObject
            // whether containment applies to its principal box. Standalone
            // Stylo only performs that adjustment for its Gecko embedding, so
            // Moli projects the same effective bits at this browser seam.
            layout_containment = true;
            paint_containment = true;
        }
        let will_change = computed.clone_will_change().bits;
        let will_change_containment = will_change.contains(WillChangeBits::CONTAIN);
        let will_change_position = will_change.contains(WillChangeBits::POSITION);
        // Stylo folds `will-change: filter/backdrop-filter` into the same
        // non-SVG fixed-position containing-block hint used by Gecko. Blink's
        // `HasNonInitialFilter*()` exposes the equivalent union of an actual
        // effect and that hint.
        let has_filter_containing_block_trigger =
            has_filter_effect || will_change.contains(WillChangeBits::FIXPOS_CB_NON_SVG);
        let will_change_stacking_context = will_change.intersects(
            WillChangeBits::STACKING_CONTEXT_UNCONDITIONAL
                | WillChangeBits::TRANSFORM
                | WillChangeBits::CONTAIN
                | WillChangeBits::OPACITY
                | WillChangeBits::PERSPECTIVE
                | WillChangeBits::Z_INDEX
                | WillChangeBits::POSITION
                | WillChangeBits::VIEW_TRANSITION_NAME,
        );
        let order = computed.clone_order();
        let list_marker_type = stylo_list_marker_type(&computed);
        let list_marker_position = match computed.clone_list_style_position() {
            style::computed_values::list_style_position::T::Inside => {
                LayoutListMarkerPosition::Inside
            }
            style::computed_values::list_style_position::T::Outside => {
                LayoutListMarkerPosition::Outside
            }
        };
        let specified_aspect_ratio = match position_style.aspect_ratio.ratio {
            PreferredRatio::None => None,
            PreferredRatio::Ratio(ratio) => Some(ratio.0.0 / ratio.1.0),
        };
        let preferred_aspect_ratio = PreferredAspectRatio::from_components(
            position_style.aspect_ratio.auto,
            specified_aspect_ratio,
        );
        let mut taffy = stylo_taffy::to_taffy_style(&computed);
        // Alignment keywords are layout protocol, not merely numeric style.
        // Keep their lossless projection at Moli's browser boundary: the
        // pinned generic converter predates Taffy's first/last-baseline model.
        // Layout algorithms, rather than style conversion, choose the
        // context-dependent fallback or baseline-sharing group.
        taffy.align_content = taffy_content_alignment(position_style.align_content);
        taffy.justify_content = taffy_justify_content(
            position_style.justify_content,
            position_style.flex_direction,
            computed.clone_direction(),
        );
        taffy.align_items = taffy_item_alignment(position_style.align_items.0);
        taffy.align_self = taffy_item_alignment(position_style.align_self.0);
        taffy.justify_items = taffy_item_alignment((position_style.justify_items.computed.0).0);
        taffy.justify_self = taffy_item_alignment(position_style.justify_self.0);
        let size = Size {
            width: project_taffy_size_value(&position_style.width, taffy.size.width),
            height: project_taffy_size_value(&position_style.height, taffy.size.height),
        };
        let min_size = Size {
            width: project_taffy_size_value(&position_style.min_width, taffy.min_size.width),
            height: project_taffy_size_value(&position_style.min_height, taffy.min_size.height),
        };
        let max_size = Size {
            width: project_taffy_max_size_dimension(
                &position_style.max_width,
                taffy.max_size.width,
            ),
            height: project_taffy_max_size_dimension(
                &position_style.max_height,
                taffy.max_size.height,
            ),
        };
        let flex_basis = project_taffy_flex_basis(&position_style.flex_basis, taffy.flex_basis);
        let anchor_sizing_deferred = [
            size.width,
            size.height,
            min_size.width,
            min_size.height,
            max_size.width,
            max_size.height,
            flex_basis,
        ]
        .into_iter()
        .any(|projection| projection.anchor_sizing_deferred);
        taffy.size = size.map(|projection| projection.dimension);
        taffy.min_size = min_size.map(|projection| projection.dimension);
        taffy.max_size = max_size.map(|projection| projection.dimension);
        taffy.flex_basis = flex_basis.dimension;
        // Taffy's generic leaf algorithm transfers aspect ratios before a
        // replaced-element measure callback runs. CSS Sizing 4 defines zero,
        // infinite and NaN ratios as degenerate, so normalize them at the
        // Stylo/Taffy seam instead of relying only on replaced measurement.
        taffy.aspect_ratio = preferred_aspect_ratio.numeric_ratio();
        taffy.item_is_table = matches!(display, LayoutDisplay::Table | LayoutDisplay::InlineTable);
        let sticky_inset = taffy.inset;
        let establishes_transform_containing_block =
            computed.get_box().has_transform_or_perspective()
                || will_change.intersects(WillChangeBits::TRANSFORM | WillChangeBits::PERSPECTIVE);
        if matches!(display, LayoutDisplay::Block | LayoutDisplay::BlockListItem)
            && (layout_containment || paint_containment)
        {
            // Layout and paint containment establish an independent formatting
            // context. Taffy's FlowRoot keeps the computed CSS display intact
            // in `display` while selecting the non-collapsing block algorithm.
            taffy.display = TaffyDisplay::FlowRoot;
        }
        let visibility = match computed.clone_visibility() {
            StyloVisibility::Visible => LayoutVisibility::Visible,
            StyloVisibility::Hidden => LayoutVisibility::Hidden,
            StyloVisibility::Collapse => LayoutVisibility::Collapse,
        };
        let pointer_events =
            computed.clone_pointer_events() != style::computed_values::pointer_events::T::None;
        if matches!(position, LayoutPosition::Static | LayoutPosition::Sticky) {
            // `stylo_taffy` must map both CSS static and relative to Taffy's
            // single in-flow `Relative` variant. Taffy would therefore apply
            // author insets to a static box unless the browser-level adapter
            // clears them here.
            taffy.inset = taffy::Rect {
                left: taffy::LengthPercentageAuto::auto(),
                right: taffy::LengthPercentageAuto::auto(),
                top: taffy::LengthPercentageAuto::auto(),
                bottom: taffy::LengthPercentageAuto::auto(),
            };
        }
        Self {
            computed: Some(computed),
            taffy,
            preferred_aspect_ratio,
            display,
            background_color,
            border_colors,
            generated_content,
            font_size,
            line_height,
            include_used_font_metrics,
            text_color,
            white_space_collapse,
            text_transform,
            text_align,
            direction,
            writing_mode,
            text_orientation,
            unicode_bidi,
            vertical_align,
            text_projection_deferred,
            overflow_clips,
            out_of_flow,
            position,
            sticky_inset,
            establishes_transform_containing_block,
            synthetic_transform: None,
            visibility,
            pointer_events,
            order,
            anchor_sizing_deferred,
            grid_template_mode_deferred,
            table_layout,
            explicit_z_index,
            opacity,
            blend_mode,
            has_filter_effect,
            has_filter_containing_block_trigger,
            has_clip_path,
            has_mask,
            isolation,
            size_containment,
            style_containment,
            layout_containment,
            paint_containment,
            will_change_containment,
            will_change_position,
            will_change_stacking_context,
            list_marker_type,
            list_marker_position,
        }
    }

    /// Creates a deterministic style for DOM-free construction tests.
    pub fn synthetic(
        display: LayoutDisplay,
        mut taffy: Style<Atom>,
        background_color: PaintColor,
    ) -> Self {
        let direction = InlineDirection::from_taffy(taffy.direction);
        taffy.aspect_ratio = usable_aspect_ratio(taffy.aspect_ratio);
        let preferred_aspect_ratio = PreferredAspectRatio::from_taffy(taffy.aspect_ratio);
        taffy.display = taffy_display(display);
        taffy.item_is_table = matches!(display, LayoutDisplay::Table | LayoutDisplay::InlineTable);
        let overflow_clips = taffy.overflow.x != taffy::Overflow::Visible
            || taffy.overflow.y != taffy::Overflow::Visible;
        let out_of_flow = taffy.position == TaffyPosition::Absolute;
        let position = if out_of_flow {
            LayoutPosition::Absolute
        } else {
            LayoutPosition::Static
        };
        let sticky_inset = taffy.inset;
        Self {
            computed: None,
            taffy,
            preferred_aspect_ratio,
            display,
            background_color,
            border_colors: PaintBorderColors::all(PaintColor::BLACK),
            generated_content: GeneratedContent::None,
            font_size: 16.0,
            line_height: 19.2,
            include_used_font_metrics: false,
            text_color: PaintColor::BLACK,
            white_space_collapse: InlineWhiteSpaceCollapse::Collapse,
            text_transform: InlineTextTransform::None,
            text_align: parley::Alignment::Start,
            direction,
            writing_mode: taffy::WritingMode::HorizontalTb,
            text_orientation: InlineTextOrientation::Mixed,
            unicode_bidi: InlineUnicodeBidi::Normal,
            vertical_align: InlineVerticalAlign::default(),
            text_projection_deferred: false,
            overflow_clips,
            out_of_flow,
            position,
            sticky_inset,
            establishes_transform_containing_block: false,
            synthetic_transform: None,
            visibility: LayoutVisibility::Visible,
            pointer_events: true,
            order: 0,
            anchor_sizing_deferred: false,
            grid_template_mode_deferred: false,
            table_layout: TableLayoutPreference::Automatic,
            explicit_z_index: None,
            opacity: 1.0,
            blend_mode: PaintBlendMode::Normal,
            has_filter_effect: false,
            has_filter_containing_block_trigger: false,
            has_clip_path: false,
            has_mask: false,
            isolation: false,
            size_containment: LayoutSizeContainmentState::default(),
            style_containment: false,
            layout_containment: false,
            paint_containment: false,
            will_change_containment: false,
            will_change_position: false,
            will_change_stacking_context: false,
            list_marker_type: LayoutListMarkerType::Disc,
            list_marker_position: LayoutListMarkerPosition::Outside,
        }
    }

    /// Adds generated string content to a synthetic pseudo style.
    pub fn with_generated_text(mut self, text: impl Into<Arc<str>>) -> Self {
        self.generated_content = GeneratedContent::Items {
            text: text.into(),
            has_unsupported_items: false,
        };
        self
    }

    /// Models the initial `content: normal` value in construction tests.
    pub fn with_normal_generated_content(mut self) -> Self {
        self.generated_content = GeneratedContent::Normal;
        self
    }

    /// Models a legal generated-content item that Phase 1 cannot materialize yet.
    pub fn with_unsupported_generated_content(mut self) -> Self {
        self.generated_content = GeneratedContent::Items {
            text: Arc::from(""),
            has_unsupported_items: true,
        };
        self
    }

    /// Overrides deterministic text metrics used before Parley lands in P3.
    pub fn with_text_metrics(mut self, font_size: f32, line_height: f32) -> Self {
        self.font_size = font_size;
        self.line_height = line_height;
        self.include_used_font_metrics = false;
        self
    }

    /// Overrides the alignment keyword for a synthetic inline style.
    pub fn with_inline_alignment(mut self, alignment: LayoutInlineAlignment) -> Self {
        self.vertical_align.kind = alignment;
        self
    }

    /// Overrides the flow-relative axes used by deterministic layout tests.
    pub fn with_writing_mode(mut self, writing_mode: taffy::WritingMode) -> Self {
        self.writing_mode = writing_mode;
        let effective_zoom = self.effective_zoom();
        self.size_containment
            .recompute(self.writing_mode, effective_zoom);
        self
    }

    /// Selects the fixed CSS table algorithm for a synthetic style. The
    /// effective mode still depends on this style having a non-auto logical
    /// inline size, exactly as it does for a Stylo-backed style.
    pub fn with_fixed_table_layout(mut self) -> Self {
        self.table_layout = TableLayoutPreference::Fixed;
        self
    }

    /// Returns the inherited writing mode retained for layout.
    pub(crate) fn writing_mode(&self) -> taffy::WritingMode {
        self.writing_mode
    }

    /// Override only the writing direction consumed by layout.
    ///
    /// HTML may propagate the body's writing mode and direction to the root
    /// LayoutObject while CSSOM keeps the root element's computed values.
    /// Retaining `computed` and changing this pass-local projection preserves
    /// that used-style/computed-style split.
    pub(crate) fn use_layout_writing_direction(
        &mut self,
        writing_direction: taffy::WritingDirection,
    ) {
        self.writing_mode = writing_direction.mode;
        self.direction = InlineDirection::from_taffy(writing_direction.direction);
        self.taffy.direction = writing_direction.direction;
        let effective_zoom = self.effective_zoom();
        self.size_containment
            .recompute(self.writing_mode, effective_zoom);
    }

    pub(crate) fn font_baseline(&self) -> FontBaseline {
        self.text_orientation.font_baseline(self.writing_mode)
    }

    pub(crate) const fn writing_direction(&self) -> taffy::WritingDirection {
        taffy::WritingDirection::new(self.writing_mode, self.direction.to_taffy())
    }

    /// Overrides the CSS-pixel baseline shift for a synthetic inline style.
    /// Positive values raise the inline box.
    pub fn with_inline_baseline_shift(mut self, shift: f32) -> Self {
        self.vertical_align.baseline_shift = shift;
        self
    }

    /// Marks a synthetic box as removed from normal flow.
    pub fn with_out_of_flow(mut self) -> Self {
        self.out_of_flow = true;
        self.position = LayoutPosition::Absolute;
        self.taffy.position = TaffyPosition::Absolute;
        self
    }

    /// Sets float/clear for deterministic BFC and IFC tests.
    pub fn with_float(mut self, float: taffy::Float, clear: taffy::Clear) -> Self {
        self.taffy.float = float;
        self.taffy.clear = clear;
        self.out_of_flow = float != taffy::Float::None;
        self
    }

    /// Sets an exact CSS positioning mode for deterministic layout tests.
    pub fn with_position(mut self, position: LayoutPosition) -> Self {
        self.out_of_flow = matches!(position, LayoutPosition::Absolute | LayoutPosition::Fixed);
        self.position = position;
        self.sticky_inset = self.taffy.inset;
        self.taffy.position = if self.out_of_flow {
            TaffyPosition::Absolute
        } else {
            TaffyPosition::Relative
        };
        if matches!(position, LayoutPosition::Static | LayoutPosition::Sticky) {
            self.taffy.inset = taffy::Rect {
                left: taffy::LengthPercentageAuto::auto(),
                right: taffy::LengthPercentageAuto::auto(),
                top: taffy::LengthPercentageAuto::auto(),
                bottom: taffy::LengthPercentageAuto::auto(),
            };
        }
        self
    }

    /// Sets the CSS order used for flex/grid layout and paint ordering.
    pub fn with_order(mut self, order: i32) -> Self {
        self.order = order;
        self
    }

    /// Overrides marker type/position for DOM-free list geometry tests.
    pub fn with_list_marker(
        mut self,
        marker_type: LayoutListMarkerType,
        position: LayoutListMarkerPosition,
    ) -> Self {
        self.list_marker_type = marker_type;
        self.list_marker_position = position;
        self
    }

    /// Marks a synthetic box as an absolute/fixed containing block created by
    /// transform/perspective without applying transform paint.
    pub fn with_transform_containing_block(mut self) -> Self {
        self.establishes_transform_containing_block = true;
        self
    }

    /// Applies an exact pass-local 2D transform in DOM-free geometry tests.
    pub fn with_2d_transform(mut self, transform: LayoutTransform2D) -> Self {
        self.establishes_transform_containing_block = true;
        self.synthetic_transform = Some(transform);
        self
    }

    /// Applies an exact group opacity in DOM-free paint-order tests.
    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    /// Applies an exact blend mode in DOM-free paint-order tests.
    pub fn with_blend_mode(mut self, blend_mode: PaintBlendMode) -> Self {
        self.blend_mode = blend_mode;
        self
    }

    /// Applies a non-auto z-index in DOM-free stacking tests.
    pub fn with_z_index(mut self, z_index: i32) -> Self {
        self.explicit_z_index = Some(z_index);
        self
    }

    pub fn display(&self) -> LayoutDisplay {
        self.display
    }

    /// Used inner formatting context after browser layout-object adjustments.
    ///
    /// `display` retains the authored/computed outer+inner value for box-tree
    /// semantics. Native controls can select a different internal algorithm
    /// without changing that observable CSS value, just as Blink's menu-list
    /// select has computed `inline-block` display but owns a LayoutFlexibleBox.
    pub(crate) fn uses_flex_formatting_context(&self) -> bool {
        self.taffy.display == TaffyDisplay::Flex
    }

    /// Whether the box would have been inline-level before absolute/fixed
    /// positioning blockified its computed `display` value.
    ///
    /// CSS static-position rules use this hypothetical display. Stylo retains
    /// it as `original_display`, exactly as Blitz and Chromium retain the
    /// pre-blockification value for out-of-flow layout.
    pub(crate) fn hypothetical_display_is_inline_level(&self) -> bool {
        self.computed.as_ref().map_or_else(
            || self.display.is_inline_level(),
            |computed| computed.get_box().original_display.outside() == DisplayOutside::Inline,
        )
    }

    /// Returns the retained Stylo allocation backing this pass-local style.
    ///
    /// Renderer-owned anonymous box construction uses the parent computed
    /// values as the inheritance input to `Stylist::style_for_anonymous`.
    /// Synthetic construction tests deliberately return `None` here.
    pub fn stylo_computed_values(&self) -> Option<&ServoArc<ComputedValues>> {
        self.computed.as_ref()
    }

    pub fn background_color(&self) -> PaintColor {
        self.background_color
    }

    pub(crate) fn border_colors(&self) -> PaintBorderColors {
        self.border_colors
    }

    pub(crate) fn border_styles(&self) -> PaintBorderStyles {
        self.computed.as_ref().map_or_else(
            || PaintBorderStyles::all(PaintBorderStyle::Solid),
            |computed| {
                let border = computed.get_border();
                PaintBorderStyles {
                    top: paint_border_style(border.border_top_style),
                    right: paint_border_style(border.border_right_style),
                    bottom: paint_border_style(border.border_bottom_style),
                    left: paint_border_style(border.border_left_style),
                }
            },
        )
    }

    pub(crate) fn border_radii(&self, width: f32, height: f32) -> PaintCornerRadii {
        let Some(computed) = self.computed.as_ref() else {
            return PaintCornerRadii::ZERO;
        };
        let width = CSSPixelLength::new(width.max(0.0));
        let height = CSSPixelLength::new(height.max(0.0));
        let resolve = |radius: &style::values::computed::BorderCornerRadius| {
            PaintCornerRadius::new(
                radius.0.width.0.resolve(width).px().max(0.0),
                radius.0.height.0.resolve(height).px().max(0.0),
            )
        };
        let border = computed.get_border();
        PaintCornerRadii {
            top_left: resolve(&border.border_top_left_radius),
            top_right: resolve(&border.border_top_right_radius),
            bottom_right: resolve(&border.border_bottom_right_radius),
            bottom_left: resolve(&border.border_bottom_left_radius),
        }
    }

    pub(crate) fn box_shadows(
        &self,
        rect: LayoutRect,
        radii: PaintCornerRadii,
        transform: LayoutTransform2D,
    ) -> Vec<PaintBoxShadow> {
        let Some(computed) = self.computed.as_ref() else {
            return Vec::new();
        };
        let current_color = computed.clone_color();
        computed
            .get_effects()
            .box_shadow
            .0
            .iter()
            .map(|shadow| PaintBoxShadow {
                rect,
                radii,
                color: absolute_paint_color(shadow.base.color.resolve_to_absolute(&current_color)),
                offset: LayoutPoint::new(shadow.base.horizontal.px(), shadow.base.vertical.px()),
                blur_radius: shadow.base.blur.px().max(0.0),
                spread_radius: shadow.spread.px(),
                inset: shadow.inset,
                transform,
            })
            .collect()
    }

    pub(crate) fn outline_fragment(
        &self,
        rect: LayoutRect,
        radii: PaintCornerRadii,
        transform: LayoutTransform2D,
    ) -> Option<PaintFragment> {
        let computed = self.computed.as_ref()?;
        let outline = computed.get_outline();
        let style = match outline.outline_style {
            StyloOutlineStyle::Auto => PaintBorderStyle::Solid,
            StyloOutlineStyle::BorderStyle(style) => paint_border_style(style),
        };
        if matches!(style, PaintBorderStyle::None | PaintBorderStyle::Hidden) {
            return None;
        }
        let width = outline.outline_width.0.to_f32_px().max(0.0);
        if width <= 0.0 {
            return None;
        }
        let offset = outline.outline_offset.to_f32_px();
        let outset = offset + width;
        let outline_rect = LayoutRect::new(
            rect.x - outset,
            rect.y - outset,
            (rect.width + outset * 2.0).max(0.0),
            (rect.height + outset * 2.0).max(0.0),
        );
        let outset_radius = |radius: PaintCornerRadius| {
            PaintCornerRadius::new((radius.x + outset).max(0.0), (radius.y + outset).max(0.0))
        };
        let radii = PaintCornerRadii {
            top_left: outset_radius(radii.top_left),
            top_right: outset_radius(radii.top_right),
            bottom_right: outset_radius(radii.bottom_right),
            bottom_left: outset_radius(radii.bottom_left),
        };
        let color = absolute_paint_color(
            outline
                .outline_color
                .resolve_to_absolute(&computed.clone_color()),
        );
        Some(PaintFragment::Border {
            rect: outline_rect,
            widths: PaintEdgeSizes::new(width, width, width, width),
            colors: PaintBorderColors::all(color),
            styles: PaintBorderStyles::all(style),
            radii,
            transform,
        })
    }

    pub fn generated_text(&self) -> Option<&str> {
        match &self.generated_content {
            GeneratedContent::Items { text, .. } => Some(text),
            GeneratedContent::Normal | GeneratedContent::None => None,
        }
    }

    pub(crate) fn generates_pseudo_box(&self, marker: bool) -> bool {
        match self.generated_content {
            GeneratedContent::Normal => marker,
            GeneratedContent::None => false,
            GeneratedContent::Items { .. } => true,
        }
    }

    pub(crate) fn has_unsupported_generated_content(&self) -> bool {
        matches!(
            self.generated_content,
            GeneratedContent::Items {
                has_unsupported_items: true,
                ..
            }
        )
    }

    pub fn is_out_of_flow(&self) -> bool {
        self.out_of_flow
    }

    pub fn position(&self) -> LayoutPosition {
        self.position
    }

    pub(crate) fn establishes_positioned_containing_block(
        &self,
        is_document_element: bool,
        is_css_box: bool,
        containment_eligible: bool,
    ) -> bool {
        self.position.is_positioned()
            || self.will_change_position
            || self.establishes_fixed_containing_block(
                is_document_element,
                is_css_box,
                containment_eligible,
            )
    }

    pub(crate) fn establishes_fixed_containing_block(
        &self,
        is_document_element: bool,
        is_css_box: bool,
        containment_eligible: bool,
    ) -> bool {
        // Filter effects apply to inline LayoutObjects too, unlike transforms,
        // but Filter Effects explicitly excludes the document element.
        (!is_document_element && self.has_filter_containing_block_trigger)
            || (is_css_box && self.establishes_transform_containing_block)
            || (containment_eligible
                && (self.layout_containment
                    || self.paint_containment
                    || self.will_change_containment))
    }

    pub(crate) const fn applies_paint_containment(&self) -> bool {
        self.paint_containment
    }

    pub(crate) const fn applies_layout_containment(&self) -> bool {
        self.layout_containment
    }

    pub(crate) const fn applies_style_containment(&self) -> bool {
        self.style_containment
    }

    /// Resolves the browser-owned display-lock decision for this layout epoch.
    ///
    /// `content-visibility:hidden` always skips. `auto_contents_skipped` is the
    /// viewport/display-lock decision for `content-visibility:auto`; it is
    /// ignored for other computed values. Remembered sizes are logical,
    /// unzoomed CSS pixels and are selected only while contents are skipped.
    pub fn resolve_content_visibility_state(
        &mut self,
        auto_contents_skipped: bool,
        remembered: LayoutLastRememberedSize,
    ) {
        let effective_zoom = self.effective_zoom();
        self.size_containment.resolve_browser_state(
            auto_contents_skipped,
            remembered,
            self.writing_mode,
            effective_zoom,
        );
    }

    /// Returns the axis policy for the document's intrinsic-size observer.
    /// `content-visibility:auto` makes both axes stateful, matching Blink's
    /// computed-style adjustment, even while the current epoch stays visible.
    pub fn last_remembered_size_policy(&self) -> LayoutLastRememberedSizePolicy {
        self.size_containment.observer_policy(self.writing_mode)
    }

    /// Whether this computed style requests the viewport-driven display-lock
    /// mode. Principal-box eligibility is resolved later by box construction.
    pub fn content_visibility_is_auto(&self) -> bool {
        self.size_containment.content_visibility == LayoutContentVisibility::Auto
    }

    pub(crate) const fn content_visibility_skips_contents(&self) -> bool {
        self.size_containment.contents_skipped
    }

    pub(crate) const fn size_containment(&self) -> SizeContainment {
        self.size_containment.used
    }

    /// Returns the accumulated CSS `zoom` applied to this box's layout values.
    ///
    /// Stylo has already applied this factor to computed CSS lengths. Layout,
    /// paint, and client rects retain that zoomed space. Browser resources are
    /// scaled into it on import, while CSSOM integer box and scroll metrics
    /// remove the factor when they are published. Synthetic styles have no
    /// Stylo value and therefore use the initial factor.
    pub(crate) fn effective_zoom(&self) -> f32 {
        self.computed
            .as_ref()
            .map_or(1.0, |computed| computed.effective_zoom.value())
    }

    pub(crate) const fn is_visible(&self) -> bool {
        matches!(self.visibility, LayoutVisibility::Visible)
    }

    pub(crate) const fn visibility_is_collapsed(&self) -> bool {
        matches!(self.visibility, LayoutVisibility::Collapse)
    }

    /// Returns the computed CSS `color` sampled for this pass.
    ///
    /// Besides text paint this is the inherited `currentColor` input for
    /// atomic resources such as an inline SVG replaced element.
    pub const fn current_color(&self) -> PaintColor {
        self.text_color
    }

    pub(crate) const fn text_color(&self) -> PaintColor {
        self.current_color()
    }

    pub(crate) const fn accepts_pointer_events(&self) -> bool {
        self.pointer_events
    }

    pub(crate) fn resolved_2d_transform(&self, width: f32, height: f32) -> ResolvedLayoutTransform {
        if let Some(transform) = self.synthetic_transform {
            return ResolvedLayoutTransform {
                transform,
                has_unsupported_3d: false,
                establishes_property_space: true,
            };
        }
        let mut resolved = self
            .computed
            .as_ref()
            .map_or(ResolvedLayoutTransform::IDENTITY, |computed| {
                resolve_stylo_2d_transform(computed.get_box(), width, height)
            });
        resolved.establishes_property_space = self.establishes_transform_containing_block;
        resolved
    }

    pub(crate) fn is_absolute_positioned(&self) -> bool {
        self.position.is_absolute()
    }

    pub(crate) fn is_fixed_positioned(&self) -> bool {
        self.position.is_fixed()
    }

    pub(crate) const fn order(&self) -> i32 {
        self.order
    }

    pub(crate) const fn explicit_z_index(&self) -> Option<i32> {
        self.explicit_z_index
    }

    pub(crate) const fn opacity(&self) -> f32 {
        self.opacity
    }

    pub(crate) const fn blend_mode(&self) -> PaintBlendMode {
        self.blend_mode
    }

    pub(crate) fn creates_stacking_context(
        &self,
        is_root: bool,
        is_flex_or_grid_item: bool,
        containment_eligible: bool,
    ) -> bool {
        if is_root
            || self.opacity < 1.0
            || self.blend_mode != PaintBlendMode::Normal
            || self.establishes_transform_containing_block
            || self.has_filter_effect
            || self.has_clip_path
            || self.has_mask
            || self.isolation
            || self.will_change_stacking_context
            || (containment_eligible && (self.layout_containment || self.paint_containment))
        {
            return true;
        }
        match self.position {
            LayoutPosition::Fixed | LayoutPosition::Sticky => true,
            LayoutPosition::Relative | LayoutPosition::Absolute => self.explicit_z_index.is_some(),
            LayoutPosition::Static => is_flex_or_grid_item && self.explicit_z_index.is_some(),
        }
    }

    pub(crate) fn list_marker_type(&self) -> &LayoutListMarkerType {
        &self.list_marker_type
    }

    pub(crate) const fn list_marker_position(&self) -> LayoutListMarkerPosition {
        self.list_marker_position
    }

    pub(crate) fn table_layout_is_fixed(&self) -> bool {
        if self.table_layout != TableLayoutPreference::Fixed {
            return false;
        }

        // CSS Tables applies the fixed algorithm only when the table has a
        // non-auto logical width. Blink additionally treats max-content as
        // automatic under the stable (non TableIsAutoFixedLayout) behavior.
        let logical_width = self.writing_mode.to_logical(self.taffy.size).inline_size;
        !logical_width.is_auto() && !logical_width.is_max_content()
    }

    pub(crate) fn table_border_is_collapsed(&self) -> bool {
        self.computed.as_ref().is_some_and(|computed| {
            computed.clone_border_collapse() == style::computed_values::border_collapse::T::Collapse
        })
    }

    pub(crate) fn table_border_spacing(&self) -> Size<f32> {
        self.computed.as_ref().map_or(Size::ZERO, |computed| {
            let spacing = computed.clone_border_spacing().0;
            Size {
                width: spacing.width.px(),
                height: spacing.height.px(),
            }
        })
    }

    pub(crate) fn caption_is_bottom(&self) -> bool {
        self.computed.as_ref().is_some_and(|computed| {
            computed.clone_caption_side() == style::values::computed::table::CaptionSide::Bottom
        })
    }

    pub(crate) fn is_floated(&self) -> bool {
        self.taffy.float != taffy::Float::None
    }

    pub(crate) fn has_deferred_anchor_sizing(&self) -> bool {
        self.anchor_sizing_deferred
    }

    pub(crate) fn has_deferred_grid_template_mode(&self) -> bool {
        self.grid_template_mode_deferred
    }

    pub(crate) fn has_auto_inset_axis(&self) -> bool {
        (self.taffy.inset.left.is_auto() && self.taffy.inset.right.is_auto())
            || (self.taffy.inset.top.is_auto() && self.taffy.inset.bottom.is_auto())
    }

    pub(crate) const fn sticky_inset(&self) -> taffy::Rect<taffy::LengthPercentageAuto> {
        self.sticky_inset
    }

    /// Applies an HTML-level `display: none` override such as the `hidden`
    /// attribute after Stylo has produced the underlying computed values.
    pub fn force_display_none(&mut self) {
        self.display = LayoutDisplay::None;
        self.taffy.display = TaffyDisplay::None;
    }

    /// Assigns the structural display selected by box construction.
    ///
    /// Stylo's anonymous-box pseudos provide inherited and UA values, while
    /// the builder owns the exact anonymous role (for example row-group,
    /// flex text item, or grid text item). Keep those two responsibilities
    /// separate instead of manufacturing a retained computed style.
    pub fn force_layout_display(&mut self, display: LayoutDisplay) {
        self.display = display;
        self.taffy.display = taffy_display(display);
    }

    /// Returns the computed CSS font size sampled for this pass.
    ///
    /// Atomic document resources such as inline SVG use this as the inherited
    /// context for relative lengths without retaining the Stylo allocation.
    pub const fn font_size(&self) -> f32 {
        self.font_size
    }

    pub(crate) fn line_height(&self) -> f32 {
        self.line_height
    }

    pub(crate) fn includes_used_font_metrics(&self) -> bool {
        self.include_used_font_metrics
    }

    pub(crate) fn white_space_collapse(&self) -> InlineWhiteSpaceCollapse {
        self.white_space_collapse
    }

    pub(crate) fn text_transform(&self) -> InlineTextTransform {
        self.text_transform
    }

    /// Resolves CSS flow-relative alignment before crossing into Parley.
    ///
    /// Parley 0.10 derives `start` and `end` from the first strong character
    /// in the text. CSS derives them from the containing block's `direction`,
    /// including for empty, numeric, and neutral-only lines. Physical
    /// alignment is therefore part of this browser-owned style seam.
    pub(crate) fn resolved_text_align(&self) -> parley::Alignment {
        match (self.text_align, self.direction) {
            (parley::Alignment::Start, InlineDirection::Ltr)
            | (parley::Alignment::End, InlineDirection::Rtl) => parley::Alignment::Left,
            (parley::Alignment::Start, InlineDirection::Rtl)
            | (parley::Alignment::End, InlineDirection::Ltr) => parley::Alignment::Right,
            (alignment, _) => alignment,
        }
    }

    pub(crate) fn direction(&self) -> InlineDirection {
        self.direction
    }

    pub(crate) fn unicode_bidi(&self) -> InlineUnicodeBidi {
        self.unicode_bidi
    }

    pub(crate) fn vertical_align(&self) -> InlineVerticalAlign {
        self.vertical_align
    }

    pub(crate) fn parley_text_style(
        &self,
    ) -> parley::TextStyle<'static, 'static, crate::stylo_to_parley::TextBrush> {
        if let Some(computed) = self.computed.as_ref() {
            return crate::stylo_to_parley::text_style(computed);
        }
        parley::TextStyle {
            font_size: self.font_size,
            line_height: parley::LineHeight::Absolute(self.line_height),
            brush: crate::stylo_to_parley::TextBrush {
                color: self.text_color,
                paint: true,
                synthetic_bold: false,
                decoration: crate::stylo_to_parley::TextDecorationBrush::default(),
                shadows: std::sync::Arc::from([]),
            },
            ..parley::TextStyle::default()
        }
    }

    pub(crate) fn text_indent(&self, containing_width: f32) -> (f32, parley::IndentOptions) {
        let Some(computed) = self.computed.as_ref() else {
            return (0.0, parley::IndentOptions::default());
        };
        let indent = computed.clone_text_indent();
        (
            indent
                .length
                .resolve(CSSPixelLength::new(containing_width.max(0.0)))
                .px(),
            parley::IndentOptions {
                each_line: indent.each_line,
                hanging: indent.hanging,
            },
        )
    }

    pub(crate) fn has_deferred_text_projection(&self) -> bool {
        self.text_projection_deferred
    }

    pub(crate) fn clips_overflow(&self) -> bool {
        self.overflow_clips
    }

    pub(crate) fn establishes_scroll_container(&self) -> bool {
        // `overflow: clip` clips paint but deliberately does not create the
        // scrolling mechanism that selects a sticky scrollport.
        [self.taffy.overflow.x, self.taffy.overflow.y]
            .into_iter()
            .any(|overflow| matches!(overflow, taffy::Overflow::Hidden | taffy::Overflow::Scroll))
    }

    pub(crate) fn allows_user_scroll_x(&self) -> bool {
        self.taffy.overflow.x == taffy::Overflow::Scroll
    }

    pub(crate) fn allows_user_scroll_y(&self) -> bool {
        self.taffy.overflow.y == taffy::Overflow::Scroll
    }

    /// The viewport treats propagated `overflow: visible` as an automatic
    /// scrolling mechanism. `hidden` and `clip` still permit programmatic
    /// scrolling, but they suppress user-initiated scrolling.
    pub(crate) fn allows_viewport_user_scroll_x(&self) -> bool {
        matches!(
            self.taffy.overflow.x,
            taffy::Overflow::Visible | taffy::Overflow::Scroll
        )
    }

    pub(crate) fn allows_viewport_user_scroll_y(&self) -> bool {
        matches!(
            self.taffy.overflow.y,
            taffy::Overflow::Visible | taffy::Overflow::Scroll
        )
    }

    pub(crate) fn text_leaf_from(parent: &Self) -> Self {
        Self {
            computed: parent.computed.clone(),
            taffy: Style {
                display: TaffyDisplay::Block,
                ..Style::default()
            },
            preferred_aspect_ratio: PreferredAspectRatio::Auto,
            display: LayoutDisplay::Inline,
            background_color: PaintColor::TRANSPARENT,
            border_colors: PaintBorderColors::default(),
            generated_content: GeneratedContent::None,
            font_size: parent.font_size,
            line_height: parent.line_height,
            include_used_font_metrics: parent.include_used_font_metrics,
            text_color: parent.text_color,
            white_space_collapse: parent.white_space_collapse,
            text_transform: parent.text_transform,
            text_align: parent.text_align,
            direction: parent.direction,
            writing_mode: parent.writing_mode,
            text_orientation: parent.text_orientation,
            unicode_bidi: InlineUnicodeBidi::Normal,
            vertical_align: parent.vertical_align,
            text_projection_deferred: parent.text_projection_deferred,
            overflow_clips: false,
            out_of_flow: false,
            position: LayoutPosition::Static,
            sticky_inset: taffy::Rect {
                left: taffy::LengthPercentageAuto::auto(),
                right: taffy::LengthPercentageAuto::auto(),
                top: taffy::LengthPercentageAuto::auto(),
                bottom: taffy::LengthPercentageAuto::auto(),
            },
            establishes_transform_containing_block: false,
            synthetic_transform: None,
            visibility: parent.visibility,
            pointer_events: parent.pointer_events,
            order: 0,
            anchor_sizing_deferred: false,
            grid_template_mode_deferred: false,
            table_layout: TableLayoutPreference::Automatic,
            explicit_z_index: None,
            opacity: 1.0,
            blend_mode: PaintBlendMode::Normal,
            has_filter_effect: false,
            has_filter_containing_block_trigger: false,
            has_clip_path: false,
            has_mask: false,
            isolation: false,
            size_containment: LayoutSizeContainmentState::default(),
            style_containment: false,
            layout_containment: false,
            paint_containment: false,
            will_change_containment: false,
            will_change_position: false,
            will_change_stacking_context: false,
            list_marker_type: parent.list_marker_type.clone(),
            list_marker_position: parent.list_marker_position,
        }
    }

    /// Derives a deterministic anonymous style when a resolver has no retained
    /// Stylo allocation (primarily DOM-free tests and conservative fallback).
    pub fn anonymous_from(parent: &Self, display: LayoutDisplay) -> Self {
        Self {
            computed: parent.computed.clone(),
            taffy: Style {
                display: taffy_display(display),
                ..Style::default()
            },
            preferred_aspect_ratio: PreferredAspectRatio::Auto,
            display,
            background_color: PaintColor::TRANSPARENT,
            border_colors: PaintBorderColors::default(),
            generated_content: GeneratedContent::None,
            font_size: parent.font_size,
            line_height: parent.line_height,
            include_used_font_metrics: parent.include_used_font_metrics,
            text_color: parent.text_color,
            white_space_collapse: parent.white_space_collapse,
            text_transform: parent.text_transform,
            text_align: parent.text_align,
            direction: parent.direction,
            writing_mode: parent.writing_mode,
            text_orientation: parent.text_orientation,
            unicode_bidi: InlineUnicodeBidi::Normal,
            vertical_align: parent.vertical_align,
            text_projection_deferred: parent.text_projection_deferred,
            overflow_clips: false,
            out_of_flow: false,
            position: LayoutPosition::Static,
            sticky_inset: taffy::Rect {
                left: taffy::LengthPercentageAuto::auto(),
                right: taffy::LengthPercentageAuto::auto(),
                top: taffy::LengthPercentageAuto::auto(),
                bottom: taffy::LengthPercentageAuto::auto(),
            },
            establishes_transform_containing_block: false,
            synthetic_transform: None,
            visibility: parent.visibility,
            pointer_events: parent.pointer_events,
            order: 0,
            anchor_sizing_deferred: false,
            grid_template_mode_deferred: false,
            table_layout: TableLayoutPreference::Automatic,
            explicit_z_index: None,
            opacity: 1.0,
            blend_mode: PaintBlendMode::Normal,
            has_filter_effect: false,
            has_filter_containing_block_trigger: false,
            has_clip_path: false,
            has_mask: false,
            isolation: false,
            size_containment: LayoutSizeContainmentState::default(),
            style_containment: false,
            layout_containment: false,
            paint_containment: false,
            will_change_containment: false,
            will_change_position: false,
            will_change_stacking_context: false,
            list_marker_type: parent.list_marker_type.clone(),
            list_marker_position: parent.list_marker_position,
        }
    }

    pub(crate) fn blockify_for_item(&mut self) {
        if self.display.is_inline_level() {
            self.taffy.display = match self.display {
                LayoutDisplay::InlineBlock => TaffyDisplay::FlowRoot,
                LayoutDisplay::InlineFlex => TaffyDisplay::Flex,
                LayoutDisplay::InlineGrid => TaffyDisplay::Grid,
                _ => TaffyDisplay::Block,
            };
        }
    }

    pub(crate) fn mark_replaced(&mut self) {
        self.taffy.item_is_replaced = true;
    }

    /// Applies element-content-dependent used-style rules at the box-tree
    /// construction boundary.
    ///
    /// A terminally unavailable HTML image is rebuilt as fallback flow
    /// content, not measured as a replaced leaf. Blink resets a standards-mode
    /// inline fallback's preferred dimensions when non-empty `alt` text makes
    /// it non-replaced; missing/empty `alt`, quirks mode, and non-inline author
    /// display retain the authored sizing behavior.
    pub(crate) fn adjust_for_element_content(&mut self, content: &LayoutElementContent) {
        let LayoutElementContent::ImageFallback(fallback) = content else {
            return;
        };

        let width_is_auto = self.taffy.size.width.is_auto();
        let height_is_auto = self.taffy.size.height.is_auto();
        if fallback.is_quirks_mode() {
            if !width_is_auto && height_is_auto {
                self.taffy.size.height = self.taffy.size.width;
            } else if !height_is_auto && width_is_auto {
                self.taffy.size.width = self.taffy.size.height;
            }
        }

        if !self.image_fallback_is_atomic(content) && self.display == LayoutDisplay::Inline {
            self.taffy.size = Size {
                width: taffy::Dimension::auto(),
                height: taffy::Dimension::auto(),
            };
            self.preferred_aspect_ratio = PreferredAspectRatio::Auto;
            self.taffy.aspect_ratio = None;
        }
    }

    /// Whether failed-image fallback content participates as a sized atomic
    /// object instead of ordinary phrasing content.
    ///
    /// This is the layout-object counterpart of Blink's
    /// `TreatImageAsReplaced`: the fallback remains content-bearing (its alt
    /// text is laid out), but an authored two-axis size or one size plus a
    /// preferred ratio makes it atomic when the document is in quirks mode or
    /// the image has no non-empty `alt` value.
    pub(crate) fn image_fallback_is_atomic(&self, content: &LayoutElementContent) -> bool {
        let LayoutElementContent::ImageFallback(fallback) = content else {
            return false;
        };
        let has_intrinsic_dimensions =
            !self.taffy.size.width.is_auto() && !self.taffy.size.height.is_auto();
        let has_dimensions_from_ratio = self.preferred_aspect_ratio != PreferredAspectRatio::Auto
            && (!self.taffy.size.width.is_auto() || !self.taffy.size.height.is_auto());
        (has_intrinsic_dimensions || has_dimensions_from_ratio)
            && (fallback.is_quirks_mode() || !fallback.has_nonempty_alt_attribute())
    }

    pub(crate) fn resolved_aspect_ratio(&self, natural_ratio: Option<f32>) -> ResolvedAspectRatio {
        self.preferred_aspect_ratio
            .resolve(natural_ratio, self.taffy.box_sizing)
    }

    pub(crate) fn adjust_button_flow_layout(&mut self) {
        // Blink's UA sheet gives <button> a private safe block-content center
        // alignment. Taffy's block algorithm already implements the standard
        // equivalent. Only flow buttons use that algorithm; author flex/grid
        // containers retain their own align-content behavior.
        if matches!(
            self.display,
            LayoutDisplay::Block
                | LayoutDisplay::FlowRoot
                | LayoutDisplay::Inline
                | LayoutDisplay::InlineBlock
        ) {
            self.taffy.align_content = Some(taffy::AlignContent::SAFE_CENTER);
        }
    }

    pub(crate) fn mark_menu_list_formatting_context(&mut self) {
        // Blink gives an appearance:auto menu-list select a LayoutFlexibleBox
        // even though its computed display remains inline-block. Keep the
        // observable display in `display` and select only the inner numeric
        // formatting algorithm here.
        self.taffy.display = TaffyDisplay::Flex;
    }
}

fn usable_aspect_ratio(ratio: Option<f32>) -> Option<f32> {
    ratio.filter(|ratio| ratio.is_finite() && *ratio > 0.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TaffyDimensionProjection {
    dimension: taffy::Dimension,
    anchor_sizing_deferred: bool,
}

impl TaffyDimensionProjection {
    const fn supported(dimension: taffy::Dimension) -> Self {
        Self {
            dimension,
            anchor_sizing_deferred: false,
        }
    }

    const fn deferred(dimension: taffy::Dimension) -> Self {
        Self {
            dimension,
            anchor_sizing_deferred: true,
        }
    }
}

/// Project one computed CSS `<width>` value through a single capability seam.
///
/// Width, height, minimum sizes, and `flex-basis` share this grammar. Intrinsic
/// and stretch keywords therefore retain the same typed layout meaning on
/// every path. Anchor sizing is different: resolving it requires the
/// anchor-query context, so the generic Stylo/Taffy fallback remains in force
/// and the same projection marks that missing context for diagnostics.
fn project_taffy_size_value(
    size: &GenericSize<style::values::computed::NonNegativeLengthPercentage>,
    fallback: taffy::Dimension,
) -> TaffyDimensionProjection {
    match size {
        GenericSize::MinContent => {
            TaffyDimensionProjection::supported(taffy::Dimension::min_content())
        }
        GenericSize::MaxContent => {
            TaffyDimensionProjection::supported(taffy::Dimension::max_content())
        }
        GenericSize::FitContent => {
            TaffyDimensionProjection::supported(taffy::Dimension::fit_content())
        }
        GenericSize::FitContentFunction(limit) => {
            TaffyDimensionProjection::supported(taffy::Dimension::fit_content_function(
                stylo_taffy::convert::length_percentage(&limit.0),
            ))
        }
        GenericSize::Stretch | GenericSize::WebkitFillAvailable => {
            TaffyDimensionProjection::supported(taffy::Dimension::stretch())
        }
        GenericSize::Auto | GenericSize::LengthPercentage(_) => {
            TaffyDimensionProjection::supported(fallback)
        }
        GenericSize::AnchorSizeFunction(_) | GenericSize::AnchorContainingCalcFunction(_) => {
            TaffyDimensionProjection::deferred(fallback)
        }
    }
}

/// Preserve the complete `flex-basis` grammar at the browser/layout boundary.
///
/// The pinned `stylo_taffy` converter predates Taffy's typed content and
/// intrinsic bases. Reusing the shared `<width>` projection keeps `auto`,
/// `content`, intrinsic functions, stretch, and deferred anchor sizing
/// distinguishable until the flex algorithm has its constraint space.
fn project_taffy_flex_basis(
    flex_basis: &style::values::computed::FlexBasis,
    fallback: taffy::Dimension,
) -> TaffyDimensionProjection {
    match flex_basis {
        GenericFlexBasis::Content => {
            TaffyDimensionProjection::supported(taffy::Dimension::content())
        }
        GenericFlexBasis::Size(size) => project_taffy_size_value(size, fallback),
    }
}

fn project_taffy_max_size_dimension(
    size: &GenericMaxSize<style::values::computed::NonNegativeLengthPercentage>,
    fallback: taffy::Dimension,
) -> TaffyDimensionProjection {
    match size {
        GenericMaxSize::MinContent => {
            TaffyDimensionProjection::supported(taffy::Dimension::min_content())
        }
        GenericMaxSize::MaxContent => {
            TaffyDimensionProjection::supported(taffy::Dimension::max_content())
        }
        GenericMaxSize::FitContent => {
            TaffyDimensionProjection::supported(taffy::Dimension::fit_content())
        }
        GenericMaxSize::FitContentFunction(limit) => {
            TaffyDimensionProjection::supported(taffy::Dimension::fit_content_function(
                stylo_taffy::convert::length_percentage(&limit.0),
            ))
        }
        GenericMaxSize::Stretch | GenericMaxSize::WebkitFillAvailable => {
            TaffyDimensionProjection::supported(taffy::Dimension::stretch())
        }
        GenericMaxSize::None | GenericMaxSize::LengthPercentage(_) => {
            TaffyDimensionProjection::supported(fallback)
        }
        GenericMaxSize::AnchorSizeFunction(_) | GenericMaxSize::AnchorContainingCalcFunction(_) => {
            TaffyDimensionProjection::deferred(fallback)
        }
    }
}

/// Losslessly project the CSS self-alignment grammar into Taffy's typed
/// alignment protocol.
///
/// This is deliberately complete rather than a `last baseline` special case:
/// all four item/self properties cross one stable browser-layout boundary,
/// including writing-mode-relative self edges and overflow safety.
fn taffy_item_alignment(input: AlignFlags) -> Option<taffy::AlignItems> {
    let mut alignment = match input.value() {
        AlignFlags::AUTO => None,
        AlignFlags::NORMAL => Some(taffy::AlignItems::NORMAL),
        AlignFlags::STRETCH => Some(taffy::AlignItems::STRETCH),
        AlignFlags::FLEX_START => Some(taffy::AlignItems::FLEX_START),
        AlignFlags::FLEX_END => Some(taffy::AlignItems::FLEX_END),
        AlignFlags::SELF_START => Some(taffy::AlignItems::SELF_START),
        AlignFlags::SELF_END => Some(taffy::AlignItems::SELF_END),
        AlignFlags::START => Some(taffy::AlignItems::START),
        AlignFlags::END => Some(taffy::AlignItems::END),
        AlignFlags::LEFT => Some(taffy::AlignItems::LEFT),
        AlignFlags::RIGHT => Some(taffy::AlignItems::RIGHT),
        AlignFlags::CENTER => Some(taffy::AlignItems::CENTER),
        AlignFlags::BASELINE => Some(taffy::AlignItems::BASELINE),
        AlignFlags::LAST_BASELINE => Some(taffy::AlignItems::LAST_BASELINE),
        _ => None,
    }?;
    alignment.safety = taffy_alignment_safety(input);
    Some(alignment)
}

/// Preserve whether the CSS overflow-position modifier was omitted, safe, or
/// explicitly unsafe. The omitted/default case has layout-specific overflow
/// behavior and therefore cannot be folded into authored `unsafe`.
fn taffy_alignment_safety(input: AlignFlags) -> taffy::AlignmentSafety {
    if input.flags().contains(AlignFlags::SAFE) {
        taffy::AlignmentSafety::Safe
    } else if input.flags().contains(AlignFlags::UNSAFE) {
        taffy::AlignmentSafety::Unsafe
    } else {
        taffy::AlignmentSafety::Default
    }
}

/// Losslessly project the CSS content-distribution grammar into Taffy's typed
/// alignment protocol. Baseline preference is retained; it is not equivalent
/// to its positional fallback until a layout context determines that no
/// baseline-sharing group exists.
fn taffy_content_alignment(input: ContentDistribution) -> Option<taffy::AlignContent> {
    let primary = input.primary();
    let mut alignment = match primary.value() {
        AlignFlags::NORMAL | AlignFlags::AUTO => None,
        AlignFlags::START | AlignFlags::LEFT => Some(taffy::AlignContent::START),
        AlignFlags::END | AlignFlags::RIGHT => Some(taffy::AlignContent::END),
        AlignFlags::FLEX_START => Some(taffy::AlignContent::FLEX_START),
        AlignFlags::FLEX_END => Some(taffy::AlignContent::FLEX_END),
        AlignFlags::CENTER => Some(taffy::AlignContent::CENTER),
        AlignFlags::BASELINE => Some(taffy::AlignContent::BASELINE),
        AlignFlags::LAST_BASELINE => Some(taffy::AlignContent::LAST_BASELINE),
        AlignFlags::STRETCH => Some(taffy::AlignContent::STRETCH),
        AlignFlags::SPACE_BETWEEN => Some(taffy::AlignContent::SPACE_BETWEEN),
        AlignFlags::SPACE_AROUND => Some(taffy::AlignContent::SPACE_AROUND),
        AlignFlags::SPACE_EVENLY => Some(taffy::AlignContent::SPACE_EVENLY),
        _ => None,
    }?;
    alignment.safety = taffy_alignment_safety(primary);
    Some(alignment)
}

/// Resolve physical `left`/`right` for `justify-content` before preserving the
/// rest of the content-alignment protocol. Physical sides only apply when the
/// Flex main axis is inline; on a column axis both values fall back to start.
fn taffy_justify_content(
    input: ContentDistribution,
    flex_direction: StyloFlexDirection,
    direction: StyloDirection,
) -> Option<taffy::JustifyContent> {
    let primary = input.primary();
    let is_right = match primary.value() {
        AlignFlags::LEFT => false,
        AlignFlags::RIGHT => true,
        _ => return taffy_content_alignment(input),
    };
    let is_row = matches!(
        flex_direction,
        StyloFlexDirection::Row | StyloFlexDirection::RowReverse
    );
    let is_rtl = direction == StyloDirection::Rtl;
    let mut alignment = if is_row && is_right != is_rtl {
        taffy::AlignContent::END
    } else {
        taffy::AlignContent::START
    };
    alignment.safety = taffy_alignment_safety(primary);
    Some(alignment)
}

fn taffy_display(display: LayoutDisplay) -> TaffyDisplay {
    match display {
        LayoutDisplay::None => TaffyDisplay::None,
        LayoutDisplay::FlowRoot | LayoutDisplay::InlineBlock => TaffyDisplay::FlowRoot,
        LayoutDisplay::Flex | LayoutDisplay::InlineFlex => TaffyDisplay::Flex,
        LayoutDisplay::Grid | LayoutDisplay::InlineGrid => TaffyDisplay::Grid,
        LayoutDisplay::Contents
        | LayoutDisplay::Block
        | LayoutDisplay::Inline
        | LayoutDisplay::BlockListItem
        | LayoutDisplay::InlineListItem
        | LayoutDisplay::Table
        | LayoutDisplay::InlineTable
        | LayoutDisplay::TableCaption
        | LayoutDisplay::TableRowGroup
        | LayoutDisplay::TableHeaderGroup
        | LayoutDisplay::TableFooterGroup
        | LayoutDisplay::TableColumnGroup
        | LayoutDisplay::TableColumn
        | LayoutDisplay::TableRow
        | LayoutDisplay::TableCell => TaffyDisplay::Block,
    }
}

fn classify_display(computed: &ComputedValues) -> LayoutDisplay {
    let display = computed.clone_display();
    if display.is_none() {
        LayoutDisplay::None
    } else if display.is_contents() {
        LayoutDisplay::Contents
    } else {
        let outside = display.outside();
        let inside = display.inside();
        if display.is_list_item() {
            return if outside == DisplayOutside::Inline {
                LayoutDisplay::InlineListItem
            } else {
                LayoutDisplay::BlockListItem
            };
        }
        match inside {
            DisplayInside::None | DisplayInside::Contents => LayoutDisplay::Block,
            DisplayInside::Flow => match outside {
                DisplayOutside::Inline => LayoutDisplay::Inline,
                DisplayOutside::TableCaption => LayoutDisplay::TableCaption,
                DisplayOutside::None | DisplayOutside::Block | DisplayOutside::InternalTable => {
                    LayoutDisplay::Block
                }
            },
            DisplayInside::FlowRoot => {
                if outside == DisplayOutside::Inline {
                    LayoutDisplay::InlineBlock
                } else {
                    LayoutDisplay::FlowRoot
                }
            }
            DisplayInside::Flex => {
                if outside == DisplayOutside::Inline {
                    LayoutDisplay::InlineFlex
                } else {
                    LayoutDisplay::Flex
                }
            }
            DisplayInside::Grid => {
                if outside == DisplayOutside::Inline {
                    LayoutDisplay::InlineGrid
                } else {
                    LayoutDisplay::Grid
                }
            }
            DisplayInside::Table => {
                if outside == DisplayOutside::Inline {
                    LayoutDisplay::InlineTable
                } else {
                    LayoutDisplay::Table
                }
            }
            DisplayInside::TableRowGroup => LayoutDisplay::TableRowGroup,
            DisplayInside::TableHeaderGroup => LayoutDisplay::TableHeaderGroup,
            DisplayInside::TableFooterGroup => LayoutDisplay::TableFooterGroup,
            DisplayInside::TableColumnGroup => LayoutDisplay::TableColumnGroup,
            DisplayInside::TableColumn => LayoutDisplay::TableColumn,
            DisplayInside::TableRow => LayoutDisplay::TableRow,
            DisplayInside::TableCell => LayoutDisplay::TableCell,
        }
    }
}

fn stylo_background_color(computed: &ComputedValues) -> PaintColor {
    let current_color = computed.clone_color();
    let absolute = computed
        .clone_background_color()
        .resolve_to_absolute(&current_color)
        .to_color_space(ColorSpace::Srgb);
    let [red, green, blue, alpha] = *absolute.raw_components();
    PaintColor::new(red, green, blue, alpha)
}

pub(crate) fn absolute_paint_color(color: style::color::AbsoluteColor) -> PaintColor {
    let color = color.to_color_space(ColorSpace::Srgb);
    let [red, green, blue, alpha] = *color.raw_components();
    PaintColor::new(red, green, blue, alpha)
}

fn paint_border_style(style: StyloBorderStyle) -> PaintBorderStyle {
    match style {
        StyloBorderStyle::None => PaintBorderStyle::None,
        StyloBorderStyle::Hidden => PaintBorderStyle::Hidden,
        StyloBorderStyle::Solid => PaintBorderStyle::Solid,
        StyloBorderStyle::Dotted => PaintBorderStyle::Dotted,
        StyloBorderStyle::Dashed => PaintBorderStyle::Dashed,
        StyloBorderStyle::Double => PaintBorderStyle::Double,
        StyloBorderStyle::Groove => PaintBorderStyle::Groove,
        StyloBorderStyle::Ridge => PaintBorderStyle::Ridge,
        StyloBorderStyle::Inset => PaintBorderStyle::Inset,
        StyloBorderStyle::Outset => PaintBorderStyle::Outset,
    }
}

fn stylo_text_color(computed: &ComputedValues) -> PaintColor {
    let absolute = computed.clone_color().to_color_space(ColorSpace::Srgb);
    let [red, green, blue, alpha] = *absolute.raw_components();
    PaintColor::new(red, green, blue, alpha)
}

fn stylo_border_colors(computed: &ComputedValues) -> PaintBorderColors {
    let current_color = computed.clone_color();
    let border = computed.get_border();
    let [top, right, bottom, left] = [
        (&border.border_top_color, border.border_top_style),
        (&border.border_right_color, border.border_right_style),
        (&border.border_bottom_color, border.border_bottom_style),
        (&border.border_left_color, border.border_left_style),
    ]
    .map(|(color, border_style)| {
        if border_style.none_or_hidden() {
            return PaintColor::TRANSPARENT;
        }
        let absolute = color
            .resolve_to_absolute(&current_color)
            .to_color_space(ColorSpace::Srgb);
        let [red, green, blue, alpha] = *absolute.raw_components();
        PaintColor::new(red, green, blue, alpha)
    });
    PaintBorderColors {
        top,
        right,
        bottom,
        left,
    }
}

fn stylo_overflow_clips(computed: &ComputedValues) -> bool {
    !matches!(computed.clone_overflow_x(), Overflow::Visible)
        || !matches!(computed.clone_overflow_y(), Overflow::Visible)
}

// Ported from Blitz `stylo_to_kurbo.rs` at d788124a. The output is kept in a
// layout-owned affine type so geometry/query consumers do not depend on a
// paint library. Three-dimensional operations are diagnosed and conservatively
// omitted until the 3D transform/paint phase rather than flattened silently.
fn resolve_stylo_2d_transform(
    box_styles: &style::properties::generated::style_structs::Box,
    width: f32,
    height: f32,
) -> ResolvedLayoutTransform {
    let reference_box = euclid::default::Rect::new(
        euclid::default::Point2D::new(CSSPixelLength::new(0.0), CSSPixelLength::new(0.0)),
        euclid::default::Size2D::new(
            CSSPixelLength::new(width.max(0.0)),
            CSSPixelLength::new(height.max(0.0)),
        ),
    );
    let mut has_unsupported_3d = matches!(box_styles.perspective, GenericPerspective::Length(_));
    let translate = match &box_styles.translate {
        Translate::None => None,
        Translate::Translate(x, y, z) => {
            has_unsupported_3d |= z.px() != 0.0;
            Some(LayoutTransform2D::translation(
                x.resolve(reference_box.width()).px(),
                y.resolve(reference_box.height()).px(),
            ))
        }
    };
    let rotate = match &box_styles.rotate {
        Rotate::None => None,
        Rotate::Rotate(angle) => Some(LayoutTransform2D::rotation(angle.radians64())),
        Rotate::Rotate3D(x, y, z, angle) if *x == 0.0 && *y == 0.0 && *z != 0.0 => {
            let radians = if *z < 0.0 {
                -angle.radians64()
            } else {
                angle.radians64()
            };
            Some(LayoutTransform2D::rotation(radians))
        }
        Rotate::Rotate3D(..) => {
            has_unsupported_3d = true;
            None
        }
    };
    let scale = match &box_styles.scale {
        Scale::None => None,
        Scale::Scale(x, y, z) => {
            has_unsupported_3d |= *z != 1.0;
            Some(LayoutTransform2D::scale(f64::from(*x), f64::from(*y)))
        }
    };
    let transform = if box_styles.transform.0.is_empty() {
        None
    } else {
        match box_styles
            .transform
            .to_transform_3d_matrix(Some(&reference_box))
        {
            Ok((_matrix, true)) => {
                has_unsupported_3d = true;
                None
            }
            Ok((matrix, false)) => Some(LayoutTransform2D::new([
                f64::from(matrix.m11),
                f64::from(matrix.m12),
                f64::from(matrix.m21),
                f64::from(matrix.m22),
                f64::from(matrix.m41),
                f64::from(matrix.m42),
            ])),
            Err(_) => {
                has_unsupported_3d = true;
                None
            }
        }
    };

    let mut resolved = LayoutTransform2D::IDENTITY;
    for transform in [translate, rotate, scale, transform].into_iter().flatten() {
        resolved = resolved.concatenate(transform);
    }
    if resolved != LayoutTransform2D::IDENTITY {
        let origin = &box_styles.transform_origin;
        let origin_x = origin.horizontal.resolve(reference_box.width()).px();
        let origin_y = origin.vertical.resolve(reference_box.height()).px();
        resolved = LayoutTransform2D::translation(origin_x, origin_y)
            .concatenate(resolved)
            .concatenate(LayoutTransform2D::translation(-origin_x, -origin_y));
    }
    if !resolved.is_finite() {
        resolved = LayoutTransform2D::IDENTITY;
        has_unsupported_3d = true;
    }
    ResolvedLayoutTransform {
        transform: resolved,
        has_unsupported_3d,
        establishes_property_space: false,
    }
}

fn stylo_generated_content(computed: &ComputedValues) -> GeneratedContent {
    match &computed.get_counters().content {
        Content::Items(item_data) => {
            let items = &item_data.items[..item_data.alt_start];
            let mut output = String::new();
            let mut has_unsupported_items = false;
            for item in items {
                match item {
                    ContentItem::String(text) => output.push_str(text),
                    _ => has_unsupported_items = true,
                }
            }
            GeneratedContent::Items {
                text: Arc::from(output),
                has_unsupported_items,
            }
        }
        Content::Normal => GeneratedContent::Normal,
        Content::None => GeneratedContent::None,
    }
}

fn stylo_list_marker_type(computed: &ComputedValues) -> LayoutListMarkerType {
    use style::counter_style::{CounterStyle, Symbol};

    match computed.clone_list_style_type().0 {
        CounterStyle::None => LayoutListMarkerType::None,
        CounterStyle::Name(name) => match &*name.0 {
            "decimal" => LayoutListMarkerType::Decimal,
            "lower-alpha" | "lower-latin" => LayoutListMarkerType::LowerAlpha,
            "upper-alpha" | "upper-latin" => LayoutListMarkerType::UpperAlpha,
            "disc" => LayoutListMarkerType::Disc,
            "circle" => LayoutListMarkerType::Circle,
            "square" => LayoutListMarkerType::Square,
            "disclosure-open" => LayoutListMarkerType::DisclosureOpen,
            "disclosure-closed" => LayoutListMarkerType::DisclosureClosed,
            _ => LayoutListMarkerType::Fallback,
        },
        CounterStyle::String(value) => LayoutListMarkerType::String(Arc::from(value.as_ref())),
        CounterStyle::Symbols { symbols, .. } => LayoutListMarkerType::Symbols(
            symbols
                .0
                .iter()
                .map(|symbol| match symbol {
                    Symbol::String(value) => Arc::from(value.as_ref()),
                    Symbol::Ident(value) => Arc::from(value.0.as_ref()),
                })
                .collect(),
        ),
    }
}

fn stylo_font_metrics(computed: &ComputedValues) -> (f32, f32) {
    use style::values::computed::font::LineHeight;

    let font_size = computed.clone_font_size().used_size().px();
    let line_height = match computed.clone_line_height() {
        LineHeight::Normal => font_size * 1.2,
        LineHeight::Number(number) => font_size * number.0,
        LineHeight::Length(length) => length.0.px(),
    };
    (font_size, line_height)
}

fn stylo_vertical_align(
    computed: &ComputedValues,
    font_size: f32,
    line_height: f32,
) -> (InlineVerticalAlign, bool) {
    let (baseline_kind, baseline_shift) = match computed.clone_baseline_shift() {
        GenericBaselineShift::Keyword(BaselineShiftKeyword::Sub) => {
            (LayoutInlineAlignment::Baseline, -font_size * 0.2)
        }
        GenericBaselineShift::Keyword(BaselineShiftKeyword::Super) => {
            (LayoutInlineAlignment::Baseline, font_size / 3.0)
        }
        GenericBaselineShift::Keyword(BaselineShiftKeyword::Top) => {
            (LayoutInlineAlignment::Top, 0.0)
        }
        GenericBaselineShift::Keyword(BaselineShiftKeyword::Center) => {
            (LayoutInlineAlignment::Middle, 0.0)
        }
        GenericBaselineShift::Keyword(BaselineShiftKeyword::Bottom) => {
            (LayoutInlineAlignment::Bottom, 0.0)
        }
        GenericBaselineShift::Length(value) => (
            LayoutInlineAlignment::Baseline,
            value.resolve(CSSPixelLength::new(line_height)).px(),
        ),
    };
    let (alignment_kind, deferred) = match computed.clone_alignment_baseline() {
        AlignmentBaseline::Baseline | AlignmentBaseline::Alphabetic => {
            (LayoutInlineAlignment::Baseline, false)
        }
        AlignmentBaseline::TextTop => (LayoutInlineAlignment::TextTop, false),
        AlignmentBaseline::Middle | AlignmentBaseline::Central => {
            (LayoutInlineAlignment::Middle, false)
        }
        AlignmentBaseline::TextBottom => (LayoutInlineAlignment::TextBottom, false),
        AlignmentBaseline::Ideographic
        | AlignmentBaseline::Mathematical
        | AlignmentBaseline::Hanging => (LayoutInlineAlignment::Baseline, true),
    };
    (
        InlineVerticalAlign {
            kind: if baseline_kind == LayoutInlineAlignment::Baseline {
                alignment_kind
            } else {
                baseline_kind
            },
            baseline_shift,
        },
        deferred,
    )
}

pub(crate) fn resolve_stylo_calc_value(calc_ptr: *const (), parent_size: f32) -> f32 {
    use style::values::computed::{CSSPixelLength, length_percentage::CalcLengthPercentage};

    // SAFETY: `stylo_taffy` creates calc pointers from a live
    // `CalcLengthPercentage`. Every converted style in this crate retains its
    // originating `ComputedValues` until the containing `LayoutWorld` drops.
    let calc = unsafe { &*(calc_ptr as *const CalcLengthPercentage) };
    calc.resolve(CSSPixelLength::new(parent_size)).px()
}

#[cfg(test)]
mod sizing_projection_tests {
    use super::*;

    #[test]
    fn flex_basis_projection_preserves_typed_sizing_functions() {
        let project = |basis| project_taffy_flex_basis(&basis, taffy::Dimension::auto()).dimension;

        assert!(project(GenericFlexBasis::Content).is_content());
        assert!(project(GenericFlexBasis::Size(GenericSize::MinContent)).is_min_content());
        assert!(project(GenericFlexBasis::Size(GenericSize::MaxContent)).is_max_content());
        assert!(project(GenericFlexBasis::Size(GenericSize::FitContent)).is_fit_content());
        assert!(project(GenericFlexBasis::Size(GenericSize::Stretch)).is_stretch());
        assert!(project(GenericFlexBasis::Size(GenericSize::Auto)).is_auto());
    }

    #[test]
    fn remembered_containment_size_stays_logical_and_reenters_zoomed_layout_space() {
        let mut state = LayoutSizeContainmentState::new(
            LogicalSize {
                inline_size: false,
                block_size: false,
            },
            Size {
                width: Some(2.0),
                height: Some(1.0),
            },
            Size {
                width: true,
                height: true,
            },
            LayoutContentVisibility::Hidden,
            taffy::WritingMode::HorizontalTb,
            1.0,
        );
        assert_eq!(
            state.used.axes,
            Size {
                width: true,
                height: true,
            }
        );
        assert_eq!(
            state.used.intrinsic_content_size,
            Size {
                width: Some(2.0),
                height: Some(1.0),
            }
        );

        state.resolve_browser_state(
            false,
            LayoutLastRememberedSize {
                inline_size: Some(100.0),
                block_size: Some(50.0),
            },
            taffy::WritingMode::HorizontalTb,
            2.0,
        );
        assert_eq!(
            state.used.intrinsic_content_size,
            Size {
                width: Some(200.0),
                height: Some(100.0),
            }
        );

        state.recompute(taffy::WritingMode::VerticalLr, 2.0);
        assert_eq!(
            state.used.intrinsic_content_size,
            Size {
                width: Some(100.0),
                height: Some(200.0),
            }
        );
    }

    #[test]
    fn physical_auto_components_map_to_the_current_logical_observer_axis() {
        let state = LayoutSizeContainmentState::new(
            LogicalSize {
                inline_size: false,
                block_size: false,
            },
            Size::NONE,
            Size {
                width: true,
                height: false,
            },
            LayoutContentVisibility::Visible,
            taffy::WritingMode::HorizontalTb,
            1.0,
        );
        let horizontal = state.observer_policy(taffy::WritingMode::HorizontalTb);
        assert!(horizontal.records_inline_size());
        assert!(!horizontal.records_block_size());

        let vertical = state.observer_policy(taffy::WritingMode::VerticalLr);
        assert!(!vertical.records_inline_size());
        assert!(vertical.records_block_size());
    }
}

#[cfg(test)]
mod aspect_ratio_tests {
    use super::*;
    use crate::LayoutImageFallbackContent;

    #[test]
    fn replaced_ratio_resolution_preserves_auto_precedence_and_box_basis() {
        let specified = PreferredAspectRatio::Ratio(1.0).resolve(Some(2.0), BoxSizing::BorderBox);
        assert_eq!(specified.ratio, Some(1.0));
        assert_eq!(specified.box_sizing, BoxSizing::BorderBox);

        let natural =
            PreferredAspectRatio::AutoAndRatio(1.0).resolve(Some(2.0), BoxSizing::BorderBox);
        assert_eq!(natural.ratio, Some(2.0));
        assert_eq!(natural.box_sizing, BoxSizing::ContentBox);

        let fallback = PreferredAspectRatio::AutoAndRatio(1.0).resolve(None, BoxSizing::BorderBox);
        assert_eq!(fallback.ratio, Some(1.0));
        assert_eq!(fallback.box_sizing, BoxSizing::ContentBox);
    }

    #[test]
    fn failed_image_fallback_adjusts_used_sizing_from_content_disposition() {
        let style_for = |display, has_nonempty_alt_attribute, quirks_mode| {
            let mut style = ResolvedLayoutStyle::synthetic(
                display,
                Style {
                    size: Size {
                        width: taffy::Dimension::length(100.0),
                        height: taffy::Dimension::auto(),
                    },
                    aspect_ratio: Some(0.2),
                    ..Style::default()
                },
                PaintColor::TRANSPARENT,
            );
            style.adjust_for_element_content(&LayoutElementContent::ImageFallback(
                LayoutImageFallbackContent::new(
                    "fallback",
                    has_nonempty_alt_attribute,
                    quirks_mode,
                ),
            ));
            style
        };

        let inline_text = style_for(LayoutDisplay::Inline, true, false);
        assert!(inline_text.taffy.size.width.is_auto());
        assert!(inline_text.taffy.size.height.is_auto());
        assert_eq!(inline_text.taffy.aspect_ratio, None);

        let missing_alt = style_for(LayoutDisplay::Inline, false, false);
        assert!(!missing_alt.taffy.size.width.is_auto());
        assert!(missing_alt.taffy.size.height.is_auto());
        assert_eq!(missing_alt.taffy.aspect_ratio, Some(0.2));

        let block = style_for(LayoutDisplay::Block, true, false);
        assert!(!block.taffy.size.width.is_auto());
        assert!(block.taffy.size.height.is_auto());
        assert_eq!(block.taffy.aspect_ratio, Some(0.2));

        let quirks = style_for(LayoutDisplay::Inline, true, true);
        assert_eq!(quirks.taffy.size.width, quirks.taffy.size.height);
        assert_eq!(quirks.taffy.aspect_ratio, Some(0.2));
    }
}

#[cfg(test)]
mod alignment_tests {
    use super::*;

    #[test]
    fn flow_relative_text_alignment_resolves_from_css_direction() {
        let style_for = |direction| {
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                Style {
                    direction,
                    ..Style::default()
                },
                PaintColor::TRANSPARENT,
            )
        };

        let mut ltr = style_for(taffy::Direction::Ltr);
        let mut rtl = style_for(taffy::Direction::Rtl);
        assert_eq!(ltr.resolved_text_align(), parley::Alignment::Left);
        assert_eq!(rtl.resolved_text_align(), parley::Alignment::Right);

        ltr.text_align = parley::Alignment::End;
        rtl.text_align = parley::Alignment::End;
        assert_eq!(ltr.resolved_text_align(), parley::Alignment::Right);
        assert_eq!(rtl.resolved_text_align(), parley::Alignment::Left);

        rtl.text_align = parley::Alignment::Center;
        assert_eq!(rtl.resolved_text_align(), parley::Alignment::Center);
    }

    #[test]
    fn inline_baseline_follows_writing_mode_and_text_orientation() {
        let mut style = ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style::default(),
            PaintColor::TRANSPARENT,
        );

        for writing_mode in [
            taffy::WritingMode::HorizontalTb,
            taffy::WritingMode::SidewaysRl,
            taffy::WritingMode::SidewaysLr,
        ] {
            style.writing_mode = writing_mode;
            assert_eq!(style.font_baseline(), FontBaseline::Alphabetic);
        }
        for writing_mode in [
            taffy::WritingMode::VerticalRl,
            taffy::WritingMode::VerticalLr,
        ] {
            style.writing_mode = writing_mode;
            for text_orientation in [InlineTextOrientation::Mixed, InlineTextOrientation::Upright] {
                style.text_orientation = text_orientation;
                assert_eq!(style.font_baseline(), FontBaseline::Central);
            }
            style.text_orientation = InlineTextOrientation::Sideways;
            assert_eq!(style.font_baseline(), FontBaseline::Alphabetic);
        }
    }

    #[test]
    fn stylo_item_alignment_preserves_layout_protocol_values() {
        assert_eq!(
            taffy_item_alignment(AlignFlags::NORMAL),
            Some(taffy::AlignItems::NORMAL)
        );
        assert_ne!(
            taffy_item_alignment(AlignFlags::NORMAL),
            taffy_item_alignment(AlignFlags::STRETCH)
        );
        assert_eq!(
            taffy_item_alignment(AlignFlags::LAST_BASELINE),
            Some(taffy::AlignItems::LAST_BASELINE)
        );
        assert_eq!(
            taffy_item_alignment(AlignFlags::SELF_START),
            Some(taffy::AlignItems::SELF_START)
        );
        assert_eq!(
            taffy_item_alignment(AlignFlags::LEFT),
            Some(taffy::AlignItems::LEFT)
        );
        assert_eq!(
            taffy_item_alignment(AlignFlags::RIGHT),
            Some(taffy::AlignItems::RIGHT)
        );

        let safe_center = taffy_item_alignment(AlignFlags::CENTER | AlignFlags::SAFE)
            .expect("safe center should project");
        assert_eq!(safe_center.keyword, taffy::AlignItemsKeyword::Center);
        assert_eq!(safe_center.safety, taffy::AlignmentSafety::Safe);

        let default_end = taffy_item_alignment(AlignFlags::END).expect("end should project");
        let unsafe_end = taffy_item_alignment(AlignFlags::END | AlignFlags::UNSAFE)
            .expect("unsafe end should project");
        assert_eq!(default_end.safety, taffy::AlignmentSafety::Default);
        assert_eq!(unsafe_end.safety, taffy::AlignmentSafety::Unsafe);
    }

    #[test]
    fn stylo_content_alignment_preserves_layout_protocol_values() {
        assert_eq!(
            taffy_content_alignment(ContentDistribution::new(AlignFlags::BASELINE)),
            Some(taffy::AlignContent::BASELINE)
        );
        assert_eq!(
            taffy_content_alignment(ContentDistribution::new(AlignFlags::LAST_BASELINE)),
            Some(taffy::AlignContent::LAST_BASELINE)
        );

        let safe_center = taffy_content_alignment(ContentDistribution::new(
            AlignFlags::CENTER | AlignFlags::SAFE,
        ))
        .expect("safe center should project");
        assert_eq!(safe_center.keyword, taffy::AlignContentKeyword::Center);
        assert_eq!(safe_center.safety, taffy::AlignmentSafety::Safe);

        assert_eq!(
            taffy_justify_content(
                ContentDistribution::new(AlignFlags::RIGHT),
                StyloFlexDirection::Row,
                StyloDirection::Ltr,
            ),
            Some(taffy::JustifyContent::END)
        );
        assert_eq!(
            taffy_justify_content(
                ContentDistribution::new(AlignFlags::RIGHT),
                StyloFlexDirection::Row,
                StyloDirection::Rtl,
            ),
            Some(taffy::JustifyContent::START)
        );
        assert_eq!(
            taffy_justify_content(
                ContentDistribution::new(AlignFlags::RIGHT),
                StyloFlexDirection::Column,
                StyloDirection::Ltr,
            ),
            Some(taffy::JustifyContent::START)
        );
    }
}
