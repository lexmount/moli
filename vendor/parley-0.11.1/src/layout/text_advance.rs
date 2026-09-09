// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Optional allocation precision, separate from font shaping precision.

use super::data::{ClusterData, RunData};
use alloc::vec::Vec;

#[cfg(feature = "libm")]
#[allow(unused_imports)]
use core_maths::CoreFloat;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextItemQuantization {
    pub(crate) quantum: f32,
    pub(crate) cluster_groups: Vec<usize>,
    original_clusters: Vec<(usize, ClusterData)>,
}

impl TextItemQuantization {
    pub(crate) fn new(
        quantum: f32,
        cluster_groups: Vec<usize>,
        runs: &[RunData],
        clusters: &mut [ClusterData],
    ) -> Self {
        let mut original_clusters = Vec::new();
        for run in runs {
            let rtl = run.bidi_level & 1 != 0;
            let mut start = run.cluster_range.start;
            while start < run.cluster_range.end {
                let end = shaping_cluster_end(clusters, start, run.cluster_range.end, rtl);
                let group = cluster_groups[start];
                let owner_end = (start + 1..end)
                    .find(|&index| cluster_groups[index] != group)
                    .unwrap_or(end);
                if owner_end < end {
                    // A shared glyph belongs to the item containing its first
                    // logical character, not to every caret component. Keep
                    // its entire advance in that item, as a shaping subrange
                    // does when it selects glyphs by source character index.
                    let glyph_source = if rtl { end - 1 } else { start };
                    let glyph_owner = if rtl { owner_end - 1 } else { start };
                    let glyph_data = clusters[glyph_source];
                    let advance = clusters[start..end].iter().map(|c| c.advance).sum::<f32>();
                    let component_advance = advance / (owner_end - start) as f32;
                    for index in start..end {
                        original_clusters.push((index, clusters[index]));
                        clusters[index].advance = if index < owner_end {
                            component_advance
                        } else {
                            0.0
                        };
                        clusters[index].glyph_len = 0;
                    }
                    clusters[glyph_owner].glyph_len = glyph_data.glyph_len;
                    clusters[glyph_owner].glyph_offset = glyph_data.glyph_offset;
                }
                start = end;
            }
        }
        Self {
            quantum,
            cluster_groups,
            original_clusters,
        }
    }

    pub(crate) fn restore_clusters(self, clusters: &mut [ClusterData]) {
        for (index, original) in self.original_clusters {
            clusters[index] = original;
        }
    }

    pub(crate) fn round(&self, value: f32) -> f32 {
        round_item_width(value, self.quantum)
    }

    pub(crate) fn cluster(&self, index: usize) -> (usize, f32) {
        (self.cluster_groups[index], self.quantum)
    }
}

/// End of the HarfRust shaping cluster starting at `start` in logical order.
/// Parley's ligature-start flag identifies the visual first component, so RTL
/// clusters have their continuation flags before that marker in the array.
pub(crate) fn shaping_cluster_end(
    clusters: &[ClusterData],
    start: usize,
    run_end: usize,
    rtl: bool,
) -> usize {
    let mut end = start + 1;
    if rtl && clusters[start].is_ligature_component() {
        while end < run_end && clusters[end].is_ligature_component() {
            end += 1;
        }
        if end < run_end && clusters[end].is_ligature_start() {
            end += 1;
        }
    } else if !rtl && clusters[start].is_ligature_start() {
        while end < run_end && clusters[end].is_ligature_component() {
            end += 1;
        }
    }
    end
}

/// Accumulate raw advances within a text item, but allocate its rounded width.
/// Cloning this value also preserves the unrounded tail at a line-break checkpoint.
#[derive(Clone, Copy, Default)]
pub(crate) struct TextAdvance {
    pub(crate) value: f32,
    tail: Option<Tail>,
}

#[derive(Clone, Copy)]
struct Tail {
    group: usize,
    quantum: f32,
    raw: f32,
    rounded: f32,
}

impl TextAdvance {
    pub(crate) fn from_width(value: f32) -> Self {
        Self { value, tail: None }
    }

    pub(crate) fn with_text(self, width: f32, quantization: Option<(usize, f32)>) -> Self {
        let Some((group, quantum)) = quantization else {
            return Self::from_width(self.value + width);
        };
        let (base, raw) = match self.tail {
            Some(tail) if tail.group == group => (self.value - tail.rounded, tail.raw + width),
            _ => (self.value, width),
        };
        let rounded = round_item_width(raw, quantum);
        Self {
            value: base + rounded,
            tail: Some(Tail {
                group,
                quantum,
                raw,
                rounded,
            }),
        }
    }

    pub(crate) fn without_trailing_space(self, width: f32) -> f32 {
        self.with_text(-width, self.tail.map(|tail| (tail.group, tail.quantum)))
            .value
    }
}

fn round_item_width(value: f32, quantum: f32) -> f32 {
    // Negative letter spacing can give a shaped item a negative raw advance,
    // but it must not allocate negative space and pull following items back.
    ((value / quantum).ceil() * quantum).max(0.0)
}
