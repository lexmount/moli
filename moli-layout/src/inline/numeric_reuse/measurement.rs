//! Bounded size-only results; accepted line layouts remain on their IFC owner.

use style::Atom;
use taffy::{LayoutOutput, RunMode, Style};

use super::{InlineNumericInputs, allocation, style_allocation};
use crate::LayoutBoxId;

// Each owner can be probed under several intrinsic/definite constraints. Keep
// a small LRU rather than replacing its accepted final-layout result. The
// document's existing byte admission additionally bounds the whole sidecar.
const CAPACITY: usize = 4;

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
    slots: Vec<Option<usize>>,
    retained_hits: usize,
}

impl InlineMeasurements {
    pub(super) fn new(box_count: usize) -> Self {
        Self {
            owners: Vec::new(),
            slots: vec![None; box_count],
            retained_hits: 0,
        }
    }

    pub(super) fn contains(&self, owner: LayoutBoxId) -> bool {
        self.slots.get(owner.index()).is_some_and(Option::is_some)
    }

    pub(super) fn retain_owners(
        &mut self,
        box_count: usize,
        mut keep: impl FnMut(LayoutBoxId) -> bool,
    ) {
        self.owners
            .retain(|entry| entry.owner.index() < box_count && keep(entry.owner));
        self.slots.clear();
        self.slots.resize(box_count, None);
        for (slot, entry) in self.owners.iter().enumerate() {
            self.slots[entry.owner.index()] = Some(slot);
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
        let slot = self.slots.get(owner.index()).copied().flatten()?;
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
        assert!(owner.index() < self.slots.len());
        let inputs = inputs.comparison_key(&style);
        let slot = *self.slots[owner.index()].get_or_insert_with(|| {
            let slot = self.owners.len();
            self.owners.push(OwnerEntry {
                owner,
                // Most fixed-width owners use one actual measurement key.
                // Grow to the bounded LRU capacity only for distinct probes.
                results: Vec::with_capacity(1),
            });
            slot
        });
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

    pub(super) fn estimated_retained_bytes(&self) -> usize {
        self.owners.iter().fold(
            std::mem::size_of::<Self>()
                .saturating_add(allocation(&self.owners))
                .saturating_add(allocation(&self.slots)),
            |bytes, owner| {
                owner.results.iter().fold(
                    bytes.saturating_add(allocation(&owner.results)),
                    |bytes, result| bytes.saturating_add(style_allocation(&result.style)),
                )
            },
        )
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
    fn inline_measurements_bound_results_and_distinguish_retained_hits() {
        let owner = LayoutBoxId::from_index(0);
        let style = Style::default();
        let mut cache = InlineMeasurements::new(1);
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
        let mut cache = InlineMeasurements::new(3);
        let owner = LayoutBoxId::from_index(2);
        let style = Style {
            grid_template_row_names: vec![Vec::with_capacity(1024)],
            ..Style::default()
        };
        cache.store(owner, inputs(100.0), style, LayoutOutput::DEFAULT);
        let charged = cache.estimated_retained_bytes();
        cache.retain_owners(1, |_| true);
        assert!(!cache.contains(owner));
        assert!(cache.owners.is_empty());
        assert!(cache.estimated_retained_bytes() < charged);
        cache.retain_owners(3, |_| true);
        assert!(
            cache
                .lookup(owner, inputs(100.0), &Style::default())
                .is_none()
        );
    }
}
