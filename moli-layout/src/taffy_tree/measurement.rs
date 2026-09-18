//! Retain the complete numeric result of Taffy's algorithms during measurement.
//!
//! Their size-only shortcuts may omit baselines and content bounds. Run the
//! numeric algorithm through its final phase, but discard layout writes and keep
//! descendant callbacks in measurement mode. In particular, Parley and nested
//! tables must not publish fragments or structural geometry during this probe.
use super::*;

pub(super) struct FullMeasurementTree<'a, N: Copy + Debug + Eq + Hash>(pub &'a mut LayoutWorld<N>);

impl<N: Copy + Debug + Eq + Hash> TraversePartialTree for FullMeasurementTree<'_, N> {
    type ChildIter<'a>
        = ChildIter<'a>
    where
        Self: 'a;

    fn child_ids(&self, node_id: NodeId) -> Self::ChildIter<'_> {
        self.0.child_ids(node_id)
    }

    fn child_count(&self, node_id: NodeId) -> usize {
        self.0.child_count(node_id)
    }

    fn get_child_id(&self, node_id: NodeId, index: usize) -> NodeId {
        self.0.get_child_id(node_id, index)
    }
}

impl<N: Copy + Debug + Eq + Hash> LayoutPartialTree for FullMeasurementTree<'_, N> {
    type CoreContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type CustomIdent = Atom;

    fn get_core_container_style(&self, node_id: NodeId) -> Self::CoreContainerStyle<'_> {
        self.0.get_core_container_style(node_id)
    }

    fn get_writing_mode(&self, node_id: NodeId) -> taffy::WritingMode {
        self.0.get_writing_mode(node_id)
    }

    fn get_scrollbar_insets(&self, node_id: NodeId) -> taffy::Rect<f32> {
        self.0.get_scrollbar_insets(node_id)
    }

    fn get_resolved_aspect_ratio(&self, node_id: NodeId) -> Option<taffy::ResolvedAspectRatio> {
        self.0.get_resolved_aspect_ratio(node_id)
    }

    fn resolve_calc_value(&self, value: *const (), basis: f32) -> f32 {
        self.0.resolve_calc_value(value, basis)
    }

    fn set_unrounded_layout(&mut self, _: NodeId, _: &Layout) {}

    fn compute_child_layout(&mut self, node_id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        if self.0.should_hide(node_id, inputs) {
            return LayoutOutput::HIDDEN;
        }
        self.0.compute_child_layout(
            node_id,
            LayoutInput {
                run_mode: RunMode::ComputeSize,
                ..inputs
            },
        )
    }

    fn compute_child_size(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
    ) -> taffy::IntrinsicSizeResult {
        // Intrinsic probes consume only size and provenance, so they can keep
        // the compact cache and the algorithms' ordinary size-only shortcuts.
        let previous = std::mem::replace(&mut self.0.measure_baselines, false);
        let output = self.0.compute_child_size(node_id, inputs);
        self.0.measure_baselines = previous;
        output
    }
}

impl<N: Copy + Debug + Eq + Hash> LayoutBlockContainer for FullMeasurementTree<'_, N> {
    type BlockContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;

    fn get_block_container_style(&self, node_id: NodeId) -> Self::BlockContainerStyle<'_> {
        self.0.get_block_container_style(node_id)
    }

    fn get_block_child_style(&self, node_id: NodeId) -> Self::BlockItemStyle<'_> {
        self.0.get_block_child_style(node_id)
    }

    fn get_block_percentage_resolution_height(
        &self,
        node_id: NodeId,
        height: Option<f32>,
    ) -> Option<f32> {
        self.0
            .get_block_percentage_resolution_height(node_id, height)
    }

    fn block_alignment_includes_floats(&self, node_id: NodeId) -> bool {
        self.0.block_alignment_includes_floats(node_id)
    }

    fn compute_block_child_layout(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        block_context: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        if self.0.should_hide(node_id, inputs) {
            return LayoutOutput::HIDDEN;
        }
        self.0.compute_block_child_layout(
            node_id,
            LayoutInput {
                run_mode: RunMode::ComputeSize,
                ..inputs
            },
            block_context,
        )
    }
}

impl<N: Copy + Debug + Eq + Hash> LayoutFlexboxContainer for FullMeasurementTree<'_, N> {
    type FlexboxContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, node_id: NodeId) -> Self::FlexboxContainerStyle<'_> {
        self.0.get_flexbox_container_style(node_id)
    }

    fn get_flexbox_child_style(&self, node_id: NodeId) -> Self::FlexboxItemStyle<'_> {
        self.0.get_flexbox_child_style(node_id)
    }
}

impl<N: Copy + Debug + Eq + Hash> LayoutGridContainer for FullMeasurementTree<'_, N> {
    type GridContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;

    fn get_grid_container_style(&self, node_id: NodeId) -> Self::GridContainerStyle<'_> {
        self.0.get_grid_container_style(node_id)
    }

    fn get_grid_child_style(&self, node_id: NodeId) -> Self::GridItemStyle<'_> {
        self.0.get_grid_child_style(node_id)
    }

    fn set_detailed_grid_info(&mut self, _: NodeId, _: DetailedGridInfo) {}
}
