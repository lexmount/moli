//! Intrinsic sizing adapters for CSS roles that Taffy's numeric style does not retain.
//!
//! Taffy sees an `inline-block` as a `flow-root`; the inline outer display is
//! deliberately owned by Moli's box tree. Consequently the parent IFC
//! must select fit-content sizing before it asks Taffy to perform the child's
//! final inner formatting-context layout.

use std::{fmt::Debug, hash::Hash};

use style::{Atom, values::computed::ContainIntrinsicSize};
use taffy::{
    AvailableSpace, Layout, LayoutGridContainer, LayoutInput, LayoutOutput, LayoutPartialTree,
    LeafLayoutContext, MaybeResolve, NodeId, RequestedAxis, ResolvedAspectRatio, RunMode, Size,
    SizingMode, SizingPurpose, Style, TraversePartialTree, WritingMode, compute_grid_layout,
    compute_leaf_layout_with_context,
};

use crate::{LayoutBoxId, LayoutDisplay, LayoutWorld, style::resolve_stylo_calc_value};

impl<N> LayoutWorld<N>
where
    N: Copy + Debug + Eq + Hash,
{
    /// Measure a container without its children. Explicit intrinsic sizes
    /// replace the content contribution, while an empty grid still has tracks.
    pub(crate) fn measure_size_contained_box(
        &self,
        child: LayoutBoxId,
        inputs: LayoutInput,
        axes: Size<bool>,
    ) -> LayoutOutput {
        debug_assert_eq!(inputs.run_mode, RunMode::ComputeSize);
        let layout_box = &self.boxes[child];
        let style = &layout_box.style;
        let computed = style
            .stylo_computed_values()
            .expect("size query containers have computed style");
        let fallback = Size {
            width: computed.clone_contain_intrinsic_width(),
            height: computed.clone_contain_intrinsic_height(),
        }
        .zip_map(axes, |size, contained| match size {
            ContainIntrinsicSize::Length(length) | ContainIntrinsicSize::AutoLength(length)
                if contained =>
            {
                Some(length.0.px())
            }
            _ => None,
        });
        let context = LeafLayoutContext::new(
            style.writing_mode(),
            layout_box.resolved_aspect_ratio(),
            self.get_scrollbar_insets(child.to_taffy()),
        );
        let measure_leaf = || {
            compute_leaf_layout_with_context(
                inputs,
                &style.taffy,
                context,
                resolve_stylo_calc_value,
                |_, _| fallback.map(|size| size.unwrap_or(0.0)),
            )
        };
        if !style.display().is_grid_container() {
            return measure_leaf();
        }

        // Resolve contain-intrinsic-size to a border-box size on each
        // overridden axis before asking grid to measure its own tracks.
        let mut grid_inputs = inputs;
        if fallback.width.is_some() || fallback.height.is_some() {
            grid_inputs.known_dimensions = inputs
                .known_dimensions
                .or(fallback.zip_map(measure_leaf().size, |fallback, size| fallback.map(|_| size)));
        }
        compute_grid_layout(&mut EmptyGrid(self), child.to_taffy(), grid_inputs)
    }

    /// Fix the container's content-dependent axes before laying out its real
    /// children. The parent's constraints and definite authored sizes retain
    /// their normal sizing and percentage-resolution behavior.
    pub(crate) fn resolve_size_contained_layout(
        &self,
        id: LayoutBoxId,
        inputs: LayoutInput,
        axes: Size<bool>,
    ) -> LayoutInput {
        let layout_box = &self.boxes[id];
        let specified = if inputs.sizing_mode == SizingMode::InherentSize {
            layout_box
                .style
                .taffy
                .size
                .maybe_resolve(inputs.parent_size, resolve_stylo_calc_value)
        } else {
            Size::NONE
        };
        let unresolved = axes.zip_map(inputs.known_dimensions.or(specified), |contained, size| {
            contained && size.is_none()
        });
        if !unresolved.width && !unresolved.height {
            return inputs;
        }
        let measured = self.measure_size_contained_box(
            id,
            LayoutInput {
                run_mode: RunMode::ComputeSize,
                axis: if unresolved.height {
                    RequestedAxis::Both
                } else {
                    RequestedAxis::Horizontal
                },
                ..inputs
            },
            axes,
        );
        let definite = inputs
            .known_dimensions
            .zip_map(inputs.known_dimensions_are_definite, |size, definite| {
                size.filter(|_| definite)
            })
            .or(specified);
        let has_ratio = inputs.sizing_mode == SizingMode::InherentSize
            && layout_box.resolved_aspect_ratio().is_some();
        LayoutInput {
            known_dimensions: inputs
                .known_dimensions
                .or(unresolved
                    .zip_map(measured.size, |unresolved, size| unresolved.then_some(size))),
            // A size derived from empty contents is still an intrinsic size.
            // A definite opposite axis can transfer through aspect-ratio.
            known_dimensions_are_definite: Size {
                width: if unresolved.width {
                    has_ratio && definite.height.is_some()
                } else {
                    inputs.known_dimensions_are_definite.width
                },
                height: if unresolved.height {
                    has_ratio && definite.width.is_some()
                } else {
                    inputs.known_dimensions_are_definite.height
                },
            },
            ..inputs
        }
    }

    /// Measure the border-box width selected by CSS fit-content sizing.
    ///
    /// Taffy's block and flex absolute-layout paths use the same two intrinsic
    /// measurements internally. Moli also needs the operation at the
    /// IFC seam, where an out-of-flow placeholder is owned by Parley and the
    /// positioned box cannot remain a normal child of Taffy's numeric tree.
    pub(crate) fn measure_fit_content_width(
        &mut self,
        child: LayoutBoxId,
        inputs: LayoutInput,
        available_width: f32,
    ) -> f32 {
        let intrinsic_input = |width| LayoutInput {
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            available_space: Size {
                width,
                height: inputs.available_space.height,
            },
            run_mode: RunMode::ComputeSize,
            sizing_purpose: SizingPurpose::IntrinsicContribution,
            axis: RequestedAxis::Horizontal,
            ..inputs
        };
        let min_content = self
            .compute_child_layout(
                child.to_taffy(),
                intrinsic_input(AvailableSpace::MinContent),
            )
            .size
            .width;
        let max_content = self
            .compute_child_layout(
                child.to_taffy(),
                intrinsic_input(AvailableSpace::MaxContent),
            )
            .size
            .width;

        available_width.max(0.0).max(min_content).min(max_content)
    }

    /// Lay out one non-replaced atomic inline-level box.
    ///
    /// CSS 2.2 §10.3.9 defines an auto-width inline-block as fit-content:
    /// `min(max(min-content, available), max-content)`. A single Taffy call
    /// with definite available space cannot express that contract for an
    /// inline-block containing block-level children: the auto-width child
    /// block legitimately stretches and makes the outer contribution equal to
    /// the whole line. Measure both intrinsic constraints first, then perform
    /// the final child layout with the selected border-box width.
    pub(crate) fn compute_atomic_inline_layout(
        &mut self,
        child: LayoutBoxId,
        inputs: LayoutInput,
        horizontal_margin: f32,
    ) -> LayoutOutput {
        let layout_box = &self.boxes[child];
        let preferred_width = layout_box.style.taffy.size.width;
        let is_replaced = layout_box.is_replaced();
        let uses_fit_content = !is_replaced
            && (preferred_width.is_auto() || preferred_width.is_fit_content())
            && matches!(
                layout_box.style.display(),
                LayoutDisplay::InlineBlock
                    | LayoutDisplay::InlineFlex
                    | LayoutDisplay::InlineGrid
                    | LayoutDisplay::InlineListItem
                    | LayoutDisplay::InlineTable
            );
        if !uses_fit_content {
            // The IFC owns an atomic inline's preferred intrinsic width just
            // as Taffy's block parent owns its children's sizing keywords.
            // A definite line width must not turn min-content into fit-content.
            let inputs = if !is_replaced
                && inputs.sizing_mode == taffy::SizingMode::InherentSize
                && inputs.known_dimensions.width.is_none()
            {
                let width = if preferred_width.is_min_content() {
                    AvailableSpace::MinContent
                } else if preferred_width.is_max_content() {
                    AvailableSpace::MaxContent
                } else {
                    inputs.available_space.width
                };
                LayoutInput {
                    available_space: Size {
                        width,
                        ..inputs.available_space
                    },
                    ..inputs
                }
            } else {
                inputs
            };
            return self.compute_child_layout(child.to_taffy(), inputs);
        }
        let AvailableSpace::Definite(available_width) = inputs.available_space.width else {
            return self.compute_child_layout(child.to_taffy(), inputs);
        };

        let intrinsic_inputs = LayoutInput {
            known_dimensions: Size::NONE,
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            ..inputs
        };
        let fit_content = self.measure_fit_content_width(
            child,
            intrinsic_inputs,
            available_width - horizontal_margin,
        );
        let known_dimensions = Size {
            width: Some(fit_content),
            height: inputs.known_dimensions.height,
        };
        let definite_dimensions = Size {
            width: Some(fit_content),
            height: inputs
                .known_dimensions
                .height
                .filter(|_| inputs.known_dimensions_are_definite.height),
        };

        self.compute_child_layout(
            child.to_taffy(),
            LayoutInput {
                known_dimensions,
                known_dimensions_are_definite: (known_dimensions)
                    .zip_map(definite_dimensions, |known, definite| {
                        known.is_none() || definite.is_some()
                    }),
                ..inputs
            },
        )
    }
}

/// A read-only view for the size-only grid algorithm. Its style retains the
/// explicit template and gutters, and its empty child list contributes no tracks.
struct EmptyGrid<'a, N: Copy + Debug + Eq + Hash>(&'a LayoutWorld<N>);

impl<N: Copy + Debug + Eq + Hash> TraversePartialTree for EmptyGrid<'_, N> {
    type ChildIter<'a>
        = std::iter::Empty<NodeId>
    where
        Self: 'a;

    fn child_ids(&self, _: NodeId) -> Self::ChildIter<'_> {
        std::iter::empty()
    }

    fn child_count(&self, _: NodeId) -> usize {
        0
    }

    fn get_child_id(&self, _: NodeId, _: usize) -> NodeId {
        unreachable!("an empty grid has no items")
    }
}

impl<N: Copy + Debug + Eq + Hash> LayoutPartialTree for EmptyGrid<'_, N> {
    type CoreContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type CustomIdent = Atom;

    fn get_core_container_style(&self, node: NodeId) -> Self::CoreContainerStyle<'_> {
        self.0.get_core_container_style(node)
    }

    fn get_writing_mode(&self, node: NodeId) -> WritingMode {
        self.0.get_writing_mode(node)
    }

    fn get_resolved_aspect_ratio(&self, node: NodeId) -> Option<ResolvedAspectRatio> {
        self.0.get_resolved_aspect_ratio(node)
    }

    fn get_scrollbar_insets(&self, node: NodeId) -> taffy::Rect<f32> {
        self.0.get_scrollbar_insets(node)
    }

    fn resolve_calc_value(&self, value: *const (), basis: f32) -> f32 {
        resolve_stylo_calc_value(value, basis)
    }

    fn set_unrounded_layout(&mut self, _: NodeId, _: &Layout) {
        unreachable!("size-only measurement does not publish geometry")
    }

    fn compute_child_layout(&mut self, _: NodeId, _: LayoutInput) -> LayoutOutput {
        unreachable!("an empty grid has no items")
    }
}

impl<N: Copy + Debug + Eq + Hash> LayoutGridContainer for EmptyGrid<'_, N> {
    type GridContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;

    fn get_grid_container_style(&self, node: NodeId) -> Self::GridContainerStyle<'_> {
        self.0.get_grid_container_style(node)
    }

    fn get_grid_child_style(&self, _: NodeId) -> Self::GridItemStyle<'_> {
        unreachable!("an empty grid has no items")
    }
}
