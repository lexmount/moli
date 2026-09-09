//! Intrinsic sizing adapters for CSS roles that Taffy's numeric style does not retain.
//!
//! Taffy sees an `inline-block` as a `flow-root`; the inline outer display is
//! deliberately owned by Moli's box tree. Consequently the parent IFC
//! must select fit-content sizing before it asks Taffy to perform the child's
//! final inner formatting-context layout.

use std::{fmt::Debug, hash::Hash};

use taffy::{
    AvailableSpace, LayoutInput, LayoutOutput, LayoutPartialTree, LogicalSize, Rect, RequestedAxis,
    RunMode, Size, SizingPurpose, WritingMode,
};

use crate::{LayoutBoxId, LayoutDisplay, LayoutWorld};

impl<N> LayoutWorld<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub(crate) fn measure_fit_content_inline_size(
        &mut self,
        child: LayoutBoxId,
        inputs: LayoutInput,
        mode: WritingMode,
        available_inline_size: f32,
    ) -> f32 {
        let intrinsic_input = |inline_size| LayoutInput {
            definite_dimensions: inputs.known_dimensions,
            available_space: mode.to_physical(LogicalSize {
                inline_size,
                block_size: mode.to_logical(inputs.available_space).block_size,
            }),
            run_mode: RunMode::ComputeSize,
            sizing_purpose: SizingPurpose::IntrinsicContribution,
            axis: if mode.is_horizontal() {
                RequestedAxis::Horizontal
            } else {
                RequestedAxis::Vertical
            },
            ..inputs
        };
        let min_content = mode
            .to_logical(
                self.compute_child_layout(
                    child.to_taffy(),
                    intrinsic_input(AvailableSpace::MinContent),
                )
                .size,
            )
            .inline_size;
        let max_content = mode
            .to_logical(
                self.compute_child_layout(
                    child.to_taffy(),
                    intrinsic_input(AvailableSpace::MaxContent),
                )
                .size,
            )
            .inline_size;

        available_inline_size
            .max(0.0)
            .max(min_content)
            .min(max_content)
    }

    /// Lay out one non-replaced atomic inline-level box.
    ///
    /// CSS 2.2 §10.3.9 defines an auto-width inline-block as fit-content;
    /// writing modes apply this rule to the child's inline axis:
    /// `min(max(min-content, available), max-content)`. A single Taffy call
    /// with definite available space cannot express that contract for an
    /// inline-block containing block-level children: the auto-sized child
    /// block legitimately stretches and makes the outer contribution equal to
    /// the whole line. Measure both intrinsic constraints first, then perform
    /// the final child layout with the selected border-box inline size.
    pub(crate) fn compute_atomic_inline_layout(
        &mut self,
        child: LayoutBoxId,
        inputs: LayoutInput,
        margins: Rect<f32>,
    ) -> LayoutOutput {
        // The parent line needs the child's baseline even during intrinsic
        // sizing. ComputeSize may legitimately return only a size for fixed
        // dimensions; that would turn a real inline-block baseline into a
        // synthesized one and change the parent's line height. Request a
        // fragment, as flex/grid do for their baseline measurements. The
        // fit-content probes below remain size-only intrinsic requests.
        let inputs = LayoutInput {
            run_mode: RunMode::PerformLayout,
            ..inputs
        };
        let layout_box = &self.boxes[child.index()];
        let mode = layout_box.style.writing_mode();
        let uses_fit_content = !layout_box.is_replaced()
            && mode
                .to_logical(layout_box.style.taffy.size)
                .inline_size
                .is_auto()
            && matches!(
                layout_box.style.display(),
                LayoutDisplay::InlineBlock
                    | LayoutDisplay::InlineFlex
                    | LayoutDisplay::InlineGrid
                    | LayoutDisplay::InlineListItem
                    | LayoutDisplay::InlineTable
            );
        let AvailableSpace::Definite(available_inline_size) =
            mode.to_logical(inputs.available_space).inline_size
        else {
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
        let fit_content = self.measure_fit_content_inline_size(
            child,
            intrinsic_inputs,
            mode,
            available_inline_size - mode.to_logical(margins.sum_axes()).inline_size,
        );
        let known_dimensions = mode.to_physical(LogicalSize {
            inline_size: Some(fit_content),
            block_size: mode.to_logical(inputs.known_dimensions).block_size,
        });
        let definite_dimensions = mode.to_physical(LogicalSize {
            inline_size: Some(fit_content),
            block_size: mode.to_logical(inputs.definite_dimensions).block_size,
        });

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
