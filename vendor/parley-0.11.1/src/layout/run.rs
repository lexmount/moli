// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::FontData;
use crate::layout::cluster::{Cluster, ClusterPath};
use crate::layout::data::{LineItemData, RunData};
use crate::layout::layout::Layout;
use crate::style::Brush;
use core::ops::Range;
use fontique::Synthesis;

/// Sequence of clusters with a single font and style.
#[derive(Copy, Clone)]
pub struct Run<'a, B: Brush> {
    pub(crate) layout: &'a Layout<B>,
    pub(crate) line_index: u32,
    pub(crate) index: u32,
    pub(crate) data: &'a RunData,
    pub(crate) line_data: Option<&'a LineItemData>,
}

impl<'a, B: Brush> Run<'a, B> {
    pub(crate) fn new(
        layout: &'a Layout<B>,
        line_index: u32,
        index: u32,
        data: &'a RunData,
        line_data: Option<&'a LineItemData>,
    ) -> Self {
        Self {
            layout,
            line_index,
            index,
            data,
            line_data,
        }
    }

    /// Returns the index of the run within the line.
    pub fn index(&self) -> usize {
        self.index as usize
    }

    /// Returns the font for the run.
    pub fn font(&self) -> &FontData {
        self.layout.data.fonts.get(self.data.font_index).unwrap()
    }

    /// Returns the font size for the run.
    pub fn font_size(&self) -> f32 {
        self.data.font_size
    }

    /// Returns the font attributes for the run.
    pub fn font_attrs(&self) -> &fontique::Attributes {
        &self.data.font_attrs
    }

    /// Returns the synthesis suggestions for the font associated with the run.
    pub fn synthesis(&self) -> Synthesis {
        self.data.synthesis
    }

    /// Returns the normalized variation coordinates for the font associated
    /// with the run.
    pub fn normalized_coords(&self) -> &[i16] {
        self.layout
            .data
            .coords
            .get(self.data.coords_range.clone())
            .unwrap_or(&[])
    }

    /// Returns metrics for the run.
    pub fn metrics(&self) -> &RunMetrics {
        &self.data.metrics
    }

    /// Returns the advance for the run.
    pub fn advance(&self) -> f32 {
        self.line_data
            .map(|d| d.advance)
            .unwrap_or(self.data.advance)
    }

    /// Returns the original text range for the run.
    pub fn text_range(&self) -> Range<usize> {
        self.line_data
            .map(|d| &d.text_range)
            .unwrap_or(&self.data.text_range)
            .clone()
    }

    /// Returns `true` if the run has right-to-left directionality.
    pub fn is_rtl(&self) -> bool {
        self.data.bidi_level & 1 != 0
    }

    /// Returns the cluster range for the run.
    pub fn cluster_range(&self) -> Range<usize> {
        self.line_data
            .map(|d| &d.cluster_range)
            .unwrap_or(&self.data.cluster_range)
            .clone()
    }

    /// Returns the number of clusters in the run.
    pub fn len(&self) -> usize {
        self.cluster_range().len()
    }

    /// Returns `true` if the run is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the cluster at the specified index.
    pub fn get(&self, index: usize) -> Option<Cluster<'a, B>> {
        let range = self
            .line_data
            .map(|d| &d.cluster_range)
            .unwrap_or(&self.data.cluster_range);
        let original_index = index;
        let index = range.start + index;
        Some(Cluster {
            path: ClusterPath::new(self.line_index, self.index, original_index as u32),
            run: self.clone(),
            data: self.layout.data.clusters.get(index)?,
        })
    }

    /// Returns an iterator over the clusters in logical order.
    pub fn clusters(&'a self) -> impl Iterator<Item = Cluster<'a, B>> + 'a + Clone {
        let range = self.cluster_range();
        Clusters {
            run: self,
            range,
            rev: false,
        }
    }

    /// Returns the visual cluster index for the specified logical cluster index.
    pub fn logical_to_visual(&self, logical_index: usize) -> Option<usize> {
        let num_clusters = self.len();
        if logical_index >= num_clusters {
            return None;
        }

        let visual_index = if self.is_rtl() {
            num_clusters - 1 - logical_index
        } else {
            logical_index
        };

        Some(visual_index)
    }

    /// Returns the logical cluster index for the specified visual cluster index.
    pub fn visual_to_logical(&self, visual_index: usize) -> Option<usize> {
        let num_clusters = self.len();
        if visual_index >= num_clusters {
            return None;
        }

        let logical_index = if self.is_rtl() {
            num_clusters - 1 - visual_index
        } else {
            visual_index
        };

        Some(logical_index)
    }

    /// Returns an iterator over the clusters in visual order.
    pub fn visual_clusters(&'a self) -> impl Iterator<Item = Cluster<'a, B>> + 'a + Clone {
        let range = self.cluster_range();
        Clusters {
            run: self,
            range,
            rev: self.is_rtl(),
        }
    }
}

struct Clusters<'a, B: Brush> {
    run: &'a Run<'a, B>,
    range: Range<usize>,
    rev: bool,
}

impl<B: Brush> Clone for Clusters<'_, B> {
    fn clone(&self) -> Self {
        Self {
            run: self.run,
            range: self.range.clone(),
            rev: self.rev,
        }
    }
}

impl<'a, B: Brush> Iterator for Clusters<'a, B> {
    type Item = Cluster<'a, B>;

    fn next(&mut self) -> Option<Self::Item> {
        let index = if self.rev {
            self.range.next_back()?
        } else {
            self.range.next()?
        };
        Some(Cluster {
            path: ClusterPath::new(
                self.run.line_index,
                self.run.index,
                (index - self.run.cluster_range().start) as u32,
            ),
            run: self.run.clone(),
            data: self.run.layout.data.clusters.get(index)?,
        })
    }
}

/// Metrics information for a run.
#[derive(Copy, Clone, Default, Debug, PartialEq)]
pub struct RunMetrics {
    /// Typographic ascent.
    pub ascent: f32,
    /// Typographic descent.
    pub descent: f32,
    /// Typographic leading.
    pub leading: f32,
    /// Offset of the top of underline decoration from the baseline.
    pub underline_offset: f32,
    /// Thickness of the underline decoration.
    pub underline_size: f32,
    /// Offset of the top of strikethrough decoration from the baseline.
    pub strikethrough_offset: f32,
    /// Thickness of the strikethrough decoration.
    pub strikethrough_size: f32,
    /// The line height
    pub line_height: f32,
    /// Distance from the baseline to the top of short lowercase letters.
    pub x_height: Option<f32>,
    /// Distance from the baseline to the top of capital letters.
    pub cap_height: Option<f32>,
}
