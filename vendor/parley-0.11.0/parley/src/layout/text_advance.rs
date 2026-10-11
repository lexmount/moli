// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Caller-owned text allocation, independent of shaping and glyph precision.

use super::data::{LayoutData, LayoutItemKind, LineItemData};
use super::spacing::{EffectiveSpacing, Justification};
use crate::Brush;
use alloc::vec::Vec;

#[cfg(feature = "libm")]
#[allow(unused_imports)]
use core_maths::CoreFloat;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextItemQuantization {
    pub(crate) quantum: f32,
    pub(crate) item_ends: Vec<usize>,
    pub(crate) cluster_groups: Vec<usize>,
}

impl TextItemQuantization {
    pub(crate) fn cluster(&self, index: u32) -> (usize, f32) {
        (self.cluster_groups[index as usize], self.quantum)
    }

    /// Keep atoms intact: the first logical character owns a shared glyph.
    /// Empty source continuations remain explicit zero-width line items.
    pub(crate) fn split_line_items<B: Brush>(
        &self,
        data: &LayoutData<B>,
        items: &[LineItemData],
    ) -> Vec<LineItemData> {
        let mut result = Vec::new();
        let mut advance = TextAdvance::default();
        for item in items {
            if item.kind == LayoutItemKind::InlineBox {
                advance = TextAdvance::default();
                result.push(item.clone());
                continue;
            }
            let slice = data.shaped_text.run_slice(item.index as u32);
            let atoms = slice.narrow(item.shaped_cluster_range.clone());
            let mut atoms = atoms.atoms_start().peekable();
            let spacing = EffectiveSpacing::new(data.runs[item.index].spacing, Justification::NONE);
            let mut start = item.text_range.start;
            let mut cluster_start = item.shaped_cluster_range.start;
            while start < item.text_range.end {
                let slot = self.item_ends.partition_point(|end| *end <= start);
                let end = self.item_ends[slot].min(item.text_range.end);
                let mut cluster_end = cluster_start;
                let mut raw = 0.0;
                let mut group = None;
                while let Some(atom) = atoms.peek() {
                    let owner = slice.text_byte_range(atom.char_range()).start;
                    if owner >= end {
                        break;
                    }
                    cluster_end = atom.shaped_clusters_range().end;
                    raw += spacing.atom_advance(atom);
                    group = Some(self.cluster(atom.shaped_clusters_range().start));
                    atoms.next();
                }
                let next = group.map_or(advance, |group| advance.with_text(raw, Some(group)));
                let mut part = item.clone();
                part.text_range = start..end;
                part.shaped_cluster_range = cluster_start..cluster_end;
                part.allocated_advance = Some(next.value - advance.value);
                result.push(part);
                advance = next;
                cluster_start = cluster_end;
                start = end;
            }
            if item.text_range.is_empty() {
                result.push(item.clone());
            }
        }
        result
    }
}

/// Accumulate raw advances within a text item, but allocate its rounded width.
/// Cloning this value also preserves the unrounded tail at a line-break checkpoint.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TextAdvance {
    pub(crate) value: f32,
    tail: Option<Tail>,
}

#[derive(Clone, Copy, Debug)]
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
