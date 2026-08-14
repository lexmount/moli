// SPDX-License-Identifier: MIT OR Apache-2.0
//
// The Stylo-to-Taffy projection uses the standalone `stylo_taffy` crate from
// DioxusLabs/blitz commit d788124ab881f9bb537cb452ec1d837604a374a8.

use std::sync::Arc;

use style::{
    Atom,
    color::ColorSpace,
    computed_values::{
        content_visibility::T as StyloContentVisibility, isolation::T as StyloIsolation,
        mix_blend_mode::T as StyloMixBlendMode,
    },
    properties::ComputedValues,
    properties::generated::longhands::position::computed_value::T as StyloPosition,
    properties::generated::longhands::{
        direction::computed_value::T as StyloDirection,
        unicode_bidi::computed_value::T as StyloUnicodeBidi,
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
                Perspective as GenericPerspective,
            },
            flex::GenericFlexBasis,
            grid::GenericGridTemplateComponent,
            image::GenericImage,
            length::{GenericMaxSize, GenericSize},
            position::PreferredRatio,
            transform::{Rotate, Scale, Translate},
        },
        specified::box_::{DisplayInside, DisplayOutside},
        specified::{
            TextAlignKeyword, WillChangeBits,
            text::{TextTransform, TextTransformCase},
        },
    },
};
use taffy::{BoxSizing, Display as TaffyDisplay, Position as TaffyPosition, Size, Style};

use crate::{
    LayoutPoint, LayoutRect, LayoutTransform2D, PaintBlendMode, PaintBorderColors,
    PaintBorderStyle, PaintBorderStyles, PaintBoxShadow, PaintColor, PaintCornerRadii,
    PaintCornerRadius, PaintEdgeSizes, PaintFragment,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ResolvedAspectRatio {
    pub(crate) ratio: Option<f32>,
    /// The box whose dimensions the ratio constrains.
    pub(crate) box_sizing: BoxSizing,
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

    fn resolve_for_replaced(
        self,
        inherent_ratio: Option<f32>,
        authored_box_sizing: BoxSizing,
    ) -> ResolvedAspectRatio {
        let inherent_ratio = usable_aspect_ratio(inherent_ratio);
        match self {
            Self::Ratio(ratio) => ResolvedAspectRatio {
                ratio: Some(ratio),
                box_sizing: authored_box_sizing,
            },
            Self::Auto => ResolvedAspectRatio {
                ratio: inherent_ratio,
                box_sizing: BoxSizing::ContentBox,
            },
            // Blink's BoxSizingForAspectRatio() uses content-box for the
            // combined `auto <ratio>` value, including when its ratio is used
            // as the fallback because no natural ratio is available.
            Self::AutoAndRatio(fallback) => ResolvedAspectRatio {
                ratio: inherent_ratio.or(Some(fallback)),
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
    unicode_bidi: InlineUnicodeBidi,
    vertical_align: InlineVerticalAlign,
    text_projection_deferred: bool,
    overflow_clips: bool,
    out_of_flow: bool,
    position: LayoutPosition,
    sticky_inset: taffy::Rect<taffy::LengthPercentageAuto>,
    establishes_transform_containing_block: bool,
    synthetic_transform: Option<LayoutTransform2D>,
    visible: bool,
    pointer_events: bool,
    order: i32,
    intrinsic_sizing_deferred: bool,
    grid_template_mode_deferred: bool,
    explicit_z_index: Option<i32>,
    opacity: f32,
    blend_mode: PaintBlendMode,
    has_filter_effect: bool,
    has_clip_path: bool,
    has_mask: bool,
    isolation: bool,
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
            .field("visible", &self.visible)
            .field("pointer_events", &self.pointer_events)
            .field("order", &self.order)
            .field("intrinsic_sizing_deferred", &self.intrinsic_sizing_deferred)
            .field(
                "grid_template_mode_deferred",
                &self.grid_template_mode_deferred,
            )
            .field("explicit_z_index", &self.explicit_z_index)
            .field("opacity", &self.opacity)
            .field("blend_mode", &self.blend_mode)
            .field("has_filter_effect", &self.has_filter_effect)
            .field("has_clip_path", &self.has_clip_path)
            .field("has_mask", &self.has_mask)
            .field("isolation", &self.isolation)
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
        let intrinsic_sizing_deferred = [&position_style.height, &position_style.min_height]
            .into_iter()
            .any(|size| !matches!(size, GenericSize::Auto | GenericSize::LengthPercentage(_)))
            || [&position_style.max_height].into_iter().any(|size| {
                !matches!(
                    size,
                    GenericMaxSize::None | GenericMaxSize::LengthPercentage(_)
                )
            })
            || [&position_style.width, &position_style.min_width]
                .into_iter()
                .any(|size| {
                    !matches!(
                        size,
                        GenericSize::Auto
                            | GenericSize::LengthPercentage(_)
                            | GenericSize::MinContent
                            | GenericSize::MaxContent
                            | GenericSize::FitContent
                            | GenericSize::Stretch
                            | GenericSize::WebkitFillAvailable
                    )
                })
            || !matches!(
                &position_style.max_width,
                GenericMaxSize::None
                    | GenericMaxSize::LengthPercentage(_)
                    | GenericMaxSize::MinContent
                    | GenericMaxSize::MaxContent
                    | GenericMaxSize::FitContent
                    | GenericMaxSize::Stretch
                    | GenericMaxSize::WebkitFillAvailable
            );
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
        let mut layout_containment = contain.contains(style::values::computed::Contain::LAYOUT);
        let mut paint_containment = contain.contains(style::values::computed::Contain::PAINT);
        if computed.clone_content_visibility() != StyloContentVisibility::Visible {
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
        taffy.size = Size {
            width: taffy_size_dimension(&position_style.width, taffy.size.width),
            height: taffy_size_dimension(&position_style.height, taffy.size.height),
        };
        taffy.min_size = Size {
            width: taffy_size_dimension(&position_style.min_width, taffy.min_size.width),
            height: taffy_size_dimension(&position_style.min_height, taffy.min_size.height),
        };
        taffy.max_size = Size {
            width: taffy_max_size_dimension(&position_style.max_width, taffy.max_size.width),
            height: taffy_max_size_dimension(&position_style.max_height, taffy.max_size.height),
        };
        // Taffy's generic leaf algorithm transfers aspect ratios before a
        // replaced-element measure callback runs. CSS Sizing 4 defines zero,
        // infinite and NaN ratios as degenerate, so normalize them at the
        // Stylo/Taffy seam instead of relying only on replaced measurement.
        taffy.aspect_ratio = preferred_aspect_ratio.numeric_ratio();
        if matches!(position_style.flex_basis, GenericFlexBasis::Content) {
            // Blitz's fixed stylo_taffy revision predates Taffy's typed
            // `content` flex-basis. Preserve the Stylo distinction here so
            // Taffy ignores the preferred main size and follows the content
            // flex-base-size algorithm instead of treating it as `auto`.
            taffy.flex_basis = taffy::Dimension::content();
        }
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
        let visible = computed.clone_visibility() == style::computed_values::visibility::T::Visible;
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
            unicode_bidi,
            vertical_align,
            text_projection_deferred,
            overflow_clips,
            out_of_flow,
            position,
            sticky_inset,
            establishes_transform_containing_block,
            synthetic_transform: None,
            visible,
            pointer_events,
            order,
            intrinsic_sizing_deferred,
            grid_template_mode_deferred,
            explicit_z_index,
            opacity,
            blend_mode,
            has_filter_effect,
            has_clip_path,
            has_mask,
            isolation,
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
            direction: InlineDirection::Ltr,
            unicode_bidi: InlineUnicodeBidi::Normal,
            vertical_align: InlineVerticalAlign::default(),
            text_projection_deferred: false,
            overflow_clips,
            out_of_flow,
            position,
            sticky_inset,
            establishes_transform_containing_block: false,
            synthetic_transform: None,
            visible: true,
            pointer_events: true,
            order: 0,
            intrinsic_sizing_deferred: false,
            grid_template_mode_deferred: false,
            explicit_z_index: None,
            opacity: 1.0,
            blend_mode: PaintBlendMode::Normal,
            has_filter_effect: false,
            has_clip_path: false,
            has_mask: false,
            isolation: false,
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
        is_css_box: bool,
        containment_eligible: bool,
    ) -> bool {
        self.position.is_positioned()
            || self.will_change_position
            || self.establishes_fixed_containing_block(is_css_box, containment_eligible)
    }

    pub(crate) fn establishes_fixed_containing_block(
        &self,
        is_css_box: bool,
        containment_eligible: bool,
    ) -> bool {
        (is_css_box && self.establishes_transform_containing_block)
            || (containment_eligible
                && (self.layout_containment
                    || self.paint_containment
                    || self.will_change_containment))
    }

    pub(crate) const fn applies_paint_containment(&self) -> bool {
        self.paint_containment
    }

    pub(crate) const fn is_visible(&self) -> bool {
        self.visible
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
        self.computed.as_ref().is_some_and(|computed| {
            computed.clone_table_layout() == style::computed_values::table_layout::T::Fixed
        })
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

    pub(crate) fn has_deferred_intrinsic_sizing(&self) -> bool {
        self.intrinsic_sizing_deferred
    }

    pub(crate) fn has_deferred_grid_template_mode(&self) -> bool {
        self.grid_template_mode_deferred
    }

    pub(crate) fn has_auto_inset_axis(&self) -> bool {
        (self.taffy.inset.left.is_auto() && self.taffy.inset.right.is_auto())
            || (self.taffy.inset.top.is_auto() && self.taffy.inset.bottom.is_auto())
    }

    /// Creates a zero-sized, non-painted absolute placeholder that asks the
    /// original block formatting context to compute a positioned descendant's
    /// hypothetical static position. The real box remains a child of its CSS
    /// containing block in the numeric tree.
    pub(crate) fn positioned_static_placeholder(&self) -> Self {
        let mut placeholder = self.clone();
        let zero_dimension = taffy::Dimension::length(0.0);
        let zero_length = taffy::LengthPercentage::length(0.0);
        placeholder.display = LayoutDisplay::Block;
        placeholder.taffy.display = TaffyDisplay::Block;
        placeholder.taffy.size = Size {
            width: zero_dimension,
            height: zero_dimension,
        };
        placeholder.taffy.min_size = Size {
            width: zero_dimension,
            height: zero_dimension,
        };
        placeholder.taffy.max_size = Size {
            width: zero_dimension,
            height: zero_dimension,
        };
        placeholder.taffy.padding = taffy::Rect {
            left: zero_length,
            right: zero_length,
            top: zero_length,
            bottom: zero_length,
        };
        placeholder.taffy.border = taffy::Rect {
            left: zero_length,
            right: zero_length,
            top: zero_length,
            bottom: zero_length,
        };
        placeholder.taffy.aspect_ratio = None;
        placeholder.preferred_aspect_ratio = PreferredAspectRatio::Auto;
        placeholder.background_color = PaintColor::TRANSPARENT;
        placeholder.border_colors = PaintBorderColors::all(PaintColor::TRANSPARENT);
        placeholder.generated_content = GeneratedContent::None;
        placeholder.establishes_transform_containing_block = false;
        placeholder.synthetic_transform = None;
        // This is an internal static-position probe, not the authored CSS
        // principal box. Do not let copied containment or `will-change`
        // metadata turn it into a containing block, stacking context, or
        // descendant clip in the later projection passes.
        placeholder.layout_containment = false;
        placeholder.paint_containment = false;
        placeholder.will_change_containment = false;
        placeholder.will_change_position = false;
        placeholder.will_change_stacking_context = false;
        placeholder.visible = false;
        placeholder.pointer_events = false;
        placeholder.explicit_z_index = None;
        placeholder
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

    pub(crate) fn text_align(&self) -> parley::Alignment {
        self.text_align
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
            visible: parent.visible,
            pointer_events: parent.pointer_events,
            order: 0,
            intrinsic_sizing_deferred: false,
            grid_template_mode_deferred: false,
            explicit_z_index: None,
            opacity: 1.0,
            blend_mode: PaintBlendMode::Normal,
            has_filter_effect: false,
            has_clip_path: false,
            has_mask: false,
            isolation: false,
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
            visible: parent.visible,
            pointer_events: parent.pointer_events,
            order: 0,
            intrinsic_sizing_deferred: false,
            grid_template_mode_deferred: false,
            explicit_z_index: None,
            opacity: 1.0,
            blend_mode: PaintBlendMode::Normal,
            has_filter_effect: false,
            has_clip_path: false,
            has_mask: false,
            isolation: false,
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

    pub(crate) fn mark_replaced(&mut self, inherent_ratio: Option<f32>) {
        self.taffy.item_is_replaced = true;
        self.taffy.aspect_ratio = self
            .preferred_aspect_ratio
            .resolve_for_replaced(inherent_ratio, self.taffy.box_sizing)
            .ratio;
    }

    pub(crate) fn resolved_replaced_aspect_ratio(
        &self,
        inherent_ratio: Option<f32>,
    ) -> ResolvedAspectRatio {
        self.preferred_aspect_ratio
            .resolve_for_replaced(inherent_ratio, self.taffy.box_sizing)
    }

    pub(crate) fn mark_intrinsic_form_control_container(&mut self) {
        // A button retains and lays out its real DOM children, but its
        // block-level outer display still uses the same non-stretch intrinsic
        // width exception as other native controls.
        self.taffy.item_is_table = true;
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
}

fn usable_aspect_ratio(ratio: Option<f32>) -> Option<f32> {
    ratio.filter(|ratio| ratio.is_finite() && *ratio > 0.0)
}

fn taffy_size_dimension(
    size: &GenericSize<style::values::computed::NonNegativeLengthPercentage>,
    fallback: taffy::Dimension,
) -> taffy::Dimension {
    match size {
        GenericSize::MinContent => taffy::Dimension::min_content(),
        GenericSize::MaxContent => taffy::Dimension::max_content(),
        GenericSize::FitContent => taffy::Dimension::fit_content(),
        GenericSize::Stretch | GenericSize::WebkitFillAvailable => taffy::Dimension::stretch(),
        _ => fallback,
    }
}

fn taffy_max_size_dimension(
    size: &GenericMaxSize<style::values::computed::NonNegativeLengthPercentage>,
    fallback: taffy::Dimension,
) -> taffy::Dimension {
    match size {
        GenericMaxSize::MinContent => taffy::Dimension::min_content(),
        GenericMaxSize::MaxContent => taffy::Dimension::max_content(),
        GenericMaxSize::FitContent => taffy::Dimension::fit_content(),
        GenericMaxSize::Stretch | GenericMaxSize::WebkitFillAvailable => {
            taffy::Dimension::stretch()
        }
        _ => fallback,
    }
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
mod aspect_ratio_tests {
    use super::*;

    #[test]
    fn replaced_ratio_resolution_preserves_auto_precedence_and_box_basis() {
        let specified =
            PreferredAspectRatio::Ratio(1.0).resolve_for_replaced(Some(2.0), BoxSizing::BorderBox);
        assert_eq!(specified.ratio, Some(1.0));
        assert_eq!(specified.box_sizing, BoxSizing::BorderBox);

        let natural = PreferredAspectRatio::AutoAndRatio(1.0)
            .resolve_for_replaced(Some(2.0), BoxSizing::BorderBox);
        assert_eq!(natural.ratio, Some(2.0));
        assert_eq!(natural.box_sizing, BoxSizing::ContentBox);

        let fallback = PreferredAspectRatio::AutoAndRatio(1.0)
            .resolve_for_replaced(None, BoxSizing::BorderBox);
        assert_eq!(fallback.ratio, Some(1.0));
        assert_eq!(fallback.box_sizing, BoxSizing::ContentBox);
    }
}
