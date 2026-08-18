use std::{fmt::Debug, hash::Hash};

use parley::{AlignmentOptions, PositionedLayoutItem, YieldData};
use style::Atom;
use taffy::{
    AlignContent, AutoSizeBehavior, AvailableSpace, BlockContext, BlockFormattingContext,
    CacheTree, Clear, DetailedGridInfo, Dimension, Display, FloatDirection, FontBaseline,
    IntrinsicSizeResult, Layout, LayoutBlockContainer, LayoutFlexboxContainer, LayoutGridContainer,
    LayoutInput, LayoutOutput, LayoutPartialTree, Line, LogicalBoxStrut, LogicalOffset,
    LogicalSize, LogicalStaticPosition, MaybeMath, MaybeResolve, NodeId, OutOfFlowCandidate,
    OutOfFlowContainingBlock, Point, ResolveOrZero, RoundTree, RunMode, Size, SizingMode,
    SizingPurpose, Style, TraversePartialTree, TraverseTree, WritingDirection,
    compute_block_layout, compute_cached_layout, compute_cached_size,
    compute_content_alignment_offset, compute_flexbox_layout, compute_grid_layout,
    compute_hidden_layout, compute_leaf_layout_with_tree, compute_out_of_flow_layout,
    compute_replaced_layout, compute_root_layout, resolve_content_alignment_fallback, round_layout,
};

use crate::{
    LayoutBoxId, LayoutBoxKind, LayoutCapabilityDiagnostic, LayoutWorld, PaintRect, PaintViewport,
    inline::{
        InlineCoordinateSpace, InlineFormattingContext, InlineFragments, InlineLinePlacement,
        InlineObjectRole, LineRelativeFragments, LineRelativeOffset, build_inline_fragments,
        build_inline_line_placements, flow_relative_line_rect, relative_atomic_inset_offset,
        synthesized_font_ascent,
    },
    style::{InlineDirection, resolve_stylo_calc_value},
    table::{compute_table_layout, prepare_table_layout_trees},
    world::{OutOfFlowCandidateChild, OutOfFlowStaticPosition},
};

// Blink stores box geometry in 1/64 CSS-pixel LayoutUnits.
const LAYOUT_SUBPIXELS_PER_CSS_PIXEL: f32 = 64.0;

#[inline]
fn box_model_percentage_basis(
    inputs: LayoutInput,
    writing_mode: taffy::WritingMode,
) -> Option<f32> {
    inputs
        .constraint_space(writing_mode)
        .margin_padding_percentage_basis()
}

#[inline]
fn logical_static_position_in_owner(
    point: Point<f32>,
    owner_size: Size<f32>,
    writing_direction: WritingDirection,
) -> LogicalStaticPosition {
    LogicalStaticPosition::new(
        writing_direction
            .converter(owner_size)
            .to_logical_point(point, Size::ZERO),
    )
}

pub(crate) fn compute_world_layout<N>(world: &mut LayoutWorld<N>, viewport: PaintViewport)
where
    N: Copy + Debug + Eq + Hash,
{
    let viewport_writing_direction = world.propagate_viewport_writing_direction();
    world.layout_environment = taffy::LayoutEnvironment {
        initial_containing_block_size: Size {
            width: Some(viewport.css_width as f32),
            height: Some(viewport.css_height as f32),
        },
    };
    for layout_box in &mut world.boxes {
        layout_box.cache.clear();
        layout_box.unrounded_layout = Layout::with_order(0);
        layout_box.final_layout = Layout::with_order(0);
        layout_box.layout_parent = None;
        layout_box.layout_children.clear();
        layout_box.out_of_flow_candidates.clear();
        layout_box.positioned_containing_block = None;
        layout_box.out_of_flow_static_position = None;
        layout_box.grid_geometry = None;
    }

    world.viewport_layout.children.clear();
    world.viewport_layout.cache.clear();
    world.viewport_layout.unrounded_layout = Layout::with_order(0);
    world.viewport_layout.final_layout = Layout::with_order(0);
    world.viewport_layout.writing_mode = viewport_writing_direction.mode;
    world.viewport_layout.style = Style {
        display: Display::Block,
        direction: viewport_writing_direction.direction,
        size: Size {
            width: Dimension::length(viewport.css_width as f32),
            height: Dimension::length(viewport.css_height as f32),
        },
        min_size: Size {
            width: Dimension::length(viewport.css_width as f32),
            height: Dimension::length(viewport.css_height as f32),
        },
        max_size: Size {
            width: Dimension::length(viewport.css_width as f32),
            height: Dimension::length(viewport.css_height as f32),
        },
        ..Style::default()
    };
    prepare_layout_tree(world);
    prepare_table_layout_trees(world);

    let root = world.viewport_taffy_node();
    compute_root_layout(
        world,
        root,
        Size {
            width: AvailableSpace::Definite(viewport.css_width as f32),
            height: AvailableSpace::Definite(viewport.css_height as f32),
        },
    );
    finish_form_control_contents(world);
    finish_outside_list_markers(world);
    finish_sticky_positioning(world, viewport);
    round_layout_to_css_subpixels(world, root);
    // Outside markers deliberately are not numeric children of the list item,
    // otherwise Taffy would allocate them a normal-flow row. Round each
    // detached numeric root explicitly so paint consumes the geometry written
    // by `finish_outside_list_markers` rather than its zeroed final layout.
    let outside_markers = (0..world.boxes.len())
        .map(LayoutBoxId::from_index)
        .filter(|id| world.boxes[id.index()].outside_list_marker)
        .collect::<Vec<_>>();
    for marker in outside_markers {
        round_layout_to_css_subpixels(world, marker.to_taffy());
    }
}

/// Quantize final browser geometry without teaching Taffy about CSS layout
/// units. Taffy's public rounding pass operates on an abstract integer grid;
/// this adapter presents one CSS pixel as 64 such units and converts the
/// result back at the ownership boundary.
fn round_layout_to_css_subpixels(tree: &mut impl RoundTree, root: NodeId) {
    let mut scaled = CssSubpixelRoundTree { tree };
    round_layout(&mut scaled, root);
}

struct CssSubpixelRoundTree<'a, Tree>
where
    Tree: RoundTree + ?Sized,
{
    tree: &'a mut Tree,
}

impl<Tree> TraversePartialTree for CssSubpixelRoundTree<'_, Tree>
where
    Tree: RoundTree + ?Sized,
{
    type ChildIter<'a>
        = Tree::ChildIter<'a>
    where
        Self: 'a;

    fn child_ids(&self, parent_node_id: NodeId) -> Self::ChildIter<'_> {
        self.tree.child_ids(parent_node_id)
    }

    fn child_count(&self, parent_node_id: NodeId) -> usize {
        self.tree.child_count(parent_node_id)
    }

    fn get_child_id(&self, parent_node_id: NodeId, child_index: usize) -> NodeId {
        self.tree.get_child_id(parent_node_id, child_index)
    }
}

impl<Tree> TraverseTree for CssSubpixelRoundTree<'_, Tree> where Tree: RoundTree + ?Sized {}

impl<Tree> RoundTree for CssSubpixelRoundTree<'_, Tree>
where
    Tree: RoundTree + ?Sized,
{
    fn get_unrounded_layout(&self, node_id: NodeId) -> Layout {
        scale_layout(
            self.tree.get_unrounded_layout(node_id),
            LAYOUT_SUBPIXELS_PER_CSS_PIXEL,
        )
    }

    fn set_final_layout(&mut self, node_id: NodeId, layout: &Layout) {
        let css_layout = scale_layout(*layout, 1.0 / LAYOUT_SUBPIXELS_PER_CSS_PIXEL);
        self.tree.set_final_layout(node_id, &css_layout);
    }
}

fn scale_layout(layout: Layout, factor: f32) -> Layout {
    Layout {
        location: layout.location.map(|value| value * factor),
        size: layout.size.map(|value| value * factor),
        content_size: layout.content_size.map(|value| value * factor),
        scrollbar_size: layout.scrollbar_size.map(|value| value * factor),
        border: layout.border.map(|value| value * factor),
        padding: layout.padding.map(|value| value * factor),
        margin: layout.margin.map(|value| value * factor),
        ..layout
    }
}

fn prepare_layout_tree<N>(world: &mut LayoutWorld<N>)
where
    N: Copy + Debug + Eq + Hash,
{
    let root = world.root;
    world.viewport_layout.children.push(root);

    let mut preorder = Vec::with_capacity(world.boxes.len().saturating_sub(1));
    let mut stack = world.boxes[root.index()]
        .children
        .iter()
        .rev()
        .copied()
        .collect::<Vec<_>>();
    while let Some(id) = stack.pop() {
        preorder.push(id);
        stack.extend(world.boxes[id.index()].children.iter().rev().copied());
    }

    for id in preorder {
        let original_parent = world.boxes[id.index()]
            .parent
            .expect("every non-root layout box has a box-tree parent");
        let inline_owner = world.boxes[id.index()].inline_context_owner;
        let is_flattened = world.boxes[id.index()].inline_flattened;
        let is_positioned = world.boxes[id.index()].style.is_absolute_positioned()
            || world.boxes[id.index()].style.is_fixed_positioned();
        let structural_parent = Some(
            world.boxes[id.index()]
                .structural_parent
                .expect("every non-root box has a structural parent"),
        );
        let positioned_containing_block = if world.boxes[id.index()].style.is_fixed_positioned() {
            nearest_fixed_containing_block(world, structural_parent)
        } else if world.boxes[id.index()].style.is_absolute_positioned() {
            nearest_positioned_ancestor(world, structural_parent)
        } else {
            None
        };
        let layout_parent = if world.boxes[id.index()].outside_list_marker {
            nearest_list_item_ancestor(world, Some(original_parent))
        } else if inline_owner.is_some() && !is_positioned {
            // Floats leave normal flow, but their placement and final rounding
            // are still owned by the IFC that consumed their inline item. Only
            // absolute/fixed descendants bypass that owner for a containing
            // block selected from the construction tree.
            inline_owner
        } else if is_positioned {
            positioned_containing_block.and_then(|containing_block| {
                let containing_box = &world.boxes[containing_block.index()];
                containing_box
                    .inline_flattened
                    .then_some(containing_box.inline_context_owner)
                    .flatten()
                    .or(Some(containing_block))
            })
        } else {
            Some(original_parent)
        };
        let needs_static_position = is_positioned
            && layout_parent != Some(original_parent)
            && world.boxes[id.index()].style.has_auto_inset_axis();
        if needs_static_position {
            if inline_owner.is_some() {
                // The IFC's Parley item stream emits this candidate after line
                // breaking, when its exact hypothetical position is known.
            } else if original_parent_emits_static_position(world, original_parent) {
                let insertion_index = world.boxes[original_parent.index()].layout_children.len();
                world.boxes[original_parent.index()]
                    .out_of_flow_candidates
                    .push(OutOfFlowCandidateChild {
                        child: id,
                        insertion_index,
                    });
            } else {
                push_layout_diagnostic(
                    world,
                    id,
                    LayoutCapabilityDiagnostic::PositionedStaticPositionDeferred,
                );
            }
        }
        world.boxes[id.index()].positioned_containing_block = positioned_containing_block;
        world.boxes[id.index()].layout_parent = layout_parent;
        // Text, line breaks and structural inline boxes are represented by the
        // owner's single Parley item stream. Atomic inline boxes remain real
        // Taffy children so they can be measured before line breaking.
        // Outside markers need the list item as their coordinate parent, but
        // they are not normal-flow numeric children: including one here
        // would give it a block row and increase the list item's height
        // before the dedicated marker placement pass moves it into the
        // marker gutter.
        if (!is_flattened || is_positioned) && !world.boxes[id.index()].outside_list_marker {
            if let Some(parent) = layout_parent {
                world.boxes[parent.index()].layout_children.push(id);
            } else {
                world.viewport_layout.children.push(id);
            }
        }
    }

    // Taffy 0.12 intentionally has no CSS `order` field. Blitz performs the
    // same stable order-modified document-order sort before handing flex/grid
    // children to Taffy. Keep the source/paint tree untouched.
    for parent_index in 0..world.boxes.len() {
        let display = world.boxes[parent_index].style.display();
        if !world.boxes[parent_index]
            .style
            .uses_flex_formatting_context()
            && !display.is_grid_container()
        {
            continue;
        }
        let mut children = std::mem::take(&mut world.boxes[parent_index].layout_children);
        children.sort_by_key(|child| world.boxes[child.index()].style.order());
        world.boxes[parent_index].layout_children = children;
    }
}

fn original_parent_emits_static_position<N>(world: &LayoutWorld<N>, parent: LayoutBoxId) -> bool
where
    N: Copy + Debug + Eq + Hash,
{
    let parent = &world.boxes[parent.index()];
    !parent.inline_formatting_context
        && !matches!(
            parent.kind,
            LayoutBoxKind::TableWrapper
                | LayoutBoxKind::InlineTableWrapper
                | LayoutBoxKind::AnonymousTableWrapper
        )
}

fn box_is_effectively_floated<N>(world: &LayoutWorld<N>, id: LayoutBoxId) -> bool
where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &world.boxes[id.index()];
    if !layout_box.style.is_floated()
        || layout_box.style.is_absolute_positioned()
        || layout_box.style.is_fixed_positioned()
    {
        return false;
    }

    layout_box.layout_parent.is_some_and(|parent| {
        let parent_display = world.boxes[parent.index()].style.display();
        !parent_display.is_flex_container() && !parent_display.is_grid_container()
    })
}

fn nearest_list_item_ancestor<N>(
    world: &LayoutWorld<N>,
    mut candidate: Option<LayoutBoxId>,
) -> Option<LayoutBoxId>
where
    N: Copy + Debug + Eq + Hash,
{
    while let Some(id) = candidate {
        if world.boxes[id.index()].style.display().is_list_item() {
            return Some(id);
        }
        candidate = world.boxes[id.index()].parent;
    }
    None
}

fn finish_outside_list_markers<N>(world: &mut LayoutWorld<N>)
where
    N: Copy + Debug + Eq + Hash,
{
    let markers = (0..world.boxes.len())
        .map(LayoutBoxId::from_index)
        .filter(|id| world.boxes[id.index()].outside_list_marker)
        .collect::<Vec<_>>();
    for marker in markers {
        let Some(item) = world.boxes[marker.index()].layout_parent else {
            continue;
        };
        let item_layout = world.boxes[item.index()].unrounded_layout;
        let parent_size = Size {
            width: (item_layout.size.width
                - item_layout.border.left
                - item_layout.border.right
                - item_layout.padding.left
                - item_layout.padding.right)
                .max(0.0),
            height: (item_layout.size.height
                - item_layout.border.top
                - item_layout.border.bottom
                - item_layout.padding.top
                - item_layout.padding.bottom)
                .max(0.0),
        };
        let parent_writing_mode = world.boxes[item.index()].style.writing_mode();
        let percentage_basis = parent_writing_mode.to_logical(parent_size).inline_size;
        let inputs = LayoutInput {
            known_dimensions: Size::NONE,
            definite_dimensions: Size::NONE,
            parent_size: parent_size.map(Some),
            parent_writing_mode,
            available_space: Size {
                width: AvailableSpace::MaxContent,
                height: AvailableSpace::MaxContent,
            },
            sizing_mode: SizingMode::InherentSize,
            sizing_purpose: SizingPurpose::Layout,
            run_mode: RunMode::PerformLayout,
            axis: taffy::RequestedAxis::Both,
            inline_auto_behavior: AutoSizeBehavior::FitContent,
            block_auto_behavior: AutoSizeBehavior::FitContent,
            block_margins_are_collapsible: Line::FALSE,
        };
        let output = world.compute_child_layout(marker.to_taffy(), inputs);
        let marker_style = &world.boxes[marker.index()].style;
        let marker_margin = marker_style
            .taffy
            .margin
            .resolve_or_zero(Some(percentage_basis), resolve_stylo_calc_value);
        let gap = marker_style.font_size() * 0.5;
        let item_style = &world.boxes[item.index()].style;
        let writing_direction = item_style.writing_direction();
        let coordinates = InlineCoordinateSpace::new(parent_writing_mode);
        let logical_marker_size = coordinates.to_logical_size(output.size);
        let logical_marker_margin = writing_direction.to_logical_box_strut(marker_margin);
        let item_baseline = world.boxes[item.index()]
            .inline_layout
            .as_ref()
            .and_then(|context| context.line_placements.first())
            .map(|line| coordinates.to_physical_line_baseline(line, parent_size));
        let marker_baseline = if output.first_baselines == Point::NONE {
            coordinates.to_physical_line_block_baseline(
                Some(synthesized_font_ascent(
                    item_style.font_baseline(),
                    logical_marker_size.block_size,
                )),
                output.size,
            )
        } else {
            output.first_baselines
        };
        // Chromium keeps an outside marker as a logical fragment: it is
        // placed before the content's inline-start and baseline-aligned on
        // the block axis. Converting that one offset handles horizontal RTL
        // and both vertical modes without separate physical x/y branches.
        let mut relative_location = writing_direction.converter(parent_size).to_physical_point(
            LogicalOffset {
                inline_offset: -logical_marker_size.inline_size
                    - logical_marker_margin.inline_end
                    - gap,
                block_offset: 0.0,
            },
            output.size,
        );
        if parent_writing_mode.is_horizontal() {
            if let (Some(item), Some(marker)) = (
                item_baseline.and_then(|baseline| baseline.y),
                marker_baseline.y,
            ) {
                relative_location.y = item - marker;
            }
        } else if let (Some(item), Some(marker)) = (
            item_baseline.and_then(|baseline| baseline.x),
            marker_baseline.x,
        ) {
            relative_location.x = item - marker;
        }
        let content_origin = Point {
            x: item_layout.border.left + item_layout.padding.left,
            y: item_layout.border.top + item_layout.padding.top,
        };
        world.set_inline_child_layout(
            marker,
            Point {
                x: content_origin.x + relative_location.x,
                y: content_origin.y + relative_location.y,
            },
            output,
            marker.index(),
            Some(percentage_basis),
        );
    }
}

fn finish_form_control_contents<N>(world: &mut LayoutWorld<N>)
where
    N: Copy + Debug + Eq + Hash,
{
    let contents = (0..world.boxes.len())
        .map(LayoutBoxId::from_index)
        .filter(|id| {
            world.boxes[id.index()].anonymous_reason
                == Some(crate::LayoutAnonymousReason::FormControlContent)
                && world.boxes[id.index()].kind == LayoutBoxKind::AnonymousBlock
        })
        .collect::<Vec<_>>();
    for content in contents {
        let Some(control) = world.boxes[content.index()].layout_parent else {
            continue;
        };
        // Replaced controls do not run a child formatting algorithm, so their
        // browser-generated label is positioned after the atomic box has been
        // sized. Content-bearing controls such as menu-list selects own a real
        // formatting context and have already laid this child out normally.
        if !world.boxes[control.index()].is_replaced() {
            continue;
        }
        let control_layout = world.boxes[control.index()].unrounded_layout;
        let content_size = Size {
            width: (control_layout.size.width
                - control_layout.border.left
                - control_layout.border.right
                - control_layout.padding.left
                - control_layout.padding.right
                - 8.0)
                .max(0.0),
            height: (control_layout.size.height
                - control_layout.border.top
                - control_layout.border.bottom
                - control_layout.padding.top
                - control_layout.padding.bottom)
                .max(0.0),
        };
        let content_width = content_size.width;
        let parent_writing_mode = world.boxes[control.index()].style.writing_mode();
        let percentage_basis = parent_writing_mode.to_logical(content_size).inline_size;
        let inputs = LayoutInput {
            known_dimensions: Size::NONE,
            definite_dimensions: Size::NONE,
            parent_size: content_size.map(Some),
            parent_writing_mode,
            available_space: Size {
                width: AvailableSpace::Definite(content_width),
                height: AvailableSpace::MaxContent,
            },
            sizing_mode: SizingMode::InherentSize,
            sizing_purpose: SizingPurpose::Layout,
            run_mode: RunMode::PerformLayout,
            axis: taffy::RequestedAxis::Both,
            inline_auto_behavior: AutoSizeBehavior::StretchImplicit,
            block_auto_behavior: AutoSizeBehavior::FitContent,
            block_margins_are_collapsible: Line::FALSE,
        };
        let output = world.compute_child_layout(content.to_taffy(), inputs);
        let x = control_layout.border.left + control_layout.padding.left + 4.0;
        let y = ((control_layout.size.height - output.size.height) * 0.5)
            .max(control_layout.border.top + control_layout.padding.top);
        world.set_inline_child_layout(
            content,
            Point { x, y },
            output,
            content.index(),
            Some(percentage_basis),
        );
    }
}

fn finish_sticky_positioning<N>(world: &mut LayoutWorld<N>, viewport: PaintViewport)
where
    N: Copy + Debug + Eq + Hash,
{
    let sticky_boxes = (0..world.boxes.len())
        .map(LayoutBoxId::from_index)
        .filter(|id| world.boxes[id.index()].style.position() == crate::LayoutPosition::Sticky)
        .collect::<Vec<_>>();
    for id in sticky_boxes {
        let layout = world.boxes[id.index()].unrounded_layout;
        let global = unrounded_global_origin(world, id);
        let scrollport = nearest_scrollport(world, id).unwrap_or(PaintRect::new(
            0.0,
            0.0,
            viewport.css_width as f32,
            viewport.css_height as f32,
        ));
        let inset = world.boxes[id.index()].style.sticky_inset();
        let left = inset
            .left
            .maybe_resolve(scrollport.width, resolve_stylo_calc_value);
        let right = inset
            .right
            .maybe_resolve(scrollport.width, resolve_stylo_calc_value);
        let top = inset
            .top
            .maybe_resolve(scrollport.height, resolve_stylo_calc_value);
        let bottom = inset
            .bottom
            .maybe_resolve(scrollport.height, resolve_stylo_calc_value);
        let mut target_x = global.x;
        let mut target_y = global.y;
        if let Some(left) = left {
            target_x = target_x.max(scrollport.x + left);
        }
        if let Some(right) = right {
            target_x = target_x.min(scrollport.x + scrollport.width - right - layout.size.width);
        }
        if let Some(top) = top {
            target_y = target_y.max(scrollport.y + top);
        }
        if let Some(bottom) = bottom {
            target_y = target_y.min(scrollport.y + scrollport.height - bottom - layout.size.height);
        }

        if let Some(containing_block) = world.boxes[id.index()].layout_parent {
            let containing_layout = world.boxes[containing_block.index()].unrounded_layout;
            let containing_origin = unrounded_global_origin(world, containing_block);
            let min_x = containing_origin.x
                + containing_layout.border.left
                + containing_layout.padding.left;
            let max_x = containing_origin.x + containing_layout.size.width
                - containing_layout.border.right
                - containing_layout.padding.right
                - layout.size.width;
            let min_y =
                containing_origin.y + containing_layout.border.top + containing_layout.padding.top;
            let max_y = containing_origin.y + containing_layout.size.height
                - containing_layout.border.bottom
                - containing_layout.padding.bottom
                - layout.size.height;
            if min_x <= max_x {
                target_x = target_x.clamp(min_x, max_x);
            }
            if min_y <= max_y {
                target_y = target_y.clamp(min_y, max_y);
            }
        }
        world.boxes[id.index()].unrounded_layout.location.x += target_x - global.x;
        world.boxes[id.index()].unrounded_layout.location.y += target_y - global.y;
    }
}

fn nearest_scrollport<N>(world: &LayoutWorld<N>, id: LayoutBoxId) -> Option<PaintRect>
where
    N: Copy + Debug + Eq + Hash,
{
    let mut ancestor = world.boxes[id.index()].parent;
    while let Some(candidate) = ancestor {
        let layout_box = &world.boxes[candidate.index()];
        if world.establishes_scroll_container(candidate) {
            let layout = layout_box.unrounded_layout;
            let origin = unrounded_global_origin(world, candidate);
            return Some(PaintRect::new(
                origin.x + layout.border.left,
                origin.y + layout.border.top,
                (layout.size.width - layout.border.left - layout.border.right).max(0.0),
                (layout.size.height - layout.border.top - layout.border.bottom).max(0.0),
            ));
        }
        ancestor = layout_box.parent;
    }
    None
}

fn push_layout_diagnostic<N>(
    world: &mut LayoutWorld<N>,
    id: LayoutBoxId,
    diagnostic: LayoutCapabilityDiagnostic,
) where
    N: Copy + Debug + Eq + Hash,
{
    let diagnostics = &mut world.boxes[id.index()].capability_diagnostics;
    if !diagnostics.contains(&diagnostic) {
        diagnostics.push(diagnostic);
    }
}

fn nearest_positioned_ancestor<N>(
    world: &LayoutWorld<N>,
    mut candidate: Option<LayoutBoxId>,
) -> Option<LayoutBoxId>
where
    N: Copy + Debug + Eq + Hash,
{
    while let Some(id) = candidate {
        if world.establishes_positioned_containing_block(id) {
            return Some(id);
        }
        candidate = world.boxes[id.index()].structural_parent;
    }
    None
}

fn nearest_fixed_containing_block<N>(
    world: &LayoutWorld<N>,
    mut candidate: Option<LayoutBoxId>,
) -> Option<LayoutBoxId>
where
    N: Copy + Debug + Eq + Hash,
{
    while let Some(id) = candidate {
        if world.establishes_fixed_containing_block(id) {
            return Some(id);
        }
        candidate = world.boxes[id.index()].structural_parent;
    }
    None
}

fn inline_box_containing_rect<N>(
    world: &LayoutWorld<N>,
    owner: LayoutBoxId,
    containing_block: LayoutBoxId,
    owner_content_size: Size<f32>,
) -> Option<PaintRect>
where
    N: Copy + Debug + Eq + Hash,
{
    let context = world.boxes[owner.index()].inline_layout.as_ref()?;
    let containing_box = &world.boxes[containing_block.index()];
    let containing_inline_size = world.boxes[owner.index()]
        .style
        .writing_mode()
        .to_logical(owner_content_size)
        .inline_size;
    let style = &containing_box.style;
    let margin = style
        .taffy
        .margin
        .resolve_or_zero(Some(containing_inline_size), resolve_stylo_calc_value);
    let padding = style
        .taffy
        .padding
        .resolve_or_zero(Some(containing_inline_size), resolve_stylo_calc_value);
    let border = style
        .taffy
        .border
        .resolve_or_zero(Some(containing_inline_size), resolve_stylo_calc_value);
    let mut fragment_rects = context
        .fragments
        .boxes
        .iter()
        .filter(|fragment| fragment.box_id == containing_block)
        .map(|fragment| {
            let geometry = crate::inline::inline_fragment_box_geometry(
                fragment,
                style.writing_direction(),
                margin,
                padding,
                border,
            );
            (fragment.line_index, geometry.border_rect)
        })
        .collect::<Vec<_>>();
    fragment_rects.sort_by_key(|(line_index, _)| *line_index);

    let start_line = fragment_rects.first()?.0;
    let start_rect = fragment_rects
        .iter()
        .take_while(|(line_index, _)| *line_index == start_line)
        .map(|(_, rect)| *rect)
        .reduce(union_paint_rect)?;
    // Blink keeps the previous end fragment when a later fragment belongs to
    // an empty line box. Moli's phantom line placement is the equivalent used
    // line-box state.
    let end_line = fragment_rects
        .iter()
        .rev()
        .find(|(line_index, _)| {
            context
                .line_placements
                .get(*line_index)
                .is_none_or(|line| !line.phantom)
        })
        .map_or(start_line, |(line_index, _)| *line_index);
    let end_rect = fragment_rects
        .iter()
        .filter(|(line_index, _)| *line_index == end_line)
        .map(|(_, rect)| *rect)
        .reduce(union_paint_rect)?;

    // Match Blink's InlineContainingBlockUtils: the logical start comes from
    // the first fragment, the logical end from the last non-empty fragment,
    // and the border edges are inset to produce the padding-box containing
    // block. Opposite inline directions retain their physical inline edges.
    let owner_direction = world.boxes[owner.index()].style.writing_direction();
    let inline_direction = style.writing_direction();
    debug_assert_eq!(owner_direction.mode, inline_direction.mode);
    let converter = owner_direction.converter(owner_content_size);
    let start_size = Size {
        width: start_rect.width,
        height: start_rect.height,
    };
    let end_size = Size {
        width: end_rect.width,
        height: end_rect.height,
    };
    let mut start = converter.to_logical_point(
        Point {
            x: start_rect.x,
            y: start_rect.y,
        },
        start_size,
    );
    let mut end = converter.to_logical_point(
        Point {
            x: end_rect.x,
            y: end_rect.y,
        },
        end_size,
    );
    let end_size = converter.to_logical_size(end_size);
    end.inline_offset += end_size.inline_size;
    end.block_offset += end_size.block_size;

    let logical_border = inline_direction.to_logical_box_strut(border);
    start.block_offset += logical_border.block_start;
    end.block_offset -= logical_border.block_end;
    if owner_direction == inline_direction {
        start.inline_offset += logical_border.inline_start;
        end.inline_offset -= logical_border.inline_end;
    }
    end.inline_offset = end.inline_offset.max(start.inline_offset);
    end.block_offset = end.block_offset.max(start.block_offset);
    let logical_size = LogicalSize {
        inline_size: end.inline_offset - start.inline_offset,
        block_size: end.block_offset - start.block_offset,
    };
    let physical_size = converter.to_physical_size(logical_size);
    let physical_offset = converter.to_physical_point(start, physical_size);
    Some(PaintRect::new(
        physical_offset.x,
        physical_offset.y,
        physical_size.width,
        physical_size.height,
    ))
}

fn union_paint_rect(left: PaintRect, right: PaintRect) -> PaintRect {
    let min_x = left.x.min(right.x);
    let min_y = left.y.min(right.y);
    let max_x = (left.x + left.width).max(right.x + right.width);
    let max_y = (left.y + left.height).max(right.y + right.height);
    PaintRect::new(
        min_x,
        min_y,
        (max_x - min_x).max(0.0),
        (max_y - min_y).max(0.0),
    )
}

fn unrounded_global_origin<N>(world: &LayoutWorld<N>, id: LayoutBoxId) -> Point<f32>
where
    N: Copy + Debug + Eq + Hash,
{
    let mut origin = Point::ZERO;
    let mut current = Some(id);
    while let Some(box_id) = current {
        let layout_box = &world.boxes[box_id.index()];
        origin.x += layout_box.unrounded_layout.location.x;
        origin.y += layout_box.unrounded_layout.location.y;
        current = layout_box.layout_parent;
    }
    origin
}

pub struct ChildIter<'a>(std::slice::Iter<'a, LayoutBoxId>);

impl Iterator for ChildIter<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().copied().map(LayoutBoxId::to_taffy)
    }
}

impl<N> TraversePartialTree for LayoutWorld<N>
where
    N: Copy + Debug + Eq + Hash,
{
    type ChildIter<'a>
        = ChildIter<'a>
    where
        Self: 'a;

    fn child_ids(&self, parent_node_id: NodeId) -> Self::ChildIter<'_> {
        if self.is_viewport_taffy_node(parent_node_id) {
            ChildIter(self.viewport_layout.children.iter())
        } else {
            ChildIter(
                self.boxes[LayoutBoxId::from_taffy(parent_node_id).index()]
                    .layout_children
                    .iter(),
            )
        }
    }

    fn child_count(&self, parent_node_id: NodeId) -> usize {
        if self.is_viewport_taffy_node(parent_node_id) {
            self.viewport_layout.children.len()
        } else {
            self.boxes[LayoutBoxId::from_taffy(parent_node_id).index()]
                .layout_children
                .len()
        }
    }

    fn get_child_id(&self, parent_node_id: NodeId, child_index: usize) -> NodeId {
        if self.is_viewport_taffy_node(parent_node_id) {
            self.viewport_layout.children[child_index].to_taffy()
        } else {
            self.boxes[LayoutBoxId::from_taffy(parent_node_id).index()].layout_children[child_index]
                .to_taffy()
        }
    }
}

impl<N> TraverseTree for LayoutWorld<N> where N: Copy + Debug + Eq + Hash {}

impl<N> LayoutPartialTree for LayoutWorld<N>
where
    N: Copy + Debug + Eq + Hash,
{
    type CoreContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type CustomIdent = Atom;

    fn get_core_container_style(&self, node_id: NodeId) -> Self::CoreContainerStyle<'_> {
        if self.is_viewport_taffy_node(node_id) {
            &self.viewport_layout.style
        } else {
            &self.boxes[LayoutBoxId::from_taffy(node_id).index()]
                .style
                .taffy
        }
    }

    fn get_writing_mode(&self, node_id: NodeId) -> taffy::WritingMode {
        if self.is_viewport_taffy_node(node_id) {
            self.viewport_layout.writing_mode
        } else {
            self.boxes[LayoutBoxId::from_taffy(node_id).index()]
                .style
                .writing_mode()
        }
    }

    fn get_font_baseline(&self, node_id: NodeId) -> FontBaseline {
        if self.is_viewport_taffy_node(node_id) {
            FontBaseline::from_writing_mode(self.viewport_layout.writing_mode)
        } else {
            self.boxes[LayoutBoxId::from_taffy(node_id).index()]
                .style
                .font_baseline()
        }
    }

    fn get_resolved_aspect_ratio(&self, node_id: NodeId) -> taffy::ResolvedAspectRatio {
        if self.is_viewport_taffy_node(node_id) {
            return taffy::ResolvedAspectRatio {
                ratio: self.viewport_layout.style.aspect_ratio,
                box_sizing: self.viewport_layout.style.box_sizing,
            };
        }
        self.boxes[LayoutBoxId::from_taffy(node_id).index()].resolved_aspect_ratio()
    }

    fn get_size_containment(&self, node_id: NodeId) -> taffy::SizeContainment {
        if self.is_viewport_taffy_node(node_id) {
            taffy::SizeContainment::NONE
        } else {
            self.boxes[LayoutBoxId::from_taffy(node_id).index()].used_size_containment()
        }
    }

    fn get_layout_environment(&self) -> taffy::LayoutEnvironment {
        self.layout_environment
    }

    fn establishes_new_formatting_context(&self, node_id: NodeId) -> bool {
        !self.is_viewport_taffy_node(node_id) && LayoutBoxId::from_taffy(node_id) == self.root
    }

    fn should_stretch_auto_inline_size_in_block_container(&self, node_id: NodeId) -> bool {
        if self.is_viewport_taffy_node(node_id) {
            return true;
        }
        !matches!(
            self.boxes[LayoutBoxId::from_taffy(node_id).index()]
                .element_semantics
                .as_ref()
                .map(|semantics| semantics.category),
            Some(crate::LayoutElementCategory::FormControl(
                crate::LayoutFormControlKind::Button
                    | crate::LayoutFormControlKind::Input(_)
                    | crate::LayoutFormControlKind::Select
                    | crate::LayoutFormControlKind::TextArea
            ))
        )
    }

    fn prepare_child_layout_input(&self, node_id: NodeId, inputs: LayoutInput) -> LayoutInput {
        let writing_mode = self.get_writing_mode(node_id);
        let mut inputs = inputs.for_child_writing_mode(writing_mode, self.layout_environment);
        if !self.is_viewport_taffy_node(node_id) {
            let layout_box_id = LayoutBoxId::from_taffy(node_id);
            if !self.is_quirky_viewport_filler(layout_box_id) {
                return inputs;
            }
            // HTML's quirks viewport filler remains content-sized. Definite
            // available block space floors its real intrinsic block size and
            // participates in initial percentage geometry through the normal
            // ratio/min/max order, matching Blink's CalculateDefaultBlockSize
            // and ClampIntrinsicBlockSize stages.
            inputs.block_auto_behavior = AutoSizeBehavior::FitContentWithAvailableIntrinsicFloor;
        }
        inputs
    }

    fn resolve_calc_value(&self, value: *const (), basis: f32) -> f32 {
        resolve_stylo_calc_value(value, basis)
    }

    fn set_unrounded_layout(&mut self, node_id: NodeId, layout: &Layout) {
        if self.is_viewport_taffy_node(node_id) {
            self.viewport_layout.unrounded_layout = *layout;
        } else {
            self.boxes[LayoutBoxId::from_taffy(node_id).index()].unrounded_layout = *layout;
        }
    }

    fn set_out_of_flow_static_position(
        &mut self,
        container_node_id: NodeId,
        child_node_id: NodeId,
        static_position: LogicalStaticPosition,
    ) {
        if self.is_viewport_taffy_node(container_node_id)
            || self.is_viewport_taffy_node(child_node_id)
        {
            return;
        }
        let container = LayoutBoxId::from_taffy(container_node_id);
        let child = LayoutBoxId::from_taffy(child_node_id);
        let emitted_by_original_context = self.boxes[child.index()].parent == Some(container);
        if self.boxes[child.index()]
            .out_of_flow_static_position
            .is_some()
            && !emitted_by_original_context
        {
            return;
        }
        self.boxes[child.index()].out_of_flow_static_position = Some(OutOfFlowStaticPosition {
            owner: container,
            position: static_position,
        });
    }

    fn get_out_of_flow_static_position(
        &self,
        containing_block_node_id: NodeId,
        child_node_id: NodeId,
        containing_block_size: Size<f32>,
        containing_block_writing_direction: WritingDirection,
    ) -> Option<LogicalStaticPosition> {
        if self.is_viewport_taffy_node(child_node_id) {
            return None;
        }
        let child = LayoutBoxId::from_taffy(child_node_id);
        let candidate = self.boxes[child.index()].out_of_flow_static_position?;
        let owner = &self.boxes[candidate.owner.index()];
        let owner_writing_direction =
            WritingDirection::new(owner.style.writing_mode(), owner.style.taffy.direction);
        let owner_size = if !self.is_viewport_taffy_node(containing_block_node_id)
            && candidate.owner == LayoutBoxId::from_taffy(containing_block_node_id)
        {
            // The containing formatting context may still be computing, so
            // its staged Layout has not necessarily received the current
            // border-box size yet.
            containing_block_size
        } else {
            owner.unrounded_layout.size
        };
        let mut physical = candidate
            .position
            .to_physical(owner_writing_direction, owner_size);
        let owner_origin = unrounded_global_origin(self, candidate.owner);
        let containing_block_origin = if self.is_viewport_taffy_node(containing_block_node_id) {
            Point::ZERO
        } else {
            unrounded_global_origin(self, LayoutBoxId::from_taffy(containing_block_node_id))
        };
        physical.offset.x += owner_origin.x - containing_block_origin.x;
        physical.offset.y += owner_origin.y - containing_block_origin.y;
        Some(physical.to_logical(containing_block_writing_direction, containing_block_size))
    }

    fn is_out_of_flow_containing_block(
        &self,
        container_node_id: NodeId,
        child_node_id: NodeId,
    ) -> bool {
        if self.is_viewport_taffy_node(child_node_id) {
            return false;
        }
        let layout_parent =
            self.boxes[LayoutBoxId::from_taffy(child_node_id).index()].layout_parent;
        if self.is_viewport_taffy_node(container_node_id) {
            layout_parent.is_none()
        } else {
            layout_parent == Some(LayoutBoxId::from_taffy(container_node_id))
        }
    }

    fn is_out_of_flow_direct_child(
        &self,
        container_node_id: NodeId,
        child_node_id: NodeId,
    ) -> bool {
        if self.is_viewport_taffy_node(container_node_id)
            || self.is_viewport_taffy_node(child_node_id)
        {
            return false;
        }
        self.boxes[LayoutBoxId::from_taffy(child_node_id).index()].parent
            == Some(LayoutBoxId::from_taffy(container_node_id))
    }

    fn out_of_flow_candidate_count(&self, container_node_id: NodeId) -> usize {
        if self.is_viewport_taffy_node(container_node_id) {
            0
        } else {
            self.boxes[LayoutBoxId::from_taffy(container_node_id).index()]
                .out_of_flow_candidates
                .len()
        }
    }

    fn get_out_of_flow_candidate(
        &self,
        container_node_id: NodeId,
        candidate_index: usize,
    ) -> OutOfFlowCandidate {
        let candidate = self.boxes[LayoutBoxId::from_taffy(container_node_id).index()]
            .out_of_flow_candidates[candidate_index];
        OutOfFlowCandidate {
            node: candidate.child.to_taffy(),
            insertion_index: candidate.insertion_index,
        }
    }

    fn compute_child_layout(&mut self, node_id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        let inputs = self.prepare_child_layout_input(node_id, inputs);
        if self.is_viewport_taffy_node(node_id) {
            return compute_cached_layout(self, node_id, inputs, |world, node_id, inputs| {
                compute_block_layout(world, node_id, inputs, None)
            });
        }
        if self.should_hide(node_id, inputs) {
            return compute_hidden_layout(self, node_id);
        }
        compute_cached_layout(self, node_id, inputs, |world, node_id, inputs| {
            world.compute_child_layout_uncached(node_id, inputs, None)
        })
    }

    fn compute_child_size(&mut self, node_id: NodeId, inputs: LayoutInput) -> IntrinsicSizeResult {
        let inputs = self.prepare_child_layout_input(node_id, inputs);
        if self.is_viewport_taffy_node(node_id) {
            return compute_cached_size(self, node_id, inputs, |world, node_id, inputs| {
                compute_block_layout(world, node_id, inputs, None).into_intrinsic_size_result()
            });
        }
        if self.should_hide(node_id, inputs) {
            return IntrinsicSizeResult::from_size(Size::ZERO);
        }
        compute_cached_size(self, node_id, inputs, |world, node_id, inputs| {
            world.compute_child_size_uncached(node_id, inputs)
        })
    }
}

impl<N> CacheTree for LayoutWorld<N>
where
    N: Copy + Debug + Eq + Hash,
{
    fn cache_get(&self, node_id: NodeId, inputs: &LayoutInput) -> Option<LayoutOutput> {
        if self.is_viewport_taffy_node(node_id) {
            self.viewport_layout
                .cache
                .get_with_environment(inputs, self.layout_environment)
        } else {
            self.boxes[LayoutBoxId::from_taffy(node_id).index()]
                .cache
                .get_with_environment(inputs, self.layout_environment)
        }
    }

    fn cache_store(&mut self, node_id: NodeId, inputs: &LayoutInput, output: LayoutOutput) {
        if self.is_viewport_taffy_node(node_id) {
            self.viewport_layout.cache.store_with_environment(
                inputs,
                output,
                self.layout_environment,
            );
        } else {
            self.boxes[LayoutBoxId::from_taffy(node_id).index()]
                .cache
                .store_with_environment(inputs, output, self.layout_environment);
        }
    }

    fn cache_get_size(&self, node_id: NodeId, inputs: &LayoutInput) -> Option<IntrinsicSizeResult> {
        if self.is_viewport_taffy_node(node_id) {
            self.viewport_layout
                .cache
                .get_size_with_environment(inputs, self.layout_environment)
        } else {
            self.boxes[LayoutBoxId::from_taffy(node_id).index()]
                .cache
                .get_size_with_environment(inputs, self.layout_environment)
        }
    }

    fn cache_store_size(
        &mut self,
        node_id: NodeId,
        inputs: &LayoutInput,
        result: IntrinsicSizeResult,
    ) {
        if self.is_viewport_taffy_node(node_id) {
            self.viewport_layout.cache.store_size_with_environment(
                inputs,
                result,
                self.layout_environment,
            );
        } else {
            self.boxes[LayoutBoxId::from_taffy(node_id).index()]
                .cache
                .store_size_with_environment(inputs, result, self.layout_environment);
        }
    }

    fn cache_clear(&mut self, node_id: NodeId) {
        if self.is_viewport_taffy_node(node_id) {
            self.viewport_layout.cache.clear();
        } else {
            self.boxes[LayoutBoxId::from_taffy(node_id).index()]
                .cache
                .clear();
        }
    }
}

impl<N> LayoutBlockContainer for LayoutWorld<N>
where
    N: Copy + Debug + Eq + Hash,
{
    type BlockContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;

    fn get_block_container_style(&self, node_id: NodeId) -> Self::BlockContainerStyle<'_> {
        self.get_core_container_style(node_id)
    }

    fn get_block_child_style(&self, child_node_id: NodeId) -> Self::BlockItemStyle<'_> {
        self.get_core_container_style(child_node_id)
    }

    fn compute_block_child_layout(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        block_context: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        let inputs = self.prepare_child_layout_input(node_id, inputs);
        if self.should_hide(node_id, inputs) {
            return compute_hidden_layout(self, node_id);
        }
        compute_cached_layout(self, node_id, inputs, |world, node_id, inputs| {
            world.compute_child_layout_uncached(node_id, inputs, block_context)
        })
    }
}

impl<N> LayoutFlexboxContainer for LayoutWorld<N>
where
    N: Copy + Debug + Eq + Hash,
{
    type FlexboxContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, node_id: NodeId) -> Self::FlexboxContainerStyle<'_> {
        self.get_core_container_style(node_id)
    }

    fn get_flexbox_child_style(&self, child_node_id: NodeId) -> Self::FlexboxItemStyle<'_> {
        self.get_core_container_style(child_node_id)
    }
}

impl<N> LayoutGridContainer for LayoutWorld<N>
where
    N: Copy + Debug + Eq + Hash,
{
    type GridContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;

    fn get_grid_container_style(&self, node_id: NodeId) -> Self::GridContainerStyle<'_> {
        self.get_core_container_style(node_id)
    }

    fn get_grid_child_style(&self, child_node_id: NodeId) -> Self::GridItemStyle<'_> {
        self.get_core_container_style(child_node_id)
    }

    fn set_detailed_grid_info(&mut self, node_id: NodeId, detailed_grid_info: DetailedGridInfo) {
        let layout_box = &self.boxes[LayoutBoxId::from_taffy(node_id).index()];
        if layout_box
            .capability_diagnostics
            .contains(&LayoutCapabilityDiagnostic::GridTemplateModeDeferred)
        {
            return;
        }
        let Some(grid_geometry) =
            crate::grid::project_grid_geometry(&layout_box.style.taffy, detailed_grid_info)
        else {
            return;
        };
        self.boxes[LayoutBoxId::from_taffy(node_id).index()].grid_geometry = Some(grid_geometry);
    }
}

impl<N> RoundTree for LayoutWorld<N>
where
    N: Copy + Debug + Eq + Hash,
{
    fn get_unrounded_layout(&self, node_id: NodeId) -> Layout {
        if self.is_viewport_taffy_node(node_id) {
            self.viewport_layout.unrounded_layout
        } else {
            self.boxes[LayoutBoxId::from_taffy(node_id).index()].unrounded_layout
        }
    }

    fn set_final_layout(&mut self, node_id: NodeId, layout: &Layout) {
        if self.is_viewport_taffy_node(node_id) {
            self.viewport_layout.final_layout = *layout;
        } else {
            self.boxes[LayoutBoxId::from_taffy(node_id).index()].final_layout = *layout;
        }
    }
}

impl<N> LayoutWorld<N>
where
    N: Copy + Debug + Eq + Hash,
{
    fn should_hide(&self, node_id: NodeId, inputs: LayoutInput) -> bool {
        inputs.run_mode == RunMode::PerformHiddenLayout
            || self.boxes[LayoutBoxId::from_taffy(node_id).index()]
                .style
                .taffy
                .display
                == Display::None
    }

    /// Measure a native layout object's browser-provided intrinsic content at
    /// the same boundary used for an ordinary descendant-based contribution.
    ///
    /// Blink's `CalculateMinMaxSizesIgnoringChildren` takes this path for
    /// controls such as menu-list selects. Running the value through Taffy's
    /// leaf sizing pipeline is important: the value itself is a content-box
    /// size, while decorations, authored constraints, and preferred-ratio
    /// transfer remain generic box-sizing operations.
    fn compute_default_intrinsic_content_layout(
        &mut self,
        id: LayoutBoxId,
        inputs: LayoutInput,
    ) -> Option<LayoutOutput> {
        let layout_box = &self.boxes[id.index()];
        if layout_box.is_replaced() {
            return None;
        }
        let intrinsic = layout_box.default_intrinsic_content_size;
        let covers_request = match inputs.axis {
            taffy::RequestedAxis::Horizontal => intrinsic.width.is_some(),
            taffy::RequestedAxis::Vertical => intrinsic.height.is_some(),
            taffy::RequestedAxis::Both => intrinsic.width.is_some() && intrinsic.height.is_some(),
        };
        if !covers_request {
            return None;
        }
        let style = layout_box.style.taffy.clone();
        Some(compute_leaf_layout_with_tree(
            self,
            id.to_taffy(),
            inputs,
            &style,
            resolve_stylo_calc_value,
            move |_, known_dimensions, _| Size {
                width: known_dimensions.width.or(intrinsic.width).unwrap_or(0.0),
                height: known_dimensions.height.or(intrinsic.height).unwrap_or(0.0),
            },
        ))
    }

    fn compute_child_layout_uncached(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        block_context: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        let id = LayoutBoxId::from_taffy(node_id);
        let layout_box = &self.boxes[id.index()];
        let kind = layout_box.kind;
        let display = layout_box.style.display();
        let uses_flex_formatting_context = layout_box.style.uses_flex_formatting_context();
        let inline_formatting_context = layout_box.inline_formatting_context;
        let is_replaced = layout_box.is_replaced();

        if inputs.sizing_mode == SizingMode::ContentSize
            && let Some(output) = self.compute_default_intrinsic_content_layout(id, inputs)
        {
            return output;
        }

        if is_replaced {
            return self.compute_leaf(id, inputs);
        }

        if inline_formatting_context {
            return self.compute_inline_formatting_context(id, inputs, block_context);
        }

        // Pseudo origins retain a pseudo-specific box kind, so their computed
        // display cannot be recovered from the kind. Dispatch their formatting
        // context exactly like a principal box. Table remains the explicit
        // conservative block fallback until its dedicated numeric phase.
        if uses_flex_formatting_context {
            return compute_flexbox_layout(self, node_id, inputs);
        }
        if display.is_grid_container() {
            return compute_grid_layout(self, node_id, inputs);
        }
        if matches!(
            kind,
            LayoutBoxKind::TableWrapper
                | LayoutBoxKind::InlineTableWrapper
                | LayoutBoxKind::AnonymousTableWrapper
        ) {
            return compute_table_layout(self, id, inputs);
        }
        if display.is_table() {
            return compute_block_layout(self, node_id, inputs, block_context);
        }

        match kind {
            LayoutBoxKind::PrincipalFlex | LayoutBoxKind::PrincipalInlineFlex => {
                compute_flexbox_layout(self, node_id, inputs)
            }
            LayoutBoxKind::PrincipalGrid | LayoutBoxKind::PrincipalInlineGrid => {
                compute_grid_layout(self, node_id, inputs)
            }
            LayoutBoxKind::PrincipalBlock
            | LayoutBoxKind::PrincipalFlowRoot
            | LayoutBoxKind::PrincipalInlineBlock
            | LayoutBoxKind::ListItem
            | LayoutBoxKind::InlineListItem
            | LayoutBoxKind::TableWrapper
            | LayoutBoxKind::InlineTableWrapper
            | LayoutBoxKind::TableCaption
            | LayoutBoxKind::TableRowGroup
            | LayoutBoxKind::TableHeaderGroup
            | LayoutBoxKind::TableFooterGroup
            | LayoutBoxKind::TableColumnGroup
            | LayoutBoxKind::TableRow
            | LayoutBoxKind::TableCell
            | LayoutBoxKind::FormControl
            | LayoutBoxKind::ImageFallback
            | LayoutBoxKind::AnonymousBlock
            | LayoutBoxKind::AnonymousFlexItem
            | LayoutBoxKind::AnonymousGridItem
            | LayoutBoxKind::AnonymousTableWrapper
            | LayoutBoxKind::AnonymousTableRowGroup
            | LayoutBoxKind::AnonymousTableRow
            | LayoutBoxKind::AnonymousTableCell => {
                compute_block_layout(self, node_id, inputs, block_context)
            }
            LayoutBoxKind::PrincipalInline
            | LayoutBoxKind::InlineContinuation
            | LayoutBoxKind::TableColumn
            | LayoutBoxKind::Text
            | LayoutBoxKind::LineBreak
            | LayoutBoxKind::PseudoMarker
            | LayoutBoxKind::PseudoBefore
            | LayoutBoxKind::PseudoAfter
            | LayoutBoxKind::Replaced => self.compute_leaf(id, inputs),
        }
    }

    fn compute_child_size_uncached(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
    ) -> IntrinsicSizeResult {
        let id = LayoutBoxId::from_taffy(node_id);
        if inputs.sizing_purpose == SizingPurpose::IntrinsicContribution
            && let Some(output) = self.compute_default_intrinsic_content_layout(id, inputs)
        {
            return output.into_intrinsic_size_result();
        }
        if self.boxes[id.index()].inline_formatting_context {
            return self
                .compute_inline_formatting_context(id, inputs, None)
                .into_intrinsic_size_result();
        }
        self.compute_child_layout_uncached(node_id, inputs, None)
            .into_intrinsic_size_result()
    }

    fn compute_leaf(&mut self, id: LayoutBoxId, inputs: LayoutInput) -> LayoutOutput {
        let layout_box = &self.boxes[id.index()];
        let style = layout_box.style.taffy.clone();
        let text = layout_box.text.clone();
        let font_size = layout_box.style.font_size();
        let line_height = layout_box.style.line_height();
        let writing_mode = layout_box.style.writing_mode();
        let replaced_context = layout_box.replaced_context;
        let resolved_aspect_ratio = layout_box.resolved_aspect_ratio();
        let size_containment = layout_box.used_size_containment();

        if let Some(context) = replaced_context {
            return compute_replaced_layout(
                inputs,
                &style,
                context.sizing_context(writing_mode, resolved_aspect_ratio, size_containment),
                resolve_stylo_calc_value,
            );
        }

        compute_leaf_layout_with_tree(
            self,
            id.to_taffy(),
            inputs,
            &style,
            resolve_stylo_calc_value,
            |_, known_dimensions, available_space| {
                if let Some(text) = text.as_deref() {
                    measure_text(
                        text,
                        font_size,
                        line_height,
                        known_dimensions,
                        available_space,
                    )
                } else {
                    Size {
                        width: known_dimensions.width.unwrap_or(0.0),
                        height: known_dimensions.height.unwrap_or(0.0),
                    }
                }
            },
        )
    }

    fn compute_inline_formatting_context(
        &mut self,
        id: LayoutBoxId,
        inputs: LayoutInput,
        block_context: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        let style = self.boxes[id.index()].style.taffy.clone();
        let writing_mode = self.boxes[id.index()].style.writing_mode();
        let writing_direction = self.boxes[id.index()].style.writing_direction();
        let inline_coordinates = InlineCoordinateSpace::new(writing_mode);
        let is_floated = box_is_effectively_floated(self, id);
        let percentage_basis = box_model_percentage_basis(inputs, writing_mode);
        let alignment = self.boxes[id.index()].style.resolved_text_align();
        // Resolving intrinsic sizing keywords may re-enter this same IFC in
        // `ContentSize` mode. Keep its mutable content in the world until
        // Taffy actually invokes the content measurer; taking it before the
        // tree-owned sizing pass would make that nested probe observe an empty
        // formatting context.
        let mut inline_context = None;
        let mut measurement = None;
        let mut output = compute_leaf_layout_with_tree(
            self,
            id.to_taffy(),
            inputs,
            &style,
            resolve_stylo_calc_value,
            |world, known_dimensions, available_space| {
                let inline_context = inline_context.get_or_insert_with(|| {
                    world.boxes[id.index()]
                        .inline_layout
                        .take()
                        .unwrap_or_else(empty_inline_context)
                });
                let result = world.measure_inline_context(
                    id,
                    inputs,
                    known_dimensions,
                    available_space,
                    alignment,
                    inline_context,
                    is_floated,
                    block_context,
                );
                let size = result.size;
                measurement = Some(result);
                size
            },
        );
        let padding = style
            .padding
            .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
        let border = style
            .border
            .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);

        let mut depends_on_block_constraints = false;
        if let (Some(mut measurement), Some(inline_context)) =
            (measurement, inline_context.as_mut())
        {
            depends_on_block_constraints = measurement.depends_on_block_constraints;
            let content_box_size = Size {
                width: (output.size.width
                    - padding.left
                    - padding.right
                    - border.left
                    - border.right)
                    .max(0.0),
                height: (output.size.height
                    - padding.top
                    - padding.bottom
                    - border.top
                    - border.bottom)
                    .max(0.0),
            };
            let logical_content_box_size = inline_coordinates.to_logical_size(content_box_size);
            let logical_padding = writing_direction.to_logical_box_strut(padding);
            let block_offset = single_subject_block_alignment_offset(
                style.align_content,
                logical_content_box_size.block_size - measurement.alignment_block_size,
            );
            measurement.translate_block_axis(block_offset);
            let mut logical_content_size = inline_coordinates.to_logical_size(output.content_size);
            logical_content_size.block_size = logical_content_size.block_size.max(
                measurement.alignment_block_size
                    + logical_padding.block_start
                    + logical_padding.block_end
                    + block_offset.max(0.0),
            );
            output.content_size = inline_coordinates.to_physical_size(logical_content_size);
            let content_offset = Point {
                x: padding.left + border.left,
                y: padding.top + border.top,
            };
            let physical_baseline = |line: Option<&InlineLinePlacement>| {
                let baseline = line.map_or(Point::NONE, |line| {
                    inline_coordinates.to_physical_line_baseline(line, content_box_size)
                });
                Point {
                    x: baseline.x.map(|x| x + content_offset.x),
                    y: baseline.y.map(|y| y + content_offset.y),
                }
            };
            output.first_baselines = physical_baseline(
                measurement
                    .line_placements
                    .iter()
                    .find(|line| !line.phantom),
            );
            output.last_baselines = physical_baseline(
                measurement
                    .line_placements
                    .iter()
                    .rev()
                    .find(|line| !line.phantom),
            );
            if measurement.line_placements.iter().any(|line| !line.phantom) {
                output.margins_can_collapse_through = false;
            }
            if inputs.run_mode == RunMode::PerformLayout {
                self.position_inline_objects(
                    inline_context,
                    &measurement,
                    content_offset,
                    content_box_size,
                    output.size,
                    writing_mode,
                    self.boxes[id.index()].style.direction(),
                );
                inline_context.fragments = measurement
                    .fragments
                    .into_physical(inline_coordinates, content_box_size);
                inline_context.line_placements = measurement.line_placements;
                inline_context.laid_out = Some(measurement.layout);
            }
        }

        if let Some(inline_context) = inline_context {
            self.boxes[id.index()].inline_layout = Some(inline_context);
        }
        if inputs.run_mode == RunMode::PerformLayout {
            self.layout_custom_context_out_of_flow_children(id, &mut output, padding, border);
        }
        output.with_block_constraint_dependency(depends_on_block_constraints)
    }

    /// Consume positioned descendants after a custom formatting context has
    /// emitted their static positions. Sizing and inset resolution remain in
    /// Taffy's shared out-of-flow resolver.
    fn layout_custom_context_out_of_flow_children(
        &mut self,
        id: LayoutBoxId,
        output: &mut LayoutOutput,
        padding: taffy::Rect<f32>,
        border: taffy::Rect<f32>,
    ) {
        let style = self.boxes[id.index()].style.taffy.clone();
        let writing_direction =
            WritingDirection::new(self.boxes[id.index()].style.writing_mode(), style.direction);
        let scrollbar_gutter = Point {
            x: if style.overflow.y == taffy::Overflow::Scroll {
                style.scrollbar_width
            } else {
                0.0
            },
            y: if style.overflow.x == taffy::Overflow::Scroll {
                style.scrollbar_width
            } else {
                0.0
            },
        };
        let area_offset = Point {
            x: border.left
                + if style.direction == taffy::Direction::Rtl {
                    scrollbar_gutter.x
                } else {
                    0.0
                },
            y: border.top,
        };
        let default_containing_block = OutOfFlowContainingBlock {
            outer_size: output.size,
            area_offset,
            area_size: (output.size
                - border.sum_axes()
                - Size {
                    width: scrollbar_gutter.x,
                    height: scrollbar_gutter.y,
                })
            .f32_max(Size::ZERO),
            writing_direction,
        };
        let content_size =
            (default_containing_block.area_size - padding.sum_axes()).f32_max(Size::ZERO);
        let fallback_static_position =
            LogicalStaticPosition::new(writing_direction.converter(output.size).to_logical_point(
                Point {
                    x: area_offset.x + padding.left,
                    y: area_offset.y + padding.top,
                },
                Size::ZERO,
            ));
        let content_offset = Point {
            x: border.left + padding.left,
            y: border.top + padding.top,
        };

        let children = self.boxes[id.index()].layout_children.clone();
        for (order, child) in children.into_iter().enumerate() {
            if self.boxes[child.index()].style.taffy.position != taffy::Position::Absolute
                || !self.is_out_of_flow_containing_block(id.to_taffy(), child.to_taffy())
            {
                continue;
            }
            let containing_block = self.custom_context_out_of_flow_containing_block(
                id,
                child,
                default_containing_block,
                content_offset,
                content_size,
            );
            let static_position = self
                .get_out_of_flow_static_position(
                    id.to_taffy(),
                    child.to_taffy(),
                    containing_block.outer_size,
                    containing_block.writing_direction,
                )
                .unwrap_or(fallback_static_position);
            if let Some(content_size) = compute_out_of_flow_layout(
                self,
                child.to_taffy(),
                order as u32,
                static_position,
                containing_block,
            ) {
                output.content_size = output.content_size.f32_max(content_size);
            }
        }
    }

    /// Select the CSS containing area for a positioned child of a custom IFC.
    /// Ordinary descendants use the owner's padding box. A flattened inline
    /// containing block instead contributes its fragment-derived rectangle,
    /// expressed directly in the owner's border-box coordinate space.
    fn custom_context_out_of_flow_containing_block(
        &self,
        owner: LayoutBoxId,
        child: LayoutBoxId,
        default: OutOfFlowContainingBlock,
        content_offset: Point<f32>,
        owner_content_size: Size<f32>,
    ) -> OutOfFlowContainingBlock {
        let Some(containing_block) = self.boxes[child.index()].positioned_containing_block else {
            return default;
        };
        let containing_box = &self.boxes[containing_block.index()];
        if !containing_box.inline_flattened || containing_box.inline_context_owner != Some(owner) {
            return default;
        }
        let Some(rect) =
            inline_box_containing_rect(self, owner, containing_block, owner_content_size)
        else {
            return default;
        };

        OutOfFlowContainingBlock {
            outer_size: default.outer_size,
            area_offset: Point {
                x: content_offset.x + rect.x,
                y: content_offset.y + rect.y,
            },
            area_size: Size {
                width: rect.width,
                height: rect.height,
            },
            writing_direction: WritingDirection::new(
                containing_box.style.writing_mode(),
                containing_box.style.taffy.direction,
            ),
        }
    }

    fn measure_inline_context(
        &mut self,
        owner: LayoutBoxId,
        inputs: LayoutInput,
        known_dimensions: Size<Option<f32>>,
        available_space: Size<AvailableSpace>,
        alignment: parley::Alignment,
        context: &InlineFormattingContext,
        is_floated: bool,
        block_context: Option<&mut BlockContext<'_>>,
    ) -> InlineMeasurement {
        let parent_writing_mode = self.boxes[owner.index()].style.writing_mode();
        let writing_direction = self.boxes[owner.index()].style.writing_direction();
        let owner_taffy_size = self.boxes[owner.index()].style.taffy.size;
        let inline_coordinates = InlineCoordinateSpace::new(parent_writing_mode);
        let logical_known_dimensions = inline_coordinates.to_logical_size(known_dimensions);
        let logical_input_known_dimensions =
            inline_coordinates.to_logical_size(inputs.known_dimensions);
        let logical_available_space = inline_coordinates.to_logical_size(available_space);
        let child_inputs = LayoutInput {
            run_mode: inputs.run_mode,
            sizing_mode: SizingMode::InherentSize,
            sizing_purpose: inputs.sizing_purpose,
            axis: inputs.axis,
            inline_auto_behavior: AutoSizeBehavior::FitContent,
            block_auto_behavior: AutoSizeBehavior::FitContent,
            known_dimensions: Size::NONE,
            definite_dimensions: Size::NONE,
            parent_size: available_space.into_options(),
            parent_writing_mode,
            available_space,
            block_margins_are_collapsible: Line::FALSE,
        };
        // A float's max-content contribution is measured independently from
        // the finite line slot it will eventually occupy. Final fit-content
        // layout still uses the IFC owner's content width; it must not use
        // MaxContent or the current exclusion slot as its available width.
        let float_max_content_inputs = LayoutInput {
            available_space: Size {
                width: AvailableSpace::MaxContent,
                height: AvailableSpace::MaxContent,
            },
            ..child_inputs
        };
        // CSS Sizing resolves cyclic percentages against zero while measuring
        // intrinsic contributions. Keeping the basis as `None` discards the
        // entire calc expression, including its absolute term (for example
        // `calc(0% + 30px)`). A final definite-width layout still supplies its
        // actual basis here.
        let percentage_basis = inline_percentage_basis(child_inputs, parent_writing_mode);
        let mut layout = context.unbroken.clone();
        let mut atomic = vec![None; context.objects.len()];
        let mut atomic_baseline_ascents = vec![None; context.objects.len()];
        let mut structural_edge_contributions = vec![false; context.objects.len()];
        let mut floats = Vec::new();
        let mut depends_on_block_constraints = false;

        for (inline_box, object) in layout.inline_boxes_mut().iter_mut().zip(&context.objects) {
            match object.role {
                InlineObjectRole::Atomic => {
                    let margins = self.boxes[object.box_id.index()]
                        .style
                        .taffy
                        .margin
                        .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
                    let physical_margin_size = Size {
                        width: margins.left + margins.right,
                        height: margins.top + margins.bottom,
                    };
                    let logical_margins = writing_direction.to_logical_box_strut(margins);
                    let line_relative_margins =
                        inline_coordinates.to_line_relative_box_strut(margins);
                    // Child layout algorithms consume the border-box space
                    // left after the parent resolves margins. Blocks, flex
                    // and grid establish the same contract before recursing;
                    // this is that boundary for the Parley-owned IFC.
                    let atomic_inputs = LayoutInput {
                        available_space: Size {
                            width: child_inputs
                                .available_space
                                .width
                                .maybe_sub(physical_margin_size.width),
                            height: child_inputs
                                .available_space
                                .height
                                .maybe_sub(physical_margin_size.height),
                        },
                        ..child_inputs
                    };
                    let child_output = if inputs.run_mode == RunMode::ComputeSize {
                        let output =
                            self.compute_atomic_inline_measurement(object.box_id, atomic_inputs);
                        let intrinsic = output.into_intrinsic_size_result();
                        depends_on_block_constraints |= intrinsic.depends_on_block_constraints;
                        output
                    } else {
                        self.compute_atomic_inline_layout(object.box_id, atomic_inputs)
                    };
                    let logical_child_size = inline_coordinates.to_logical_size(child_output.size);
                    inline_box.width = (logical_margins.inline_start
                        + logical_margins.inline_end
                        + logical_child_size.inline_size)
                        .max(0.0);
                    inline_box.height = (logical_margins.block_start
                        + logical_margins.block_end
                        + logical_child_size.block_size)
                        .max(0.0);
                    let object_index = usize::try_from(inline_box.id)
                        .expect("Parley returned an inline object id outside usize");
                    atomic_baseline_ascents[object_index] = self
                        .atomic_inline_baseline(object.box_id, child_output, parent_writing_mode)
                        .map(|baseline| logical_margins.block_start + baseline);
                    atomic[object_index] = Some(AtomicMeasurement {
                        output: child_output,
                        margins: line_relative_margins,
                    });
                }
                InlineObjectRole::OutOfFlow => {
                    inline_box.width = 0.0;
                    inline_box.height = 0.0;
                }
                InlineObjectRole::Float => {
                    inline_box.width = 0.0;
                    inline_box.height = 0.0;
                }
                InlineObjectRole::StartEdge | InlineObjectRole::EndEdge => {
                    let child_style = &self.boxes[object.box_id.index()].style;
                    let margins = child_style
                        .taffy
                        .margin
                        .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
                    let padding = child_style
                        .taffy
                        .padding
                        .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
                    let border = child_style
                        .taffy
                        .border
                        .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
                    let logical_margins = child_style
                        .writing_direction()
                        .to_logical_box_strut(margins);
                    let logical_padding = child_style
                        .writing_direction()
                        .to_logical_box_strut(padding);
                    let logical_border =
                        child_style.writing_direction().to_logical_box_strut(border);
                    let (margin, padding, border) = if object.role == InlineObjectRole::StartEdge {
                        (
                            logical_margins.inline_start,
                            logical_padding.inline_start,
                            logical_border.inline_start,
                        )
                    } else {
                        (
                            logical_margins.inline_end,
                            logical_padding.inline_end,
                            logical_border.inline_end,
                        )
                    };
                    inline_box.width = (margin + padding + border).max(0.0);
                    inline_box.height = 0.0;
                    let object_index = usize::try_from(inline_box.id)
                        .expect("Parley returned an inline object id outside usize");
                    structural_edge_contributions[object_index] =
                        margin != 0.0 || padding != 0.0 || border != 0.0;
                }
            }
        }

        let containing_inline_size = logical_known_dimensions
            .inline_size
            .or_else(|| logical_available_space.inline_size.into_option())
            .unwrap_or_default();
        let (indent, indent_options) = self.boxes[owner.index()]
            .style
            .text_indent(containing_inline_size);
        layout.set_text_indent(indent, indent_options);
        let content_inline_sizes = layout.calculate_content_widths();
        let logical_style_size = inline_coordinates.to_logical_size(owner_taffy_size);
        let logical_parent_size = inline_coordinates.to_logical_size(inputs.parent_size);
        let has_definite_inline_size = logical_known_dimensions.inline_size.is_some()
            || logical_input_known_dimensions.inline_size.is_some()
            || logical_style_size
                .inline_size
                .maybe_resolve(logical_parent_size.inline_size, resolve_stylo_calc_value)
                .is_some();
        let is_unstretched_flex_or_grid_item = inputs.run_mode == RunMode::PerformLayout
            && logical_input_known_dimensions.inline_size.is_none()
            && self.boxes[owner.index()]
                .layout_parent
                .is_some_and(|parent| {
                    let display = self.boxes[parent.index()].style.display();
                    display.is_flex_container() || display.is_grid_container()
                });
        let is_intrinsic_contribution =
            inputs.sizing_purpose == SizingPurpose::IntrinsicContribution;
        let shrink_to_fit = !has_definite_inline_size
            && (is_floated
                // The parent formatting context has already selected the
                // CSS fit-content behavior for this auto inline size (for
                // example a native control in ordinary block layout). The
                // IFC must clamp its min/max-content widths to the supplied
                // slot instead of treating that finite slot as a used width.
                || inputs.inline_auto_behavior == AutoSizeBehavior::FitContent
                // A content-based block parent can probe this IFC with a
                // finite available inline size while its own content inline
                // size is still unknown. That is an intrinsic contribution, so clamp the
                // available inline size between the IFC's min/max-content sizes
                // instead of stretching to the probe constraint.
                || is_intrinsic_contribution
                // Flex/grid layout passes an auto inline size as unknown when
                // cross-axis stretch does not apply. In final layout that item
                // must return its fit-content width within the definite area.
                // Restrict this to actual flex/grid items: internal IFCs such
                // as form-control content are also measured with an unknown
                // width but deliberately fill their supplied content area.
                || is_unstretched_flex_or_grid_item
                || matches!(
                    self.boxes[owner.index()].style.display(),
                    crate::LayoutDisplay::InlineBlock | crate::LayoutDisplay::InlineListItem
                )
                || self.boxes[owner.index()].style.taffy.item_is_table);
        let min_float_inputs = LayoutInput {
            available_space: inline_coordinates.to_physical_size(LogicalSize {
                inline_size: AvailableSpace::MinContent,
                block_size: AvailableSpace::MaxContent,
            }),
            ..child_inputs
        };
        let mut float_min_inline_size: f32 = 0.0;
        let mut float_max_inline_size: f32 = 0.0;
        let mut left_band: f32 = 0.0;
        let mut right_band: f32 = 0.0;
        if !matches!(
            logical_available_space.inline_size,
            AvailableSpace::Definite(_)
        ) || shrink_to_fit
        {
            for object in context
                .objects
                .iter()
                .filter(|object| object.role == InlineObjectRole::Float)
            {
                let style = &self.boxes[object.box_id.index()].style.taffy;
                let float = style.float;
                let clear = style.clear;
                let margin = style
                    .margin
                    .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
                let logical_margin = writing_direction.to_logical_box_strut(margin);
                if matches!(clear, taffy::Clear::Left | taffy::Clear::Both) {
                    left_band = 0.0;
                }
                if matches!(clear, taffy::Clear::Right | taffy::Clear::Both) {
                    right_band = 0.0;
                }
                let (min_size, max_size) = if inputs.run_mode == RunMode::ComputeSize {
                    let min_result =
                        self.compute_child_size(object.box_id.to_taffy(), min_float_inputs);
                    let max_result =
                        self.compute_child_size(object.box_id.to_taffy(), float_max_content_inputs);
                    depends_on_block_constraints |= min_result.depends_on_block_constraints
                        || max_result.depends_on_block_constraints;
                    (min_result.size, max_result.size)
                } else {
                    (
                        self.compute_child_layout(object.box_id.to_taffy(), min_float_inputs)
                            .size,
                        self.compute_child_layout(
                            object.box_id.to_taffy(),
                            float_max_content_inputs,
                        )
                        .size,
                    )
                };
                let min_inline_size = inline_coordinates.to_logical_size(min_size).inline_size;
                let max_inline_size = inline_coordinates.to_logical_size(max_size).inline_size;
                float_min_inline_size = float_min_inline_size
                    .max(min_inline_size + logical_margin.inline_start + logical_margin.inline_end);
                let outer_inline_size =
                    max_inline_size + logical_margin.inline_start + logical_margin.inline_end;
                match float {
                    taffy::Float::Left => left_band += outer_inline_size,
                    taffy::Float::Right => right_band += outer_inline_size,
                    taffy::Float::None => {}
                }
                float_max_inline_size = float_max_inline_size.max(left_band + right_band);
            }
        }
        let inline_size = logical_known_dimensions.inline_size.unwrap_or_else(|| {
            match logical_available_space.inline_size {
                AvailableSpace::MinContent => content_inline_sizes.min.max(float_min_inline_size),
                AvailableSpace::MaxContent => content_inline_sizes.max + float_max_inline_size,
                // Taffy has already resolved and clamped the content-box
                // inline size before invoking the leaf measure function. A
                // normal block IFC must lay out into that definite inline size;
                // shrinking it to max-content here made RTL alignment and
                // text-indent observe an unrelated inner size.
                AvailableSpace::Definite(limit) if shrink_to_fit => (content_inline_sizes.max
                    + float_max_inline_size)
                    .min(limit)
                    .max(content_inline_sizes.min.max(float_min_inline_size)),
                AvailableSpace::Definite(limit) => limit,
            }
            .max(0.0)
        });
        // Taffy may feed an intrinsic inline size back through a quantized
        // definite flex/grid constraint, while Parley's content-width and
        // line-breaking passes can accumulate the same glyph advances in a
        // slightly different order. Preserve the max-content boundary plus a
        // small floating-point margin when those inline sizes differ only by
        // noise. Otherwise a one-line flex item can immediately rewrap during
        // final layout. Keep genuinely constrained sizes unchanged so normal
        // wrapping is unaffected.
        let intrinsic_max_inline_size = content_inline_sizes.max + float_max_inline_size;
        let intrinsic_tolerance = inline_size.abs().max(1.0) * f32::EPSILON * 8.0;
        let line_break_inline_size =
            if intrinsic_max_inline_size <= inline_size + intrinsic_tolerance {
                inline_size.max(intrinsic_max_inline_size + intrinsic_tolerance)
            } else {
                inline_size
            };
        let max_advance = match logical_available_space.inline_size {
            AvailableSpace::MaxContent => None,
            AvailableSpace::MinContent | AvailableSpace::Definite(_) => {
                Some(line_break_inline_size)
            }
        };
        let has_inline_float = context
            .objects
            .iter()
            .any(|object| object.role == InlineObjectRole::Float);
        let mut contained_float_block_size = None;
        let mut alignment_float_block_size = 0.0;
        if has_inline_float
            || block_context
                .as_ref()
                .is_some_and(|context| context.has_floats())
        {
            let container_style = &self.boxes[owner.index()].style.taffy;
            let padding = container_style.padding.resolve_or_zero(
                box_model_percentage_basis(inputs, parent_writing_mode),
                resolve_stylo_calc_value,
            );
            let border = container_style.border.resolve_or_zero(
                box_model_percentage_basis(inputs, parent_writing_mode),
                resolve_stylo_calc_value,
            );
            let padding_border = padding + border;
            let logical_padding_border = writing_direction.to_logical_box_strut(padding_border);
            let line_insets = if writing_direction.direction == taffy::Direction::Rtl {
                [
                    logical_padding_border.inline_end,
                    logical_padding_border.inline_start,
                ]
            } else {
                [
                    logical_padding_border.inline_start,
                    logical_padding_border.inline_end,
                ]
            };
            let outer_inline_size = inline_size
                + logical_padding_border.inline_start
                + logical_padding_border.inline_end;
            if let Some(block_context) = block_context {
                let contains_floats = block_context.is_bfc_root();
                if contains_floats {
                    block_context.set_inline_size(outer_inline_size);
                }
                let mut content_context =
                    block_context.sub_context(logical_padding_border.block_start, line_insets);
                self.break_inline_lines_with_floats(
                    context,
                    &mut layout,
                    inline_size,
                    child_inputs,
                    &mut content_context,
                    writing_direction,
                    &mut floats,
                );
                alignment_float_block_size = content_context.floated_block_size_contribution();
                if contains_floats {
                    contained_float_block_size = Some(alignment_float_block_size);
                }
            } else {
                let mut formatting_context = BlockFormattingContext::new();
                let mut root_context = formatting_context.root_block_context();
                root_context.set_inline_size(outer_inline_size);
                let mut content_context =
                    root_context.sub_context(logical_padding_border.block_start, line_insets);
                self.break_inline_lines_with_floats(
                    context,
                    &mut layout,
                    inline_size,
                    child_inputs,
                    &mut content_context,
                    writing_direction,
                    &mut floats,
                );
                alignment_float_block_size = content_context.floated_block_size_contribution();
                contained_float_block_size = Some(alignment_float_block_size);
            }
        } else {
            layout.break_all_lines(max_advance);
        }
        layout.align(
            alignment,
            AlignmentOptions {
                align_when_overflowing: false,
            },
        );

        let (line_placements, line_expansion) = build_inline_line_placements(
            context,
            &layout,
            &atomic_baseline_ascents,
            &structural_edge_contributions,
        );
        let mut block_size = layout.height() + line_expansion;
        if let Some(float_block_size) = contained_float_block_size {
            block_size = block_size.max(float_block_size);
        }
        let alignment_block_size = block_size.max(alignment_float_block_size);
        let fragments = build_inline_fragments(context, &layout, &line_placements);
        InlineMeasurement {
            size: inline_coordinates.to_physical_size(LogicalSize {
                inline_size: logical_known_dimensions.inline_size.unwrap_or(inline_size),
                block_size: logical_known_dimensions.block_size.unwrap_or(block_size),
            }),
            alignment_block_size,
            layout,
            atomic,
            floats,
            percentage_basis,
            line_placements,
            fragments,
            depends_on_block_constraints,
        }
    }

    fn atomic_inline_baseline(
        &self,
        id: LayoutBoxId,
        output: LayoutOutput,
        parent_writing_mode: taffy::WritingMode,
    ) -> Option<f32> {
        let layout_box = &self.boxes[id.index()];
        // A physical baseline is meaningful to the parent line only when
        // both boxes use the same writing mode. Blink's LogicalBoxFragment
        // enforces the same boundary before exposing First/LastBaseline;
        // orthogonal atomic boxes synthesize against the parent's dominant
        // baseline instead.
        if layout_box.style.writing_mode() != parent_writing_mode {
            return None;
        }
        let physical_baseline = match layout_box.style.display() {
            // Blink's block layout marks these atomic fragments to use their
            // last baseline. A scrolling inline-block instead forces baseline
            // synthesis from its margin-box edge.
            crate::LayoutDisplay::InlineBlock | crate::LayoutDisplay::InlineListItem => {
                if layout_box.style.taffy.overflow.x == taffy::Overflow::Visible
                    && layout_box.style.taffy.overflow.y == taffy::Overflow::Visible
                {
                    output.last_baselines
                } else {
                    Point::NONE
                }
            }
            // Flex, grid, and table formatting contexts expose their first
            // baseline as the automatic inline-level baseline. Do not apply
            // the inline-block overflow exception to these fragment types.
            crate::LayoutDisplay::InlineFlex
            | crate::LayoutDisplay::InlineGrid
            | crate::LayoutDisplay::InlineTable => output.first_baselines,
            // Replaced and other atomic inline-level boxes synthesize their
            // baseline at the appropriate box edge in the caller.
            _ => Point::NONE,
        };
        InlineCoordinateSpace::new(parent_writing_mode)
            .to_line_block_baseline(physical_baseline, output.size)
    }

    fn break_inline_lines_with_floats(
        &mut self,
        context: &InlineFormattingContext,
        layout: &mut parley::Layout<crate::stylo_to_parley::TextBrush>,
        width: f32,
        child_inputs: LayoutInput,
        block_context: &mut BlockContext<'_>,
        writing_direction: WritingDirection,
        floats: &mut Vec<InlineFloatPlacement>,
    ) {
        let mut breaker = layout.break_lines();
        let initial_slot = block_context.find_content_slot(0.0, Clear::None, None);
        let mut has_active_floats = initial_slot.segment_id.is_some();
        {
            let state = breaker.state_mut();
            state.set_layout_max_advance(width);
            state.set_line_max_advance(initial_slot.width.max(0.0));
            state.set_line_x(initial_slot.x);
            state.set_line_y(f64::from(initial_slot.y));
        }

        while let Some(yield_data) = breaker.break_next() {
            match yield_data {
                YieldData::LineBreak(_) => {
                    let state = breaker.state_mut();
                    if has_active_floats {
                        let next_slot = block_context.find_content_slot(
                            state.line_y() as f32,
                            Clear::None,
                            None,
                        );
                        has_active_floats = next_slot.segment_id.is_some();
                        state.set_line_max_advance(next_slot.width.max(0.0));
                        state.set_line_x(next_slot.x);
                        state.set_line_y(f64::from(next_slot.y));
                    } else {
                        state.set_line_x(0.0);
                        state.set_line_max_advance(width);
                    }
                }
                YieldData::MaxHeightExceeded(_) => {}
                YieldData::InlineBoxBreak(data) => {
                    let Some(object) = context.object(data.inline_box_id) else {
                        continue;
                    };
                    if object.role != InlineObjectRole::Float {
                        continue;
                    }
                    let child = object.box_id;
                    let style = self.boxes[child.index()].style.taffy.clone();
                    let direction = match style.float {
                        taffy::Float::Left => FloatDirection::Left,
                        taffy::Float::Right => FloatDirection::Right,
                        taffy::Float::None => continue,
                    };
                    let margin = style.margin.resolve_or_zero(
                        box_model_percentage_basis(child_inputs, child_inputs.parent_writing_mode),
                        resolve_stylo_calc_value,
                    );
                    let logical_margin = writing_direction.to_logical_box_strut(margin);
                    // Child available space is the border-box slot remaining
                    // after margins for both replaced and non-replaced floats.
                    let layout_inputs = LayoutInput {
                        available_space: Size {
                            width: child_inputs
                                .available_space
                                .width
                                .maybe_sub(margin.left + margin.right),
                            height: child_inputs
                                .available_space
                                .height
                                .maybe_sub(margin.top + margin.bottom),
                        },
                        ..child_inputs
                    };
                    let output = self.compute_child_layout(child.to_taffy(), layout_inputs);
                    let state = breaker.state_mut();
                    let margin_box_size =
                        writing_direction.mode.to_logical(output.size) + logical_margin.sum_axes();
                    let position = block_context.place_floated_box(
                        margin_box_size,
                        state.line_y() as f32,
                        direction,
                        style.clear,
                        false,
                    );
                    let inline_offset = if writing_direction.direction == taffy::Direction::Rtl {
                        width - position.line_offset - margin_box_size.inline_size
                    } else {
                        position.line_offset
                    };
                    floats.push(InlineFloatPlacement {
                        child,
                        location: LogicalOffset {
                            inline_offset: inline_offset + logical_margin.inline_start,
                            block_offset: position.block_offset + logical_margin.block_start,
                        },
                        output,
                        order: usize::try_from(data.inline_box_id).unwrap_or(usize::MAX),
                        percentage_basis: box_model_percentage_basis(
                            child_inputs,
                            child_inputs.parent_writing_mode,
                        ),
                    });
                    let next_slot =
                        block_context.find_content_slot(state.line_y() as f32, Clear::None, None);
                    has_active_floats = next_slot.segment_id.is_some();
                    state.set_line_max_advance(next_slot.width.max(0.0));
                    state.set_line_x(next_slot.x);
                    state.set_line_y(f64::from(next_slot.y));
                    state.append_inline_box_to_line(data.advance, 0.0);
                }
            }
        }
        breaker.finish();
    }

    fn position_inline_objects(
        &mut self,
        context: &InlineFormattingContext,
        measurement: &InlineMeasurement,
        content_offset: Point<f32>,
        content_box_size: Size<f32>,
        border_box_size: Size<f32>,
        container_writing_mode: taffy::WritingMode,
        container_direction: InlineDirection,
    ) {
        let container_taffy_direction = match container_direction {
            InlineDirection::Ltr => taffy::Direction::Ltr,
            InlineDirection::Rtl => taffy::Direction::Rtl,
        };
        let writing_direction =
            WritingDirection::new(container_writing_mode, container_taffy_direction);
        let inline_coordinates = InlineCoordinateSpace::new(container_writing_mode);
        let converter = writing_direction.converter(content_box_size);
        for floated in &measurement.floats {
            let relative_location =
                converter.to_physical_point(floated.location, floated.output.size);
            self.set_inline_child_layout(
                floated.child,
                Point {
                    x: content_offset.x + relative_location.x,
                    y: content_offset.y + relative_location.y,
                },
                floated.output,
                floated.order,
                floated.percentage_basis,
            );
        }
        for (line_index, line) in measurement.layout.lines().enumerate() {
            let line_placement = measurement.line_placements.get(line_index);
            let line_rect = flow_relative_line_rect(&line, line_placement);
            for (item_index, item) in line.items().enumerate() {
                let PositionedLayoutItem::InlineBox(positioned) = item else {
                    continue;
                };
                let Some(object) = context.object(positioned.id) else {
                    continue;
                };
                let object_index = usize::try_from(positioned.id)
                    .expect("Parley returned an inline object id outside usize");
                let vertical_offset = line_placement
                    .map(|placement| placement.item_offset(item_index))
                    .unwrap_or_default();
                if object.role == InlineObjectRole::OutOfFlow {
                    let inline_level = self.boxes[object.box_id.index()]
                        .style
                        .hypothetical_display_is_inline_level();
                    let owner = self.boxes[object.box_id.index()]
                        .inline_context_owner
                        .unwrap_or_else(|| panic!("out-of-flow IFC object lost its owner"));
                    let relative_point = inline_coordinates.to_physical_line_point(
                        line_rect,
                        LineRelativeOffset::new(
                            if inline_level {
                                positioned.x - line_rect.inline_offset
                            } else {
                                0.0
                            },
                            positioned.y + vertical_offset - line_rect.block_offset,
                        ),
                        Size::ZERO,
                        content_box_size,
                    );
                    let point = Point {
                        x: content_offset.x + relative_point.x,
                        y: content_offset.y + relative_point.y,
                    };
                    self.boxes[object.box_id.index()].out_of_flow_static_position =
                        Some(OutOfFlowStaticPosition {
                            owner,
                            position: logical_static_position_in_owner(
                                point,
                                border_box_size,
                                writing_direction,
                            ),
                        });
                    continue;
                }
                if object.role == InlineObjectRole::Float {
                    continue;
                }
                if object.role != InlineObjectRole::Atomic {
                    continue;
                }
                let Some(atomic) = measurement.atomic[object_index] else {
                    continue;
                };
                let inset_offset = relative_atomic_inset_offset(
                    &self.boxes[object.box_id.index()].style.taffy,
                    content_box_size,
                    writing_direction,
                );
                let relative_location = inline_coordinates.to_physical_line_point(
                    line_rect,
                    LineRelativeOffset::new(
                        positioned.x - line_rect.inline_offset + atomic.margins.inline_start,
                        positioned.y + vertical_offset - line_rect.block_offset
                            + atomic.margins.block_start,
                    ),
                    atomic.output.size,
                    content_box_size,
                );
                self.set_inline_child_layout(
                    object.box_id,
                    Point {
                        x: content_offset.x + relative_location.x + inset_offset.x,
                        y: content_offset.y + relative_location.y + inset_offset.y,
                    },
                    atomic.output,
                    object_index,
                    measurement.percentage_basis,
                );
            }
        }
    }

    fn set_inline_child_layout(
        &mut self,
        child: LayoutBoxId,
        location: Point<f32>,
        output: LayoutOutput,
        order: usize,
        percentage_basis: Option<f32>,
    ) {
        let style = &self.boxes[child.index()].style.taffy;
        let padding = style
            .padding
            .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
        let border = style
            .border
            .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
        let margin = style
            .margin
            .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
        let scrollbar_size = Size {
            width: if style.overflow.y == taffy::Overflow::Scroll {
                style.scrollbar_width
            } else {
                0.0
            },
            height: if style.overflow.x == taffy::Overflow::Scroll {
                style.scrollbar_width
            } else {
                0.0
            },
        };

        self.boxes[child.index()].unrounded_layout = Layout {
            order: u32::try_from(order).unwrap_or(u32::MAX),
            location,
            size: output.size,
            content_size: output.content_size,
            scrollbar_size,
            border,
            padding,
            margin,
        };
    }
}

fn inline_percentage_basis(inputs: LayoutInput, writing_mode: taffy::WritingMode) -> Option<f32> {
    box_model_percentage_basis(inputs, writing_mode)
        .or_else(|| (inputs.sizing_purpose == SizingPurpose::IntrinsicContribution).then_some(0.0))
}

#[cfg(test)]
mod tests {
    use super::{
        inline_percentage_basis, logical_static_position_in_owner, round_layout_to_css_subpixels,
    };
    use taffy::{
        Direction, Layout, LayoutInput, NodeId, Point, RoundTree, Size, SizingPurpose,
        TraversePartialTree, TraverseTree, WritingDirection, WritingMode,
    };

    struct RoundNode {
        children: Vec<NodeId>,
        unrounded: Layout,
        final_layout: Layout,
    }

    struct TestRoundTree(Vec<RoundNode>);

    impl TraversePartialTree for TestRoundTree {
        type ChildIter<'a> = std::iter::Copied<std::slice::Iter<'a, NodeId>>;

        fn child_ids(&self, parent_node_id: NodeId) -> Self::ChildIter<'_> {
            self.0[usize::from(parent_node_id)].children.iter().copied()
        }

        fn child_count(&self, parent_node_id: NodeId) -> usize {
            self.0[usize::from(parent_node_id)].children.len()
        }

        fn get_child_id(&self, parent_node_id: NodeId, child_index: usize) -> NodeId {
            self.0[usize::from(parent_node_id)].children[child_index]
        }
    }

    impl TraverseTree for TestRoundTree {}

    impl RoundTree for TestRoundTree {
        fn get_unrounded_layout(&self, node_id: NodeId) -> Layout {
            self.0[usize::from(node_id)].unrounded
        }

        fn set_final_layout(&mut self, node_id: NodeId, layout: &Layout) {
            self.0[usize::from(node_id)].final_layout = *layout;
        }
    }

    #[test]
    fn css_subpixel_adapter_preserves_cumulative_edge_rounding() {
        let root = NodeId::new(0);
        let child = NodeId::new(1);
        let layout = |x, width| Layout {
            location: Point { x, y: 0.0 },
            size: Size {
                width,
                height: 10.0,
            },
            ..Layout::with_order(0)
        };
        let mut tree = TestRoundTree(vec![
            RoundNode {
                children: vec![child],
                unrounded: layout(0.2, 100.3),
                final_layout: Layout::with_order(0),
            },
            RoundNode {
                children: Vec::new(),
                unrounded: layout(0.333, 10.333),
                final_layout: Layout::with_order(0),
            },
        ]);

        round_layout_to_css_subpixels(&mut tree, root);

        assert_eq!(tree.0[0].final_layout.location.x, 0.203_125);
        assert_eq!(tree.0[0].final_layout.size.width, 100.296_875);
        assert_eq!(tree.0[1].final_layout.location.x, 0.328_125);
        assert_eq!(tree.0[1].final_layout.size.width, 10.328_125);
    }

    fn percentage_inputs(
        parent_size: Size<Option<f32>>,
        parent_writing_mode: WritingMode,
        sizing_purpose: SizingPurpose,
    ) -> LayoutInput {
        LayoutInput {
            parent_size,
            parent_writing_mode,
            sizing_purpose,
            ..LayoutInput::HIDDEN
        }
    }

    #[test]
    fn intrinsic_inline_percentages_use_zero_when_the_parent_inline_size_is_indefinite() {
        assert_eq!(
            inline_percentage_basis(
                percentage_inputs(
                    Size::NONE,
                    WritingMode::HorizontalTb,
                    SizingPurpose::IntrinsicContribution,
                ),
                WritingMode::HorizontalTb,
            ),
            Some(0.0)
        );
        assert_eq!(
            inline_percentage_basis(
                percentage_inputs(
                    Size {
                        width: Some(240.0),
                        height: None,
                    },
                    WritingMode::HorizontalTb,
                    SizingPurpose::Layout,
                ),
                WritingMode::HorizontalTb,
            ),
            Some(240.0)
        );
        assert_eq!(
            inline_percentage_basis(
                percentage_inputs(Size::NONE, WritingMode::HorizontalTb, SizingPurpose::Layout,),
                WritingMode::HorizontalTb,
            ),
            None
        );
    }

    #[test]
    fn rtl_static_candidates_use_the_owner_border_box_coordinate_space() {
        let owner_size = Size {
            width: 200.0,
            height: 100.0,
        };
        let direction = WritingDirection::new(WritingMode::HorizontalTb, Direction::Rtl);
        let candidate =
            logical_static_position_in_owner(Point { x: 120.0, y: 30.0 }, owner_size, direction);

        assert_eq!(candidate.offset.inline_offset, 80.0);
        assert_eq!(
            candidate.to_physical(direction, owner_size).offset,
            Point { x: 120.0, y: 30.0 },
        );
    }

    #[test]
    fn inline_box_percentages_follow_a_vertical_containing_blocks_inline_axis() {
        let inputs = percentage_inputs(
            Size {
                width: Some(100.0),
                height: Some(240.0),
            },
            WritingMode::VerticalRl,
            SizingPurpose::Layout,
        );

        assert_eq!(
            inline_percentage_basis(inputs, WritingMode::VerticalRl),
            Some(240.0)
        );
    }
}

#[derive(Clone, Copy)]
struct AtomicMeasurement {
    output: LayoutOutput,
    margins: LogicalBoxStrut<f32>,
}

#[derive(Clone, Copy)]
struct InlineFloatPlacement {
    child: LayoutBoxId,
    location: LogicalOffset<f32>,
    output: LayoutOutput,
    order: usize,
    percentage_basis: Option<f32>,
}

struct InlineMeasurement {
    size: Size<f32>,
    /// Block-end extent of every IFC child used as the single alignment
    /// subject. Unlike the block axis of `size`, this includes non-contained
    /// floats without making them contribute to normal-flow auto block-size.
    alignment_block_size: f32,
    layout: parley::Layout<crate::stylo_to_parley::TextBrush>,
    atomic: Vec<Option<AtomicMeasurement>>,
    floats: Vec<InlineFloatPlacement>,
    percentage_basis: Option<f32>,
    line_placements: Vec<InlineLinePlacement>,
    fragments: LineRelativeFragments,
    /// Whether any atomic or floated child contribution changes with the
    /// containing block's block-size.
    depends_on_block_constraints: bool,
}

impl InlineMeasurement {
    fn translate_block_axis(&mut self, offset: f32) {
        if offset == 0.0 {
            return;
        }
        for placement in &mut self.line_placements {
            placement.translate_block_axis(offset);
        }
        for floated in &mut self.floats {
            floated.location.block_offset += offset;
        }
        self.fragments.translate_block_axis(offset);
    }
}

/// Returns the offset for one block-axis alignment subject.
///
/// Taffy's block algorithm applies these same single-subject fallbacks to its
/// numeric children. A Parley IFC is exposed to Taffy as one measured leaf, so
/// its line fragments and child placements must consume the alignment value at
/// this adapter boundary instead. This is the leaf equivalent of Chromium's
/// `AlignBlockContent` plus `BoxFragmentBuilder::MoveChildrenInDirection`, not
/// a post-layout paint translation.
fn single_subject_block_alignment_offset(alignment: Option<AlignContent>, free_space: f32) -> f32 {
    let Some(alignment) = alignment else {
        return 0.0;
    };
    let keyword = resolve_content_alignment_fallback(free_space, 1, alignment);
    compute_content_alignment_offset(free_space, 1, 0.0, keyword, false, true)
}

fn empty_inline_context() -> InlineFormattingContext {
    InlineFormattingContext {
        root_style: LayoutBoxId::from_index(0),
        font_baseline: FontBaseline::Alphabetic,
        unbroken: parley::Layout::default(),
        laid_out: None,
        text_units: Vec::new(),
        source_map: Vec::new(),
        selection: None,
        objects: Vec::new(),
        font_metrics: Vec::new(),
        parent_strut: None,
        root_includes_used_font_metrics: false,
        style_parents: Vec::new(),
        resolved_style_runs: Vec::new(),
        structural_boxes: Vec::new(),
        line_placements: Vec::new(),
        fragments: InlineFragments::default(),
    }
}

fn measure_text(
    text: &str,
    font_size: f32,
    line_height: f32,
    known_dimensions: Size<Option<f32>>,
    available_space: Size<AvailableSpace>,
) -> Size<f32> {
    if text.is_empty() {
        return Size {
            width: known_dimensions.width.unwrap_or(0.0),
            height: known_dimensions.height.unwrap_or(0.0),
        };
    }

    let character_width = (font_size * 0.6).max(0.0);
    let collapsed_words = text.split_whitespace().collect::<Vec<_>>();
    let character_count = if collapsed_words.is_empty() {
        1.0
    } else {
        let word_characters = collapsed_words
            .iter()
            .map(|word| word.chars().count())
            .sum::<usize>();
        (word_characters + collapsed_words.len().saturating_sub(1)) as f32
    };
    let natural_width = character_count * character_width;
    let longest_word = collapsed_words
        .iter()
        .map(|word| word.chars().count())
        .max()
        .unwrap_or(0) as f32
        * character_width;
    let width_limit = match available_space.width {
        AvailableSpace::Definite(width) => width.max(0.0),
        AvailableSpace::MinContent => longest_word,
        AvailableSpace::MaxContent => natural_width,
    };
    let measured_width = if width_limit > 0.0 {
        natural_width.min(width_limit)
    } else {
        0.0
    };
    let line_count = if measured_width > 0.0 {
        (natural_width / measured_width).ceil().max(1.0)
    } else {
        1.0
    };

    Size {
        width: known_dimensions.width.unwrap_or(measured_width),
        height: known_dimensions
            .height
            .unwrap_or(line_height.max(0.0) * line_count),
    }
}
