//! Final and size-only IFC results for an optional, document-owned working set.
//!
//! The accepted Parley layout and local fragments stay on their existing IFC
//! owner. This sidecar validates final numeric inputs and separately retains
//! bounded complete measurements under their exact constraints. It is
//! restored together with an unchanged IFC after construction/font validation;
//! ordinary one-shot worlds never allocate its per-owner index or entries.

use std::num::NonZeroU32;

use slotmap::SecondaryMap;
use style::Atom;
use taffy::{
    AvailableSpace, CompactLength, LayoutInput, LayoutOutput, Rect, ResolvedAspectRatio, RunMode,
    Style, WritingMode,
};

use crate::{LayoutBoxId, LayoutDisplay, style::InlineDirection};

fn cache_slot(index: usize) -> NonZeroU32 {
    NonZeroU32::new(
        u32::try_from(index.checked_add(1).expect("IFC cache index overflow"))
            .expect("IFC cache exceeds u32"),
    )
    .unwrap()
}

mod measurement;
pub(crate) use measurement::InlineMeasurementInputs;
use measurement::InlineMeasurements;

/// Inputs outside the shaped paragraph and its exact computed-style owner.
/// Preserve Taffy's input dependencies, including percentage bases, purpose
/// and requested axis. Only proven-unused raw width constraints are normalized
/// in the comparison copy. The parent classification changes fit-content
/// behavior even when the child's own physical constraints have not changed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InlineNumericInputs {
    pub(crate) layout: LayoutInput,
    pub(crate) writing_mode: WritingMode,
    pub(crate) resolved_aspect_ratio: Option<ResolvedAspectRatio>,
    pub(crate) scrollbar_insets: Rect<f32>,
    pub(crate) display: LayoutDisplay,
    pub(crate) alignment: parley::Alignment,
    pub(crate) direction: InlineDirection,
    pub(crate) parent_is_flex_or_grid: bool,
}

impl InlineNumericInputs {
    fn comparison_key(mut self, style: &Style<Atom>) -> Self {
        fn finite_nonnegative(value: f32) -> bool {
            value.is_finite() && value >= 0.0
        }
        fn absolute(value: CompactLength) -> bool {
            value.tag() == CompactLength::LENGTH_TAG && value.value().is_finite()
        }
        fn absolute_or_auto(value: CompactLength) -> bool {
            value.tag() == CompactLength::AUTO_TAG || absolute(value)
        }

        if !matches!(
            self.layout.run_mode,
            RunMode::PerformLayout | RunMode::ComputeSize
        ) || self.writing_mode != WritingMode::HorizontalTb
            || self.layout.parent_writing_mode != WritingMode::HorizontalTb
            || !self
                .layout
                .known_dimensions
                .width
                .is_some_and(finite_nonnegative)
            || !self
                .layout
                .parent_size
                .width
                .is_some_and(finite_nonnegative)
            || !matches!(self.layout.available_space.width, AvailableSpace::Definite(width) if finite_nonnegative(width))
            || ![
                style.size.width.into_raw(),
                style.size.height.into_raw(),
                style.min_size.width.into_raw(),
                style.min_size.height.into_raw(),
                style.max_size.width.into_raw(),
                style.max_size.height.into_raw(),
                style.margin.left.into_raw(),
                style.margin.right.into_raw(),
                style.margin.top.into_raw(),
                style.margin.bottom.into_raw(),
            ]
            .into_iter()
            .all(absolute_or_auto)
            || ![
                style.padding.left.into_raw(),
                style.padding.right.into_raw(),
                style.padding.top.into_raw(),
                style.padding.bottom.into_raw(),
                style.border.left.into_raw(),
                style.border.right.into_raw(),
                style.border.top.into_raw(),
                style.border.bottom.into_raw(),
            ]
            .into_iter()
            .all(absolute)
        {
            return self;
        }

        // Taffy 645bf5a, compute/leaf.rs::resolve_available_axis, applies
        // resolve_used_axis(known_width, ...) before maybe_set overrides the
        // raw available width. With only absolute/auto size and edge values,
        // the raw parent width cannot change that resolved content width or
        // complete measurement or final box. Moli's pure-text measurement uses
        // the content width; known_width also disables its shrink-to-fit branch.
        // This remains true for ComputeSize (including full baseline probes):
        // only the raw unused widths are normalized, not purpose or run mode.
        // Unchanged fixed-width IFCs can therefore survive parent column-width
        // changes and auto-scrollbar feedback without repeating text layout.
        //
        // Normalize only this comparison copy; layout receives the original
        // inputs. Keep definite sentinels, so an unknown input that fails the
        // checks above cannot collide with a normalized key. All other input
        // fields and the full actual numeric style remain part of the key.
        self.layout.parent_size.width = Some(0.0);
        self.layout.available_space.width = AvailableSpace::Definite(0.0);
        self
    }
}

#[derive(Debug)]
struct InlineNumericEntry {
    owner: LayoutBoxId,
    inputs: InlineNumericInputs,
    // Compare the actual numeric projection at the call boundary, including
    // temporary adapter overrides. Retained construction inputs keep the
    // computed owner alive for any calc pointers held by this projection.
    style: Style<Atom>,
    output: LayoutOutput,
    restored: bool,
}

#[derive(Debug)]
pub(crate) struct InlineNumericReuse {
    // Keep entries dense: most boxes are not eligible IFC owners. The small
    // index exists only when the caller opted into a retained working set.
    entries: Vec<InlineNumericEntry>,
    owner_slots: SecondaryMap<LayoutBoxId, NonZeroU32>,
    retained_hits: usize,
    measurements: Option<Box<InlineMeasurements>>,
}

impl InlineNumericReuse {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
            owner_slots: SecondaryMap::new(),
            retained_hits: 0,
            measurements: None,
        }
    }

    pub(crate) fn contains(&self, owner: LayoutBoxId) -> bool {
        self.owner_slots.contains_key(owner)
            || self
                .measurements
                .as_ref()
                .is_some_and(|cache| cache.contains(owner))
    }

    pub(crate) fn discard_owner(&mut self, owner: LayoutBoxId) {
        self.remove(owner);
        if let Some(cache) = &mut self.measurements {
            cache.discard_owner(owner);
        }
    }

    pub(crate) fn begin_pass(&mut self) {
        self.reset_hits();
    }

    /// Retire owners rejected by construction/input validation. Surviving
    /// entries keep their generational handles across order changes.
    pub(crate) fn retain_owners(&mut self, mut keep: impl FnMut(LayoutBoxId) -> bool) {
        self.entries.retain(|entry| keep(entry.owner));
        self.owner_slots.clear();

        for (slot, entry) in self.entries.iter().enumerate() {
            self.owner_slots.insert(entry.owner, cache_slot(slot));
        }
        if let Some(cache) = &mut self.measurements {
            cache.retain_owners(keep);
        }
    }

    /// Begin a new pass after the caller has validated the surviving owners.
    /// Newly computed entries in this pass remain distinct from restored ones.
    pub(crate) fn reset_hits(&mut self) {
        self.retained_hits = 0;
        for entry in &mut self.entries {
            entry.restored = true;
        }
        if let Some(cache) = &mut self.measurements {
            cache.reset_hits();
        }
    }

    /// Number of final computations skipped using validated prior-pass data.
    /// This counts calls, not distinct owners; local newly computed hits do
    /// not contribute to evidence of cross-pass reuse.
    pub(crate) fn hits(&self) -> usize {
        self.retained_hits
    }

    pub(crate) fn measurement_hits(&self) -> usize {
        self.measurements.as_ref().map_or(0, |cache| cache.hits())
    }

    #[cfg(test)]
    pub(crate) fn has_measurement_storage_for_test(&self) -> bool {
        self.measurements.is_some()
    }

    pub(crate) fn lookup_measurement(
        &mut self,
        owner: LayoutBoxId,
        inputs: InlineMeasurementInputs,
        style: &Style<Atom>,
    ) -> Option<LayoutOutput> {
        self.measurements.as_mut()?.lookup(owner, inputs, style)
    }

    pub(crate) fn store_measurement(
        &mut self,
        owner: LayoutBoxId,
        inputs: InlineMeasurementInputs,
        style: Style<Atom>,
        output: LayoutOutput,
    ) {
        self.measurements
            .get_or_insert_with(|| Box::new(InlineMeasurements::new()))
            .store(owner, inputs, style, output);
    }

    pub(crate) fn lookup(
        &mut self,
        owner: LayoutBoxId,
        inputs: InlineNumericInputs,
        style: &Style<Atom>,
    ) -> Option<LayoutOutput> {
        let slot = (self.owner_slots.get(owner)?.get() - 1) as usize;
        let entry = &self.entries[slot];
        if entry.inputs != inputs.comparison_key(style) || entry.style != *style {
            return None;
        }
        if entry.restored {
            self.retained_hits = self.retained_hits.saturating_add(1);
        }
        Some(entry.output.clone())
    }

    pub(crate) fn remove(&mut self, owner: LayoutBoxId) {
        let Some(slot) = self.owner_slots.remove(owner) else {
            return;
        };
        let slot = (slot.get() - 1) as usize;
        self.entries.swap_remove(slot);
        if let Some(moved) = self.entries.get(slot) {
            self.owner_slots.insert(moved.owner, cache_slot(slot));
        }
    }

    pub(crate) fn store(
        &mut self,
        owner: LayoutBoxId,
        inputs: InlineNumericInputs,
        style: Style<Atom>,
        output: LayoutOutput,
    ) {
        self.remove(owner);
        // Numeric placeholders have no IFC input owner and must never enter
        // the persistent construction graph's index.
        self.owner_slots
            .insert(owner, cache_slot(self.entries.len()));
        let inputs = inputs.comparison_key(&style);
        self.entries.push(InlineNumericEntry {
            owner,
            inputs,
            style,
            output,
            restored: false,
        });
    }
}
