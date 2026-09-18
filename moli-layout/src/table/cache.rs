//! Pass-local table measurements, including baselines and content bounds.

use taffy::{LayoutInput, LayoutOutput};

const CAPACITY: usize = 8;

/// Exact constraint matches avoid reusing a used size as a guarantee of
/// percentage definiteness. Allocated only for measured tables, with a fixed
/// entry limit independent of document size and the number of probes.
#[derive(Debug, Default)]
pub(crate) struct TableMeasureCache {
    entries: Vec<(LayoutInput, LayoutOutput)>,
    #[cfg(test)]
    pub(super) computations: usize,
}

impl TableMeasureCache {
    pub(crate) fn get(&mut self, inputs: LayoutInput) -> Option<LayoutOutput> {
        let index = self.entries.iter().position(|(key, _)| *key == inputs)?;
        let entry = self.entries.remove(index);
        self.entries.push(entry);
        Some(entry.1)
    }

    pub(crate) fn store(&mut self, inputs: LayoutInput, output: LayoutOutput) {
        if let Some(index) = self.entries.iter().position(|(key, _)| *key == inputs) {
            self.entries.remove(index);
        } else if self.entries.len() == CAPACITY {
            self.entries.remove(0);
        }
        self.entries.push((inputs, output));
        #[cfg(test)]
        {
            self.computations += 1;
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use taffy::{AvailableSpace, Point, RunMode, Size, SizingPurpose, WritingMode};

    fn inputs(width: f32) -> LayoutInput {
        LayoutInput {
            run_mode: RunMode::ComputeSize,
            known_dimensions: Size {
                width: Some(width),
                height: Some(100.0),
            },
            ..LayoutInput::HIDDEN
        }
    }

    #[test]
    fn measured_baselines_survive_reuse_but_not_different_constraints() {
        let mut cache = TableMeasureCache::default();
        let input = inputs(80.0);
        let output = LayoutOutput::from_sizes_and_baseline_sets(
            Size {
                width: 80.0,
                height: 100.0,
            },
            Size {
                width: 80.0,
                height: 120.0,
            },
            Point {
                x: None,
                y: Some(15.0),
            },
            Point {
                x: None,
                y: Some(85.0),
            },
        );
        cache.store(input, output);
        assert_eq!(cache.get(input), Some(output));
        for changed in [
            LayoutInput {
                definite_dimensions: input.known_dimensions,
                ..input
            },
            LayoutInput {
                parent_size: input.known_dimensions,
                ..input
            },
            LayoutInput {
                parent_writing_mode: WritingMode::VerticalRl,
                ..input
            },
            LayoutInput {
                available_space: Size {
                    width: AvailableSpace::MinContent,
                    height: AvailableSpace::MaxContent,
                },
                ..input
            },
            LayoutInput {
                sizing_purpose: SizingPurpose::IntrinsicContribution,
                ..input
            },
            LayoutInput {
                run_mode: RunMode::PerformLayout,
                ..input
            },
        ] {
            assert!(
                cache.get(changed).is_none(),
                "constraints must match: {changed:?}"
            );
        }
        cache.clear();
        assert!(cache.get(input).is_none());
    }

    #[test]
    fn measurements_are_bounded_and_recently_used_entries_survive_eviction() {
        let mut cache = TableMeasureCache::default();
        for index in 0..CAPACITY {
            cache.store(inputs(index as f32), LayoutOutput::HIDDEN);
        }
        assert!(cache.get(inputs(0.0)).is_some());
        cache.store(inputs(CAPACITY as f32), LayoutOutput::HIDDEN);
        assert_eq!(cache.entries.len(), CAPACITY);
        assert!(cache.get(inputs(0.0)).is_some());
        assert!(cache.get(inputs(1.0)).is_none());
        for index in 0..100 {
            cache.store(inputs(index as f32), LayoutOutput::HIDDEN);
            assert!(cache.entries.len() <= CAPACITY);
        }
    }
}
