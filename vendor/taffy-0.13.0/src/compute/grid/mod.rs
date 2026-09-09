//! This module is a partial implementation of the CSS Grid Level 1 specification
//! <https://www.w3.org/TR/css-grid-1>
use crate::geometry::{AbsoluteAxis, AbstractAxis, InBothAbsAxis};
use crate::geometry::{Line, Point, Rect, Size};
use crate::style::{AlignItems, AlignSelf, AvailableSpace, Position};
use crate::tree::{
    ChildLayoutInput, Layout, LayoutInput, LayoutOutput, LayoutPartialTreeExt, NodeId, RunMode, SizingMode,
};
use crate::util::debug::debug_log;
use crate::util::sys::{f32_max, f32_min, GridTrackVec, Vec};
use crate::util::MaybeMath;
use crate::util::{MaybeResolve, ResolveOrZero};
use crate::{
    style_helpers::*, AlignContent, BoxGenerationMode, BoxSizing, CoreStyle, GridContainerStyle, GridItemStyle,
    JustifyContent, LayoutGridContainer, RequestedAxis,
};
use alignment::{align_and_position_item, align_tracks};
use explicit_grid::{compute_explicit_grid_size_in_axis, initialize_grid_tracks, AutoRepeatStrategy};
use implicit_grid::compute_grid_size_estimate;
use placement::place_grid_items;
use track_sizing::{
    determine_if_item_crosses_flexible_or_intrinsic_tracks, resolve_item_track_indexes, track_sizing_algorithm,
};
use types::{CellOccupancyMatrix, GridItem, GridTrack, NamedLineResolver, TrackCounts};

use super::common::aspect_ratio::{resolve_size_constraints, SizeConstraintInput, TransferredSizesMode};
use super::common::used_size::resolve_used_size;

#[cfg(feature = "detailed_layout_info")]
use types::GridTrackKind;

pub(crate) use types::{GridCoordinate, GridLine, OriginZeroLine, MAX_GRID_TRACKS, MAX_OZ_LINE, MIN_OZ_LINE};

mod alignment;
mod explicit_grid;
mod implicit_grid;
mod placement;
mod track_sizing;
mod types;
mod util;

/// Grid layout algorithm
/// This consists of a few phases:
///   - Resolving the explicit grid
///   - Placing items (which also resolves the implicit grid)
///   - Track (row/column) sizing
///   - Alignment & Final item placement
pub fn compute_grid_layout<Tree: LayoutGridContainer>(
    tree: &mut Tree,
    node: NodeId,
    inputs: LayoutInput,
) -> LayoutOutput {
    let writing_mode = tree.get_writing_mode(node);
    let percentage_basis = inputs.constraint_space(writing_mode).margin_padding_percentage_basis();
    let LayoutInput { known_dimensions, parent_size, available_space, run_mode, .. } = inputs;

    let resolved_aspect_ratio = tree.get_resolved_aspect_ratio(node);
    let scrollbar_insets = tree.get_scrollbar_insets(node);
    let style = tree.get_grid_container_style(node);
    let direction = style.direction();

    // 1. Compute "available grid space"
    // https://www.w3.org/TR/css-grid-1/#available-grid-space
    let aspect_ratio = if inputs.sizing_mode == SizingMode::InherentSize { resolved_aspect_ratio } else { None };
    let padding = style.padding().resolve_or_zero(percentage_basis, |val, basis| tree.calc(val, basis));
    let border = style.border().resolve_or_zero(percentage_basis, |val, basis| tree.calc(val, basis));
    let padding_border = padding + border;
    let padding_border_size = padding_border.sum_axes();
    let box_sizing = style.box_sizing();
    let box_sizing_adjustment = if box_sizing == BoxSizing::ContentBox { padding_border_size } else { Size::ZERO };

    let (min_size, max_size, preferred_size, preferred_inline_from_aspect_ratio) = match inputs.sizing_mode {
        SizingMode::ContentSize => (Size::NONE, Size::NONE, Size::NONE, false),
        SizingMode::InherentSize => {
            let raw_size = style.size();
            let resolved = resolve_size_constraints(SizeConstraintInput {
                size: raw_size
                    .maybe_resolve(parent_size, |val, basis| tree.calc(val, basis))
                    .maybe_add(box_sizing_adjustment),
                min_size: style
                    .min_size()
                    .maybe_resolve(parent_size, |val, basis| tree.calc(val, basis))
                    .maybe_add(box_sizing_adjustment),
                max_size: style
                    .max_size()
                    .maybe_resolve(parent_size, |val, basis| tree.calc(val, basis))
                    .maybe_add(box_sizing_adjustment),
                size_is_auto: raw_size.map(|dimension| dimension.is_auto()),
                writing_mode,
                block_auto_behavior: inputs.block_auto_behavior,
                transferred_sizes_mode: TransferredSizesMode::Normal,
                aspect_ratio,
                padding_border: padding_border_size,
            });
            (resolved.min_size, resolved.max_size, resolved.size, resolved.aspect_ratio_applied.width)
        }
    };
    let applied_aspect_ratio =
        run_mode == RunMode::ComputeSize && known_dimensions.width.is_none() && preferred_inline_from_aspect_ratio;

    let content_box_inset = padding_border + scrollbar_insets;

    let align_content = style.align_content().unwrap_or(AlignContent::STRETCH);
    let justify_content = style.justify_content().unwrap_or(JustifyContent::STRETCH);
    let align_items = style.align_items();
    let justify_items = style.justify_items();

    // Note: we avoid accessing the grid rows/columns methods more than once as this can
    // cause an expensive-ish computation
    let grid_template_columns = style.grid_template_columns();
    let grid_template_rows = style.grid_template_rows();
    let grid_auto_columns = style.grid_auto_columns();
    let grid_auto_rows = style.grid_auto_rows();

    let outer_node_size = resolve_used_size(known_dimensions, preferred_size, min_size, max_size, padding_border_size);
    let constrained_available_space = outer_node_size
        .map(|size| size.map(AvailableSpace::Definite))
        .unwrap_or(available_space.maybe_clamp(min_size, max_size).maybe_max(padding_border_size));

    let available_grid_space = Size {
        width: constrained_available_space
            .width
            .map_definite_value(|space| space - content_box_inset.horizontal_axis_sum()),
        height: constrained_available_space
            .height
            .map_definite_value(|space| space - content_box_inset.vertical_axis_sum()),
    };

    // The track sizing algorithm operates on the grid container's content box, so the min/max sizes
    // (which are border-box sizes) need converting to content-box sizes before being passed to it
    let inner_min_size = min_size.maybe_sub(content_box_inset.sum_axes());
    let inner_max_size = max_size.maybe_sub(content_box_inset.sum_axes());
    let mut inner_node_size = Size {
        width: outer_node_size.width.map(|space| space - content_box_inset.horizontal_axis_sum()),
        height: outer_node_size.height.map(|space| space - content_box_inset.vertical_axis_sum()),
    };

    debug_log!("parent_size", dbg:parent_size);
    debug_log!("outer_node_size", dbg:outer_node_size);
    debug_log!("inner_node_size", dbg:inner_node_size);

    // Short-circuit layout if the container's size is fully determined by the container's size and the run mode
    // is ComputeSize (and thus the container's size is all that we're interested in)
    if run_mode == RunMode::ComputeSize {
        if let Size { width: Some(width), height: Some(height) } = outer_node_size {
            return LayoutOutput::from_outer_size(Size { width, height })
                .with_applied_aspect_ratio(applied_aspect_ratio);
        }

        // We can also short-circuit if the width is known and only the width has been requested.
        if inputs.axis == RequestedAxis::Horizontal {
            if let Some(width) = outer_node_size.width {
                return LayoutOutput::from_outer_size(Size { width, height: 0.0 })
                    .with_applied_aspect_ratio(applied_aspect_ratio);
            }
        }
    }

    // Absolutely positioned children do not take part in grid placement and do not create
    // implicit tracks, so they are excluded from the grid size estimate.
    let get_child_styles_iter = |node| {
        tree.child_ids(node).map(|child_node: NodeId| tree.get_grid_child_style(child_node)).filter(|style| {
            style.box_generation_mode() != BoxGenerationMode::None && style.position() != Position::Absolute
        })
    };
    let child_styles_iter = get_child_styles_iter(node);

    // 2. Resolve the explicit grid

    // This is very similar to the inner_node_size except if the inner_node_size is not definite but the node
    // has a min- or max- size style then that will be used in it's place.
    let auto_fit_container_size = outer_node_size
        .or(max_size)
        .or(min_size)
        .maybe_clamp(min_size, max_size)
        .maybe_max(padding_border_size)
        .maybe_sub(content_box_inset.sum_axes());

    // If the grid container has a definite size or max size in the relevant axis:
    //   - then the number of repetitions is the largest possible positive integer that does not cause the grid to overflow the content
    //     box of its grid container.
    // Otherwise, if the grid container has a definite min size in the relevant axis:
    //   - then the number of repetitions is the smallest possible positive integer that fulfills that minimum requirement
    // Otherwise, the specified track list repeats only once.
    let auto_repeat_fit_strategy = outer_node_size.or(max_size).map(|val| match val {
        Some(_) => AutoRepeatStrategy::MaxRepetitionsThatDoNotOverflow,
        None => AutoRepeatStrategy::MinRepetitionsThatDoOverflow,
    });

    // Compute the number of rows and columns in the explicit grid *template*
    // (explicit tracks from grid_areas are computed separately below)
    let (col_auto_repetition_count, grid_template_col_count) = compute_explicit_grid_size_in_axis(
        &style,
        auto_fit_container_size.width,
        auto_repeat_fit_strategy.width,
        |val, basis| tree.calc(val, basis),
        AbsoluteAxis::Horizontal,
    );
    let (row_auto_repetition_count, grid_template_row_count) = compute_explicit_grid_size_in_axis(
        &style,
        auto_fit_container_size.height,
        auto_repeat_fit_strategy.height,
        |val, basis| tree.calc(val, basis),
        AbsoluteAxis::Vertical,
    );

    // type CustomIdent<'a> = <<Tree as LayoutPartialTree>::CoreContainerStyle<'_> as CoreStyle>::CustomIdent;
    let mut name_resolver = NamedLineResolver::new(&style, col_auto_repetition_count, row_auto_repetition_count);

    // Clamp the explicit grid to MAX_GRID_TRACKS tracks in each axis
    // https://www.w3.org/TR/css-grid-1/#overlarge-grids
    let explicit_col_count = grid_template_col_count.max(name_resolver.area_column_count()).min(MAX_GRID_TRACKS);
    let explicit_row_count = grid_template_row_count.max(name_resolver.area_row_count()).min(MAX_GRID_TRACKS);

    name_resolver.set_explicit_column_count(explicit_col_count);
    name_resolver.set_explicit_row_count(explicit_row_count);

    // 3. Implicit Grid: Estimate Track Counts
    // Estimate the number of rows and columns in the implicit grid (= the entire grid)
    // This is necessary as part of placement. Doing it early here is a perf optimisation to reduce allocations.
    let (est_col_counts, est_row_counts) =
        compute_grid_size_estimate(explicit_col_count, explicit_row_count, direction, child_styles_iter);

    // 4. Grid Item Placement
    // Match items (children) to a definite grid position (row start/end and column start/end position)
    let mut items = Vec::with_capacity(tree.child_count(node));
    let mut cell_occupancy_matrix = CellOccupancyMatrix::with_track_counts(est_col_counts, est_row_counts);
    let in_flow_children_iter = || {
        tree.child_ids(node)
            .enumerate()
            .map(|(index, child_node)| (index, child_node, tree.get_grid_child_style(child_node)))
            .filter(|(_, _, style)| {
                style.box_generation_mode() != BoxGenerationMode::None && style.position() != Position::Absolute
            })
    };
    place_grid_items(
        &mut cell_occupancy_matrix,
        &mut items,
        in_flow_children_iter,
        writing_mode,
        direction,
        style.grid_auto_flow(),
        align_items.unwrap_or(AlignItems::STRETCH),
        justify_items.unwrap_or(AlignItems::STRETCH),
        &name_resolver,
    );
    for item in &mut items {
        item.aspect_ratio = tree.get_resolved_aspect_ratio(item.node);
    }

    // Extract track counts from previous step (auto-placement can expand the number of tracks)
    let final_col_counts = *cell_occupancy_matrix.track_counts(AbsoluteAxis::Horizontal);
    let final_row_counts = *cell_occupancy_matrix.track_counts(AbsoluteAxis::Vertical);

    // 5. Initialize Tracks
    // Initialize (explicit and implicit) grid tracks (and gutters)
    // This resolves the min and max track sizing functions for all tracks and gutters
    let mut columns = GridTrackVec::new();
    let mut rows = GridTrackVec::new();
    let mut column_track_counts_for_init = final_col_counts;
    if direction.is_rtl() && final_col_counts.explicit <= 1 {
        column_track_counts_for_init.negative_implicit = final_col_counts.positive_implicit;
        column_track_counts_for_init.positive_implicit = final_col_counts.negative_implicit;
    }
    initialize_grid_tracks(
        &mut columns,
        column_track_counts_for_init,
        &style,
        AbsoluteAxis::Horizontal,
        col_auto_repetition_count,
        |column_index| {
            let occupancy_index = if direction.is_rtl() {
                rtl_column_occupancy_index_for_initialization(column_index, final_col_counts)
            } else {
                column_index
            };
            cell_occupancy_matrix.column_is_occupied(occupancy_index)
        },
    );
    initialize_grid_tracks(
        &mut rows,
        final_row_counts,
        &style,
        AbsoluteAxis::Vertical,
        row_auto_repetition_count,
        |row_index| cell_occupancy_matrix.row_is_occupied(row_index),
    );
    if direction.is_rtl() {
        reverse_non_gutter_tracks(&mut columns, final_col_counts);
    }

    drop(grid_template_rows);
    drop(grid_template_columns);
    drop(grid_auto_rows);
    drop(grid_auto_columns);
    drop(style);

    // 6. Track Sizing

    // Convert grid placements in origin-zero coordinates to indexes into the GridTrack (rows and columns) vectors
    // This computation is relatively trivial, but it requires the final number of negative (implicit) tracks in
    // each axis, and doing it up-front here means we don't have to keep repeating that calculation
    resolve_item_track_indexes(&mut items, final_col_counts, final_row_counts);
    // For each item, and in each axis, determine whether the item crosses any flexible (fr) tracks
    // Record this as a boolean (per-axis) on each item for later use in the track-sizing algorithm
    determine_if_item_crosses_flexible_or_intrinsic_tracks(&mut items, &columns, &rows);

    // Determine if the grid has any baseline aligned items
    let has_baseline_aligned_item = items.iter().any(|item| item.align_self == AlignSelf::BASELINE);

    // Run track sizing algorithm for Inline axis
    track_sizing_algorithm(
        tree,
        AbstractAxis::Inline,
        inner_min_size.get(AbstractAxis::Inline),
        inner_max_size.get(AbstractAxis::Inline),
        justify_content,
        align_content,
        available_grid_space,
        inner_node_size,
        &mut columns,
        &mut rows,
        &mut items,
        |track: &GridTrack, parent_size: Option<f32>, tree: &Tree| {
            track.max_track_sizing_function.definite_value(parent_size, |val, basis| tree.calc(val, basis))
        },
        has_baseline_aligned_item,
    );
    let initial_column_sum = columns.iter().map(|track| track.base_size).sum::<f32>();
    inner_node_size.width = inner_node_size.width.or_else(|| initial_column_sum.into());

    items.iter_mut().for_each(|item| item.grid_area_size_cache = None);

    // Run track sizing algorithm for Block axis
    track_sizing_algorithm(
        tree,
        AbstractAxis::Block,
        inner_min_size.get(AbstractAxis::Block),
        inner_max_size.get(AbstractAxis::Block),
        align_content,
        justify_content,
        available_grid_space,
        inner_node_size,
        &mut rows,
        &mut columns,
        &mut items,
        |track: &GridTrack, _, _| Some(track.base_size),
        false, // TODO: Support baseline alignment in the vertical axis
    );
    let initial_row_sum = rows.iter().map(|track| track.base_size).sum::<f32>();
    inner_node_size.height = inner_node_size.height.or_else(|| initial_row_sum.into());

    debug_log!("initial_column_sum", dbg:initial_column_sum);
    debug_log!(dbg: columns.iter().map(|track| track.base_size).collect::<Vec<_>>());
    debug_log!("initial_row_sum", dbg:initial_row_sum);
    debug_log!(dbg: rows.iter().map(|track| track.base_size).collect::<Vec<_>>());

    // 6. Compute container size
    let resolved_style_size = known_dimensions.or(preferred_size);
    let mut container_border_box = Size {
        width: resolved_style_size
            .get(AbstractAxis::Inline)
            .unwrap_or_else(|| initial_column_sum + content_box_inset.horizontal_axis_sum())
            .maybe_clamp(min_size.width, max_size.width)
            .max(padding_border_size.width),
        height: resolved_style_size
            .get(AbstractAxis::Block)
            .unwrap_or_else(|| initial_row_sum + content_box_inset.vertical_axis_sum())
            .maybe_clamp(min_size.height, max_size.height)
            .max(padding_border_size.height),
    };
    let mut container_content_box = Size {
        width: f32_max(0.0, container_border_box.width - content_box_inset.horizontal_axis_sum()),
        height: f32_max(0.0, container_border_box.height - content_box_inset.vertical_axis_sum()),
    };

    // If only the container's size has been requested
    if run_mode == RunMode::ComputeSize {
        let depends_on_block_constraints = items.iter().any(|item| item.depends_on_block_constraints);
        return LayoutOutput::from_outer_size(container_border_box)
            .with_block_constraint_dependency(depends_on_block_constraints)
            .with_applied_aspect_ratio(applied_aspect_ratio);
    }

    // 7. Resolve percentage track base sizes
    // In the case of an indefinitely sized container these resolve to zero during the "Initialise Tracks" step
    // and therefore need to be re-resolved here based on the content-sized content box of the container
    if !available_grid_space.width.is_definite() {
        for column in &mut columns {
            let min: Option<f32> = column
                .min_track_sizing_function
                .resolved_percentage_size(container_content_box.width, |val, basis| tree.calc(val, basis));
            let max: Option<f32> = column
                .max_track_sizing_function
                .resolved_percentage_size(container_content_box.width, |val, basis| tree.calc(val, basis));
            column.base_size = column.base_size.maybe_clamp(min, max);
        }
    }
    if !available_grid_space.height.is_definite() {
        for row in &mut rows {
            let min: Option<f32> = row
                .min_track_sizing_function
                .resolved_percentage_size(container_content_box.height, |val, basis| tree.calc(val, basis));
            let max: Option<f32> = row
                .max_track_sizing_function
                .resolved_percentage_size(container_content_box.height, |val, basis| tree.calc(val, basis));
            row.base_size = row.base_size.maybe_clamp(min, max);
        }
    }

    // Column sizing must be re-run (once) if:
    //   - The grid container's width was initially indefinite and there are any columns with percentage track sizing functions
    //   - Any grid item crossing an intrinsically sized track's min content contribution width has changed
    // TODO: Only rerun sizing for tracks that actually require it rather than for all tracks if any need it.
    let mut rerun_column_sizing;
    let mut intrinsic_column_contribution_changed = false;

    let has_percentage_column = columns.iter().any(|track| track.uses_percentage());
    let has_percentage_row = rows.iter().any(|track| track.uses_percentage());
    let parent_width_indefinite = !available_space.width.is_definite();
    rerun_column_sizing = parent_width_indefinite && has_percentage_column;

    if !rerun_column_sizing {
        intrinsic_column_contribution_changed =
            items.iter_mut().filter(|item| item.crosses_intrinsic_column).any(|item| {
                let grid_area_size = item.grid_area_size(
                    AbstractAxis::Inline,
                    &columns,
                    &rows,
                    inner_node_size,
                    |track: &GridTrack, _| Some(track.base_size),
                    &|val, basis| tree.calc(val, basis),
                );
                let available_space = grid_area_size.with(AbstractAxis::Inline, None);
                let new_min_content_contribution =
                    item.min_content_contribution(AbstractAxis::Inline, tree, grid_area_size, available_space);

                let has_changed = Some(new_min_content_contribution) != item.min_content_contribution_cache.width;

                item.grid_area_size_cache = Some(grid_area_size);
                item.min_content_contribution_cache.width = Some(new_min_content_contribution);
                item.max_content_contribution_cache.width = None;
                item.minimum_contribution_cache.width = None;

                has_changed
            });
        rerun_column_sizing = intrinsic_column_contribution_changed;
    } else {
        // Clear intrinsic width caches
        items.iter_mut().for_each(|item| {
            item.grid_area_size_cache = None;
            item.min_content_contribution_cache.width = None;
            item.max_content_contribution_cache.width = None;
            item.minimum_contribution_cache.width = None;
        });
    }

    let mut intrinsic_row_contribution_changed = false;

    if rerun_column_sizing {
        // Re-run track sizing algorithm for Inline axis
        track_sizing_algorithm(
            tree,
            AbstractAxis::Inline,
            inner_min_size.get(AbstractAxis::Inline),
            inner_max_size.get(AbstractAxis::Inline),
            justify_content,
            align_content,
            available_grid_space,
            inner_node_size,
            &mut columns,
            &mut rows,
            &mut items,
            |track: &GridTrack, _, _| Some(track.base_size),
            has_baseline_aligned_item,
        );

        // Row sizing must be re-run (once) if:
        //   - The grid container's height was initially indefinite and there are any rows with percentage track sizing functions
        //   - Any grid item crossing an intrinsically sized track's min content contribution height has changed
        // TODO: Only rerun sizing for tracks that actually require it rather than for all tracks if any need it.
        let mut rerun_row_sizing;

        let parent_height_indefinite = !available_space.height.is_definite();
        rerun_row_sizing = parent_height_indefinite && has_percentage_row;

        if !rerun_row_sizing {
            intrinsic_row_contribution_changed =
                items.iter_mut().filter(|item| item.crosses_intrinsic_column).any(|item| {
                    let grid_area_size = item.grid_area_size(
                        AbstractAxis::Block,
                        &rows,
                        &columns,
                        inner_node_size,
                        |track: &GridTrack, _| Some(track.base_size),
                        &|val, basis| tree.calc(val, basis),
                    );
                    let available_space = grid_area_size.with(AbstractAxis::Block, None);
                    let new_min_content_contribution =
                        item.min_content_contribution(AbstractAxis::Block, tree, grid_area_size, available_space);

                    let has_changed = Some(new_min_content_contribution) != item.min_content_contribution_cache.height;

                    item.grid_area_size_cache = Some(grid_area_size);
                    item.min_content_contribution_cache.height = Some(new_min_content_contribution);
                    item.max_content_contribution_cache.height = None;
                    item.minimum_contribution_cache.height = None;

                    has_changed
                });
            rerun_row_sizing = intrinsic_row_contribution_changed;
        } else {
            items.iter_mut().for_each(|item| {
                // Clear intrinsic height caches
                item.grid_area_size_cache = None;
                item.min_content_contribution_cache.height = None;
                item.max_content_contribution_cache.height = None;
                item.minimum_contribution_cache.height = None;
            });
        }

        if rerun_row_sizing {
            // Re-run track sizing algorithm for Block axis
            track_sizing_algorithm(
                tree,
                AbstractAxis::Block,
                inner_min_size.get(AbstractAxis::Block),
                inner_max_size.get(AbstractAxis::Block),
                align_content,
                justify_content,
                available_grid_space,
                inner_node_size,
                &mut rows,
                &mut columns,
                &mut items,
                |track: &GridTrack, _, _| Some(track.base_size),
                false, // TODO: Support baseline alignment in the vertical axis
            );
        }
    }

    if (intrinsic_column_contribution_changed && !has_percentage_column)
        || (intrinsic_row_contribution_changed && !has_percentage_row)
    {
        let final_column_sum = columns.iter().map(|track| track.base_size).sum::<f32>();
        let final_row_sum = rows.iter().map(|track| track.base_size).sum::<f32>();

        if intrinsic_column_contribution_changed && !has_percentage_column {
            container_border_box.width = resolved_style_size
                .get(AbstractAxis::Inline)
                .unwrap_or_else(|| final_column_sum + content_box_inset.horizontal_axis_sum())
                .maybe_clamp(min_size.width, max_size.width)
                .max(padding_border_size.width);
            container_content_box.width =
                f32_max(0.0, container_border_box.width - content_box_inset.horizontal_axis_sum());
        }

        if intrinsic_row_contribution_changed && !has_percentage_row {
            container_border_box.height = resolved_style_size
                .get(AbstractAxis::Block)
                .unwrap_or_else(|| final_row_sum + content_box_inset.vertical_axis_sum())
                .maybe_clamp(min_size.height, max_size.height)
                .max(padding_border_size.height);
            container_content_box.height =
                f32_max(0.0, container_border_box.height - content_box_inset.vertical_axis_sum());
        }
    }

    // If only the container's size has been requested
    if run_mode == RunMode::ComputeSize {
        let depends_on_block_constraints = items.iter().any(|item| item.depends_on_block_constraints);
        return LayoutOutput::from_outer_size(container_border_box)
            .with_block_constraint_dependency(depends_on_block_constraints)
            .with_applied_aspect_ratio(applied_aspect_ratio);
    }

    // 8. Track Alignment

    // Align columns
    let inline_size_without_scrollbar = f32_max(container_border_box.width - padding_border_size.width, 0.0);
    let inline_scrollbar_scale = {
        let total = scrollbar_insets.horizontal_axis_sum();
        if total > 0.0 {
            f32_min(1.0, inline_size_without_scrollbar / total)
        } else {
            1.0
        }
    };
    align_tracks(
        container_content_box.get(AbstractAxis::Inline),
        Line {
            start: padding.left + scrollbar_insets.left * inline_scrollbar_scale,
            end: padding.right + scrollbar_insets.right * inline_scrollbar_scale,
        },
        Line { start: border.left, end: border.right },
        &mut columns,
        justify_content,
        direction.is_rtl(),
    );
    // Align rows
    let block_size_without_scrollbar = f32_max(container_border_box.height - padding_border_size.height, 0.0);
    let block_scrollbar_scale = {
        let total = scrollbar_insets.vertical_axis_sum();
        if total > 0.0 {
            f32_min(1.0, block_size_without_scrollbar / total)
        } else {
            1.0
        }
    };
    align_tracks(
        container_content_box.get(AbstractAxis::Block),
        Line {
            start: padding.top + scrollbar_insets.top * block_scrollbar_scale,
            end: padding.bottom + scrollbar_insets.bottom * block_scrollbar_scale,
        },
        Line { start: border.top, end: border.bottom },
        &mut rows,
        align_content,
        false,
    );

    // 9. Size, Align, and Position Grid Items

    #[cfg_attr(not(feature = "content_size"), allow(unused_mut))]
    let mut item_content_size_contribution = Size::ZERO;
    #[cfg_attr(not(feature = "content_size"), allow(unused_mut, unused))]
    let mut absolute_content_size = Size::ZERO;

    // Sort items back into original order to allow them to be matched up with styles
    items.sort_by_key(|item| item.source_order);

    let container_alignment_styles = InBothAbsAxis { horizontal: justify_items, vertical: align_items };

    // Position in-flow children (stored in items vector)
    for (index, item) in items.iter_mut().enumerate() {
        let grid_area = Rect {
            top: rows[item.row_indexes.start as usize + 1].offset,
            bottom: rows[item.row_indexes.end as usize].offset,
            left: columns[item.column_indexes.start as usize + 1].offset,
            right: columns[item.column_indexes.end as usize].offset,
        };
        #[cfg_attr(not(feature = "content_size"), allow(unused_variables))]
        let placement = align_and_position_item(
            tree,
            item.node,
            index as u32,
            grid_area,
            container_alignment_styles,
            item.baseline_shim,
            direction,
            writing_mode,
            container_border_box.width,
            border,
            scrollbar_insets,
        );
        item.y_position = placement.block_start;
        item.height = placement.block_size;
        item.first_baseline = placement.first_baseline;
        item.last_baseline = placement.last_baseline;

        #[cfg(feature = "content_size")]
        {
            item_content_size_contribution =
                item_content_size_contribution.f32_max(placement.content_size_contribution);
        }
    }

    // Position hidden and absolutely positioned children
    let mut order = items.len() as u32;
    (0..tree.child_count(node)).for_each(|index| {
        let child = tree.get_child_id(node, index);
        let child_style = tree.get_grid_child_style(child);

        // Position hidden child
        if child_style.box_generation_mode() == BoxGenerationMode::None {
            drop(child_style);
            tree.set_unrounded_layout(child, &Layout::with_order(order));
            tree.perform_child_layout(
                child,
                ChildLayoutInput::new(
                    Size::NONE,
                    Size::NONE,
                    writing_mode,
                    Size::MAX_CONTENT,
                    SizingMode::InherentSize,
                    Line::FALSE,
                ),
            );
            order += 1;
            return;
        }

        // Position absolutely positioned child
        if child_style.position() == Position::Absolute {
            // Convert grid-col-{start/end} into Option's of indexes into the columns vector
            // The Option is None if the style property is Auto and an unresolvable Span
            let maybe_col_indexes = name_resolver
                .resolve_column_names(&child_style.grid_column())
                .into_origin_zero(final_col_counts.explicit)
                .resolve_absolutely_positioned_grid_tracks()
                .map(|maybe_grid_line| {
                    maybe_grid_line
                        .map(|line: OriginZeroLine| {
                            if direction.is_rtl() {
                                OriginZeroLine(final_col_counts.explicit as i16 - line.0)
                            } else {
                                line
                            }
                        })
                        .and_then(|line| line.try_into_track_vec_index(final_col_counts))
                });
            let maybe_col_indexes = if direction.is_rtl() {
                Line { start: maybe_col_indexes.end, end: maybe_col_indexes.start }
            } else {
                maybe_col_indexes
            };
            // Convert grid-row-{start/end} into Option's of indexes into the row vector
            // The Option is None if the style property is Auto and an unresolvable Span
            let maybe_row_indexes = name_resolver
                .resolve_row_names(&child_style.grid_row())
                .into_origin_zero(final_row_counts.explicit)
                .resolve_absolutely_positioned_grid_tracks()
                .map(|maybe_grid_line| {
                    maybe_grid_line.and_then(|line: OriginZeroLine| line.try_into_track_vec_index(final_row_counts))
                });

            // Content alignment (align-content/justify-content) may distribute free space before, between,
            // or after tracks. Grid lines used by absolutely positioned items resolve to the edges of the
            // tracks adjacent to the line rather than to the raw gutter offset:
            //   - As a start edge, a line resolves to the start of the track that follows it
            //   - As an end edge, a line resolves to the end of the track that precedes it
            /// Resolve a grid line (by track vector index) used as a start edge to a position
            fn line_as_start_edge(tracks: &[GridTrack], index: usize) -> f32 {
                tracks.get(index + 1).unwrap_or(&tracks[index]).offset
            }
            /// Resolve a grid line (by track vector index) used as an end edge to a position
            fn line_as_end_edge(tracks: &[GridTrack], index: usize) -> f32 {
                if index == 0 {
                    tracks.get(1).unwrap_or(&tracks[0]).offset
                } else {
                    tracks[index].offset
                }
            }

            let grid_area = Rect {
                top: maybe_row_indexes
                    .start
                    .map(|index| line_as_start_edge(&rows, index))
                    .unwrap_or(border.top + scrollbar_insets.top),
                bottom: maybe_row_indexes
                    .end
                    .map(|index| line_as_end_edge(&rows, index))
                    .unwrap_or(container_border_box.height - border.bottom - scrollbar_insets.bottom),
                left: maybe_col_indexes
                    .start
                    .map(|index| line_as_start_edge(&columns, index))
                    .unwrap_or(border.left + scrollbar_insets.left),
                right: maybe_col_indexes
                    .end
                    .map(|index| line_as_end_edge(&columns, index))
                    .unwrap_or(container_border_box.width - border.right - scrollbar_insets.right),
            };
            drop(child_style);

            // TODO: Baseline alignment support for absolutely positioned items (should check if is actually specified)
            #[cfg_attr(not(feature = "content_size"), allow(unused_variables))]
            let placement = align_and_position_item(
                tree,
                child,
                order,
                grid_area,
                container_alignment_styles,
                0.0,
                direction,
                writing_mode,
                container_border_box.width,
                border,
                scrollbar_insets,
            );
            #[cfg(feature = "content_size")]
            {
                absolute_content_size = absolute_content_size.f32_max(placement.content_size_contribution);
            }

            order += 1;
        }
    });

    // Set detailed grid information
    #[cfg(feature = "detailed_layout_info")]
    tree.set_detailed_grid_info(
        node,
        DetailedGridInfo {
            rows: DetailedGridTracksInfo::from_grid_tracks_and_track_count(
                final_row_counts,
                row_auto_repetition_count,
                rows,
            ),
            columns: DetailedGridTracksInfo::from_grid_tracks_and_track_count(
                final_col_counts,
                col_auto_repetition_count,
                columns,
            ),
            items: items.iter().map(DetailedGridItemsInfo::from_grid_item).collect(),
        },
    );

    // If there are not items then return just the container size (no baseline)
    if items.is_empty() {
        return LayoutOutput::from_outer_size(container_border_box);
    }

    let (first_baseline, last_baseline) = grid_container_baselines(&items);

    // The container's own padding at the end of the content is part of its scrollable
    // overflow region, so it is included in the in-flow content size.
    #[cfg(feature = "content_size")]
    let content_size = {
        let mut content_size = item_content_size_contribution;
        content_size.width += if direction.is_rtl() { padding.left } else { padding.right };
        content_size.height += padding.bottom;
        content_size.f32_max(absolute_content_size)
    };
    #[cfg(not(feature = "content_size"))]
    let content_size = item_content_size_contribution;

    LayoutOutput::from_sizes_and_baseline_sets(
        container_border_box,
        content_size,
        Point { x: None, y: Some(first_baseline) },
        Point { x: None, y: Some(last_baseline) },
    )
}

/// Select the grid container's baselines from final item fragments.
///
/// This is the horizontal-writing-mode subset of Blink's
/// `GridBaselineAccumulator`: a baseline-sharing group in the first/last
/// occupied row wins, otherwise selection falls back to the first/last item in
/// grid order. Fallback selection uses the child's corresponding baseline and
/// synthesizes one at its block-end border edge only when that baseline is
/// absent.
fn grid_container_baselines(items: &[GridItem]) -> (f32, f32) {
    debug_assert!(!items.is_empty());

    let first_occupied_row = items.iter().map(|item| item.row_indexes.start).min().unwrap();
    let first_item = items
        .iter()
        .filter(|item| item.row_indexes.start == first_occupied_row && item.align_self == AlignSelf::BASELINE)
        .min_by_key(|item| (item.column_indexes.start, item.source_order))
        .or_else(|| {
            items
                .iter()
                .filter(|item| item.row_indexes.start == first_occupied_row)
                .min_by_key(|item| (item.column_indexes.start, item.source_order))
        })
        .unwrap();
    let first_baseline = first_item.y_position + first_item.first_baseline.unwrap_or(first_item.height);

    // GridTrackVec stores one line/gutter slot on each side of a row track, so
    // the start index of an item's last occupied row is two slots before its
    // end index.
    let last_occupied_row = items.iter().map(|item| item.row_indexes.end.saturating_sub(2)).max().unwrap();
    let last_item = items
        .iter()
        .filter(|item| item.row_indexes.start == last_occupied_row && item.align_self == AlignSelf::BASELINE)
        .max_by_key(|item| (item.column_indexes.end, item.source_order))
        .or_else(|| items.iter().max_by_key(|item| (item.row_indexes.end, item.column_indexes.end, item.source_order)))
        .unwrap();
    let last_baseline =
        if last_item.align_self == AlignSelf::BASELINE && last_item.row_indexes.start == last_occupied_row {
            // Taffy currently supports first-baseline sharing groups. When such a
            // group exists in the last occupied row, its shared major baseline is
            // also the grid container's last baseline.
            last_item.first_baseline.unwrap_or(last_item.height)
        } else {
            last_item.last_baseline.unwrap_or(last_item.height)
        };

    (first_baseline, last_item.y_position + last_baseline)
}

/// Reverses only non-gutter column tracks in-place while preserving line/gutter slots.
fn reverse_non_gutter_tracks(tracks: &mut [GridTrack], track_counts: TrackCounts) {
    // When the explicit grid has 0/1 tracks, visual RTL mirroring is entirely determined by implicit tracks.
    // Reverse all non-gutter tracks in that case.
    if track_counts.explicit <= 1 {
        const MIN_TRACK_VEC_LEN_TO_REVERSE_COLUMNS: usize = 5;
        if tracks.len() < MIN_TRACK_VEC_LEN_TO_REVERSE_COLUMNS {
            return;
        }
        let mut left = 1;
        let mut right = tracks.len() - 2;
        while left < right {
            tracks.swap(left, right);
            left += 2;
            right = right.saturating_sub(2);
        }
        return;
    }

    let explicit_track_count = track_counts.explicit as usize;
    if explicit_track_count < 2 {
        return;
    }

    let mut left = track_counts.negative_implicit as usize;
    let mut right = left + explicit_track_count - 1;
    while left < right {
        tracks.swap((2 * left) + 1, (2 * right) + 1);
        left += 1;
        right = right.saturating_sub(1);
    }
}

/// Maps initialized column indexes to occupancy-matrix indexes for auto-fit collapsing in RTL.
fn rtl_column_occupancy_index_for_initialization(column_index: usize, track_counts: TrackCounts) -> usize {
    if track_counts.explicit <= 1 {
        return track_counts.len() - column_index - 1;
    }

    let explicit_start = track_counts.negative_implicit as usize;
    let explicit_end = explicit_start + track_counts.explicit as usize;
    if (explicit_start..explicit_end).contains(&column_index) {
        explicit_start + (explicit_end - column_index - 1)
    } else {
        column_index
    }
}

/// Information from the computation of grid
#[derive(Debug, Clone, PartialEq)]
#[cfg(feature = "detailed_layout_info")]
pub struct DetailedGridInfo {
    /// <https://drafts.csswg.org/css-grid-1/#grid-row>
    pub rows: DetailedGridTracksInfo,
    /// <https://drafts.csswg.org/css-grid-1/#grid-column>
    pub columns: DetailedGridTracksInfo,
    /// <https://drafts.csswg.org/css-grid-1/#grid-items>
    pub items: Vec<DetailedGridItemsInfo>,
}

/// Information from the computation of grids tracks
#[derive(Debug, Clone, PartialEq)]
#[cfg(feature = "detailed_layout_info")]
pub struct DetailedGridTracksInfo {
    /// Number of leading implicit grid tracks
    pub negative_implicit_tracks: u16,
    /// Number of explicit grid tracks
    pub explicit_tracks: u16,
    /// Number of trailing implicit grid tracks
    pub positive_implicit_tracks: u16,
    /// Number of expansions of the axis' `repeat(auto-fill, ...)` or
    /// `repeat(auto-fit, ...)` component.
    ///
    /// This is retained separately because `explicit_tracks` can also be
    /// enlarged by `grid-template-areas`, so the final track count cannot be
    /// used to reconstruct the auto-repeat expansion unambiguously.
    pub auto_repetitions: u16,

    /// Gutters between tracks
    pub gutters: Vec<f32>,
    /// The used size of the tracks
    pub sizes: Vec<f32>,
}

#[cfg(feature = "detailed_layout_info")]
impl DetailedGridTracksInfo {
    /// Get the base_size of [`GridTrack`] with a kind [`types::GridTrackKind`]
    #[inline(always)]
    fn grid_track_base_size_of_kind(grid_tracks: &[GridTrack], kind: GridTrackKind) -> Vec<f32> {
        grid_tracks
            .iter()
            .filter_map(|track| match track.kind == kind {
                true => Some(track.base_size),
                false => None,
            })
            .collect()
    }

    /// Get the sizes of the gutters
    fn gutters_from_grid_track_layout(grid_tracks: &[GridTrack]) -> Vec<f32> {
        DetailedGridTracksInfo::grid_track_base_size_of_kind(grid_tracks, GridTrackKind::Gutter)
    }

    /// Get the sizes of the tracks
    fn sizes_from_grid_track_layout(grid_tracks: &[GridTrack]) -> Vec<f32> {
        DetailedGridTracksInfo::grid_track_base_size_of_kind(grid_tracks, GridTrackKind::Track)
    }

    /// Construct DetailedGridTracksInfo from TrackCounts and GridTracks
    fn from_grid_tracks_and_track_count(
        track_count: TrackCounts,
        auto_repetitions: u16,
        grid_tracks: Vec<GridTrack>,
    ) -> Self {
        DetailedGridTracksInfo {
            negative_implicit_tracks: track_count.negative_implicit,
            explicit_tracks: track_count.explicit,
            positive_implicit_tracks: track_count.positive_implicit,
            auto_repetitions,
            gutters: DetailedGridTracksInfo::gutters_from_grid_track_layout(&grid_tracks),
            sizes: DetailedGridTracksInfo::sizes_from_grid_track_layout(&grid_tracks),
        }
    }
}

/// Grid area information from the placement algorithm
///
/// The values is 1-indexed grid line numbers bounding the area.
/// This matches the Chrome and Firefox's format as of 2nd Jan 2024.
#[derive(Debug, Clone, PartialEq)]
#[cfg(feature = "detailed_layout_info")]
pub struct DetailedGridItemsInfo {
    /// row-start with 1-indexed grid line numbers
    pub row_start: u16,
    /// row-end with 1-indexed grid line numbers
    pub row_end: u16,
    /// column-start with 1-indexed grid line numbers
    pub column_start: u16,
    /// column-end with 1-indexed grid line numbers
    pub column_end: u16,
}

/// Grid area information from the placement algorithm
#[cfg(feature = "detailed_layout_info")]
impl DetailedGridItemsInfo {
    /// Construct from GridItems
    #[inline(always)]
    fn from_grid_item(grid_item: &GridItem) -> Self {
        /// Conversion from the indexes of Vec<GridTrack> into 1-indexed grid line numbers. See [`GridItem::row_indexes`] or [`GridItem::column_indexes`]
        #[inline(always)]
        fn to_one_indexed_grid_line(grid_track_index: u16) -> u16 {
            grid_track_index / 2 + 1
        }

        DetailedGridItemsInfo {
            row_start: to_one_indexed_grid_line(grid_item.row_indexes.start),
            row_end: to_one_indexed_grid_line(grid_item.row_indexes.end),
            column_start: to_one_indexed_grid_line(grid_item.column_indexes.start),
            column_end: to_one_indexed_grid_line(grid_item.column_indexes.end),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Style;

    #[allow(clippy::too_many_arguments)]
    fn baseline_item(
        source_order: u16,
        row_indexes: Line<u16>,
        column_indexes: Line<u16>,
        participates_in_baseline_alignment: bool,
        y_position: f32,
        height: f32,
        first_baseline: Option<f32>,
        last_baseline: Option<f32>,
    ) -> GridItem {
        let style: Style =
            Style { align_self: participates_in_baseline_alignment.then_some(AlignSelf::BASELINE), ..Style::default() };
        let mut item = GridItem::new_with_placement_style_and_order(
            NodeId::new(u64::from(source_order)),
            crate::WritingMode::HorizontalTb,
            InBothAbsAxis {
                horizontal: Line { start: OriginZeroLine(0), end: OriginZeroLine(1) },
                vertical: Line { start: OriginZeroLine(0), end: OriginZeroLine(1) },
            },
            style,
            InBothAbsAxis { horizontal: AlignItems::STRETCH, vertical: AlignItems::STRETCH },
            source_order,
        );
        item.row_indexes = row_indexes;
        item.column_indexes = column_indexes;
        item.y_position = y_position;
        item.height = height;
        item.first_baseline = first_baseline;
        item.last_baseline = last_baseline;
        item
    }

    #[test]
    fn grid_propagates_distinct_final_child_baseline_sets() {
        let items = vec![
            baseline_item(
                0,
                Line { start: 0, end: 2 },
                Line { start: 0, end: 2 },
                false,
                10.0,
                30.0,
                Some(8.0),
                Some(24.0),
            ),
            baseline_item(
                1,
                Line { start: 2, end: 4 },
                Line { start: 0, end: 2 },
                false,
                50.0,
                40.0,
                Some(10.0),
                Some(32.0),
            ),
        ];

        assert_eq!(grid_container_baselines(&items), (18.0, 82.0));
    }

    #[test]
    fn grid_baseline_sharing_groups_take_priority_in_edge_rows() {
        let items = vec![
            baseline_item(
                0,
                Line { start: 0, end: 2 },
                Line { start: 0, end: 2 },
                false,
                0.0,
                20.0,
                Some(5.0),
                Some(15.0),
            ),
            baseline_item(
                1,
                Line { start: 0, end: 2 },
                Line { start: 2, end: 4 },
                true,
                0.0,
                24.0,
                Some(12.0),
                Some(18.0),
            ),
            baseline_item(
                2,
                Line { start: 2, end: 4 },
                Line { start: 0, end: 2 },
                false,
                40.0,
                34.0,
                Some(6.0),
                Some(28.0),
            ),
            baseline_item(
                3,
                Line { start: 2, end: 4 },
                Line { start: 2, end: 4 },
                true,
                44.0,
                30.0,
                Some(14.0),
                Some(20.0),
            ),
        ];

        assert_eq!(grid_container_baselines(&items), (12.0, 58.0));
    }
}
