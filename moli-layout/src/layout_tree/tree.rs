//! The immutable retained tree and its memory-retention boundary.

use std::{fmt::Debug, hash::Hash, ops::Deref};

use crate::LayoutError;

use super::model::{
    FrozenCoordinateSpace, LayoutBoxGeometry, LayoutClipNode, LayoutCoordinateSpaceId,
    LayoutFragment, LayoutFragmentId, LayoutOutputBoxId, LayoutPoint, LayoutScrollExtent,
    LayoutSize, LayoutViewport,
};

/// One node in the immutable layout tree retained after a full pass.
///
/// `geometry_source` associates ordinary CSSOM geometry with its source. A
/// split inline continuation can therefore share its originating source while
/// remaining a distinct box-tree node. `hit_source` is separate because a
/// generated pseudo box participates in hit testing as its originating DOM
/// element without manufacturing CSSOM rects for that element.
#[derive(Clone, Debug, PartialEq)]
pub struct FrozenLayoutBox<N> {
    pub geometry: LayoutBoxGeometry,
    pub scroll_extent: LayoutScrollExtent,
    pub coordinate_space: FrozenCoordinateSpace,
    pub geometry_source: Option<N>,
    pub principal_source: Option<N>,
    pub hit_source: Option<N>,
    pub resolved_grid_tracks: Option<super::model::LayoutResolvedGridTracks>,
    /// Paint ordinal of this box's scrollbar/corner surface, if any.
    pub(crate) control_paint_order: Option<u32>,
}

/// One child browsing context owned by the same latest frozen snapshot as its
/// parent. It is deliberately recursive rather than an independently keyed
/// cache entry, so capturing input geometry does not publish a child over the
/// top-level tree.
pub struct FrozenEmbeddedFrame<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub source: N,
    pub tree: Box<FrozenLayoutTree<N>>,
}

impl<N> Deref for FrozenLayoutBox<N> {
    type Target = LayoutBoxGeometry;

    fn deref(&self) -> &Self::Target {
        &self.geometry
    }
}

/// Retained footprint for one frozen layout tree.
///
/// The byte count is an allocation-capacity estimate for the tree's box,
/// fragment, source-provenance, scroll, transform, and clip storage. It
/// excludes allocator metadata and every pass-only diagnostic or paint value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutTreeRetentionMetrics {
    pub box_count: usize,
    pub fragment_count: usize,
    pub estimated_geometry_bytes: usize,
}

/// Maximum geometry boxes retained in the single latest-layout snapshot.
pub const MAX_RETAINED_LAYOUT_BOXES: usize = 1_000_000;
/// Maximum fragments retained in the single latest-layout snapshot.
pub const MAX_RETAINED_LAYOUT_FRAGMENTS: usize = 4_000_000;
/// Maximum estimated allocation capacity retained by one frozen layout tree.
pub const MAX_RETAINED_LAYOUT_TREE_BYTES: usize = 256 * 1024 * 1024;

/// Immutable, DOM-independent layout tree produced by one complete pass.
///
/// The box tree is stored densely through parent IDs. Text/inline fragments,
/// coordinate spaces, clips, and scroll extents are canonical layout data:
/// they preserve results that cannot be reconstructed after the working tree,
/// Taffy caches, Parley state, and computed styles are dropped. Source and
/// hit-test indexes are derived from the box provenance and fragments.
pub struct FrozenLayoutTree<N>
where
    N: Copy + Debug + Eq + Hash,
{
    source_root: N,
    pub viewport: LayoutViewport,
    pub viewport_scroll: LayoutPoint,
    pub content_size: LayoutSize,
    pub root_box: LayoutOutputBoxId,
    pub boxes: Vec<FrozenLayoutBox<N>>,
    pub fragments: Vec<LayoutFragment>,
    /// Source/box relationships for `display: contents` nodes, which own no
    /// principal box but can still nominate rendered descendants for scroll.
    pub scroll_proxy_links: Vec<(N, LayoutOutputBoxId)>,
    viewport_coordinate_space: FrozenCoordinateSpace,
    pub clip_chain: Vec<LayoutClipNode>,
    embedded_frames: Vec<FrozenEmbeddedFrame<N>>,
    embedded_frames_complete: bool,
}

impl<N> FrozenLayoutTree<N>
where
    N: Copy + Debug + Eq + Hash,
{
    /// Source root whose complete layout projection this member tree owns.
    pub const fn source_root(&self) -> N {
        self.source_root
    }

    /// Finds an exact Document/source-root member without a per-Document
    /// index. Browsing-context depth is bounded, so this walks only the small
    /// recursive frame projection rather than the member trees' boxes.
    pub fn tree_for_root(&self, source_root: N) -> Option<&FrozenLayoutTree<N>> {
        if self.source_root == source_root {
            return Some(self);
        }
        self.embedded_frames
            .iter()
            .find_map(|frame| frame.tree.tree_for_root(source_root))
    }

    pub fn retention_metrics(&self) -> LayoutTreeRetentionMetrics {
        fn allocation<T>(capacity: usize) -> usize {
            capacity.saturating_mul(std::mem::size_of::<T>())
        }

        let box_allocations = self.boxes.iter().fold(0usize, |bytes, layout_box| {
            let fragment_bytes = allocation::<LayoutFragmentId>(layout_box.fragments.capacity());
            let grid_bytes = layout_box.resolved_grid_tracks.as_ref().map_or(0, |grid| {
                let numeric_bytes = [
                    grid.rows.used_track_sizes.capacity(),
                    grid.columns.used_track_sizes.capacity(),
                ]
                .into_iter()
                .fold(0usize, |bytes, capacity| {
                    bytes.saturating_add(allocation::<f32>(capacity))
                });
                let line_name_bytes =
                    [&grid.rows, &grid.columns]
                        .into_iter()
                        .fold(0usize, |bytes, tracks| {
                            let outer =
                                allocation::<Vec<String>>(tracks.explicit_line_names.capacity());
                            tracks.explicit_line_names.iter().fold(
                                bytes.saturating_add(outer),
                                |bytes, names| {
                                    let atoms = allocation::<style::Atom>(names.capacity());
                                    let retained_text = names.iter().fold(0usize, |bytes, name| {
                                        bytes.saturating_add(name.len())
                                    });
                                    bytes.saturating_add(atoms).saturating_add(retained_text)
                                },
                            )
                        });
                numeric_bytes.saturating_add(line_name_bytes)
            });
            bytes
                .saturating_add(fragment_bytes)
                .saturating_add(grid_bytes)
        });
        let own_estimated_geometry_bytes = std::mem::size_of::<Self>()
            .saturating_add(allocation::<FrozenLayoutBox<N>>(self.boxes.capacity()))
            .saturating_add(allocation::<LayoutFragment>(self.fragments.capacity()))
            .saturating_add(allocation::<(N, LayoutOutputBoxId)>(
                self.scroll_proxy_links.capacity(),
            ))
            .saturating_add(allocation::<LayoutClipNode>(self.clip_chain.capacity()))
            .saturating_add(allocation::<FrozenEmbeddedFrame<N>>(
                self.embedded_frames.capacity(),
            ))
            .saturating_add(box_allocations);
        let embedded = self.embedded_frames.iter().fold(
            LayoutTreeRetentionMetrics::default(),
            |total, frame| {
                let child = frame.tree.retention_metrics();
                LayoutTreeRetentionMetrics {
                    box_count: total.box_count.saturating_add(child.box_count),
                    fragment_count: total.fragment_count.saturating_add(child.fragment_count),
                    estimated_geometry_bytes: total
                        .estimated_geometry_bytes
                        .saturating_add(child.estimated_geometry_bytes),
                }
            },
        );
        LayoutTreeRetentionMetrics {
            box_count: self.boxes.len().saturating_add(embedded.box_count),
            fragment_count: self.fragments.len().saturating_add(embedded.fragment_count),
            estimated_geometry_bytes: own_estimated_geometry_bytes
                .saturating_add(embedded.estimated_geometry_bytes),
        }
    }

    /// Rejects a tree that would make the single latest-layout slot an
    /// unbounded retained allocation.
    pub fn validate_retention_budget(&self) -> Result<(), LayoutError> {
        validate_retention_metrics(self.retention_metrics())
    }

    pub fn box_geometry(&self, id: LayoutOutputBoxId) -> Option<&LayoutBoxGeometry> {
        self.boxes
            .get(id.index())
            .map(|layout_box| &layout_box.geometry)
    }

    pub fn fragment(&self, id: LayoutFragmentId) -> Option<&LayoutFragment> {
        self.fragments.get(id.index())
    }

    pub fn coordinate_space(&self, id: LayoutCoordinateSpaceId) -> Option<&FrozenCoordinateSpace> {
        match id.index() {
            0 => Some(&self.viewport_coordinate_space),
            index => self
                .boxes
                .get(index - 1)
                .map(|layout_box| &layout_box.coordinate_space),
        }
    }

    pub fn scroll_extent(&self, id: LayoutOutputBoxId) -> Option<&LayoutScrollExtent> {
        self.boxes
            .get(id.index())
            .map(|layout_box| &layout_box.scroll_extent)
    }

    /// Returns the embedded child tree frozen into this same retained snapshot.
    pub fn embedded_frame_tree(&self, source: N) -> Option<&FrozenLayoutTree<N>> {
        self.embedded_frames
            .iter()
            .find(|frame| frame.source == source)
            .map(|frame| frame.tree.as_ref())
    }

    /// Whether every live child frame encountered by this pass was projected.
    /// Hit-test demands require this bit before reusing a tree produced by a
    /// geometry-only pass.
    pub const fn embedded_frames_complete(&self) -> bool {
        self.embedded_frames_complete
    }

    pub fn embedded_frames(&self) -> impl ExactSizeIterator<Item = &FrozenEmbeddedFrame<N>> {
        self.embedded_frames.iter()
    }
}

impl<N> FrozenLayoutTree<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub(crate) fn new(
        source_root: N,
        viewport: LayoutViewport,
        viewport_scroll: LayoutPoint,
        content_size: LayoutSize,
        root_box: LayoutOutputBoxId,
        boxes: Vec<FrozenLayoutBox<N>>,
        fragments: Vec<LayoutFragment>,
        scroll_proxy_links: Vec<(N, LayoutOutputBoxId)>,
        viewport_coordinate_space: FrozenCoordinateSpace,
        clip_chain: Vec<LayoutClipNode>,
        embedded_frames: Vec<FrozenEmbeddedFrame<N>>,
        embedded_frames_complete: bool,
    ) -> Self {
        Self {
            source_root,
            viewport,
            viewport_scroll,
            content_size,
            root_box,
            boxes,
            fragments,
            scroll_proxy_links,
            viewport_coordinate_space,
            clip_chain,
            embedded_frames,
            embedded_frames_complete,
        }
    }
}

fn validate_retention_metrics(metrics: LayoutTreeRetentionMetrics) -> Result<(), LayoutError> {
    if metrics.box_count > MAX_RETAINED_LAYOUT_BOXES
        || metrics.fragment_count > MAX_RETAINED_LAYOUT_FRAGMENTS
        || metrics.estimated_geometry_bytes > MAX_RETAINED_LAYOUT_TREE_BYTES
    {
        return Err(LayoutError::TreeRetentionBudgetExceeded {
            boxes: metrics.box_count,
            fragments: metrics.fragment_count,
            estimated_bytes: metrics.estimated_geometry_bytes,
            max_boxes: MAX_RETAINED_LAYOUT_BOXES,
            max_fragments: MAX_RETAINED_LAYOUT_FRAGMENTS,
            max_bytes: MAX_RETAINED_LAYOUT_TREE_BYTES,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_tree_budget_reports_each_bounded_dimension() {
        for metrics in [
            LayoutTreeRetentionMetrics {
                box_count: MAX_RETAINED_LAYOUT_BOXES + 1,
                ..Default::default()
            },
            LayoutTreeRetentionMetrics {
                fragment_count: MAX_RETAINED_LAYOUT_FRAGMENTS + 1,
                ..Default::default()
            },
            LayoutTreeRetentionMetrics {
                estimated_geometry_bytes: MAX_RETAINED_LAYOUT_TREE_BYTES + 1,
                ..Default::default()
            },
        ] {
            assert!(matches!(
                validate_retention_metrics(metrics),
                Err(LayoutError::TreeRetentionBudgetExceeded { .. })
            ));
        }
        validate_retention_metrics(LayoutTreeRetentionMetrics {
            box_count: MAX_RETAINED_LAYOUT_BOXES,
            fragment_count: MAX_RETAINED_LAYOUT_FRAGMENTS,
            estimated_geometry_bytes: MAX_RETAINED_LAYOUT_TREE_BYTES,
        })
        .expect("each exact retention limit should be accepted");
    }
}
