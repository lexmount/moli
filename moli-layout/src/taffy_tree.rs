use std::{fmt::Debug, hash::Hash};

use parley::{AlignmentOptions, PositionedLayoutItem, YieldData};
use style::Atom;
use taffy::{
    AlignContent, AlignContentKeyword, AlignmentSafety, AvailableSpace, BlockContext,
    BlockFormattingContext, BoxSizing, CacheTree, Clear, Dimension, Display, FloatDirection,
    Layout, LayoutBlockContainer, LayoutFlexboxContainer, LayoutGridContainer, LayoutInput,
    LayoutOutput, LayoutPartialTree, Line, MaybeMath, MaybeResolve, NodeId, Point, ResolveOrZero,
    RoundTree, RunMode, Size, SizingMode, SizingPurpose, Style, TraversePartialTree, TraverseTree,
    compute_block_layout, compute_cached_layout, compute_flexbox_layout, compute_grid_layout,
    compute_hidden_layout, compute_leaf_layout, compute_root_layout, round_layout,
};

use crate::{
    LayoutBoxId, LayoutBoxKind, LayoutCapabilityDiagnostic, LayoutWorld, PaintRect, PaintViewport,
    form::form_control_context,
    inline::{
        InlineFormattingContext, InlineFragments, InlineLinePlacement, InlineObjectRole,
        break_inline_lines, build_inline_fragments, build_inline_line_placements,
        relative_atomic_inset_offset,
    },
    positioned::resolve_absolute_axis_margins,
    replaced::{ReplacedContext, measure_replaced},
    style::{InlineDirection, resolve_stylo_calc_value},
    table::{compute_table_layout, prepare_table_layout_trees},
    world::InlineStaticPosition,
};

// Blink stores box geometry in 1/64 CSS-pixel LayoutUnits.
const LAYOUT_SUBPIXELS_PER_CSS_PIXEL: f32 = 64.0;

pub(crate) fn compute_world_layout<N>(world: &mut LayoutWorld<N>, viewport: PaintViewport)
where
    N: Copy + Debug + Eq + Hash,
{
    for layout_box in &mut world.boxes {
        layout_box.cache.clear();
        layout_box.unrounded_layout = Layout::with_order(0);
        layout_box.final_layout = Layout::with_order(0);
        layout_box.layout_parent = None;
        layout_box.layout_children.clear();
        layout_box.positioned_containing_block = None;
        layout_box.inline_static_position = None;
    }

    world.viewport_layout.children.clear();
    world.viewport_layout.cache.clear();
    world.viewport_layout.unrounded_layout = Layout::with_order(0);
    world.viewport_layout.final_layout = Layout::with_order(0);
    world.viewport_layout.style = Style {
        display: Display::Block,
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
    let positioned_static_placeholders = prepare_layout_tree(world);
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
    finish_block_positioned_layout(world, viewport, &positioned_static_placeholders);
    finish_inline_positioned_layout(world, viewport);
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

fn prepare_layout_tree<N>(world: &mut LayoutWorld<N>) -> Vec<PositionedStaticPlaceholder>
where
    N: Copy + Debug + Eq + Hash,
{
    let mut positioned_static_placeholders = Vec::new();
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
        let positioned_containing_block = if world.boxes[id.index()].style.is_fixed_positioned() {
            nearest_fixed_containing_block(world, Some(original_parent))
        } else if world.boxes[id.index()].style.is_absolute_positioned() {
            nearest_positioned_ancestor(world, Some(original_parent))
        } else {
            None
        };
        let layout_parent = if world.boxes[id.index()].outside_list_marker {
            nearest_list_item_ancestor(world, Some(original_parent))
        } else if inline_owner.is_some() && (is_flattened || !is_positioned) {
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
            && world.boxes[id.index()].style.has_auto_inset_axis()
            && inline_owner.is_none();
        if needs_static_position {
            if original_parent_uses_block_layout(world, original_parent) {
                let placeholder_style = world.boxes[id.index()]
                    .style
                    .positioned_static_placeholder();
                let mut placeholder = LayoutWorld::new_box(
                    None,
                    None,
                    None,
                    format!(
                        "positioned-static-placeholder({})",
                        world.boxes[id.index()].source_label
                    ),
                    None,
                    None,
                    None,
                    LayoutBoxKind::PrincipalBlock,
                    placeholder_style,
                    None,
                    None,
                );
                placeholder.capability_diagnostics.clear();
                placeholder.layout_parent = Some(original_parent);
                let placeholder = world.allocate(placeholder);
                world.boxes[original_parent.index()]
                    .layout_children
                    .push(placeholder);
                positioned_static_placeholders.push(PositionedStaticPlaceholder {
                    child: id,
                    placeholder,
                    original_parent,
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
        if !is_flattened && !world.boxes[id.index()].outside_list_marker {
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
        if !display.is_flex_container() && !display.is_grid_container() {
            continue;
        }
        let mut children = std::mem::take(&mut world.boxes[parent_index].layout_children);
        children.sort_by_key(|child| world.boxes[child.index()].style.order());
        world.boxes[parent_index].layout_children = children;
    }
    positioned_static_placeholders
}

fn original_parent_uses_block_layout<N>(world: &LayoutWorld<N>, parent: LayoutBoxId) -> bool
where
    N: Copy + Debug + Eq + Hash,
{
    let parent = &world.boxes[parent.index()];
    !parent.inline_formatting_context
        && !parent.style.display().is_flex_container()
        && !parent.style.display().is_grid_container()
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
        let parent_width = (item_layout.size.width
            - item_layout.border.left
            - item_layout.border.right
            - item_layout.padding.left
            - item_layout.padding.right)
            .max(0.0);
        let inputs = LayoutInput {
            known_dimensions: Size::NONE,
            definite_dimensions: Size::NONE,
            parent_size: Size {
                width: Some(parent_width),
                height: None,
            },
            available_space: Size {
                width: AvailableSpace::MaxContent,
                height: AvailableSpace::MaxContent,
            },
            sizing_mode: SizingMode::InherentSize,
            sizing_purpose: SizingPurpose::Layout,
            run_mode: RunMode::PerformLayout,
            axis: taffy::RequestedAxis::Both,
            vertical_margins_are_collapsible: Line::FALSE,
        };
        let output = world.compute_child_layout(marker.to_taffy(), inputs);
        let marker_style = &world.boxes[marker.index()].style;
        let marker_margin = marker_style
            .taffy
            .margin
            .resolve_or_zero(Some(parent_width), resolve_stylo_calc_value);
        let gap = marker_style.font_size() * 0.5;
        let direction = world.boxes[item.index()].style.direction();
        let x = match direction {
            InlineDirection::Ltr => {
                item_layout.border.left + item_layout.padding.left
                    - output.size.width
                    - marker_margin.right
                    - gap
            }
            InlineDirection::Rtl => {
                item_layout.size.width - item_layout.border.right - item_layout.padding.right
                    + marker_margin.left
                    + gap
            }
        };
        let item_baseline = world.boxes[item.index()]
            .inline_layout
            .as_ref()
            .and_then(|context| context.line_placements.first())
            .map(|line| item_layout.border.top + item_layout.padding.top + line.baseline);
        let marker_baseline = world.boxes[marker.index()]
            .inline_layout
            .as_ref()
            .and_then(|context| context.line_placements.first())
            .map(|line| line.baseline)
            .unwrap_or(output.size.height);
        let y = item_baseline
            .map(|baseline| baseline - marker_baseline)
            .unwrap_or(item_layout.border.top + item_layout.padding.top);
        world.set_inline_child_layout(
            marker,
            Point { x, y },
            output,
            marker.index(),
            Some(parent_width),
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
        let control_layout = world.boxes[control.index()].unrounded_layout;
        let content_width = (control_layout.size.width
            - control_layout.border.left
            - control_layout.border.right
            - control_layout.padding.left
            - control_layout.padding.right
            - 8.0)
            .max(0.0);
        let inputs = LayoutInput {
            known_dimensions: Size::NONE,
            definite_dimensions: Size::NONE,
            parent_size: Size {
                width: Some(content_width),
                height: None,
            },
            available_space: Size {
                width: AvailableSpace::Definite(content_width),
                height: AvailableSpace::MaxContent,
            },
            sizing_mode: SizingMode::InherentSize,
            sizing_purpose: SizingPurpose::Layout,
            run_mode: RunMode::PerformLayout,
            axis: taffy::RequestedAxis::Both,
            vertical_margins_are_collapsible: Line::FALSE,
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
            Some(content_width),
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
        if layout_box.style.establishes_scroll_container() {
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
        let layout_box = &world.boxes[id.index()];
        if layout_box.establishes_positioned_containing_block() {
            return Some(id);
        }
        candidate = layout_box.parent;
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
        let layout_box = &world.boxes[id.index()];
        if layout_box.establishes_fixed_containing_block() {
            return Some(id);
        }
        candidate = layout_box.parent;
    }
    None
}

#[derive(Clone, Copy, Debug)]
struct PositionedContainingArea {
    origin: Point<f32>,
    size: Size<f32>,
    direction: taffy::Direction,
    requires_inline_layout: bool,
}

#[derive(Clone, Copy, Debug)]
struct PositionedStaticPlaceholder {
    child: LayoutBoxId,
    placeholder: LayoutBoxId,
    original_parent: LayoutBoxId,
}

/// Applies block-container static positions gathered by zero-sized absolute
/// placeholders in the original numeric parent. This is the block analogue
/// of Parley's out-of-flow inline placeholder and keeps the real box attached
/// to its actual absolute/fixed containing block.
fn finish_block_positioned_layout<N>(
    world: &mut LayoutWorld<N>,
    viewport: PaintViewport,
    placeholders: &[PositionedStaticPlaceholder],
) where
    N: Copy + Debug + Eq + Hash,
{
    for placeholder in placeholders {
        let placeholder_layout = world.boxes[placeholder.placeholder.index()].unrounded_layout;
        let parent_origin = unrounded_global_origin(world, placeholder.original_parent);
        let parent_direction = world.boxes[placeholder.original_parent.index()]
            .style
            .taffy
            .direction;
        let parent_is_rtl = parent_direction == taffy::Direction::Rtl;
        let static_local_x = if parent_is_rtl {
            placeholder_layout.location.x
                + placeholder_layout.size.width
                + placeholder_layout.margin.right
        } else {
            placeholder_layout.location.x - placeholder_layout.margin.left
        };
        let static_global = Point {
            x: parent_origin.x + static_local_x,
            y: parent_origin.y + placeholder_layout.location.y - placeholder_layout.margin.top,
        };
        let area = positioned_containing_area(world, placeholder.child, viewport);
        let static_in_area = Point {
            x: static_global.x - area.origin.x,
            y: static_global.y - area.origin.y,
        };
        let numeric_parent_origin = world.boxes[placeholder.child.index()]
            .layout_parent
            .map(|parent| unrounded_global_origin(world, parent))
            .unwrap_or(Point::ZERO);
        apply_inline_static_position(
            world,
            placeholder.child,
            area,
            static_in_area,
            parent_is_rtl,
            numeric_parent_origin,
        );
    }
}

/// Completes positioned descendants whose hypothetical position came from an
/// IFC. Taffy can size ordinary absolute children itself, but an IFC is a leaf
/// in the numeric tree and a flattened positioned inline is not a numeric node
/// at all. Parley's zero-sized out-of-flow placeholder is therefore the sole
/// owner of the static position for these cases.
fn finish_inline_positioned_layout<N>(world: &mut LayoutWorld<N>, viewport: PaintViewport)
where
    N: Copy + Debug + Eq + Hash,
{
    let mut processed = vec![false; world.boxes.len()];
    while let Some(index) = world
        .boxes
        .iter()
        .enumerate()
        .find_map(|(index, layout_box)| {
            (!processed[index] && layout_box.inline_static_position.is_some()).then_some(index)
        })
    {
        processed[index] = true;
        let child = LayoutBoxId::from_index(index);
        let static_position = world.boxes[index]
            .inline_static_position
            .expect("selected positioned box has an IFC static position");
        let area = positioned_containing_area(world, child, viewport);
        let owner_origin = unrounded_global_origin(world, static_position.owner);
        let static_global = Point {
            x: owner_origin.x + static_position.point.x,
            y: owner_origin.y + static_position.point.y,
        };
        let static_in_area = Point {
            x: static_global.x - area.origin.x,
            y: static_global.y - area.origin.y,
        };
        let numeric_parent_origin = world.boxes[index]
            .layout_parent
            .map(|parent| unrounded_global_origin(world, parent))
            .unwrap_or(Point::ZERO);

        if area.requires_inline_layout {
            layout_inline_absolute_child(
                world,
                child,
                area,
                static_in_area,
                static_position.inline_level,
                numeric_parent_origin,
            );
        } else {
            apply_inline_static_position(
                world,
                child,
                area,
                static_in_area,
                area.direction == taffy::Direction::Rtl && static_position.inline_level,
                numeric_parent_origin,
            );
        }
    }
}

fn positioned_containing_area<N>(
    world: &LayoutWorld<N>,
    child: LayoutBoxId,
    viewport: PaintViewport,
) -> PositionedContainingArea
where
    N: Copy + Debug + Eq + Hash,
{
    let Some(containing_block) = world.boxes[child.index()].positioned_containing_block else {
        return PositionedContainingArea {
            origin: Point::ZERO,
            size: Size {
                width: viewport.css_width as f32,
                height: viewport.css_height as f32,
            },
            direction: world.boxes[world.root.index()].style.taffy.direction,
            requires_inline_layout: false,
        };
    };
    let containing_box = &world.boxes[containing_block.index()];
    if containing_box.inline_flattened
        && let Some(owner) = containing_box.inline_context_owner
        && let Some(rect) = inline_box_containing_rect(world, owner, containing_block)
    {
        let owner_box = &world.boxes[owner.index()];
        let owner_layout = owner_box.unrounded_layout;
        let owner_origin = unrounded_global_origin(world, owner);
        return PositionedContainingArea {
            origin: Point {
                x: owner_origin.x + owner_layout.border.left + owner_layout.padding.left + rect.x,
                y: owner_origin.y + owner_layout.border.top + owner_layout.padding.top + rect.y,
            },
            size: Size {
                width: rect.width,
                height: rect.height,
            },
            direction: containing_box.style.taffy.direction,
            requires_inline_layout: true,
        };
    }

    let layout = containing_box.unrounded_layout;
    let origin = unrounded_global_origin(world, containing_block);
    let padding_box_size = Size {
        width: (layout.size.width - layout.border.left - layout.border.right).max(0.0),
        height: (layout.size.height - layout.border.top - layout.border.bottom).max(0.0),
    };
    PositionedContainingArea {
        origin: Point {
            x: origin.x + layout.border.left,
            y: origin.y + layout.border.top,
        },
        size: padding_box_size,
        direction: containing_box.style.taffy.direction,
        requires_inline_layout: containing_box.inline_formatting_context,
    }
}

fn inline_box_containing_rect<N>(
    world: &LayoutWorld<N>,
    owner: LayoutBoxId,
    containing_block: LayoutBoxId,
) -> Option<PaintRect>
where
    N: Copy + Debug + Eq + Hash,
{
    let context = world.boxes[owner.index()].inline_layout.as_ref()?;
    context
        .fragments
        .boxes
        .iter()
        .filter(|fragment| fragment.box_id == containing_block)
        .map(|fragment| fragment.rect)
        .reduce(union_paint_rect)
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

fn apply_inline_static_position<N>(
    world: &mut LayoutWorld<N>,
    child: LayoutBoxId,
    area: PositionedContainingArea,
    static_position: Point<f32>,
    static_position_at_inline_end: bool,
    numeric_parent_origin: Point<f32>,
) where
    N: Copy + Debug + Eq + Hash,
{
    let style = &world.boxes[child.index()].style.taffy;
    let both_horizontal_insets_auto = style.inset.left.is_auto() && style.inset.right.is_auto();
    let both_vertical_insets_auto = style.inset.top.is_auto() && style.inset.bottom.is_auto();
    if !both_horizontal_insets_auto && !both_vertical_insets_auto {
        return;
    }
    let layout = &mut world.boxes[child.index()].unrounded_layout;
    if both_horizontal_insets_auto {
        let x = if static_position_at_inline_end {
            static_position.x - layout.size.width - layout.margin.right
        } else {
            static_position.x + layout.margin.left
        };
        layout.location.x = area.origin.x + x - numeric_parent_origin.x;
    }
    if both_vertical_insets_auto {
        layout.location.y =
            area.origin.y + static_position.y + layout.margin.top - numeric_parent_origin.y;
    }
}

fn layout_inline_absolute_child<N>(
    world: &mut LayoutWorld<N>,
    child: LayoutBoxId,
    area: PositionedContainingArea,
    static_position: Point<f32>,
    inline_level: bool,
    numeric_parent_origin: Point<f32>,
) where
    N: Copy + Debug + Eq + Hash,
{
    let style = world.boxes[child.index()].style.taffy.clone();
    if style.display == Display::None || style.position != taffy::Position::Absolute {
        return;
    }

    let area_width = area.size.width;
    let area_height = area.size.height;
    let aspect_ratio = style.aspect_ratio;
    let margin = style
        .margin
        .map(|value| value.maybe_resolve(area_width, resolve_stylo_calc_value));
    let padding = style
        .padding
        .resolve_or_zero(Some(area_width), resolve_stylo_calc_value);
    let border = style
        .border
        .resolve_or_zero(Some(area_width), resolve_stylo_calc_value);
    let padding_border_sum = (padding + border).sum_axes();
    let box_sizing_adjustment = if style.box_sizing == BoxSizing::ContentBox {
        padding_border_sum
    } else {
        Size::ZERO
    };
    let left = style
        .inset
        .left
        .maybe_resolve(area_width, resolve_stylo_calc_value);
    let right = style
        .inset
        .right
        .maybe_resolve(area_width, resolve_stylo_calc_value);
    let top = style
        .inset
        .top
        .maybe_resolve(area_height, resolve_stylo_calc_value);
    let bottom = style
        .inset
        .bottom
        .maybe_resolve(area_height, resolve_stylo_calc_value);
    let style_size = style
        .size
        .maybe_resolve(area.size, resolve_stylo_calc_value)
        .maybe_apply_aspect_ratio(aspect_ratio)
        .maybe_add(box_sizing_adjustment);
    let min_size = style
        .min_size
        .maybe_resolve(area.size, resolve_stylo_calc_value)
        .maybe_apply_aspect_ratio(aspect_ratio)
        .maybe_add(box_sizing_adjustment)
        .or(padding_border_sum.map(Some))
        .maybe_max(padding_border_sum);
    let max_size = style
        .max_size
        .maybe_resolve(area.size, resolve_stylo_calc_value)
        .maybe_apply_aspect_ratio(aspect_ratio)
        .maybe_add(box_sizing_adjustment);
    let mut known_dimensions = style_size.maybe_clamp(min_size, max_size);

    if let (None, Some(left), Some(right)) = (known_dimensions.width, left, right) {
        known_dimensions.width = Some(
            (area_width.maybe_sub(margin.left).maybe_sub(margin.right) - left - right).max(0.0),
        );
        known_dimensions = known_dimensions
            .maybe_apply_aspect_ratio(aspect_ratio)
            .maybe_clamp(min_size, max_size);
    }
    if let (None, Some(top), Some(bottom)) = (known_dimensions.height, top, bottom) {
        known_dimensions.height = Some(
            (area_height.maybe_sub(margin.top).maybe_sub(margin.bottom) - top - bottom).max(0.0),
        );
        known_dimensions = known_dimensions
            .maybe_apply_aspect_ratio(aspect_ratio)
            .maybe_clamp(min_size, max_size);
    }

    let available_space = Size {
        width: AvailableSpace::Definite(area_width.maybe_clamp(min_size.width, max_size.width)),
        height: AvailableSpace::Definite(area_height.maybe_clamp(min_size.height, max_size.height)),
    };
    if known_dimensions.width.is_none() {
        // CSS 2.2 §10.3.7 resolves an auto-width absolute box with at least
        // one auto horizontal inset as fit-content. Taffy's block/flex paths
        // already implement this contract, but an IFC's Parley placeholder
        // requires Moli to perform the same sizing at this custom seam.
        let non_auto_margin_width = margin.left.unwrap_or(0.0) + margin.right.unwrap_or(0.0);
        let available_width = match (left, right) {
            (Some(left), None) => area_width - left,
            (None, Some(right)) => area_width - right,
            (None, None) if area.direction == taffy::Direction::Rtl && inline_level => {
                static_position.x
            }
            (None, None) => area_width - static_position.x,
            (Some(_), Some(_)) => unreachable!("both insets already resolve auto width"),
        } - non_auto_margin_width;
        known_dimensions.width = Some(world.measure_fit_content_width(
            child,
            LayoutInput {
                known_dimensions,
                definite_dimensions: known_dimensions,
                parent_size: area.size.map(Some),
                available_space,
                sizing_mode: SizingMode::ContentSize,
                sizing_purpose: SizingPurpose::IntrinsicContribution,
                run_mode: RunMode::ComputeSize,
                axis: taffy::RequestedAxis::Horizontal,
                vertical_margins_are_collapsible: Line::FALSE,
            },
            available_width,
        ));
        known_dimensions = known_dimensions
            .maybe_apply_aspect_ratio(aspect_ratio)
            .maybe_clamp(min_size, max_size);
    }
    let measured_size = world
        .compute_child_layout(
            child.to_taffy(),
            LayoutInput {
                known_dimensions,
                definite_dimensions: known_dimensions,
                parent_size: area.size.map(Some),
                available_space,
                sizing_mode: SizingMode::ContentSize,
                sizing_purpose: SizingPurpose::Layout,
                run_mode: RunMode::ComputeSize,
                axis: taffy::RequestedAxis::Both,
                vertical_margins_are_collapsible: Line::FALSE,
            },
        )
        .size;
    let final_size = known_dimensions
        .unwrap_or(measured_size)
        .maybe_clamp(min_size, max_size);
    let output = world.compute_child_layout(
        child.to_taffy(),
        LayoutInput {
            known_dimensions: final_size.map(Some),
            definite_dimensions: known_dimensions,
            parent_size: area.size.map(Some),
            available_space,
            sizing_mode: SizingMode::ContentSize,
            sizing_purpose: SizingPurpose::Layout,
            run_mode: RunMode::PerformLayout,
            axis: taffy::RequestedAxis::Both,
            vertical_margins_are_collapsible: Line::FALSE,
        },
    );

    let horizontal_margin = resolve_absolute_axis_margins(
        Line {
            start: margin.left,
            end: margin.right,
        },
        Line {
            start: left,
            end: right,
        },
        area_width,
        final_size.width,
        false,
        area.direction != taffy::Direction::Rtl,
    );
    let vertical_margin = resolve_absolute_axis_margins(
        Line {
            start: margin.top,
            end: margin.bottom,
        },
        Line {
            start: top,
            end: bottom,
        },
        area_height,
        final_size.height,
        true,
        true,
    );
    let resolved_margin = taffy::Rect {
        left: horizontal_margin.start,
        right: horizontal_margin.end,
        top: vertical_margin.start,
        bottom: vertical_margin.end,
    };
    let x = match (left, right) {
        (Some(left), Some(right)) => {
            if area.direction == taffy::Direction::Rtl {
                area_width - final_size.width - right - resolved_margin.right
            } else {
                left + resolved_margin.left
            }
        }
        (Some(left), None) => left + resolved_margin.left,
        (None, Some(right)) => area_width - final_size.width - right - resolved_margin.right,
        (None, None) if area.direction == taffy::Direction::Rtl && inline_level => {
            static_position.x - final_size.width - resolved_margin.right
        }
        (None, None) => static_position.x + resolved_margin.left,
    };
    let y = top
        .map(|top| top + resolved_margin.top)
        .or_else(|| {
            bottom.map(|bottom| area_height - final_size.height - bottom - resolved_margin.bottom)
        })
        .unwrap_or(static_position.y + resolved_margin.top);
    world.boxes[child.index()].unrounded_layout = Layout {
        order: 0,
        size: final_size,
        content_size: output.content_size,
        scrollbar_size: Size {
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
        },
        location: Point {
            x: area.origin.x + x - numeric_parent_origin.x,
            y: area.origin.y + y - numeric_parent_origin.y,
        },
        padding,
        border,
        margin: resolved_margin,
    };
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

    fn compute_child_layout(&mut self, node_id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        // Float parents own horizontal margin subtraction in both the Taffy
        // block path and Moli's Parley IFC path. The generic intrinsic
        // resolver follows Taffy's ordinary child-input contract and subtracts
        // the child's margins itself, so temporarily restore them here exactly
        // as the leaf adapter below does for final measurement.
        let intrinsic_inputs = if !self.is_viewport_taffy_node(node_id) {
            let id = LayoutBoxId::from_taffy(node_id);
            if box_is_effectively_floated(self, id) {
                let style = &self.boxes[id.index()].style.taffy;
                let margin = style
                    .margin
                    .resolve_or_zero(inputs.parent_size.width, resolve_stylo_calc_value);
                LayoutInput {
                    available_space: inputs
                        .available_space
                        .map_width(|width| width.maybe_add(margin.left + margin.right)),
                    ..inputs
                }
            } else {
                inputs
            }
        } else {
            inputs
        };
        let resolved_intrinsic_inputs =
            taffy::compute::resolve_intrinsic_width_inputs(self, node_id, intrinsic_inputs);
        let inputs = LayoutInput {
            // The float parent has already removed horizontal margins from the
            // child's available space. Restoring them above is only an adapter
            // for Taffy's intrinsic resolver, which owns its own margin
            // subtraction; carrying that restored space into layout would make
            // an auto-width float stretch across its margins again. Only the
            // resolved border-box width crosses this seam.
            known_dimensions: resolved_intrinsic_inputs.known_dimensions,
            ..inputs
        };
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
}

impl<N> CacheTree for LayoutWorld<N>
where
    N: Copy + Debug + Eq + Hash,
{
    fn cache_get(&self, node_id: NodeId, inputs: &LayoutInput) -> Option<LayoutOutput> {
        if self.is_viewport_taffy_node(node_id) {
            self.viewport_layout.cache.get(inputs)
        } else {
            self.boxes[LayoutBoxId::from_taffy(node_id).index()]
                .cache
                .get(inputs)
        }
    }

    fn cache_store(&mut self, node_id: NodeId, inputs: &LayoutInput, output: LayoutOutput) {
        if self.is_viewport_taffy_node(node_id) {
            self.viewport_layout.cache.store(inputs, output);
        } else {
            self.boxes[LayoutBoxId::from_taffy(node_id).index()]
                .cache
                .store(inputs, output);
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
        let inline_formatting_context = layout_box.inline_formatting_context;
        let is_replaced = layout_box.is_replaced();

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
        if display.is_flex_container() {
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

    fn compute_leaf(&mut self, id: LayoutBoxId, inputs: LayoutInput) -> LayoutOutput {
        let layout_box = &self.boxes[id.index()];
        let style = layout_box.style.taffy.clone();
        let text = layout_box.text.clone();
        let font_size = layout_box.style.font_size();
        let line_height = layout_box.style.line_height();
        let replaced_context = layout_box.element_semantics.as_ref().and_then(|semantics| {
            if matches!(
                semantics.category,
                crate::LayoutElementCategory::FormControl(crate::LayoutFormControlKind::Input(
                    crate::LayoutInputControlKind::Image
                ))
            ) {
                // Image buttons use ordinary image replaced sizing, including
                // HTML width/height hints. They remain classified as form
                // controls for DOM state and paint diagnostics only.
                return Some(ReplacedContext::for_element(
                    crate::LayoutReplacedKind::Image,
                    layout_box.replaced_metrics,
                ));
            }
            form_control_context(semantics, font_size, line_height).or_else(|| {
                semantics
                    .replaced
                    .map(|kind| ReplacedContext::for_element(kind, layout_box.replaced_metrics))
            })
        });

        if let Some(context) = replaced_context {
            // `measure_replaced` is the complete CSS replaced-element sizing
            // algorithm ported from Blitz: it resolves preferred/min/max
            // sizes and returns a border-box size. Taffy's generic leaf
            // helper instead expects a content-box measurement callback and
            // adds CSS padding and borders itself. Routing the complete
            // replaced result through that helper would therefore apply the
            // box model twice (most visibly on padded form controls).
            let resolved_aspect_ratio = layout_box
                .style
                .resolved_replaced_aspect_ratio(context.inherent_ratio());
            let size = measure_replaced(
                inputs.known_dimensions,
                inputs.parent_size,
                inputs.available_space,
                &context,
                resolved_aspect_ratio,
                &style,
                inputs.sizing_mode,
                inputs.axis,
            );
            return LayoutOutput {
                size,
                content_size: size,
                first_baselines: Point::NONE,
                last_baselines: Point::NONE,
                top_margin: taffy::tree::CollapsibleMarginSet::ZERO,
                bottom_margin: taffy::tree::CollapsibleMarginSet::ZERO,
                margins_can_collapse_through: false,
            };
        }

        compute_leaf_layout(
            inputs,
            &style,
            resolve_stylo_calc_value,
            |known_dimensions, available_space| {
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
        let is_floated = box_is_effectively_floated(self, id);
        // Both Taffy's block-float parent and Moli's IFC float parent
        // pass the border-box space left after horizontal margins. Taffy's
        // generic leaf adapter normally subtracts a leaf's own margins, so
        // restore them only around that adapter to keep the parent-owned
        // float-margin contract from being applied twice.
        let leaf_inputs = if is_floated {
            let margin = style
                .margin
                .resolve_or_zero(inputs.parent_size.width, resolve_stylo_calc_value);
            LayoutInput {
                available_space: inputs
                    .available_space
                    .map_width(|width| width.maybe_add(margin.left + margin.right)),
                ..inputs
            }
        } else {
            inputs
        };
        let alignment = self.boxes[id.index()].style.text_align();
        let mut inline_context = self.boxes[id.index()]
            .inline_layout
            .take()
            .unwrap_or_else(empty_inline_context);
        let mut measurement = None;
        let mut output = compute_leaf_layout(
            leaf_inputs,
            &style,
            resolve_stylo_calc_value,
            |known_dimensions, available_space| {
                let result = self.measure_inline_context(
                    id,
                    inputs,
                    known_dimensions,
                    available_space,
                    alignment,
                    &inline_context,
                    is_floated,
                    block_context,
                );
                let size = result.size;
                measurement = Some(result);
                size
            },
        );

        if let Some(mut measurement) = measurement {
            let padding = style
                .padding
                .resolve_or_zero(inputs.parent_size.width, resolve_stylo_calc_value);
            let border = style
                .border
                .resolve_or_zero(inputs.parent_size.width, resolve_stylo_calc_value);
            let content_box_height =
                (output.size.height - padding.top - padding.bottom - border.top - border.bottom)
                    .max(0.0);
            let content_box_size = Size {
                width: (output.size.width
                    - padding.left
                    - padding.right
                    - border.left
                    - border.right)
                    .max(0.0),
                height: content_box_height,
            };
            let block_offset = single_subject_block_alignment_offset(
                style.align_content,
                content_box_height - measurement.alignment_block_size,
            );
            measurement.translate_block_axis(block_offset);
            output.content_size.height = output.content_size.height.max(
                measurement.alignment_block_size
                    + padding.top
                    + padding.bottom
                    + block_offset.max(0.0),
            );
            output.first_baselines.y = measurement
                .first_baseline
                .map(|baseline| baseline + padding.top + border.top);
            output.last_baselines.y = measurement
                .last_baseline
                .map(|baseline| baseline + padding.top + border.top);
            if measurement.line_placements.iter().any(|line| !line.phantom) {
                output.margins_can_collapse_through = false;
            }
            if inputs.run_mode == RunMode::PerformLayout {
                self.position_inline_objects(
                    &inline_context,
                    &measurement,
                    Point {
                        x: padding.left + border.left,
                        y: padding.top + border.top,
                    },
                    content_box_size,
                    self.boxes[id.index()].style.direction(),
                );
                inline_context.fragments = measurement.fragments;
                inline_context.line_placements = measurement.line_placements;
                inline_context.laid_out = Some(measurement.layout);
            }
        }

        self.boxes[id.index()].inline_layout = Some(inline_context);
        output
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
        let child_inputs = LayoutInput {
            run_mode: inputs.run_mode,
            sizing_mode: SizingMode::InherentSize,
            sizing_purpose: inputs.sizing_purpose,
            axis: inputs.axis,
            known_dimensions: Size::NONE,
            definite_dimensions: Size::NONE,
            parent_size: available_space.into_options(),
            available_space,
            vertical_margins_are_collapsible: Line::FALSE,
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
        let percentage_basis =
            inline_percentage_basis(available_space.width, inputs.sizing_purpose);
        let mut layout = context.unbroken.clone();
        let mut atomic = vec![None; context.objects.len()];
        let mut atomic_baseline_ascents = vec![None; context.objects.len()];
        let mut structural_edge_contributions = vec![false; context.objects.len()];
        let mut floats = Vec::new();

        for (inline_box, object) in layout.inline_boxes_mut().iter_mut().zip(&context.objects) {
            match object.role {
                InlineObjectRole::Atomic => {
                    let margins = self.boxes[object.box_id.index()]
                        .style
                        .taffy
                        .margin
                        .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
                    let child_output = self.compute_atomic_inline_layout(
                        object.box_id,
                        child_inputs,
                        margins.left + margins.right,
                    );
                    inline_box.width =
                        (margins.left + margins.right + child_output.size.width).max(0.0);
                    inline_box.height =
                        (margins.top + margins.bottom + child_output.size.height).max(0.0);
                    let object_index = usize::try_from(inline_box.id)
                        .expect("Parley returned an inline object id outside usize");
                    atomic_baseline_ascents[object_index] = self
                        .atomic_inline_baseline(object.box_id, child_output)
                        .map(|baseline| margins.top + baseline);
                    atomic[object_index] = Some(AtomicMeasurement {
                        output: child_output,
                        margins,
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
                    let logical_start = object.role == InlineObjectRole::StartEdge;
                    let physical_left =
                        logical_start == (child_style.direction() == InlineDirection::Ltr);
                    let (margin, padding, border) = if physical_left {
                        (margins.left, padding.left, border.left)
                    } else {
                        (margins.right, padding.right, border.right)
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

        let containing_width = known_dimensions
            .width
            .or_else(|| available_space.width.into_option())
            .unwrap_or_default();
        let (indent, indent_options) = self.boxes[owner.index()]
            .style
            .text_indent(containing_width);
        layout.set_text_indent(indent, indent_options);
        let content_widths = layout.calculate_content_widths();
        let has_definite_width = known_dimensions.width.is_some()
            || inputs.known_dimensions.width.is_some()
            || self.boxes[owner.index()]
                .style
                .taffy
                .size
                .width
                .maybe_resolve(inputs.parent_size.width, resolve_stylo_calc_value)
                .is_some();
        let is_unstretched_flex_or_grid_item = inputs.run_mode == RunMode::PerformLayout
            && inputs.known_dimensions.width.is_none()
            && self.boxes[owner.index()]
                .layout_parent
                .is_some_and(|parent| {
                    let display = self.boxes[parent.index()].style.display();
                    display.is_flex_container() || display.is_grid_container()
                });
        let is_intrinsic_contribution =
            inputs.sizing_purpose == SizingPurpose::IntrinsicContribution;
        let shrink_to_fit = !has_definite_width
            && (is_floated
                // A content-based block parent can probe this IFC with a
                // finite available width while its own content width is still
                // unknown. That is an intrinsic contribution, so clamp the
                // available width between the IFC's min/max-content sizes
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
            available_space: Size {
                width: AvailableSpace::MinContent,
                height: AvailableSpace::MaxContent,
            },
            ..child_inputs
        };
        let mut float_min_width: f32 = 0.0;
        let mut float_max_width: f32 = 0.0;
        let mut left_band: f32 = 0.0;
        let mut right_band: f32 = 0.0;
        if !matches!(available_space.width, AvailableSpace::Definite(_)) || shrink_to_fit {
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
                if matches!(clear, taffy::Clear::Left | taffy::Clear::Both) {
                    left_band = 0.0;
                }
                if matches!(clear, taffy::Clear::Right | taffy::Clear::Both) {
                    right_band = 0.0;
                }
                let min_output =
                    self.compute_child_layout(object.box_id.to_taffy(), min_float_inputs);
                let max_output =
                    self.compute_child_layout(object.box_id.to_taffy(), float_max_content_inputs);
                float_min_width =
                    float_min_width.max(min_output.size.width + margin.left + margin.right);
                let outer_width = max_output.size.width + margin.left + margin.right;
                match float {
                    taffy::Float::Left => left_band += outer_width,
                    taffy::Float::Right => right_band += outer_width,
                    taffy::Float::None => {}
                }
                float_max_width = float_max_width.max(left_band + right_band);
            }
        }
        let width = known_dimensions.width.unwrap_or_else(|| {
            match available_space.width {
                AvailableSpace::MinContent => content_widths.min.max(float_min_width),
                AvailableSpace::MaxContent => content_widths.max + float_max_width,
                // Taffy has already resolved and clamped the content-box
                // inline size before invoking the leaf measure function. A
                // normal block IFC must lay out into that definite width;
                // shrinking it to max-content here made RTL alignment and
                // text-indent observe an unrelated inner width.
                AvailableSpace::Definite(limit) if shrink_to_fit => (content_widths.max
                    + float_max_width)
                    .min(limit)
                    .max(content_widths.min.max(float_min_width)),
                AvailableSpace::Definite(limit) => limit,
            }
            .max(0.0)
        });
        // Taffy may feed an intrinsic inline size back through a quantized
        // definite flex/grid constraint, while Parley's content-width and
        // line-breaking passes can accumulate the same glyph advances in a
        // slightly different order. Preserve the max-content boundary plus a
        // small floating-point margin when those widths differ only by noise.
        // Otherwise a one-line flex item can immediately rewrap during final
        // layout. Keep genuinely constrained widths unchanged so normal
        // wrapping is unaffected.
        let intrinsic_max_width = content_widths.max + float_max_width;
        let intrinsic_tolerance = width.abs().max(1.0) * f32::EPSILON * 8.0;
        let line_break_width = if intrinsic_max_width <= width + intrinsic_tolerance {
            width.max(intrinsic_max_width + intrinsic_tolerance)
        } else {
            width
        };
        let max_advance = match available_space.width {
            AvailableSpace::MaxContent => None,
            AvailableSpace::MinContent | AvailableSpace::Definite(_) => Some(line_break_width),
        };
        let has_inline_float = context
            .objects
            .iter()
            .any(|object| object.role == InlineObjectRole::Float);
        let mut float_height = None;
        let mut alignment_float_height = 0.0;
        if has_inline_float
            || block_context
                .as_ref()
                .is_some_and(|context| context.has_floats())
        {
            let container_style = &self.boxes[owner.index()].style.taffy;
            let padding = container_style
                .padding
                .resolve_or_zero(inputs.parent_size.width, resolve_stylo_calc_value);
            let border = container_style
                .border
                .resolve_or_zero(inputs.parent_size.width, resolve_stylo_calc_value);
            let padding_border = padding + border;
            if let Some(block_context) = block_context {
                let contains_floats = block_context.is_bfc_root();
                if contains_floats {
                    block_context.set_width(width + padding_border.left + padding_border.right);
                }
                let mut content_context = block_context.sub_context(
                    padding_border.top,
                    [padding_border.left, padding_border.right],
                );
                self.break_inline_lines_with_floats(
                    context,
                    &mut layout,
                    width,
                    child_inputs,
                    &mut content_context,
                    Point {
                        x: padding_border.left,
                        y: padding_border.top,
                    },
                    &mut floats,
                );
                alignment_float_height = content_context.floated_content_height_contribution();
                if contains_floats {
                    float_height = Some(alignment_float_height);
                }
            } else {
                let mut formatting_context = BlockFormattingContext::new();
                let mut root_context = formatting_context.root_block_context();
                root_context.set_width(width + padding_border.left + padding_border.right);
                let mut content_context = root_context.sub_context(
                    padding_border.top,
                    [padding_border.left, padding_border.right],
                );
                self.break_inline_lines_with_floats(
                    context,
                    &mut layout,
                    width,
                    child_inputs,
                    &mut content_context,
                    Point {
                        x: padding_border.left,
                        y: padding_border.top,
                    },
                    &mut floats,
                );
                alignment_float_height = content_context.floated_content_height_contribution();
                float_height = Some(alignment_float_height);
            }
        } else {
            break_inline_lines(context, &mut layout, max_advance);
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
        let mut height = layout.height() + line_expansion;
        if let Some(float_height) = float_height {
            height = height.max(float_height);
        }
        let alignment_block_size = height.max(alignment_float_height);
        let first_baseline = line_placements
            .iter()
            .find(|line| !line.phantom)
            .map(|line| line.baseline);
        let last_baseline = line_placements
            .iter()
            .rev()
            .find(|line| !line.phantom)
            .map(|line| line.baseline);
        let fragments = build_inline_fragments(context, &layout, &line_placements);
        InlineMeasurement {
            size: Size {
                width: known_dimensions.width.unwrap_or(width),
                height: known_dimensions.height.unwrap_or(height),
            },
            alignment_block_size,
            first_baseline,
            last_baseline,
            layout,
            atomic,
            floats,
            percentage_basis,
            line_placements,
            fragments,
        }
    }

    fn atomic_inline_baseline(&self, id: LayoutBoxId, output: LayoutOutput) -> Option<f32> {
        let layout_box = &self.boxes[id.index()];
        match layout_box.style.display() {
            // Blink's block layout marks these atomic fragments to use their
            // last baseline. A scrolling inline-block instead forces baseline
            // synthesis from its margin-box edge.
            crate::LayoutDisplay::InlineBlock | crate::LayoutDisplay::InlineListItem => {
                (layout_box.style.taffy.overflow.x == taffy::Overflow::Visible
                    && layout_box.style.taffy.overflow.y == taffy::Overflow::Visible)
                    .then_some(output.last_baselines.y)
                    .flatten()
            }
            // Flex, grid, and table formatting contexts expose their first
            // baseline as the automatic inline-level baseline. Do not apply
            // the inline-block overflow exception to these fragment types.
            crate::LayoutDisplay::InlineFlex
            | crate::LayoutDisplay::InlineGrid
            | crate::LayoutDisplay::InlineTable => output.first_baselines.y,
            // Replaced and other atomic inline-level boxes synthesize their
            // baseline at the appropriate box edge in the caller.
            _ => None,
        }
    }

    fn break_inline_lines_with_floats(
        &mut self,
        context: &InlineFormattingContext,
        layout: &mut parley::Layout<crate::stylo_to_parley::TextBrush>,
        width: f32,
        child_inputs: LayoutInput,
        block_context: &mut BlockContext<'_>,
        content_offset: Point<f32>,
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
                    let margin = style
                        .margin
                        .resolve_or_zero(child_inputs.parent_size.width, resolve_stylo_calc_value);
                    // A non-replaced float's formatting-context algorithm
                    // owns its content size; pass it the slot remaining after
                    // margins just like Taffy's block-float parent does. A
                    // replaced leaf retains Taffy's intrinsic-size adapter,
                    // which consumes the full slot and subtracts its margin.
                    let layout_inputs = if self.boxes[child.index()].is_replaced() {
                        child_inputs
                    } else {
                        LayoutInput {
                            available_space: child_inputs
                                .available_space
                                .map_width(|width| width.maybe_sub(margin.left + margin.right)),
                            ..child_inputs
                        }
                    };
                    let output = self.compute_child_layout(child.to_taffy(), layout_inputs);
                    let state = breaker.state_mut();
                    let position = block_context.place_floated_box(
                        output.size + margin.sum_axes(),
                        state.line_y() as f32,
                        direction,
                        style.clear,
                        false,
                    );
                    floats.push(InlineFloatPlacement {
                        child,
                        location: Point {
                            x: content_offset.x + position.x + margin.left,
                            y: content_offset.y + position.y + margin.top,
                        },
                        output,
                        order: usize::try_from(data.inline_box_id).unwrap_or(usize::MAX),
                        parent_width: child_inputs.parent_size.width,
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
        containing_block_size: Size<f32>,
        container_direction: InlineDirection,
    ) {
        for floated in &measurement.floats {
            self.set_inline_child_layout(
                floated.child,
                floated.location,
                floated.output,
                floated.order,
                floated.parent_width,
            );
        }
        for (line_index, line) in measurement.layout.lines().enumerate() {
            let line_placement = measurement.line_placements.get(line_index);
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
                    self.boxes[object.box_id.index()].inline_static_position =
                        Some(InlineStaticPosition {
                            owner: self.boxes[object.box_id.index()]
                                .inline_context_owner
                                .unwrap_or_else(|| panic!("out-of-flow IFC object lost its owner")),
                            point: Point {
                                x: content_offset.x + if inline_level { positioned.x } else { 0.0 },
                                y: content_offset.y + positioned.y + vertical_offset,
                            },
                            inline_level,
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
                    containing_block_size,
                    container_direction,
                );
                self.set_inline_child_layout(
                    object.box_id,
                    Point {
                        x: content_offset.x + positioned.x + atomic.margins.left + inset_offset.x,
                        y: content_offset.y
                            + positioned.y
                            + atomic.margins.top
                            + vertical_offset
                            + inset_offset.y,
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
        parent_width: Option<f32>,
    ) {
        let style = &self.boxes[child.index()].style.taffy;
        let padding = style
            .padding
            .resolve_or_zero(parent_width, resolve_stylo_calc_value);
        let border = style
            .border
            .resolve_or_zero(parent_width, resolve_stylo_calc_value);
        let margin = style
            .margin
            .resolve_or_zero(parent_width, resolve_stylo_calc_value);
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

fn inline_percentage_basis(
    available: AvailableSpace,
    sizing_purpose: SizingPurpose,
) -> Option<f32> {
    available
        .into_option()
        .or_else(|| (sizing_purpose == SizingPurpose::IntrinsicContribution).then_some(0.0))
}

#[cfg(test)]
mod tests {
    use super::{inline_percentage_basis, round_layout_to_css_subpixels};
    use taffy::{
        AvailableSpace, Layout, NodeId, Point, RoundTree, Size, SizingPurpose, TraversePartialTree,
        TraverseTree,
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

    #[test]
    fn intrinsic_inline_percentages_use_a_zero_basis() {
        assert_eq!(
            inline_percentage_basis(
                AvailableSpace::MinContent,
                SizingPurpose::IntrinsicContribution,
            ),
            Some(0.0)
        );
        assert_eq!(
            inline_percentage_basis(
                AvailableSpace::MaxContent,
                SizingPurpose::IntrinsicContribution,
            ),
            Some(0.0)
        );
        assert_eq!(
            inline_percentage_basis(AvailableSpace::Definite(240.0), SizingPurpose::Layout),
            Some(240.0)
        );
        assert_eq!(
            inline_percentage_basis(AvailableSpace::MaxContent, SizingPurpose::Layout),
            None
        );
    }
}

#[derive(Clone, Copy)]
struct AtomicMeasurement {
    output: LayoutOutput,
    margins: taffy::Rect<f32>,
}

#[derive(Clone, Copy)]
struct InlineFloatPlacement {
    child: LayoutBoxId,
    location: Point<f32>,
    output: LayoutOutput,
    order: usize,
    parent_width: Option<f32>,
}

struct InlineMeasurement {
    size: Size<f32>,
    /// Block-end extent of every IFC child used as the single alignment
    /// subject. Unlike `size.height`, this includes non-contained floats
    /// without making them contribute to normal-flow auto height.
    alignment_block_size: f32,
    first_baseline: Option<f32>,
    last_baseline: Option<f32>,
    layout: parley::Layout<crate::stylo_to_parley::TextBrush>,
    atomic: Vec<Option<AtomicMeasurement>>,
    floats: Vec<InlineFloatPlacement>,
    percentage_basis: Option<f32>,
    line_placements: Vec<InlineLinePlacement>,
    fragments: InlineFragments,
}

impl InlineMeasurement {
    fn translate_block_axis(&mut self, offset: f32) {
        if offset == 0.0 {
            return;
        }
        if let Some(first_baseline) = &mut self.first_baseline {
            *first_baseline += offset;
        }
        if let Some(last_baseline) = &mut self.last_baseline {
            *last_baseline += offset;
        }
        for placement in &mut self.line_placements {
            placement.translate_block_axis(offset);
        }
        for floated in &mut self.floats {
            floated.location.y += offset;
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
    let (mut keyword, safe) = match alignment.keyword {
        AlignContentKeyword::Stretch | AlignContentKeyword::SpaceBetween => {
            (AlignContentKeyword::FlexStart, true)
        }
        AlignContentKeyword::SpaceAround | AlignContentKeyword::SpaceEvenly => {
            (AlignContentKeyword::Center, true)
        }
        keyword => (keyword, alignment.safety == AlignmentSafety::Safe),
    };
    if free_space <= 0.0 && safe {
        keyword = AlignContentKeyword::Start;
    }
    match keyword {
        AlignContentKeyword::Start
        | AlignContentKeyword::FlexStart
        | AlignContentKeyword::Stretch
        | AlignContentKeyword::SpaceBetween => 0.0,
        AlignContentKeyword::End | AlignContentKeyword::FlexEnd => free_space,
        AlignContentKeyword::Center
        | AlignContentKeyword::SpaceAround
        | AlignContentKeyword::SpaceEvenly => free_space / 2.0,
    }
}

fn empty_inline_context() -> InlineFormattingContext {
    InlineFormattingContext {
        root_style: LayoutBoxId::from_index(0),
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
