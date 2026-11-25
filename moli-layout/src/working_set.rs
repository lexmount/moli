//! Owned box construction shared by consecutive layout passes.

use std::{collections::HashMap, fmt::Debug, hash::Hash};

use crate::{LayoutBox, LayoutBoxId, LayoutWorld, builder::ConstructionInputs};

#[cfg(test)]
mod owned_construction_tests;

/// Construction residency and reuse in the last successful pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutWorkingSetStats {
    pub retained_boxes: usize,
    pub reused_owned_boxes: usize,
}

/// Optional construction state for an exact document and layout view. The
/// caller retires it when the view or owner changes. Frozen layout and paint
/// results have independent ownership.
pub struct LayoutWorkingSet<N> {
    retained: Option<RetainedConstruction<N>>,
}

pub(crate) struct RetainedConstruction<N> {
    pub(crate) boxes: Vec<LayoutBox<N>>,
    pub(crate) source_mapping: HashMap<N, LayoutBoxId>,
    pub(crate) inputs: Box<ConstructionInputs<N>>,
}

impl<N> Default for LayoutWorkingSet<N> {
    fn default() -> Self {
        Self { retained: None }
    }
}

impl<N> LayoutWorkingSet<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub fn clear(&mut self) {
        self.retained = None;
    }

    pub fn stats(&self) -> LayoutWorkingSetStats {
        self.retained
            .as_ref()
            .map_or_else(LayoutWorkingSetStats::default, |retained| {
                LayoutWorkingSetStats {
                    retained_boxes: retained.boxes.len(),
                    reused_owned_boxes: retained.inputs.reused_boxes(),
                }
            })
    }

    pub(crate) fn take(&mut self) -> Option<RetainedConstruction<N>> {
        self.retained.take()
    }

    pub(crate) fn publish(&mut self, mut world: LayoutWorld<N>, construction_box_count: usize) {
        self.clear();
        let Some(inputs) = world.construction_inputs.take() else {
            return;
        };
        let started = moli_trace::cpu_profile_enabled().then(std::time::Instant::now);
        // Numeric-only placeholders have no source mapping.
        assert!(construction_box_count <= world.boxes.len());
        world.boxes.truncate(construction_box_count);
        for layout_box in &mut world.boxes {
            // Only construction survives. Releasing each container also
            // releases its backing allocation and any referenced resources.
            layout_box.inline_layout = None;
            layout_box.layout_parent = None;
            layout_box.layout_children = Vec::new();
            layout_box.positioned_containing_block = None;
            layout_box.inline_static_position = None;
            layout_box.inline_context_owner = None;
            layout_box.inline_flattened = false;
            layout_box.scroll_offset = crate::LayoutPoint::ZERO;
            layout_box.replaced_context = None;
            layout_box.replaced_image = None;
            layout_box.css_images = Default::default();
            layout_box.collapsed_table_border_part = false;
            layout_box.clear_numeric_cache_slot();
            layout_box.table_measure_cache = None;
            layout_box.unrounded_layout = taffy::Layout::with_order(0);
            layout_box.final_layout = taffy::Layout::with_order(0);
            layout_box.capability_diagnostics = Vec::new();
            layout_box.style.prepare_for_construction_retention();
        }
        self.retained = Some(RetainedConstruction {
            boxes: std::mem::take(&mut world.boxes),
            source_mapping: std::mem::take(&mut world.source_mapping),
            inputs,
        });
        if let Some(started) = started {
            tracing::info!(
                target: "moli_cpu_profile",
                stage = "layout_working_set",
                retention_us = started.elapsed().as_micros(),
                stats = ?self.stats(),
            );
        }
    }
}
