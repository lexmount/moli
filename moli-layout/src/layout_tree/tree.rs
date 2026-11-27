//! The retained layout tree and its memory-retention boundary.

use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    hash::Hash,
    ops::Deref,
};

use crate::{LayoutDisplay, LayoutError, LayoutListMarkerPosition};

use super::model::{
    FrozenCoordinateSpace, LayoutBoxGeometry, LayoutClipNode, LayoutCoordinateSpaceId,
    LayoutFragment, LayoutFragmentId, LayoutOutputBoxId, LayoutPoint, LayoutQuad, LayoutRect,
    LayoutScrollExtent, LayoutSize, LayoutTransform2D, LayoutViewport,
};

pub(super) fn frame_viewport_transform(
    content: LayoutRect,
    viewport_to_local: LayoutTransform2D,
) -> LayoutTransform2D {
    LayoutTransform2D::translation(-content.x, -content.y).concatenate(viewport_to_local)
}

/// A source object without a CSS box, expressed in its owner's local space.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutObjectGeometry<N> {
    pub source: N,
    pub owner: LayoutOutputBoxId,
    pub quad: LayoutQuad,
}

impl<N> LayoutObjectGeometry<N> {
    pub(crate) fn place_in_box(&mut self, geometry: &LayoutBoxGeometry) {
        let content = geometry.content_box;
        let zoom = geometry.effective_zoom;
        self.quad.points = self
            .quad
            .points
            .map(|point| LayoutPoint::new(content.x + point.x * zoom, content.y + point.y * zoom));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CssSizingBox {
    Content,
    Border,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CssSizing {
    pub box_kind: CssSizingBox,
    pub horizontal: bool,
}

pub(crate) struct InlineSplitPart {
    pub box_id: LayoutOutputBoxId,
    pub is_empty_inline_run: bool,
    pub use_inline_contents: bool,
    /// Adjustment in the fragment's local space for this querying inline.
    pub local_offset: LayoutPoint,
}

/// Ordered children of boxes whose CDP geometry comes from their contents.
/// Empty entries represent split edges with no content of their own.
#[derive(Default)]
pub(crate) struct FrozenBoxContents {
    starts: Vec<(LayoutOutputBoxId, u32)>,
    children: Vec<LayoutOutputBoxId>,
}

impl FrozenBoxContents {
    pub(crate) fn append(
        &mut self,
        parent: LayoutOutputBoxId,
        children: impl Iterator<Item = LayoutOutputBoxId>,
    ) {
        let start = u32::try_from(self.children.len())
            .expect("layout content children exceeded the u32 limit");
        self.starts.push((parent, start));
        self.children.extend(children);
    }

    pub(super) fn children_of(&self, box_id: LayoutOutputBoxId) -> Option<&[LayoutOutputBoxId]> {
        let index = self
            .starts
            .binary_search_by_key(&box_id, |&(id, _)| id)
            .ok()?;
        let start = self.starts[index].1 as usize;
        let end = self
            .starts
            .get(index + 1)
            .map_or(self.children.len(), |&(_, end)| end as usize);
        Some(&self.children[start..end])
    }
}

/// One node retained after a full layout pass. Scrolling can refresh its
/// viewport coordinates and scroll extent without rebuilding the box.
///
/// `geometry_source` associates ordinary CSSOM geometry with its source. A
/// split inline continuation can therefore share its originating source while
/// remaining a distinct box-tree node. `hit_source` is separate because a
/// generated pseudo box participates in hit testing as its originating DOM
/// element without manufacturing CSSOM rects for that element.
#[derive(Clone, Debug, PartialEq)]
pub struct FrozenLayoutBox<N> {
    /// Computed display used to construct this box.
    pub display: LayoutDisplay,
    /// Marker placement used when constructing the layout box.
    pub list_marker_position: LayoutListMarkerPosition,
    pub geometry: LayoutBoxGeometry,
    pub scroll_extent: LayoutScrollExtent,
    /// Base mapping at the last geometry publication. Use the tree's
    /// coordinate query to include subsequent scrolling.
    pub(crate) coordinate_space: FrozenCoordinateSpace,
    pub geometry_source: Option<N>,
    pub principal_source: Option<N>,
    pub hit_source: Option<N>,
    #[cfg(feature = "molio")]
    pub(crate) pseudo: Option<crate::LayoutPseudo>,
    /// Box selection and logical-axis orientation for CSSOM sizes, sampled
    /// with the geometry. `None` means no used size (for example, a non-replaced
    /// inline). Retaining this discriminator avoids consulting live display
    /// or box-sizing/writing-mode when an ordinary read reuses an older layout
    /// snapshot. This is interpretation metadata, not another copy of sizes.
    pub(crate) css_sizing: Option<CssSizing>,
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

/// The outcome of reprojecting a published layout after scrolling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrollProjectionRefresh {
    /// No matching snapshot with complete scroll inputs. Nothing was changed.
    Unavailable,
    /// The snapshot already reflects the current scroll offsets.
    Unchanged,
    /// The snapshot's scroll geometry was updated.
    Changed,
}

/// Inputs retained by a frozen snapshot until a descendant requests geometry.
pub(crate) struct FrozenInlineSvgInputs<N> {
    inputs: HashMap<N, crate::InlineSvgInput<N>>,
    owners: HashMap<N, N>,
    payload_bytes: usize,
}

impl<N: Copy + Eq + Hash> FrozenInlineSvgInputs<N> {
    pub(crate) fn new() -> Self {
        Self {
            inputs: HashMap::new(),
            owners: HashMap::new(),
            payload_bytes: 0,
        }
    }

    pub(crate) fn insert(&mut self, root: N, input: crate::InlineSvgInput<N>) {
        for element in input.elements.iter().copied().skip(1) {
            self.owners.insert(element, root);
        }
        self.payload_bytes = self.payload_bytes.saturating_add(input.estimated_bytes());
        let previous = self.inputs.insert(root, input);
        debug_assert!(
            previous.is_none(),
            "each frozen scene has one measured owner"
        );
    }

    pub(crate) fn input(&self, root: N) -> Option<&crate::InlineSvgInput<N>> {
        self.inputs.get(&root)
    }

    fn remove(&mut self, root: N) {
        if let Some(input) = self.inputs.remove(&root) {
            self.payload_bytes -= input.estimated_bytes();
            for element in input.elements.into_iter().skip(1) {
                if self.owners.get(&element) == Some(&root) {
                    self.owners.remove(&element);
                }
            }
        }
    }

    fn estimated_bytes(&self) -> usize {
        let keys = self
            .inputs
            .capacity()
            .saturating_mul(std::mem::size_of::<(N, crate::InlineSvgInput<N>)>());
        let owners = self
            .owners
            .capacity()
            .saturating_mul(std::mem::size_of::<(N, N)>());
        std::mem::size_of::<Self>()
            .saturating_add(keys)
            .saturating_add(owners)
            .saturating_add(self.payload_bytes)
    }
}

/// DOM-independent layout tree produced by one complete pass.
///
/// The box tree is stored densely through parent IDs. Text/inline fragments,
/// coordinate spaces, clips, and scroll extents are canonical layout data:
/// they preserve results that cannot be reconstructed after the working tree,
/// Taffy caches, Parley state, and computed styles are dropped. Source and
/// hit-test indexes are derived from the box provenance and fragments.
/// A scroll refresh updates the coordinate mappings and scroll extents for
/// the existing box structure, local sizes, and fragments.
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
    /// Content-order continuations and promoted blocks, sorted by principal
    /// box ID. Each sequence starts with the principal inline.
    pub(super) inline_splits: Vec<Box<[InlineSplitPart]>>,
    pub(crate) box_contents: FrozenBoxContents,
    /// Source/box relationships for `display: contents` nodes, which own no
    /// principal box but can still nominate rendered descendants for scroll.
    pub scroll_proxy_links: Vec<(N, LayoutOutputBoxId)>,
    pub object_geometry: Vec<LayoutObjectGeometry<N>>,
    pub(crate) svg_viewports: HashMap<N, crate::pass::InlineSvgViewport>,
    pub(crate) frozen_inline_svg: Option<Box<FrozenInlineSvgInputs<N>>>,
    viewport_coordinate_space: FrozenCoordinateSpace,
    pub clip_chain: Vec<LayoutClipNode>,
    embedded_frames: Vec<FrozenEmbeddedFrame<N>>,
    pub(crate) scroll_projection: Option<Box<crate::projection::ScrollProjection<N>>>,
}

impl<N> FrozenLayoutTree<N>
where
    N: Copy + Debug + Eq + Hash,
{
    /// The measured viewport of an inline SVG whose object quads are needed
    /// for the first time in this publication.
    pub fn pending_svg_viewport(&self, source: N) -> Option<LayoutSize> {
        self.svg_viewports
            .get(&source)
            .filter(|svg| !svg.prepared)
            .map(|svg| svg.size)
    }

    pub fn pending_svg_viewports(&self) -> impl Iterator<Item = (N, LayoutSize)> + '_ {
        self.svg_viewports
            .iter()
            .filter_map(|(&source, svg)| (!svg.prepared).then_some((source, svg.size)))
    }

    pub fn has_svg_viewports(&self) -> bool {
        !self.svg_viewports.is_empty()
    }

    /// Locate a scene by the descendant identity from the sampled DOM.
    pub fn pending_frozen_svg_input(
        &self,
        element: N,
    ) -> Option<(N, LayoutSize, &crate::InlineSvgInput<N>)> {
        let frozen = self.frozen_inline_svg.as_ref()?;
        let root = *frozen.owners.get(&element)?;
        Some((root, self.pending_svg_viewport(root)?, frozen.input(root)?))
    }

    /// Publish object quads in the exact layout member that measured the SVG.
    /// Each input quad is local to its SVG viewport in unzoomed CSS pixels.
    pub fn append_svg_geometry(
        &mut self,
        root: N,
        scenes: Vec<(N, Vec<(N, LayoutQuad)>)>,
    ) -> Result<(), LayoutError> {
        let mut metrics = self.retention_metrics();
        self.append_svg_geometry_with_metrics(root, scenes, &mut metrics)
    }

    /// Updates the caller's retained metrics for this exact tree on success.
    /// The metrics must include all embedded members and outstanding inputs.
    pub fn append_svg_geometry_with_metrics(
        &mut self,
        root: N,
        mut scenes: Vec<(N, Vec<(N, LayoutQuad)>)>,
        metrics: &mut LayoutTreeRetentionMetrics,
    ) -> Result<(), LayoutError> {
        let member = self
            .tree_for_root_mut(root)
            .ok_or(LayoutError::NoLayoutSnapshot)?;
        member.retain_pending_svg_scenes(&mut scenes);
        if scenes.is_empty() {
            return Ok(());
        }

        let additional = scenes
            .iter()
            .map(|(_, objects)| objects.len())
            .sum::<usize>();
        let capacity = member.object_geometry.capacity();
        let growth = member
            .object_geometry
            .len()
            .saturating_add(additional)
            .saturating_sub(capacity);
        let frozen_before = member
            .frozen_inline_svg
            .as_ref()
            .map_or(0, |frozen| frozen.estimated_bytes());
        let released = member.frozen_inline_svg.as_ref().map_or(0, |frozen| {
            let mut count = 0;
            let mut bytes = 0usize;
            for (source, _) in &scenes {
                if let Some(input) = frozen.input(*source) {
                    count += 1;
                    bytes = bytes.saturating_add(input.estimated_bytes());
                }
            }
            if count == frozen.inputs.len() {
                frozen_before
            } else {
                bytes
            }
        });
        let geometry_size = std::mem::size_of::<LayoutObjectGeometry<N>>();
        let fragment_count = metrics.fragment_count.saturating_add(additional);
        validate_retention_metrics(LayoutTreeRetentionMetrics {
            fragment_count,
            estimated_geometry_bytes: metrics
                .estimated_geometry_bytes
                .saturating_sub(released)
                .saturating_add(growth.saturating_mul(geometry_size)),
            ..*metrics
        })?;

        member.apply_svg_geometry(scenes, additional);
        let frozen_after = member
            .frozen_inline_svg
            .as_ref()
            .map_or(0, |frozen| frozen.estimated_bytes());
        let growth = member.object_geometry.capacity() - capacity;
        metrics.fragment_count = fragment_count;
        metrics.estimated_geometry_bytes = metrics
            .estimated_geometry_bytes
            .saturating_sub(frozen_before)
            .saturating_add(frozen_after)
            .saturating_add(growth.saturating_mul(geometry_size));
        Ok(())
    }

    fn retain_pending_svg_scenes(&self, scenes: &mut Vec<(N, Vec<(N, LayoutQuad)>)>) {
        let multiple = scenes.len() > 1;
        let mut seen = HashSet::new();
        scenes.retain(|(root, _)| {
            self.pending_svg_viewport(*root).is_some() && (!multiple || seen.insert(*root))
        });
    }

    /// Pass-local geometry is validated once after all frame members are built.
    pub(crate) fn record_painted_svg_geometry(
        &mut self,
        mut scenes: Vec<(N, Vec<(N, LayoutQuad)>)>,
    ) {
        self.retain_pending_svg_scenes(&mut scenes);
        let additional = scenes.iter().map(|(_, objects)| objects.len()).sum();
        self.apply_svg_geometry(scenes, additional);
    }

    fn apply_svg_geometry(&mut self, scenes: Vec<(N, Vec<(N, LayoutQuad)>)>, additional: usize) {
        self.object_geometry.reserve_exact(additional);
        for (source, objects) in scenes {
            let svg = self
                .svg_viewports
                .get_mut(&source)
                .expect("pending measured scene");
            svg.prepared = true;
            if let Some(frozen) = &mut self.frozen_inline_svg {
                frozen.remove(source);
                if frozen.inputs.is_empty() {
                    self.frozen_inline_svg = None;
                }
            }
            let owner = svg.owner;
            let geometry = &self.boxes[owner.index()].geometry;
            self.object_geometry
                .extend(objects.into_iter().map(|(source, quad)| {
                    let mut object = LayoutObjectGeometry {
                        source,
                        owner,
                        quad,
                    };
                    object.place_in_box(geometry);
                    object
                }));
        }
    }

    /// Invalidate sampled SVG content after a style-only publication update.
    pub fn clear_inline_svg_geometry(&mut self) {
        if !self.svg_viewports.is_empty() {
            self.object_geometry.clear();
            for svg in self.svg_viewports.values_mut() {
                svg.prepared = false;
            }
        }
        for frame in &mut self.embedded_frames {
            frame.tree.clear_inline_svg_geometry();
        }
    }

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

    fn tree_for_root_mut(&mut self, source_root: N) -> Option<&mut Self> {
        if self.source_root == source_root {
            return Some(self);
        }
        self.embedded_frames
            .iter_mut()
            .find_map(|frame| frame.tree.tree_for_root_mut(source_root))
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
            .saturating_add(allocation::<Box<[InlineSplitPart]>>(
                self.inline_splits.capacity(),
            ))
            .saturating_add(self.inline_splits.iter().fold(0usize, |bytes, parts| {
                bytes.saturating_add(allocation::<InlineSplitPart>(parts.len()))
            }))
            .saturating_add(allocation::<(LayoutOutputBoxId, u32)>(
                self.box_contents.starts.capacity(),
            ))
            .saturating_add(allocation::<LayoutOutputBoxId>(
                self.box_contents.children.capacity(),
            ))
            .saturating_add(allocation::<LayoutObjectGeometry<N>>(
                self.object_geometry.capacity(),
            ))
            .saturating_add(allocation::<(N, crate::pass::InlineSvgViewport)>(
                self.svg_viewports.capacity(),
            ))
            .saturating_add(
                self.frozen_inline_svg
                    .as_ref()
                    .map_or(0, |frozen| frozen.estimated_bytes()),
            )
            .saturating_add(allocation::<(N, LayoutOutputBoxId)>(
                self.scroll_proxy_links.capacity(),
            ))
            .saturating_add(allocation::<LayoutClipNode>(self.clip_chain.capacity()))
            .saturating_add(allocation::<FrozenEmbeddedFrame<N>>(
                self.embedded_frames.capacity(),
            ))
            .saturating_add(box_allocations)
            .saturating_add(
                self.scroll_projection
                    .as_ref()
                    .map_or(0, |projection| projection.estimated_bytes()),
            );
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
            fragment_count: self
                .fragments
                .len()
                .saturating_add(self.object_geometry.len())
                .saturating_add(embedded.fragment_count),
            estimated_geometry_bytes: own_estimated_geometry_bytes
                .saturating_add(embedded.estimated_geometry_bytes),
        }
    }

    /// Rejects a tree that would make the single latest-layout slot an
    /// unbounded retained allocation.
    pub fn validate_retention_budget(&self) -> Result<(), LayoutError> {
        validate_retention_metrics(self.retention_metrics())
    }

    /// Prefer lazy inputs. If they would exceed the existing snapshot byte
    /// budget, materialize their quads before admitting the publication.
    pub fn validate_retention_budget_with_svg_fallback(
        &mut self,
        mut decode: impl FnMut(N, &crate::InlineSvgInput<N>, LayoutSize) -> Vec<(N, LayoutQuad)>,
    ) -> Result<(), LayoutError> {
        let mut metrics = self.retention_metrics();
        if metrics.estimated_geometry_bytes > MAX_RETAINED_LAYOUT_TREE_BYTES
            && metrics.box_count <= MAX_RETAINED_LAYOUT_BOXES
            && metrics.fragment_count <= MAX_RETAINED_LAYOUT_FRAGMENTS
            && self.materialize_frozen_svg_geometry(&mut decode)
        {
            metrics = self.retention_metrics();
        }
        validate_retention_metrics(metrics)
    }

    fn materialize_frozen_svg_geometry(
        &mut self,
        decode: &mut impl FnMut(N, &crate::InlineSvgInput<N>, LayoutSize) -> Vec<(N, LayoutQuad)>,
    ) -> bool {
        let mut materialized = false;
        if let Some(frozen) = self.frozen_inline_svg.take() {
            let scenes = frozen
                .inputs
                .into_iter()
                .filter_map(|(root, input)| {
                    let size = self.pending_svg_viewport(root)?;
                    Some((root, decode(root, &input, size)))
                })
                .collect();
            self.record_painted_svg_geometry(scenes);
            materialized = true;
        }
        for frame in &mut self.embedded_frames {
            materialized |= frame.tree.materialize_frozen_svg_geometry(decode);
        }
        materialized
    }

    pub fn box_geometry(&self, id: LayoutOutputBoxId) -> Option<&LayoutBoxGeometry> {
        self.boxes
            .get(id.index())
            .map(|layout_box| &layout_box.geometry)
    }

    /// Maps this member's viewport into the embedded viewport owned by `id`.
    /// The transform is independent of the child tree and of hit-test bounds.
    pub fn frame_viewport_transform(&self, id: LayoutOutputBoxId) -> Option<LayoutTransform2D> {
        let geometry = self.box_geometry(id)?;
        let viewport_to_local = self
            .coordinate_space(geometry.coordinate_space)?
            .local_to_viewport
            .inverse()?;
        let transform = frame_viewport_transform(geometry.content_box, viewport_to_local);
        transform.is_finite().then_some(transform)
    }

    pub fn fragment(&self, id: LayoutFragmentId) -> Option<&LayoutFragment> {
        self.fragments.get(id.index())
    }

    pub fn coordinate_space(&self, id: LayoutCoordinateSpaceId) -> Option<FrozenCoordinateSpace> {
        let Some(index) = id.index().checked_sub(1) else {
            return Some(self.viewport_coordinate_space);
        };
        let mut space = self.boxes.get(index)?.coordinate_space;
        if let Some(offset) = self.local_scroll_translation(index) {
            space.local_to_viewport = space
                .local_to_viewport
                .translated_in_viewport(offset.visual);
            space.local_to_viewport_ignoring_css_transforms = space
                .local_to_viewport_ignoring_css_transforms
                .translated_in_viewport(offset.untransformed);
        }
        Some(space)
    }

    #[inline]
    fn local_scroll_translation(
        &self,
        index: usize,
    ) -> Option<&crate::projection::ScrollTranslation> {
        self.scroll_projection.as_ref()?.local_translation(index)
    }

    #[inline]
    pub(super) fn local_to_viewport(
        &self,
        id: LayoutCoordinateSpaceId,
    ) -> Option<LayoutTransform2D> {
        let Some(index) = id.index().checked_sub(1) else {
            return Some(self.viewport_coordinate_space.local_to_viewport);
        };
        let base = self.boxes.get(index)?.coordinate_space.local_to_viewport;
        Some(
            self.local_scroll_translation(index)
                .map_or(base, |offset| base.translated_in_viewport(offset.visual)),
        )
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

    pub fn embedded_frames(&self) -> impl ExactSizeIterator<Item = &FrozenEmbeddedFrame<N>> {
        self.embedded_frames.iter()
    }

    /// Whether the entire snapshot has the placement inputs for reprojecting scroll.
    fn has_scroll_projection(&self) -> bool {
        self.scroll_projection.is_some()
            && self
                .embedded_frames
                .iter()
                .all(|frame| frame.tree.has_scroll_projection())
    }

    /// Refreshes viewport geometry, scrollbars, and sticky positions in place.
    /// The offsets are in the same CSS pixels consumed by LayoutSource.
    /// Checks scroll input availability for every frame before updating any.
    pub fn refresh_scroll_projection(
        &mut self,
        offset: impl Fn(N) -> LayoutPoint,
    ) -> Result<ScrollProjectionRefresh, LayoutError> {
        if !self.has_scroll_projection() {
            return Ok(ScrollProjectionRefresh::Unavailable);
        }
        if self.refresh_scroll_projection_recursive(&offset)? {
            Ok(ScrollProjectionRefresh::Changed)
        } else {
            Ok(ScrollProjectionRefresh::Unchanged)
        }
    }

    fn refresh_scroll_projection_recursive(
        &mut self,
        offset: &impl Fn(N) -> LayoutPoint,
    ) -> Result<bool, LayoutError> {
        let projection = self
            .scroll_projection
            .as_mut()
            .expect("validated scroll inputs");
        let mut changed = projection.refresh(&mut self.boxes, &mut self.viewport_scroll, offset)?;
        for child in &mut self.embedded_frames {
            changed |= child.tree.refresh_scroll_projection_recursive(offset)?;
        }
        Ok(changed)
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
        inline_splits: Vec<Box<[InlineSplitPart]>>,
        scroll_proxy_links: Vec<(N, LayoutOutputBoxId)>,
        viewport_coordinate_space: FrozenCoordinateSpace,
        clip_chain: Vec<LayoutClipNode>,
        embedded_frames: Vec<FrozenEmbeddedFrame<N>>,
    ) -> Self {
        Self {
            source_root,
            viewport,
            viewport_scroll,
            content_size,
            root_box,
            boxes,
            fragments,
            inline_splits,
            box_contents: FrozenBoxContents::default(),
            scroll_proxy_links,
            object_geometry: Vec::new(),
            svg_viewports: HashMap::new(),
            frozen_inline_svg: None,
            viewport_coordinate_space,
            clip_chain,
            embedded_frames,
            scroll_projection: None,
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
