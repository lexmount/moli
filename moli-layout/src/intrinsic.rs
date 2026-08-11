//! Intrinsic sizing adapters for CSS roles that Taffy's numeric style does not retain.
//!
//! Taffy sees an `inline-block` as a `flow-root`; the inline outer display is
//! deliberately owned by Moli's box tree. Consequently the parent IFC
//! must select fit-content sizing before it asks Taffy to perform the child's
//! final inner formatting-context layout.

use std::{fmt::Debug, hash::Hash};

use taffy::{
    AvailableSpace, LayoutInput, LayoutOutput, LayoutPartialTree, RequestedAxis, RunMode, Size,
    SizingPurpose,
};

use crate::{LayoutBoxId, LayoutDisplay, LayoutWorld};

impl<N> LayoutWorld<N>
where
    N: Copy + Debug + Eq + Hash,
{
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
            definite_dimensions: inputs.known_dimensions,
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
        let layout_box = &self.boxes[child.index()];
        let uses_fit_content = !layout_box.is_replaced()
            && layout_box.style.taffy.size.width.is_auto()
            && matches!(
                layout_box.style.display(),
                LayoutDisplay::InlineBlock
                    | LayoutDisplay::InlineFlex
                    | LayoutDisplay::InlineGrid
                    | LayoutDisplay::InlineListItem
                    | LayoutDisplay::InlineTable
            );
        let AvailableSpace::Definite(available_width) = inputs.available_space.width else {
            return self.compute_child_layout(child.to_taffy(), inputs);
        };
        if !uses_fit_content {
            return self.compute_child_layout(child.to_taffy(), inputs);
        }

        let intrinsic_inputs = LayoutInput {
            known_dimensions: Size::NONE,
            definite_dimensions: Size::NONE,
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
            height: inputs.definite_dimensions.height,
        };

        self.compute_child_layout(
            child.to_taffy(),
            LayoutInput {
                known_dimensions,
                definite_dimensions,
                ..inputs
            },
        )
    }
}
