// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::BoundingBox;
#[cfg(feature = "accesskit")]
use crate::analysis::cluster::Whitespace;
use crate::layout::{Affinity, BreakReason, Cluster, ClusterSide, Layout, Line};
#[cfg(feature = "accesskit")]
use crate::layout::{ClusterPath, LayoutAccessibility};
use crate::style::Brush;
#[cfg(feature = "accesskit")]
use accesskit::TextPosition;

/// Defines a position with a text layout.
#[derive(Copy, Clone, PartialEq, Eq, Default, Debug)]
pub struct Cursor {
    pub(crate) index: usize,
    pub(crate) affinity: Affinity,
}

impl Cursor {
    /// Creates a new cursor from the given byte index and affinity.
    pub fn from_byte_index<B: Brush>(layout: &Layout<B>, index: usize, affinity: Affinity) -> Self {
        if let Some(cluster) = Cluster::from_byte_index(layout, index) {
            let index = cluster.text_range().start;
            Self {
                index,
                affinity: if index != 0 {
                    affinity
                } else {
                    // There is no Upstream cluster of the 0 position so we force Downstream affinity.
                    Affinity::Downstream
                },
            }
        } else {
            Self {
                index: layout.data.text_len,
                affinity: Affinity::Upstream,
            }
        }
    }

    /// Creates a new cursor from the given coordinates.
    pub fn from_point<B: Brush>(layout: &Layout<B>, x: f32, y: f32) -> Self {
        let (index, affinity) = if let Some((cluster, side)) = Cluster::from_point(layout, x, y) {
            let is_leading = side == ClusterSide::Left;
            if cluster.is_rtl() {
                if is_leading {
                    (cluster.text_range().end, Affinity::Upstream)
                } else {
                    (cluster.text_range().start, Affinity::Downstream)
                }
            } else {
                // We never want to position the cursor _after_ a hard
                // line since that cursor appears visually at the start
                // of the next line
                if is_leading || cluster.is_line_break() == Some(BreakReason::Explicit) {
                    (cluster.text_range().start, Affinity::Downstream)
                } else {
                    (cluster.text_range().end, Affinity::Upstream)
                }
            }
        } else {
            (layout.data.text_len, Affinity::Downstream)
        };
        Self { index, affinity }
    }

    #[cfg(feature = "accesskit")]
    pub fn from_access_position<B: Brush>(
        pos: &TextPosition,
        layout: &Layout<B>,
        layout_access: &LayoutAccessibility,
    ) -> Option<Self> {
        let span_path = layout_access.span_paths_by_access_id.get(&pos.node)?;
        let run = span_path.run(layout)?;
        let index = run
            .get(span_path.logical_index() + pos.character_index)
            .map(|cluster| cluster.text_range().start)
            .unwrap_or(layout.data.text_len);
        Some(Self::from_byte_index(layout, index, Affinity::Downstream))
    }

    pub(crate) fn from_cluster<B: Brush>(
        layout: &Layout<B>,
        cluster: Cluster<'_, B>,
        moving_right: bool,
    ) -> Self {
        Self::from_byte_index(
            layout,
            cluster.text_range().start,
            affinity_for_dir(cluster.is_rtl(), moving_right),
        )
    }

    /// Returns the logical text index of the cursor.
    pub fn index(&self) -> usize {
        self.index
    }

    /// Returns the affinity of the cursor.
    ///
    /// This defines the direction from which the cursor entered its current
    /// position and affects the visual location of the rendered cursor.
    pub fn affinity(&self) -> Affinity {
        self.affinity
    }

    /// Returns a new cursor that is guaranteed to be within the bounds of the
    /// given layout.
    #[must_use]
    pub fn refresh<B: Brush>(&self, layout: &Layout<B>) -> Self {
        Self::from_byte_index(layout, self.index, self.affinity)
    }

    /// Returns a new cursor that is positioned at the previous cluster boundary
    /// in visual order.
    #[must_use]
    pub fn previous_visual<B: Brush>(&self, layout: &Layout<B>) -> Self {
        let [left, right] = self.visual_clusters(layout);
        if let (Some(left), Some(right)) = (&left, &right) {
            if left.is_soft_line_break() {
                if left.is_rtl() && self.affinity == Affinity::Upstream {
                    let index = if right.is_rtl() {
                        left.text_range().start
                    } else {
                        left.text_range().end
                    };
                    return Self::from_byte_index(layout, index, Affinity::Downstream);
                } else if !left.is_rtl() && self.affinity == Affinity::Downstream {
                    let index = if right.is_rtl() {
                        right.text_range().end
                    } else {
                        right.text_range().start
                    };
                    return Self::from_byte_index(layout, index, Affinity::Upstream);
                }
            }
        }
        if let Some(left) = left {
            let index = if left.is_rtl() {
                left.text_range().end
            } else {
                left.text_range().start
            };
            return Self::from_byte_index(layout, index, affinity_for_dir(left.is_rtl(), false));
        }
        *self
    }

    /// Returns a new cursor that is positioned at the next cluster boundary
    /// in visual order.
    #[must_use]
    pub fn next_visual<B: Brush>(&self, layout: &Layout<B>) -> Self {
        let [left, right] = self.visual_clusters(layout);
        if let (Some(left), Some(right)) = (&left, &right) {
            if left.is_soft_line_break() {
                if left.is_rtl() && self.affinity == Affinity::Downstream {
                    let index = if right.is_rtl() {
                        right.text_range().end
                    } else {
                        right.text_range().start
                    };
                    return Self::from_byte_index(layout, index, Affinity::Upstream);
                } else if !left.is_rtl() && self.affinity == Affinity::Upstream {
                    let index = if right.is_rtl() {
                        right.text_range().end
                    } else {
                        right.text_range().start
                    };
                    return Self::from_byte_index(layout, index, Affinity::Downstream);
                }
            }
            let index = if right.is_rtl() {
                right.text_range().start
            } else {
                right.text_range().end
            };
            return Self::from_byte_index(layout, index, affinity_for_dir(right.is_rtl(), true));
        }
        if let Some(right) = right {
            let index = if right.is_rtl() {
                right.text_range().start
            } else {
                right.text_range().end
            };
            return Self::from_byte_index(layout, index, affinity_for_dir(right.is_rtl(), true));
        }
        *self
    }

    /// Returns a new cursor that is positioned at the next word boundary
    /// in visual order.
    #[must_use]
    pub fn next_visual_word<B: Brush>(&self, layout: &Layout<B>) -> Self {
        let mut cur = *self;
        loop {
            let next = cur.next_visual(layout);
            if next == cur {
                break;
            }
            cur = next;
            let [Some(left), Some(right)] = cur.visual_clusters(layout) else {
                break;
            };
            if left.is_rtl() {
                if left.is_word_boundary() && !left.is_space_or_nbsp() {
                    break;
                }
            } else if right.is_word_boundary() && !left.is_space_or_nbsp() {
                break;
            }
        }
        cur
    }

    /// Returns a new cursor that is positioned at the previous word boundary
    /// in visual order.
    #[must_use]
    pub fn previous_visual_word<B: Brush>(&self, layout: &Layout<B>) -> Self {
        let mut cur = *self;
        loop {
            let next = cur.previous_visual(layout);
            if next == cur {
                break;
            }
            cur = next;
            let [Some(left), Some(right)] = cur.visual_clusters(layout) else {
                break;
            };
            if left.is_rtl() {
                if left.is_word_boundary()
                    && (left.is_space_or_nbsp()
                        || (right.is_word_boundary() && !right.is_space_or_nbsp()))
                {
                    break;
                }
            } else if right.is_word_boundary() && !right.is_space_or_nbsp() {
                break;
            }
        }
        cur
    }

    /// Returns a new cursor that is positioned at the next word boundary
    /// in logical order.
    #[must_use]
    pub fn next_logical_word<B: Brush>(&self, layout: &Layout<B>) -> Self {
        let [left, right] = self.logical_clusters(layout);
        if let Some(cluster) = right.or(left) {
            let start = cluster.clone();
            let cluster = cluster.next_logical_word().unwrap_or(cluster);
            if cluster.path == start.path {
                return Self::from_byte_index(layout, usize::MAX, Affinity::Downstream);
            }
            return Self::from_cluster(layout, cluster, true);
        }
        *self
    }

    /// Returns a new cursor that is positioned at the previous word boundary
    /// in logical order.
    #[must_use]
    pub fn previous_logical_word<B: Brush>(&self, layout: &Layout<B>) -> Self {
        let [left, right] = self.logical_clusters(layout);
        if let Some(cluster) = left.or(right) {
            let cluster = cluster.previous_logical_word().unwrap_or(cluster);
            return Self::from_cluster(layout, cluster, true);
        }
        *self
    }

    /// Returns a rectangle that represents the visual geometry of the cursor
    /// in layout space.
    ///
    /// The `width` parameter defines the width of the resulting rectangle.
    pub fn geometry<B: Brush>(&self, layout: &Layout<B>, width: f32) -> BoundingBox {
        match self.visual_clusters(layout) {
            [Some(left), Some(right)] => {
                if left.is_end_of_line() {
                    if left.is_soft_line_break() {
                        let (cluster, at_end) = if left.is_rtl()
                            && self.affinity == Affinity::Downstream
                            || !left.is_rtl() && self.affinity == Affinity::Upstream
                        {
                            (left, true)
                        } else {
                            (right, false)
                        };
                        cursor_rect(&cluster, at_end, width)
                    } else {
                        cursor_rect(&right, false, width)
                    }
                } else {
                    cursor_rect(&left, true, width)
                }
            }
            [Some(left), None] if left.is_hard_line_break() => last_line_cursor_rect(layout, width),
            [Some(left), _] => cursor_rect(&left, true, width),
            [_, Some(right)] => cursor_rect(&right, false, width),
            _ => last_line_cursor_rect(layout, width),
        }
    }

    /// Returns the pair of clusters that logically bound the cursor
    /// position.
    ///
    /// The order in the array is upstream followed by downstream.
    pub fn logical_clusters<'a, B: Brush>(
        &self,
        layout: &'a Layout<B>,
    ) -> [Option<Cluster<'a, B>>; 2] {
        let upstream = self
            .index
            .checked_sub(1)
            .and_then(|index| Cluster::from_byte_index(layout, index));
        let downstream = Cluster::from_byte_index(layout, self.index);
        [upstream, downstream]
    }

    /// Returns the pair of clusters that visually bound the cursor
    /// position.
    ///
    /// The order in the array is left followed by right.
    pub fn visual_clusters<'a, B: Brush>(
        &self,
        layout: &'a Layout<B>,
    ) -> [Option<Cluster<'a, B>>; 2] {
        if self.affinity == Affinity::Upstream {
            if let Some(cluster) = self.upstream_cluster(layout) {
                if cluster.is_rtl() {
                    [cluster.previous_visual(), Some(cluster)]
                } else {
                    [Some(cluster.clone()), cluster.next_visual()]
                }
            } else if let Some(cluster) = self.downstream_cluster(layout) {
                if cluster.is_rtl() {
                    [None, Some(cluster)]
                } else {
                    [Some(cluster), None]
                }
            } else {
                [None, None]
            }
        } else if let Some(cluster) = self.downstream_cluster(layout) {
            if cluster.is_rtl() {
                [Some(cluster.clone()), cluster.next_visual()]
            } else {
                [cluster.previous_visual(), Some(cluster)]
            }
        } else if let Some(cluster) = self.upstream_cluster(layout) {
            if cluster.is_rtl() {
                [None, Some(cluster)]
            } else {
                [Some(cluster), None]
            }
        } else {
            [None, None]
        }
    }

    pub(crate) fn line<B: Brush>(self, layout: &Layout<B>) -> Option<(usize, Line<'_, B>)> {
        let geometry = self.geometry(layout, 0.0);
        layout.line_for_offset(geometry.y0 as f32)
    }

    pub(crate) fn upstream_cluster<B: Brush>(self, layout: &Layout<B>) -> Option<Cluster<'_, B>> {
        self.index
            .checked_sub(1)
            .and_then(|index| Cluster::from_byte_index(layout, index))
    }

    pub(crate) fn downstream_cluster<B: Brush>(self, layout: &Layout<B>) -> Option<Cluster<'_, B>> {
        Cluster::from_byte_index(layout, self.index)
    }

    #[cfg(feature = "accesskit")]
    pub fn to_access_position<B: Brush>(
        &self,
        layout: &Layout<B>,
        layout_access: &LayoutAccessibility,
    ) -> Option<TextPosition> {
        if layout.data.text_len == 0 {
            // If the text is empty, just return the first node with a
            // character index of 0.
            return Some(TextPosition {
                node: *layout_access
                    .access_ids_by_span_path
                    .get(&ClusterPath::new(0, 0, 0))?,
                character_index: 0,
            });
        }
        // Prefer the downstream cluster except at the end of the text
        // where we'll choose the upstream cluster and add 1 to the
        // character index.
        let (offset, path) = self
            .downstream_cluster(layout)
            .map(|cluster| (0, cluster.path))
            .or_else(|| {
                self.upstream_cluster(layout)
                    .map(|cluster| (1, cluster.path))
            })?;
        // If we're at the end of the layout and the layout ends with a newline
        // then make sure we use the "phantom" run at the end so that
        // AccessKit has correct visual geometry for the cursor.
        let (span_path, character_index) = if self.index == layout.data.text_len
            && layout
                .data
                .clusters
                .last()
                .map(|cluster| cluster.info.whitespace() == Whitespace::Newline)
                .unwrap_or_default()
        {
            (ClusterPath::new(path.line_index + 1, 0, 0), 0)
        } else {
            let span_path = layout_access.span_paths_by_cluster_path.get(&path).unwrap();
            (
                *span_path,
                path.logical_index() - span_path.logical_index() + offset,
            )
        };
        let id = layout_access.access_ids_by_span_path.get(&span_path)?;
        Some(TextPosition {
            node: *id,
            character_index,
        })
    }
}

// ---

fn affinity_for_dir(is_rtl: bool, moving_right: bool) -> Affinity {
    match (is_rtl, moving_right) {
        (true, true) | (false, false) => Affinity::Downstream,
        _ => Affinity::Upstream,
    }
}

fn cursor_rect<B: Brush>(cluster: &Cluster<'_, B>, at_end: bool, size: f32) -> BoundingBox {
    let mut line_x = cluster.visual_offset().unwrap_or_default();
    if at_end {
        line_x += cluster.advance();
    }
    let line = cluster.line();
    let metrics = line.metrics();
    BoundingBox::new(
        line_x as f64,
        metrics.block_min_coord as f64,
        (line_x + size) as f64,
        metrics.block_max_coord as f64,
    )
}

fn last_line_cursor_rect<B: Brush>(layout: &Layout<B>, size: f32) -> BoundingBox {
    if let Some(line) = layout.get(layout.len().saturating_sub(1)) {
        let metrics = line.metrics();
        BoundingBox::new(
            metrics.offset as f64,
            metrics.block_min_coord as f64,
            (metrics.offset + size) as f64,
            metrics.block_max_coord as f64,
        )
    } else {
        BoundingBox::default()
    }
}
