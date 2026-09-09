// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Range based style application.

use alloc::vec;

use super::{Brush, RangedProperty, RangedStyle, ResolvedProperty, ResolvedStyle, StyleRun, Vec};
use core::ops::{Bound, Range, RangeBounds};

/// Builder for constructing an ordered sequence of non-overlapping ranged
/// styles from a collection of ranged style properties.
#[derive(Clone)]
pub(crate) struct RangedStyleBuilder<B: Brush> {
    properties: Vec<RangedProperty<B>>,
    scratch_styles: Vec<RangedStyle<B>>,
    root_style: ResolvedStyle<B>,
    len: usize,
}

impl<B: Brush> Default for RangedStyleBuilder<B> {
    fn default() -> Self {
        Self {
            properties: vec![],
            scratch_styles: vec![],
            root_style: ResolvedStyle::default(),
            // We use `usize::MAX` as a sentinel that `begin` hasn't been called.
            // This is required (rather than requiring the root style in the constructor)
            // as we want to support re-using this value.
            len: usize::MAX,
        }
    }
}

impl<B: Brush> RangedStyleBuilder<B> {
    /// Prepares the builder for accepting ranged properties for text of the specified length.
    ///
    /// The provided `root_style` is the default style applied to all text unless overridden.
    pub(crate) fn begin(&mut self, root_style: ResolvedStyle<B>, len: usize) {
        self.properties.clear();
        self.scratch_styles.clear();
        self.root_style = root_style;
        self.len = len;
    }

    /// Change a property of the root style, which covers the full range of text.
    ///
    /// # Panics
    ///
    /// If [`begin`](Self::begin) has not been called before using this method.
    pub(crate) fn push_default(&mut self, property: ResolvedProperty<B>) {
        assert!(
            self.len != usize::MAX,
            "Internal error: Must call `begin` before setting properties on a `RangedStyleBuilder`."
        );
        self.root_style.apply(property);
    }

    /// Override a property for the specified range of text.
    ///
    /// # Panics
    ///
    /// If [`begin`](Self::begin) has not been called before using this method.
    pub(crate) fn push(&mut self, property: ResolvedProperty<B>, range: impl RangeBounds<usize>) {
        assert!(
            self.len != usize::MAX,
            "Internal error: Must call `begin` before setting properties on a `RangedStyleBuilder`."
        );
        let range = resolve_range(range, self.len);
        self.properties.push(RangedProperty { property, range });
    }

    /// Computes style table + style runs for the ranged properties.
    pub(crate) fn finish(
        &mut self,
        style_table: &mut Vec<ResolvedStyle<B>>,
        style_runs: &mut Vec<StyleRun>,
    ) {
        style_table.clear();
        style_runs.clear();
        if self.len == usize::MAX {
            self.properties.clear();
            self.scratch_styles.clear();
            self.root_style = ResolvedStyle::default();
            return;
        }
        let styles = &mut self.scratch_styles;
        styles.push(RangedStyle {
            style: self.root_style.clone(),
            range: 0..self.len,
        });
        for prop in &self.properties {
            if prop.range.start > prop.range.end {
                continue;
            }
            let split_range = split_range(prop, styles);
            let mut inserted = 0;
            if let Some(first) = split_range.first {
                let original_span = &mut styles[first];
                if !original_span.style.check(&prop.property) {
                    let mut new_span = original_span.clone();
                    let original_end = original_span.range.end;
                    original_span.range.end = prop.range.start;
                    new_span.range.start = prop.range.start;
                    new_span.style.apply(prop.property.clone());
                    if split_range.replace_len == 0 && split_range.last == Some(first) {
                        let mut new_end_span = original_span.clone();
                        new_end_span.range.start = prop.range.end;
                        new_end_span.range.end = original_end;
                        new_span.range.end = prop.range.end;
                        styles.splice(
                            first + 1..first + 1,
                            [new_span, new_end_span].iter().cloned(),
                        );
                        continue;
                    } else {
                        styles.insert(first + 1, new_span);
                    }
                    inserted += 1;
                }
            }
            let replace_start = split_range.replace_start + inserted;
            let replace_end = replace_start + split_range.replace_len;
            for style in &mut styles[replace_start..replace_end] {
                style.style.apply(prop.property.clone());
            }
            if let Some(mut last) = split_range.last {
                last += inserted;
                let original_span = &mut styles[last];
                if !original_span.style.check(&prop.property) {
                    let mut new_span = original_span.clone();
                    original_span.range.start = prop.range.end;
                    new_span.range.end = prop.range.end;
                    new_span.style.apply(prop.property.clone());
                    styles.insert(last, new_span);
                }
            }
        }
        let mut prev_index = 0;
        let mut merged_count = 0;
        for i in 1..styles.len() {
            if styles[prev_index].style == styles[i].style {
                let end = styles[i].range.end;
                styles[prev_index].range.end = end;
                merged_count += 1;
            } else {
                prev_index += 1;
                if prev_index != i {
                    let moved_span = styles[i].clone();
                    styles[prev_index] = moved_span;
                }
            }
        }
        styles.truncate(styles.len() - merged_count);

        style_table.reserve(styles.len());
        style_runs.reserve(styles.len());
        for (style_index, style) in styles.drain(..).enumerate() {
            style_table.push(style.style);
            style_runs.push(StyleRun {
                style_index: style_index as u16,
                range: style.range,
            });
        }

        self.properties.clear();
        self.root_style = ResolvedStyle::default();
        self.len = usize::MAX;
    }
}

#[derive(Default)]
struct SplitRange {
    first: Option<usize>,
    replace_start: usize,
    replace_len: usize,
    last: Option<usize>,
}

fn split_range<B: Brush>(prop: &RangedProperty<B>, spans: &[RangedStyle<B>]) -> SplitRange {
    let mut range = SplitRange::default();
    let start_span_index =
        match spans.binary_search_by(|span| span.range.start.cmp(&prop.range.start)) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        };
    let mut end_span_index = spans.len() - 1;
    for (i, span) in spans[start_span_index..].iter().enumerate() {
        if span.range.end >= prop.range.end {
            end_span_index = i + start_span_index;
            break;
        }
    }
    let start_span = &spans[start_span_index];
    let end_span = &spans[end_span_index];
    if start_span.range.start < prop.range.start {
        range.first = Some(start_span_index);
        range.replace_start = start_span_index + 1;
    } else {
        range.replace_start = start_span_index;
    }
    if end_span.range.end > prop.range.end {
        range.last = Some(end_span_index);
        range.replace_len = end_span_index.saturating_sub(range.replace_start);
    } else {
        range.replace_len = (end_span_index + 1).saturating_sub(range.replace_start);
    }
    range
}

/// Resolves a `RangeBounds` into a range in the range 0..len.
fn resolve_range(range: impl RangeBounds<usize>, len: usize) -> Range<usize> {
    let start = match range.start_bound() {
        Bound::Unbounded => 0,
        Bound::Included(n) => *n,
        Bound::Excluded(n) => *n + 1,
    };
    let end = match range.end_bound() {
        Bound::Unbounded => len,
        Bound::Included(n) => *n + 1,
        Bound::Excluded(n) => *n,
    };
    start.min(len)..end.min(len)
}
