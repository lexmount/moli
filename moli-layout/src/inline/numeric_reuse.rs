//! Final IFC results for an optional, document-owned working set.
//!
//! The accepted Parley layout and local fragments stay on their existing IFC
//! owner. This sidecar only validates that result's numeric inputs. It is
//! restored together with an unchanged IFC after construction/font validation;
//! ordinary one-shot worlds never allocate its per-owner index or entries.

use style::Atom;
use taffy::{
    AvailableSpace, CompactLength, LayoutInput, LayoutOutput, Rect, ResolvedAspectRatio, RunMode,
    Style, WritingMode,
};

use crate::{LayoutBoxId, LayoutDisplay, style::InlineDirection};

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

        if self.layout.run_mode != RunMode::PerformLayout
            || self.writing_mode != WritingMode::HorizontalTb
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
        // the final box. Moli's pure-text measurement uses the content width;
        // known_width also disables its shrink-to-fit branch. This allows an
        // unchanged fixed-width IFC to survive the auto-scrollbar feedback
        // pass without reproducing any of the shared sizing calculations.
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
    owner_slots: Vec<Option<usize>>,
    retained_hits: usize,
}

impl InlineNumericReuse {
    pub(crate) fn new(box_count: usize) -> Self {
        Self {
            entries: Vec::new(),
            owner_slots: vec![None; box_count],
            retained_hits: 0,
        }
    }

    pub(crate) fn contains(&self, owner: LayoutBoxId) -> bool {
        self.owner_slots
            .get(owner.index())
            .is_some_and(Option::is_some)
    }

    pub(crate) fn retain_owners(&mut self, mut keep: impl FnMut(LayoutBoxId) -> bool) {
        self.entries.retain(|entry| keep(entry.owner));
        self.owner_slots.fill(None);
        for (slot, entry) in self.entries.iter().enumerate() {
            self.owner_slots[entry.owner.index()] = Some(slot);
        }
    }

    /// Begin a new pass after the caller has validated the surviving owners.
    /// Newly computed entries in this pass remain distinct from restored ones.
    pub(crate) fn reset_hits(&mut self) {
        self.retained_hits = 0;
        for entry in &mut self.entries {
            entry.restored = true;
        }
    }

    /// Number of final computations skipped using validated prior-pass data.
    /// This counts calls, not distinct owners; local newly computed hits do
    /// not contribute to evidence of cross-pass reuse.
    pub(crate) fn hits(&self) -> usize {
        self.retained_hits
    }

    pub(crate) fn lookup(
        &mut self,
        owner: LayoutBoxId,
        inputs: InlineNumericInputs,
        style: &Style<Atom>,
    ) -> Option<LayoutOutput> {
        let slot = self.owner_slots.get(owner.index()).copied().flatten()?;
        let entry = &self.entries[slot];
        if entry.inputs != inputs.comparison_key(style) || entry.style != *style {
            return None;
        }
        if entry.restored {
            self.retained_hits = self.retained_hits.saturating_add(1);
        }
        Some(entry.output)
    }

    pub(crate) fn remove(&mut self, owner: LayoutBoxId) {
        let Some(slot) = self
            .owner_slots
            .get_mut(owner.index())
            .and_then(Option::take)
        else {
            return;
        };
        self.entries.swap_remove(slot);
        if let Some(moved) = self.entries.get(slot) {
            self.owner_slots[moved.owner.index()] = Some(slot);
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
        assert!(owner.index() < self.owner_slots.len());
        self.owner_slots[owner.index()] = Some(self.entries.len());
        let inputs = inputs.comparison_key(&style);
        self.entries.push(InlineNumericEntry {
            owner,
            inputs,
            style,
            output,
            restored: false,
        });
    }

    /// Owned sidecar buffers; shared Atom and computed-style payloads follow
    /// the working set's existing exclusion from estimated admission charge.
    pub(crate) fn estimated_retained_bytes(&self) -> usize {
        self.entries.iter().fold(
            std::mem::size_of::<Self>()
                .saturating_add(allocation(&self.entries))
                .saturating_add(allocation(&self.owner_slots)),
            |bytes, entry| bytes.saturating_add(style_allocation(&entry.style)),
        )
    }
}

fn allocation<T>(values: &Vec<T>) -> usize {
    values.capacity().saturating_mul(std::mem::size_of::<T>())
}

fn style_allocation(style: &Style<Atom>) -> usize {
    fn line_names(names: &Vec<Vec<Atom>>) -> usize {
        names.iter().fold(allocation(names), |bytes, line| {
            bytes.saturating_add(allocation(line))
        })
    }
    fn template(tracks: &Vec<taffy::GridTemplateComponent<Atom>>) -> usize {
        tracks.iter().fold(allocation(tracks), |bytes, track| {
            let extra = match track {
                taffy::GridTemplateComponent::Single(_) => 0,
                taffy::GridTemplateComponent::Repeat(repetition) => allocation(&repetition.tracks)
                    .saturating_add(line_names(&repetition.line_names)),
            };
            bytes.saturating_add(extra)
        })
    }
    template(&style.grid_template_rows)
        .saturating_add(template(&style.grid_template_columns))
        .saturating_add(allocation(&style.grid_auto_rows))
        .saturating_add(allocation(&style.grid_auto_columns))
        .saturating_add(line_names(&style.grid_template_row_names))
        .saturating_add(line_names(&style.grid_template_column_names))
        .saturating_add(
            style
                .grid_template_areas
                .as_ref()
                .map_or(0, |areas| allocation(&areas.areas)),
        )
}
