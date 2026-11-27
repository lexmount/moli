//! Bounded size-only results; accepted line layouts remain on their IFC owner.

use std::num::NonZeroU32;

use slotmap::SecondaryMap;
use style::Atom;
use taffy::{LayoutOutput, RunMode, Style};

use super::{InlineNumericInputs, cache_slot};
use crate::LayoutBoxId;

// Each owner can be probed under several intrinsic/definite constraints. Keep
// a small LRU rather than replacing its accepted final-layout result.
// Match Blink's per-object complete-measure LRU; this is not a document quota.
const CAPACITY: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InlineMeasurementInputs {
    pub(crate) numeric: InlineNumericInputs,
    // Full table measurements request baselines even when both dimensions
    // are known; ordinary size probes may use Taffy's size-only shortcut.
    pub(crate) measure_baselines: bool,
}

impl InlineMeasurementInputs {
    fn comparison_key(mut self, style: &Style<Atom>) -> Self {
        self.numeric = self.numeric.comparison_key(style);
        self
    }
}

#[derive(Debug)]
struct ResultEntry {
    inputs: InlineMeasurementInputs,
    style: Style<Atom>,
    output: LayoutOutput,
    restored: bool,
}

#[derive(Debug)]
struct OwnerEntry {
    owner: LayoutBoxId,
    results: Vec<ResultEntry>,
}

#[derive(Debug)]
pub(super) struct InlineMeasurements {
    owners: Vec<OwnerEntry>,
    slots: SecondaryMap<LayoutBoxId, NonZeroU32>,
    retained_hits: usize,
}

impl InlineMeasurements {
    pub(super) fn new() -> Self {
        Self {
            owners: Vec::new(),
            slots: SecondaryMap::new(),
            retained_hits: 0,
        }
    }

    pub(super) fn contains(&self, owner: LayoutBoxId) -> bool {
        self.slots.contains_key(owner)
    }

    pub(super) fn retain_owners(&mut self, mut keep: impl FnMut(LayoutBoxId) -> bool) {
        self.owners.retain(|entry| keep(entry.owner));
        self.slots.clear();

        for (slot, entry) in self.owners.iter().enumerate() {
            self.slots.insert(entry.owner, cache_slot(slot));
        }
    }

    pub(super) fn discard_owner(&mut self, owner: LayoutBoxId) {
        let Some(slot) = self.slots.remove(owner) else {
            return;
        };
        let slot = (slot.get() - 1) as usize;
        self.owners.swap_remove(slot);
        if let Some(moved) = self.owners.get(slot) {
            self.slots.insert(moved.owner, cache_slot(slot));
        }
    }

    pub(super) fn reset_hits(&mut self) {
        self.retained_hits = 0;
        for owner in &mut self.owners {
            for result in &mut owner.results {
                result.restored = true;
            }
        }
    }

    pub(super) fn hits(&self) -> usize {
        self.retained_hits
    }

    pub(super) fn lookup(
        &mut self,
        owner: LayoutBoxId,
        inputs: InlineMeasurementInputs,
        style: &Style<Atom>,
    ) -> Option<LayoutOutput> {
        let slot = (self.slots.get(owner)?.get() - 1) as usize;
        let inputs = inputs.comparison_key(style);
        let results = &mut self.owners[slot].results;
        let index = results
            .iter()
            .position(|result| result.inputs == inputs && result.style == *style)?;
        let result = results.remove(index);
        let output = result.output.clone();
        if result.restored {
            self.retained_hits = self.retained_hits.saturating_add(1);
        }
        results.push(result);
        Some(output)
    }

    pub(super) fn store(
        &mut self,
        owner: LayoutBoxId,
        inputs: InlineMeasurementInputs,
        style: Style<Atom>,
        output: LayoutOutput,
    ) {
        assert_eq!(inputs.numeric.layout.run_mode, RunMode::ComputeSize);
        let inputs = inputs.comparison_key(&style);
        let slot = if let Some(slot) = self.slots.get(owner) {
            (slot.get() - 1) as usize
        } else {
            let slot = self.owners.len();
            self.owners.push(OwnerEntry {
                owner,
                results: Vec::with_capacity(1),
            });
            self.slots.insert(owner, cache_slot(slot));
            slot
        };
        let results = &mut self.owners[slot].results;
        if let Some(index) = results
            .iter()
            .position(|result| result.inputs == inputs && result.style == style)
        {
            results.remove(index);
        } else if results.len() == CAPACITY {
            results.remove(0);
        }
        results.push(ResultEntry {
            inputs,
            style,
            output,
            restored: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LayoutDisplay, style::InlineDirection};
    use taffy::{AvailableSpace, Rect, Size, WritingMode};

    fn inputs(width: f32) -> InlineMeasurementInputs {
        InlineMeasurementInputs {
            numeric: InlineNumericInputs {
                layout: taffy::LayoutInput {
                    run_mode: RunMode::ComputeSize,
                    available_space: Size {
                        width: AvailableSpace::Definite(width),
                        height: AvailableSpace::MaxContent,
                    },
                    ..taffy::LayoutInput::HIDDEN
                },
                writing_mode: WritingMode::HorizontalTb,
                resolved_aspect_ratio: None,
                scrollbar_insets: Rect::zero(),
                display: LayoutDisplay::Block,
                alignment: parley::Alignment::Start,
                direction: InlineDirection::Ltr,
                parent_is_flex_or_grid: false,
            },
            measure_baselines: true,
        }
    }

    #[test]
    fn dense_owner_indices_reject_a_retired_generation_without_removing_its_replacement() {
        let mut arena = slotmap::SlotMap::<LayoutBoxId, ()>::with_key();
        let retired = arena.insert(());
        let style = Style::default();
        let input = inputs(100.0);
        let mut measurement = InlineMeasurements::new();
        let mut accepted = super::super::InlineNumericReuse::new();
        let mut final_input = input.numeric;
        final_input.layout.run_mode = RunMode::PerformLayout;
        measurement.store(retired, input, style.clone(), LayoutOutput::HIDDEN);
        accepted.store(retired, final_input, style.clone(), LayoutOutput::HIDDEN);
        arena.remove(retired);
        let current = arena.insert(());
        assert_ne!(retired, current);
        measurement.store(current, input, style.clone(), LayoutOutput::DEFAULT);
        accepted.store(current, final_input, style.clone(), LayoutOutput::DEFAULT);
        assert!(measurement.lookup(retired, input, &style).is_none());
        assert!(accepted.lookup(retired, final_input, &style).is_none());
        measurement.discard_owner(retired);
        accepted.discard_owner(retired);
        assert_eq!(
            measurement.lookup(current, input, &style),
            Some(LayoutOutput::DEFAULT)
        );
        assert_eq!(
            accepted.lookup(current, final_input, &style),
            Some(LayoutOutput::DEFAULT)
        );
    }

    #[test]
    fn inline_measurements_bound_results_and_distinguish_retained_hits() {
        let owner = LayoutBoxId::from_index(0);
        let style = Style::default();
        let mut cache = InlineMeasurements::new();
        for width in 0..CAPACITY {
            cache.store(
                owner,
                inputs(width as f32),
                style.clone(),
                LayoutOutput::HIDDEN,
            );
        }
        assert!(cache.lookup(owner, inputs(0.0), &style).is_some());
        assert_eq!(cache.hits(), 0);
        cache.reset_hits();
        assert!(cache.lookup(owner, inputs(0.0), &style).is_some());
        assert_eq!(cache.hits(), 1);
        cache.store(owner, inputs(100.0), style.clone(), LayoutOutput::DEFAULT);
        assert_eq!(
            cache.lookup(owner, inputs(100.0), &style),
            Some(LayoutOutput::DEFAULT)
        );
        assert_eq!(cache.hits(), 1, "new results are not previous-pass hits");
        assert!(cache.lookup(owner, inputs(1.0), &style).is_none());
        assert!(cache.lookup(owner, inputs(0.0), &style).is_some());
        for width in 100..140 {
            cache.store(
                owner,
                inputs(width as f32),
                style.clone(),
                LayoutOutput::HIDDEN,
            );
            assert_eq!(cache.owners[0].results.len(), CAPACITY);
        }
    }

    #[test]
    fn inline_measurement_owner_retirement_releases_keys_and_never_revives_them() {
        let mut cache = InlineMeasurements::new();
        let owner = LayoutBoxId::from_index(2);
        let style = Style::default();
        cache.store(owner, inputs(100.0), style.clone(), LayoutOutput::DEFAULT);
        cache.retain_owners(|id| id != owner);
        assert!(!cache.contains(owner));
        assert!(cache.owners.is_empty());
        cache.retain_owners(|_| true);
        assert!(cache.lookup(owner, inputs(100.0), &style).is_none());
    }
}
