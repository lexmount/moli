use std::{collections::HashMap, fmt::Debug, hash::Hash, time::Instant};

use taffy::ResolveOrZero;

use crate::stacking::{PaintOrderEvent, build_paint_order};
use crate::{
    LayoutAnonymousReason, LayoutBoxGeometry, LayoutBoxId, LayoutClipChainId, LayoutClipNode,
    LayoutCoordinateSpace, LayoutCoordinateSpaceId, LayoutError, LayoutFlushReason, LayoutFragment,
    LayoutFragmentBoxModel, LayoutFragmentId, LayoutFragmentKind, LayoutHitTestEntry,
    LayoutHitTestIndex, LayoutNodeOutput, LayoutOutputBoxId, LayoutPassMetrics, LayoutPassOutput,
    LayoutPoint, LayoutRect, LayoutScrollExtent, LayoutScrollExtentId, LayoutSize,
    LayoutTransform2D, LayoutViewport, LayoutWorld, PaintCaptureRequest, PaintDiagnostic,
    PaintDiagnosticSeverity,
};

pub(crate) fn project_layout_output<N>(
    world: &LayoutWorld<N>,
    viewport: LayoutViewport,
    reason: LayoutFlushReason,
    started: Instant,
    paint_capture: Option<PaintCaptureRequest>,
    embedded_frames: &mut HashMap<LayoutBoxId, crate::PaintSnapshot>,
) -> Result<LayoutPassOutput<N>, LayoutError>
where
    N: Copy + Debug + Eq + Hash,
{
    let mut projection = OutputProjection::new(world, viewport);
    projection.build_local_box_geometry();
    projection.resolve_scrollable_overflow();
    projection.build_coordinate_spaces()?;
    projection.build_fragments_and_source_mapping();
    projection.assign_clip_and_paint_order();

    let content_size = projection.document_content_size();
    let mut paint_snapshot = paint_capture
        .map(|request| {
            request
                .resolve(viewport, projection.viewport_scroll, content_size)
                .map(|capture| {
                    crate::paint::project_paint_snapshot(&projection, capture, embedded_frames)
                })
        })
        .transpose()?;
    let mut diagnostics = paint_snapshot
        .as_ref()
        .map(|snapshot| snapshot.diagnostics.clone())
        .unwrap_or_default();
    for diagnostic in &projection.diagnostics {
        if !diagnostics.contains(diagnostic) {
            diagnostics.push(diagnostic.clone());
        }
    }
    if let Some(snapshot) = paint_snapshot.as_mut() {
        snapshot.content_size = content_size;
        for diagnostic in &projection.diagnostics {
            if !snapshot.diagnostics.contains(diagnostic) {
                snapshot.push_diagnostic(diagnostic.clone());
            }
        }
    }
    let metrics = LayoutPassMetrics {
        reason,
        elapsed: started.elapsed(),
        box_count: projection.boxes.len(),
        fragment_count: projection.fragments.len(),
        paint_operation_count: paint_snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.fragments.len()),
        fallback_count: diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == PaintDiagnosticSeverity::Warning)
            .count(),
    };

    Ok(LayoutPassOutput::new(
        viewport,
        projection.viewport_scroll,
        content_size,
        LayoutOutputBoxId::from_index(world.root.index()),
        projection.boxes,
        projection.box_sources,
        projection.fragments,
        projection.source_mapping,
        projection.scroll_extents,
        projection.coordinate_spaces,
        projection.clip_chain,
        projection.paint_order,
        LayoutHitTestIndex {
            entries: projection.hit_entries,
        },
        diagnostics,
        metrics,
        paint_snapshot,
    ))
}

pub(crate) struct OutputProjection<'a, N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub(crate) world: &'a LayoutWorld<N>,
    pub(crate) viewport: LayoutViewport,
    viewport_scroll: LayoutPoint,
    pub(crate) boxes: Vec<LayoutBoxGeometry>,
    box_sources: Vec<Option<N>>,
    fragments: Vec<LayoutFragment>,
    source_mapping: HashMap<N, LayoutNodeOutput>,
    pub(crate) scroll_extents: Vec<LayoutScrollExtent>,
    pub(crate) coordinate_spaces: Vec<LayoutCoordinateSpace>,
    pub(crate) clip_chain: Vec<LayoutClipNode>,
    paint_order: Vec<LayoutFragmentId>,
    hit_entries: Vec<LayoutHitTestEntry<N>>,
    pub(crate) diagnostics: Vec<PaintDiagnostic>,
    pub(crate) resolved_transforms: Vec<LayoutTransform2D>,
    overflow: Vec<LayoutRect>,
    viewport_anchored: Vec<bool>,
    direct_fragments: Vec<Option<LayoutFragmentId>>,
    owner_paint_fragments: Vec<Vec<LayoutFragmentId>>,
    pub(crate) background_clips: Vec<Option<LayoutClipChainId>>,
    pub(crate) content_clips: Vec<Option<LayoutClipChainId>>,
    pub(crate) paint_events: Vec<PaintOrderEvent>,
}

impl<'a, N> OutputProjection<'a, N>
where
    N: Copy + Debug + Eq + Hash,
{
    fn new(world: &'a LayoutWorld<N>, viewport: LayoutViewport) -> Self {
        let count = world.boxes.len();
        Self {
            world,
            viewport,
            viewport_scroll: LayoutPoint::ZERO,
            boxes: Vec::with_capacity(count),
            box_sources: Vec::with_capacity(count),
            fragments: Vec::new(),
            source_mapping: HashMap::with_capacity(world.source_mapping.len()),
            scroll_extents: Vec::with_capacity(count),
            coordinate_spaces: Vec::with_capacity(count + 1),
            clip_chain: Vec::new(),
            paint_order: Vec::new(),
            hit_entries: Vec::new(),
            diagnostics: Vec::new(),
            resolved_transforms: vec![LayoutTransform2D::IDENTITY; count],
            overflow: vec![LayoutRect::ZERO; count],
            viewport_anchored: vec![false; count],
            direct_fragments: vec![None; count],
            owner_paint_fragments: vec![Vec::new(); count],
            background_clips: vec![None; count],
            content_clips: vec![None; count],
            paint_events: Vec::new(),
        }
    }

    fn build_local_box_geometry(&mut self) {
        for (index, layout_box) in self.world.boxes.iter().enumerate() {
            for diagnostic in layout_box.capability_diagnostics() {
                self.diagnostics.push(PaintDiagnostic::new(
                    diagnostic.code(),
                    format!(
                        "{} uses a conservative layout/paint fallback for {}",
                        layout_box.source_label,
                        diagnostic.code()
                    ),
                    PaintDiagnosticSeverity::Warning,
                ));
            }
            let id = LayoutOutputBoxId::from_index(index);
            let layout = layout_box.final_layout;
            let border_box = LayoutRect::new(
                0.0,
                0.0,
                layout.size.width.max(0.0),
                layout.size.height.max(0.0),
            );
            let padding_box = inset_rect(
                border_box,
                layout.border.top,
                layout.border.right,
                layout.border.bottom,
                layout.border.left,
            );
            let content_box = inset_rect(
                padding_box,
                layout.padding.top,
                layout.padding.right,
                layout.padding.bottom,
                layout.padding.left,
            );
            let margin_box = outset_rect(
                border_box,
                layout.margin.top,
                layout.margin.right,
                layout.margin.bottom,
                layout.margin.left,
            );
            let resolved = layout_box
                .style
                .resolved_2d_transform(border_box.width, border_box.height);
            self.resolved_transforms[index] = resolved.transform;
            if resolved.has_unsupported_3d {
                self.diagnostics.push(PaintDiagnostic::new(
                    "transform-3d-geometry-fallback",
                    format!(
                        "{} contains a 3D transform or perspective that cannot be represented by the Phase 5 2D coordinate space",
                        layout_box.source_label
                    ),
                    PaintDiagnosticSeverity::Warning,
                ));
            }
            let mut overflow = border_box;
            let content_extent = LayoutRect::new(
                0.0,
                0.0,
                layout.size.width.max(layout.content_size.width).max(0.0),
                layout.size.height.max(layout.content_size.height).max(0.0),
            );
            overflow = overflow.union(content_extent);
            if let Some(context) = layout_box.inline_layout.as_ref() {
                let origin = LayoutPoint::new(
                    layout.border.left + layout.padding.left,
                    layout.border.top + layout.padding.top,
                );
                for line in &context.fragments.lines {
                    overflow = overflow.union(offset_rect(line.rect, origin));
                }
                for fragment in &context.fragments.text {
                    overflow = overflow.union(offset_rect(fragment.rect, origin));
                }
                for fragment in &context.fragments.boxes {
                    overflow = overflow.union(offset_rect(fragment.rect, origin));
                }
            }
            self.overflow[index] = overflow;
            let source = layout_box.source.or_else(|| {
                (layout_box.anonymous_reason
                    == Some(LayoutAnonymousReason::InlineSplitContinuation))
                .then_some(layout_box.owner)
                .flatten()
            });
            self.box_sources.push(source);
            let semantics = layout_box.element_semantics();
            let (layout_x, layout_y) = self
                .world
                .global_layout_origin(LayoutBoxId::from_index(index));
            self.boxes.push(LayoutBoxGeometry {
                id,
                parent: layout_box
                    .parent
                    .map(|parent| LayoutOutputBoxId::from_index(parent.index())),
                layout_parent: layout_box
                    .layout_parent
                    .map(|parent| LayoutOutputBoxId::from_index(parent.index())),
                positioned_containing_block: layout_box
                    .positioned_containing_block
                    .map(|parent| LayoutOutputBoxId::from_index(parent.index())),
                role: layout_box.kind,
                source_label: layout_box.source_label.clone(),
                position: layout_box.style.position(),
                coordinate_space: LayoutCoordinateSpaceId::from_index(index + 1),
                clip_chain: None,
                content_box,
                padding_box,
                border_box,
                margin_box,
                scroll_extent: LayoutScrollExtentId::from_index(index),
                fragments: Vec::new(),
                layout_origin_in_document: LayoutPoint::new(layout_x, layout_y),
                is_body_element: semantics.is_some_and(|element| element.is_html_element("body")),
                is_table_offset_parent: semantics.is_some_and(|element| {
                    element.is_html_element("table")
                        || element.is_html_element("td")
                        || element.is_html_element("th")
                }),
                establishes_positioned_containing_block: layout_box
                    .establishes_positioned_containing_block(),
                establishes_fixed_containing_block: layout_box.establishes_fixed_containing_block(),
                visible: layout_box.style.is_visible(),
                pointer_events: layout_box.style.accepts_pointer_events(),
            });
        }
    }

    fn resolve_scrollable_overflow(&mut self) {
        for index in (0..self.world.boxes.len()).rev() {
            let parent_id = self.world.boxes[index].layout_parent.or_else(|| {
                // An absolutely positioned box whose containing block is the
                // initial containing block is a direct child of Taffy's
                // virtual viewport. It still contributes to the root
                // element's scrollable overflow. Fixed boxes are excluded
                // below because they remain anchored to that viewport.
                (index != self.world.root.index()).then_some(self.world.root)
            });
            let Some(parent_id) = parent_id else {
                continue;
            };
            if self.world.boxes[index].style.is_fixed_positioned() {
                continue;
            }
            let child_geometry = &self.boxes[index];
            let visual_overflow = if self.world.boxes[index].style.clips_overflow() {
                child_geometry.border_box
            } else {
                self.overflow[index]
            };
            let location = self.world.boxes[index].final_layout.location;
            let layout_translation = LayoutTransform2D::translation(location.x, location.y);
            let local_to_parent = layout_translation.concatenate(self.resolved_transforms[index]);
            let mapped = local_to_parent
                .map_rect(visual_overflow)
                .bounding_rect()
                .union(
                    layout_translation
                        .map_rect(child_geometry.margin_box)
                        .bounding_rect(),
                );
            self.overflow[parent_id.index()] = self.overflow[parent_id.index()].union(mapped);
        }

        for index in 0..self.world.boxes.len() {
            let geometry = &self.boxes[index];
            let overflow = self.overflow[index];
            let is_root = index == self.world.root.index();
            let scrollport = if is_root {
                LayoutRect::new(
                    0.0,
                    0.0,
                    self.viewport.css_width as f32,
                    self.viewport.css_height as f32,
                )
            } else {
                geometry.padding_box
            };
            let scroll_size = LayoutSize::new(
                scrollport
                    .width
                    .max((overflow.right() - scrollport.x).max(0.0)),
                scrollport
                    .height
                    .max((overflow.bottom() - scrollport.y).max(0.0)),
            );
            let horizontal_range = (scroll_size.width - scrollport.width).max(0.0);
            let vertical_range = (scroll_size.height - scrollport.height).max(0.0);
            let is_scroll_container =
                is_root || self.world.boxes[index].style.establishes_scroll_container();
            let requested = finite_point(self.world.boxes[index].scroll_offset);
            let (minimum_offset, maximum_offset) = if is_scroll_container {
                if self.world.boxes[index].style.direction() == crate::style::InlineDirection::Rtl {
                    (
                        LayoutPoint::new(-horizontal_range, 0.0),
                        LayoutPoint::new(0.0, vertical_range),
                    )
                } else {
                    (
                        LayoutPoint::ZERO,
                        LayoutPoint::new(horizontal_range, vertical_range),
                    )
                }
            } else {
                (LayoutPoint::ZERO, LayoutPoint::ZERO)
            };
            let applied = if is_scroll_container {
                LayoutPoint::new(
                    requested.x.clamp(minimum_offset.x, maximum_offset.x),
                    requested.y.clamp(minimum_offset.y, maximum_offset.y),
                )
            } else {
                LayoutPoint::ZERO
            };
            self.scroll_extents.push(LayoutScrollExtent {
                id: LayoutScrollExtentId::from_index(index),
                owner: LayoutOutputBoxId::from_index(index),
                scrollport,
                scrollable_overflow: overflow,
                scroll_size,
                requested_offset: requested,
                applied_offset: applied,
                minimum_offset,
                maximum_offset,
                is_scroll_container,
                clips_overflow: self.world.boxes[index].style.clips_overflow(),
            });
        }
        self.viewport_scroll = self.scroll_extents[self.world.root.index()].applied_offset;
    }

    fn build_coordinate_spaces(&mut self) -> Result<(), LayoutError> {
        let mut spaces = vec![None; self.world.boxes.len() + 1];
        spaces[0] = Some(LayoutCoordinateSpace {
            id: LayoutCoordinateSpaceId::from_index(0),
            owner: None,
            parent: None,
            local_to_parent: LayoutTransform2D::IDENTITY,
            local_to_document: LayoutTransform2D::IDENTITY,
            local_to_viewport: LayoutTransform2D::IDENTITY,
        });
        let mut state = vec![0_u8; self.world.boxes.len()];
        for index in 0..self.world.boxes.len() {
            resolve_coordinate_space(
                index,
                self.world,
                self.viewport_scroll,
                &self.scroll_extents,
                &self.resolved_transforms,
                &mut self.viewport_anchored,
                &mut spaces,
                &mut state,
            )?;
        }
        self.coordinate_spaces = spaces
            .into_iter()
            .enumerate()
            .map(|(index, space)| {
                space.ok_or(LayoutError::InvalidBoxReference {
                    index: index.saturating_sub(1),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(())
    }

    fn build_fragments_and_source_mapping(&mut self) {
        for (source, box_id) in &self.world.source_mapping {
            self.source_mapping
                .entry(*source)
                .or_default()
                .principal_box = Some(LayoutOutputBoxId::from_index(box_id.index()));
        }
        for (source, boxes) in &self.world.display_contents_mapping {
            self.source_mapping
                .entry(*source)
                .or_default()
                .scroll_proxy_boxes = boxes
                .iter()
                .map(|id| LayoutOutputBoxId::from_index(id.index()))
                .collect();
        }

        for index in 0..self.world.boxes.len() {
            let layout_box = &self.world.boxes[index];
            let output_box = LayoutOutputBoxId::from_index(index);
            let coordinate_space = LayoutCoordinateSpaceId::from_index(index + 1);
            if !layout_box.inline_flattened {
                let fragment = self.push_fragment(LayoutFragment {
                    id: LayoutFragmentId::from_index(0),
                    kind: LayoutFragmentKind::Box { box_id: output_box },
                    rect: self.boxes[index].border_box,
                    box_model: Some(LayoutFragmentBoxModel {
                        content: self.boxes[index].content_box,
                        padding: self.boxes[index].padding_box,
                        border: self.boxes[index].border_box,
                        margin: self.boxes[index].margin_box,
                    }),
                    baseline: None,
                    coordinate_space,
                    clip_chain: None,
                    paint_order: None,
                });
                self.direct_fragments[index] = Some(fragment);
                self.register_box_fragment(index, fragment);
            }

            let Some(context) = layout_box.inline_layout.as_ref() else {
                continue;
            };
            let layout = layout_box.final_layout;
            let content_origin = LayoutPoint::new(
                layout.border.left + layout.padding.left,
                layout.border.top + layout.padding.top,
            );
            for line in &context.fragments.lines {
                let fragment = self.push_fragment(LayoutFragment {
                    id: LayoutFragmentId::from_index(0),
                    kind: LayoutFragmentKind::Line {
                        owner: output_box,
                        line_index: line.line_index,
                    },
                    rect: offset_rect(line.rect, content_origin),
                    box_model: None,
                    baseline: Some(content_origin.y + line.baseline),
                    coordinate_space,
                    clip_chain: None,
                    paint_order: None,
                });
                self.boxes[index].fragments.push(fragment);
            }
            for inline in &context.fragments.boxes {
                let target = inline.box_id.index();
                let box_model = inline_fragment_box_model(self.world, index, inline);
                let fragment = self.push_fragment(LayoutFragment {
                    id: LayoutFragmentId::from_index(0),
                    kind: LayoutFragmentKind::InlineBox {
                        box_id: LayoutOutputBoxId::from_index(target),
                        line_index: inline.line_index,
                        has_start_edge: inline.has_start_edge,
                        has_end_edge: inline.has_end_edge,
                    },
                    rect: box_model.border,
                    box_model: Some(box_model),
                    baseline: None,
                    coordinate_space,
                    clip_chain: None,
                    paint_order: None,
                });
                self.owner_paint_fragments[index].push(fragment);
                self.register_box_fragment(target, fragment);
            }
            for text in &context.fragments.text {
                let target = text.box_id.index();
                let fragment = self.push_fragment(LayoutFragment {
                    id: LayoutFragmentId::from_index(0),
                    kind: LayoutFragmentKind::Text {
                        box_id: LayoutOutputBoxId::from_index(target),
                        line_index: text.line_index,
                        source_byte_range: text.source_byte_range.clone(),
                        source_utf16_range: text.source_utf16_range.clone(),
                        rtl: text.rtl,
                    },
                    rect: offset_rect(text.rect, content_origin),
                    box_model: None,
                    baseline: None,
                    coordinate_space,
                    clip_chain: None,
                    paint_order: None,
                });
                self.owner_paint_fragments[index].push(fragment);
                self.register_box_fragment(target, fragment);
            }
        }
    }

    fn register_box_fragment(&mut self, box_index: usize, fragment: LayoutFragmentId) {
        self.boxes[box_index].fragments.push(fragment);
        let layout_box = &self.world.boxes[box_index];
        let source = layout_box.source.or_else(|| {
            (layout_box.anonymous_reason == Some(LayoutAnonymousReason::InlineSplitContinuation))
                .then_some(layout_box.owner)
                .flatten()
        });
        if let Some(source) = source {
            self.source_mapping
                .entry(source)
                .or_default()
                .fragments
                .push(fragment);
        }
    }

    fn push_fragment(&mut self, mut fragment: LayoutFragment) -> LayoutFragmentId {
        let id = LayoutFragmentId::from_index(self.fragments.len());
        fragment.id = id;
        self.fragments.push(fragment);
        id
    }

    fn assign_clip_and_paint_order(&mut self) {
        let viewport_space = LayoutCoordinateSpaceId::from_index(0);
        let viewport_clip = self.push_clip(
            None,
            None,
            viewport_space,
            LayoutRect::new(
                0.0,
                0.0,
                self.viewport.css_width as f32,
                self.viewport.css_height as f32,
            ),
        );
        self.assign_box_clip_metadata(self.world.root, Some(viewport_clip), viewport_clip);
        self.paint_events = build_paint_order(self.world);
        for event in self.paint_events.clone() {
            match event {
                PaintOrderEvent::BoxBackground(id) => {
                    let index = id.index();
                    if let Some(fragment) = self.direct_fragments[index] {
                        self.assign_fragment_paint_metadata(fragment, self.background_clips[index]);
                    }
                }
                PaintOrderEvent::BoxContents(id) => {
                    let index = id.index();
                    for fragment in self.owner_paint_fragments[index].clone() {
                        self.assign_fragment_paint_metadata(fragment, self.content_clips[index]);
                    }
                }
                PaintOrderEvent::BoxOutsetShadow(_)
                | PaintOrderEvent::PushStackingContext(_)
                | PaintOrderEvent::TableCollapsedBorders(_)
                | PaintOrderEvent::BoxOutline(_)
                | PaintOrderEvent::PopStackingContext(_) => {}
            }
        }
        self.hit_entries
            .sort_by_key(|entry| std::cmp::Reverse(entry.paint_order));
    }

    fn assign_box_clip_metadata(
        &mut self,
        id: LayoutBoxId,
        ancestor_clip: Option<LayoutClipChainId>,
        viewport_clip: LayoutClipChainId,
    ) {
        let index = id.index();
        // CSS overflow clips follow the containing-block chain, not the box
        // parent chain. In particular, an absolute/fixed descendant skips any
        // overflow clips between itself and its containing block. A
        // viewport-fixed descendant therefore starts from the viewport clip,
        // while a transformed/contained fixed descendant starts from the clip
        // accumulated by that fixed containing block. Blink maintains the
        // same separate in-flow, absolute-position and fixed-position clip
        // contexts in its paint property tree builder.
        let box_clip = if self.world.boxes[index].style.is_absolute_positioned()
            || self.world.boxes[index].style.is_fixed_positioned()
        {
            self.world.boxes[index]
                .positioned_containing_block
                .map_or(Some(viewport_clip), |containing_block| {
                    self.content_clips[containing_block.index()]
                })
        } else {
            ancestor_clip
        };
        self.boxes[index].clip_chain = box_clip;
        self.background_clips[index] = box_clip;

        let child_clip =
            if id != self.world.root && self.world.boxes[index].clips_descendant_paint() {
                Some(self.push_clip(
                    box_clip,
                    Some(LayoutOutputBoxId::from_index(index)),
                    self.boxes[index].coordinate_space,
                    self.boxes[index].padding_box,
                ))
            } else {
                box_clip
            };
        self.content_clips[index] = child_clip;
        for child in self.world.boxes[index].children.clone() {
            self.assign_box_clip_metadata(child, child_clip, viewport_clip);
        }
    }

    fn assign_fragment_paint_metadata(
        &mut self,
        fragment_id: LayoutFragmentId,
        clip_chain: Option<LayoutClipChainId>,
    ) {
        let order = u32::try_from(self.paint_order.len())
            .expect("one layout output exceeded the u32 paint-order limit");
        let (box_index, is_text) = match self.fragments[fragment_id.index()].kind {
            LayoutFragmentKind::Box { box_id } => (box_id.index(), false),
            LayoutFragmentKind::InlineBox { box_id, .. } => (box_id.index(), false),
            LayoutFragmentKind::Text { box_id, .. } => (box_id.index(), true),
            LayoutFragmentKind::Line { .. } => return,
        };
        let fragment = &mut self.fragments[fragment_id.index()];
        fragment.clip_chain = clip_chain;
        fragment.paint_order = Some(order);
        self.paint_order.push(fragment_id);

        let layout_box = &self.world.boxes[box_index];
        if !layout_box.style.is_visible() {
            return;
        }
        let source = layout_box.source.or_else(|| {
            (layout_box.pseudo.is_some()
                || layout_box.anonymous_reason
                    == Some(LayoutAnonymousReason::InlineSplitContinuation))
            .then_some(layout_box.owner)
            .flatten()
        });
        if let Some(source) = source {
            self.hit_entries.push(LayoutHitTestEntry {
                source,
                fragment: fragment_id,
                coordinate_space: fragment.coordinate_space,
                clip_chain,
                local_rect: fragment.rect,
                paint_order: order,
                is_text,
                pointer_events: layout_box.style.accepts_pointer_events(),
            });
        }
    }

    fn push_clip(
        &mut self,
        parent: Option<LayoutClipChainId>,
        owner: Option<LayoutOutputBoxId>,
        coordinate_space: LayoutCoordinateSpaceId,
        rect: LayoutRect,
    ) -> LayoutClipChainId {
        let id = LayoutClipChainId::from_index(self.clip_chain.len());
        self.clip_chain.push(LayoutClipNode {
            id,
            parent,
            owner,
            coordinate_space,
            rect,
        });
        id
    }

    pub(crate) fn document_content_size(&self) -> LayoutSize {
        let root = self.world.root.index();
        let root_layout = self.world.boxes[root].final_layout.location;
        let layout_translation = LayoutTransform2D::translation(root_layout.x, root_layout.y);
        let overflow = layout_translation
            .concatenate(self.resolved_transforms[root])
            .map_rect(self.overflow[root])
            .bounding_rect()
            .union(
                layout_translation
                    .map_rect(self.boxes[root].margin_box)
                    .bounding_rect(),
            );
        LayoutSize::new(
            (self.viewport.css_width as f32).max(overflow.right().max(0.0)),
            (self.viewport.css_height as f32).max(overflow.bottom().max(0.0)),
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_coordinate_space<N>(
    index: usize,
    world: &LayoutWorld<N>,
    viewport_scroll: LayoutPoint,
    scroll_extents: &[LayoutScrollExtent],
    resolved_transforms: &[LayoutTransform2D],
    viewport_anchored: &mut [bool],
    spaces: &mut [Option<LayoutCoordinateSpace>],
    state: &mut [u8],
) -> Result<(), LayoutError>
where
    N: Copy + Debug + Eq + Hash,
{
    let Some(slot_state) = state.get(index).copied() else {
        return Err(LayoutError::InvalidBoxReference { index });
    };
    match slot_state {
        2 => return Ok(()),
        1 => return Err(LayoutError::NumericTreeCycle { index }),
        _ => state[index] = 1,
    }

    let layout_box = world
        .boxes
        .get(index)
        .ok_or(LayoutError::InvalidBoxReference { index })?;
    let parent_box = layout_box.layout_parent;
    if let Some(parent) = parent_box {
        if parent.index() >= world.boxes.len() {
            return Err(LayoutError::InvalidBoxReference {
                index: parent.index(),
            });
        }
        resolve_coordinate_space(
            parent.index(),
            world,
            viewport_scroll,
            scroll_extents,
            resolved_transforms,
            viewport_anchored,
            spaces,
            state,
        )?;
    }

    let parent_space = parent_box.map_or(LayoutCoordinateSpaceId::from_index(0), |parent| {
        LayoutCoordinateSpaceId::from_index(parent.index() + 1)
    });
    let parent_transform = match parent_box {
        Some(parent) => spaces
            .get(parent.index() + 1)
            .and_then(Option::as_ref)
            .map(|space| space.local_to_document)
            .ok_or(LayoutError::InvalidBoxReference {
                index: parent.index(),
            })?,
        None => LayoutTransform2D::IDENTITY,
    };
    let is_viewport_anchored = parent_box.map_or_else(
        || layout_box.style.is_fixed_positioned(),
        |parent| viewport_anchored[parent.index()],
    );
    viewport_anchored[index] = is_viewport_anchored;
    let parent_scroll = parent_box.map_or(LayoutPoint::ZERO, |parent| {
        if parent == world.root {
            LayoutPoint::ZERO
        } else {
            scroll_extents[parent.index()].applied_offset
        }
    });
    let location = layout_box.final_layout.location;
    let local_to_parent =
        LayoutTransform2D::translation(location.x - parent_scroll.x, location.y - parent_scroll.y)
            .concatenate(resolved_transforms[index]);
    let local_to_document = if parent_box.is_none() && is_viewport_anchored {
        LayoutTransform2D::translation(viewport_scroll.x, viewport_scroll.y)
            .concatenate(local_to_parent)
    } else {
        parent_transform.concatenate(local_to_parent)
    };
    let local_to_viewport = LayoutTransform2D::translation(-viewport_scroll.x, -viewport_scroll.y)
        .concatenate(local_to_document);
    spaces[index + 1] = Some(LayoutCoordinateSpace {
        id: LayoutCoordinateSpaceId::from_index(index + 1),
        owner: Some(LayoutOutputBoxId::from_index(index)),
        parent: Some(parent_space),
        local_to_parent,
        local_to_document,
        local_to_viewport,
    });
    state[index] = 2;
    Ok(())
}

fn inset_rect(rect: LayoutRect, top: f32, right: f32, bottom: f32, left: f32) -> LayoutRect {
    let top = top.max(0.0);
    let right = right.max(0.0);
    let bottom = bottom.max(0.0);
    let left = left.max(0.0);
    LayoutRect::new(
        rect.x + left,
        rect.y + top,
        (rect.width - left - right).max(0.0),
        (rect.height - top - bottom).max(0.0),
    )
}

fn outset_rect(rect: LayoutRect, top: f32, right: f32, bottom: f32, left: f32) -> LayoutRect {
    LayoutRect::new(
        rect.x - left,
        rect.y - top,
        (rect.width + left + right).max(0.0),
        (rect.height + top + bottom).max(0.0),
    )
}

fn offset_rect(rect: LayoutRect, offset: LayoutPoint) -> LayoutRect {
    LayoutRect::new(
        rect.x + offset.x,
        rect.y + offset.y,
        rect.width,
        rect.height,
    )
}

fn finite_point(point: LayoutPoint) -> LayoutPoint {
    LayoutPoint::new(
        if point.x.is_finite() { point.x } else { 0.0 },
        if point.y.is_finite() { point.y } else { 0.0 },
    )
}

fn inline_fragment_box_model<N>(
    world: &LayoutWorld<N>,
    owner_index: usize,
    fragment: &crate::inline::InlineBoxFragment,
) -> LayoutFragmentBoxModel
where
    N: Copy + Debug + Eq + Hash,
{
    let owner = &world.boxes[owner_index];
    let owner_layout = owner.final_layout;
    let content_origin = LayoutPoint::new(
        owner_layout.border.left + owner_layout.padding.left,
        owner_layout.border.top + owner_layout.padding.top,
    );
    let containing_width = (owner_layout.size.width
        - owner_layout.border.left
        - owner_layout.border.right
        - owner_layout.padding.left
        - owner_layout.padding.right)
        .max(0.0);
    let inline_box = &world.boxes[fragment.box_id.index()];
    let style = &inline_box.style;
    let padding = style.taffy.padding.resolve_or_zero(
        Some(containing_width),
        crate::style::resolve_stylo_calc_value,
    );
    let border = style.taffy.border.resolve_or_zero(
        Some(containing_width),
        crate::style::resolve_stylo_calc_value,
    );
    let margin = style.taffy.margin.resolve_or_zero(
        Some(containing_width),
        crate::style::resolve_stylo_calc_value,
    );
    let ltr = style.direction() == crate::style::InlineDirection::Ltr;
    let has_left_edge = if ltr {
        fragment.has_start_edge
    } else {
        fragment.has_end_edge
    };
    let has_right_edge = if ltr {
        fragment.has_end_edge
    } else {
        fragment.has_start_edge
    };
    let left_margin = if has_left_edge {
        margin.left.max(0.0)
    } else {
        0.0
    };
    let right_margin = if has_right_edge {
        margin.right.max(0.0)
    } else {
        0.0
    };
    let left_padding = if has_left_edge { padding.left } else { 0.0 };
    let right_padding = if has_right_edge { padding.right } else { 0.0 };
    let left_border = if has_left_edge { border.left } else { 0.0 };
    let right_border = if has_right_edge { border.right } else { 0.0 };
    let border_box = LayoutRect::new(
        content_origin.x + fragment.rect.x + left_margin,
        content_origin.y + fragment.rect.y - padding.top - border.top,
        (fragment.rect.width - left_margin - right_margin).max(0.0),
        fragment.rect.height + padding.top + padding.bottom + border.top + border.bottom,
    );
    let padding_box = inset_rect(
        border_box,
        border.top,
        right_border,
        border.bottom,
        left_border,
    );
    let content_box = inset_rect(
        padding_box,
        padding.top,
        right_padding,
        padding.bottom,
        left_padding,
    );
    let margin_box = outset_rect(
        border_box,
        margin.top,
        right_margin,
        margin.bottom,
        left_margin,
    );
    LayoutFragmentBoxModel {
        content: content_box,
        padding: padding_box,
        border: border_box,
        margin: margin_box,
    }
}
