//! CSSOM box, range, element-metric, and source-provenance projections.

use std::{fmt::Debug, hash::Hash, ops::Range};

use crate::LayoutPosition;

use super::{
    model::{
        LayoutBoxModel, LayoutCoordinateSpaceId, LayoutFragmentBoxModel, LayoutFragmentKind,
        LayoutOutputBoxId, LayoutPoint, LayoutQuad, LayoutRect, LayoutResolvedGridTracks,
        LayoutSize, LayoutTransform2D,
    },
    query::{LayoutElementMetrics, LayoutNodeOutput},
    tree::FrozenLayoutTree,
};

#[derive(Clone, Copy, Debug, PartialEq)]
struct InlineOffsetGeometry {
    layout_origin_in_document: LayoutPoint,
    border_origin_in_viewport_ignoring_css_transforms: LayoutPoint,
    size: LayoutSize,
}

impl<N> FrozenLayoutTree<N>
where
    N: Copy + Debug + Eq + Hash,
{
    /// Derives the source view from canonical box provenance.
    ///
    /// No source hash table survives the pass. The returned IDs are copied
    /// into one short-lived query value.
    pub fn source_output(&self, source: N) -> Option<LayoutNodeOutput> {
        let mut found = false;
        let mut output = LayoutNodeOutput::default();
        for layout_box in &self.boxes {
            if layout_box.principal_source == Some(source) {
                output.principal_box = Some(layout_box.id);
                found = true;
            }
            if layout_box.geometry_source == Some(source) {
                output
                    .fragments
                    .extend(layout_box.fragments.iter().copied().filter(|id| {
                        self.fragment(*id).is_some_and(|fragment| {
                            !matches!(fragment.kind, LayoutFragmentKind::Line { .. })
                        })
                    }));
                found = true;
            }
        }
        for (proxy_source, box_id) in &self.scroll_proxy_links {
            if *proxy_source == source {
                output.scroll_proxy_boxes.push(*box_id);
                found = true;
            }
        }
        found.then_some(output)
    }

    pub fn element_metrics_for_source(&self, source: N) -> Option<LayoutElementMetrics<N>> {
        self.element_metrics_for_source_with_offset_parent_filter(source, |_| true)
    }

    /// Returns the exact unprojected content box produced for a source's
    /// principal layout box.
    ///
    /// Embedded browsing contexts use this as their LocalFrameView size. The
    /// value deliberately comes from used layout geometry rather than
    /// authored width/height CSS, and therefore remains correct for
    /// percentages, `calc()`, box sizing, borders, padding, and transforms.
    pub fn local_content_box_for_source(&self, source: N) -> Option<LayoutRect> {
        let box_id = self.source_output(source)?.principal_box?;
        self.box_geometry(box_id)
            .map(|geometry| geometry.content_box)
    }

    /// Resolves CSSOM View element metrics while allowing the renderer to hide
    /// flat-tree ancestors that do not belong to the queried element's
    /// ancestor tree scopes.
    ///
    /// Shadow DOM tree-scope visibility is an HTML/DOM concern, not a CSS box
    /// tree concern. The frozen tree therefore retains the complete box chain
    /// and lets its short-lived consumer supply that one predicate. Geometry
    /// is still derived wholly from this pass; no live layout state is read or
    /// retained here.
    pub fn element_metrics_for_source_with_offset_parent_filter(
        &self,
        source: N,
        mut offset_parent_is_exposed: impl FnMut(N) -> bool,
    ) -> Option<LayoutElementMetrics<N>> {
        let output = self.source_output(source)?;
        let box_id = output.principal_box?;
        let geometry = self.box_geometry(box_id)?;
        let extent = self.scroll_extent(box_id)?;
        let coordinate_space = self.coordinate_space(geometry.coordinate_space)?;
        let is_root = box_id == self.root_box;
        let offset_parent_id = self.offset_parent_box(box_id, &mut offset_parent_is_exposed);
        let offset_parent = offset_parent_id.and_then(|id| {
            self.boxes
                .get(id.index())
                .and_then(|layout_box| layout_box.geometry_source)
        });
        let offset_parent_origin = offset_parent_id
            .and_then(|id| self.box_geometry(id))
            .map(|parent| {
                if parent.is_body_element {
                    if parent.position == LayoutPosition::Static {
                        LayoutPoint::ZERO
                    } else {
                        parent.layout_origin_in_document
                    }
                } else {
                    LayoutPoint::new(
                        parent.layout_origin_in_document.x + parent.padding_box.x,
                        parent.layout_origin_in_document.y + parent.padding_box.y,
                    )
                }
            })
            .unwrap_or(LayoutPoint::ZERO);
        let inline_offset_geometry = self.inline_offset_geometry(&output, box_id);
        let layout_origin = inline_offset_geometry
            .map(|geometry| geometry.layout_origin_in_document)
            .unwrap_or(geometry.layout_origin_in_document);
        let offset_size = inline_offset_geometry
            .map(|geometry| geometry.size)
            .unwrap_or_else(|| {
                LayoutSize::new(geometry.border_box.width, geometry.border_box.height)
            });
        let border_origin_in_viewport_ignoring_css_transforms = inline_offset_geometry
            .map(|geometry| geometry.border_origin_in_viewport_ignoring_css_transforms)
            .unwrap_or_else(|| {
                coordinate_space
                    .local_to_viewport_ignoring_css_transforms
                    .map_point(LayoutPoint::new(
                        geometry.border_box.x,
                        geometry.border_box.y,
                    ))
            });
        let unzoom = CssomAbsoluteZoom::new(geometry.effective_zoom);
        let client_size = if is_root {
            // CSSOM defines root client dimensions from the viewport and only
            // subtracts an actually-present viewport scrollbar. Stable empty
            // gutters still reduce root layout/scrollWidth, but not
            // documentElement.clientWidth/clientHeight.
            LayoutSize::new(
                (self.viewport.css_width as f32
                    - extent
                        .vertical_scrollbar
                        .map_or(0.0, |scrollbar| scrollbar.frame.width))
                .max(0.0),
                (self.viewport.css_height as f32
                    - extent
                        .horizontal_scrollbar
                        .map_or(0.0, |scrollbar| scrollbar.frame.height))
                .max(0.0),
            )
        } else {
            unzoom.size(LayoutSize::new(
                extent.scrollport.width,
                extent.scrollport.height,
            ))
        };
        Some(LayoutElementMetrics {
            offset_parent,
            offset_position: unzoom.point(LayoutPoint::new(
                layout_origin.x - offset_parent_origin.x,
                layout_origin.y - offset_parent_origin.y,
            )),
            border_origin_in_viewport_ignoring_css_transforms,
            offset_size: unzoom.size(offset_size),
            content_size: unzoom.size(LayoutSize::new(
                geometry.content_box.width,
                geometry.content_box.height,
            )),
            client_size,
            client_border: unzoom.point(LayoutPoint::new(
                if is_root {
                    0.0
                } else {
                    extent.scrollport.x - geometry.border_box.x
                },
                if is_root {
                    0.0
                } else {
                    extent.scrollport.y - geometry.border_box.y
                },
            )),
            scroll_size: unzoom.size(extent.scroll_size),
            scroll_offset: unzoom.point(extent.applied_offset),
            minimum_scroll_offset: unzoom.point(extent.minimum_offset),
            maximum_scroll_offset: unzoom.point(extent.maximum_offset),
            scrollport: if is_root {
                LayoutTransform2D::IDENTITY.map_rect(extent.scrollport)
            } else {
                coordinate_space
                    .local_to_viewport
                    .map_rect(extent.scrollport)
            },
            scrollable_overflow: coordinate_space
                .local_to_viewport
                .map_rect(extent.scrollable_overflow),
            is_scroll_container: extent.is_scroll_container,
            allows_user_scroll_x: extent.allows_user_scroll_x,
            allows_user_scroll_y: extent.allows_user_scroll_y,
            clips_overflow: extent.clips_overflow,
            visible: geometry.visible,
            pointer_events: geometry.pointer_events,
        })
    }

    /// Returns used Grid tracks from the same frozen epoch as other CSSOM
    /// geometry, normalized out of the container's effective CSS zoom.
    pub fn used_grid_tracks_for_source(&self, source: N) -> Option<LayoutResolvedGridTracks> {
        let output = self.source_output(source)?;
        let layout_box = self.boxes.get(output.principal_box?.index())?;
        let mut grid = layout_box.resolved_grid_tracks.clone()?;
        let unzoom = CssomAbsoluteZoom::new(layout_box.geometry.effective_zoom);
        for tracks in [&mut grid.rows, &mut grid.columns] {
            for size in &mut tracks.used_track_sizes {
                *size = unzoom.scalar(*size);
            }
        }
        Some(grid)
    }

    /// Resolves a viewport point into the coordinate system Blink uses for
    /// `MouseEvent.offsetX/Y`: a box target's padding edge, or the shared IFC
    /// coordinate space for a flattened inline layout object.
    pub fn event_offset_for_source(
        &self,
        source: N,
        viewport_point: LayoutPoint,
    ) -> Option<LayoutPoint> {
        let output = self.source_output(source)?;
        let box_id = output.principal_box?;
        if let Some(inline_fragment) = output.fragments.iter().find_map(|id| {
            let fragment = self.fragment(*id)?;
            matches!(
                fragment.kind,
                LayoutFragmentKind::InlineBox {
                    box_id: fragment_box,
                    ..
                } if fragment_box == box_id
            )
            .then_some(fragment)
        }) {
            let inverse = self
                .coordinate_space(inline_fragment.coordinate_space)?
                .local_to_viewport
                .inverse()?;
            return Some(inverse.map_point(viewport_point));
        }

        let geometry = self.box_geometry(box_id)?;
        let inverse = self
            .coordinate_space(geometry.coordinate_space)?
            .local_to_viewport
            .inverse()?;
        let mut local = inverse.map_point(viewport_point);
        local.x -= geometry.padding_box.x - geometry.border_box.x;
        local.y -= geometry.padding_box.y - geometry.border_box.y;
        Some(local)
    }

    pub fn box_model_for_source(&self, source: N) -> Option<LayoutBoxModel> {
        let output = self.source_output(source)?;
        let fragment_models = output
            .fragments
            .iter()
            .filter_map(|id| self.fragment(*id))
            .filter_map(|fragment| {
                fragment
                    .box_model
                    .map(|model| (fragment.coordinate_space, model))
            })
            .collect::<Vec<_>>();
        if !fragment_models.is_empty() {
            return self.project_fragment_box_models(&fragment_models);
        }

        let box_id = output.principal_box?;
        let geometry = self.box_geometry(box_id)?;
        self.project_local_box_model(
            geometry.coordinate_space,
            LayoutFragmentBoxModel {
                content: geometry.content_box,
                padding: geometry.padding_box,
                border: geometry.border_box,
                margin: geometry.margin_box,
            },
        )
    }

    pub fn client_rects_for_source(&self, source: N) -> Vec<LayoutQuad> {
        let Some(output) = self.source_output(source) else {
            return Vec::new();
        };
        output
            .fragments
            .iter()
            .filter_map(|id| self.fragment(*id))
            .filter(|fragment| {
                matches!(
                    fragment.kind,
                    LayoutFragmentKind::Box { .. } | LayoutFragmentKind::InlineBox { .. }
                )
            })
            .filter_map(|fragment| {
                self.coordinate_space(fragment.coordinate_space)
                    .map(|space| space.local_to_viewport.map_rect(fragment.rect))
            })
            .collect()
    }

    pub fn content_quads_for_source(&self, source: N) -> Vec<LayoutQuad> {
        let Some(output) = self.source_output(source) else {
            return Vec::new();
        };
        output
            .fragments
            .iter()
            .filter_map(|id| self.fragment(*id))
            .filter(|fragment| {
                matches!(
                    fragment.kind,
                    LayoutFragmentKind::Box { .. }
                        | LayoutFragmentKind::InlineBox { .. }
                        | LayoutFragmentKind::Text { .. }
                )
            })
            .filter_map(|fragment| {
                let rect = fragment
                    .box_model
                    .map(|model| model.content)
                    .unwrap_or(fragment.rect);
                self.coordinate_space(fragment.coordinate_space)
                    .map(|space| space.local_to_viewport.map_rect(rect))
            })
            .collect()
    }

    pub fn text_range_rects(&self, source: N, utf16_range: Range<usize>) -> Vec<LayoutQuad> {
        let Some(output) = self.source_output(source) else {
            return Vec::new();
        };
        #[derive(Clone, Copy)]
        struct SelectedTextRect {
            box_id: LayoutOutputBoxId,
            line_index: usize,
            rtl: bool,
            coordinate_space: LayoutCoordinateSpaceId,
            rect: LayoutRect,
        }

        let mut selected = output
            .fragments
            .iter()
            .filter_map(|id| self.fragment(*id))
            .filter_map(|fragment| {
                let LayoutFragmentKind::Text {
                    box_id,
                    line_index,
                    source_utf16_range,
                    rtl,
                    ..
                } = &fragment.kind
                else {
                    return None;
                };
                let source_len = source_utf16_range
                    .end
                    .saturating_sub(source_utf16_range.start);
                let (selected_start, selected_end) = if utf16_range.is_empty() {
                    if utf16_range.start < source_utf16_range.start
                        || utf16_range.start > source_utf16_range.end
                    {
                        return None;
                    }
                    let point = utf16_range
                        .start
                        .saturating_sub(source_utf16_range.start)
                        .min(source_len);
                    (point, point)
                } else {
                    let start = source_utf16_range.start.max(utf16_range.start);
                    let end = source_utf16_range.end.min(utf16_range.end);
                    if start >= end {
                        return None;
                    }
                    (
                        start.saturating_sub(source_utf16_range.start),
                        end.saturating_sub(source_utf16_range.start),
                    )
                };
                let denominator = source_len.max(1) as f32;
                let start_ratio = selected_start as f32 / denominator;
                let end_ratio = selected_end as f32 / denominator;
                let visual_start_ratio = if *rtl { 1.0 - end_ratio } else { start_ratio };
                let rect = LayoutRect::new(
                    fragment.rect.x + fragment.rect.width * visual_start_ratio,
                    fragment.rect.y,
                    fragment.rect.width * (end_ratio - start_ratio),
                    fragment.rect.height,
                );
                Some(SelectedTextRect {
                    box_id: *box_id,
                    line_index: *line_index,
                    rtl: *rtl,
                    coordinate_space: fragment.coordinate_space,
                    rect,
                })
            })
            .collect::<Vec<_>>();

        // Parley exposes cluster-level source fragments, while CSSOM View
        // exposes one Range rect per contiguous directional run on a line.
        // Keep opposite bidi runs separate, but merge adjacent clusters from
        // the same source box before mapping through transforms.
        selected.sort_by(|left, right| {
            left.coordinate_space
                .index()
                .cmp(&right.coordinate_space.index())
                .then_with(|| left.box_id.index().cmp(&right.box_id.index()))
                .then_with(|| left.line_index.cmp(&right.line_index))
                .then_with(|| left.rtl.cmp(&right.rtl))
                .then_with(|| left.rect.y.total_cmp(&right.rect.y))
                .then_with(|| left.rect.x.total_cmp(&right.rect.x))
        });
        let mut merged: Vec<SelectedTextRect> = Vec::with_capacity(selected.len());
        for fragment in selected {
            let can_merge = merged.last().is_some_and(|previous| {
                let tolerance = previous
                    .rect
                    .width
                    .abs()
                    .max(fragment.rect.width.abs())
                    .max(1.0)
                    * f32::EPSILON
                    * 16.0;
                previous.box_id == fragment.box_id
                    && previous.line_index == fragment.line_index
                    && previous.rtl == fragment.rtl
                    && previous.coordinate_space == fragment.coordinate_space
                    && (previous.rect.y - fragment.rect.y).abs() <= tolerance
                    && (previous.rect.height - fragment.rect.height).abs() <= tolerance
                    && fragment.rect.x <= previous.rect.right() + tolerance
            });
            if can_merge {
                let previous = merged.last_mut().expect("checked above");
                previous.rect = previous.rect.union(fragment.rect);
            } else {
                merged.push(fragment);
            }
        }

        let mut quads = merged
            .into_iter()
            .filter_map(|fragment| {
                self.coordinate_space(fragment.coordinate_space)
                    .map(|space| space.local_to_viewport.map_rect(fragment.rect))
            })
            .collect::<Vec<_>>();
        quads.sort_by(|left, right| {
            let left = left.bounding_rect();
            let right = right.bounding_rect();
            left.y
                .total_cmp(&right.y)
                .then_with(|| left.x.total_cmp(&right.x))
        });
        quads
    }
}

/// Converts effective-zoomed layout scalars to the coordinate space exposed
/// by CSSOM integer box and scroll metrics. Viewport quads intentionally stay
/// zoomed: their normalized bases map points back into these unzoomed sizes.
#[derive(Clone, Copy)]
struct CssomAbsoluteZoom(f32);

impl CssomAbsoluteZoom {
    fn new(effective_zoom: f32) -> Self {
        debug_assert!(effective_zoom.is_finite() && effective_zoom > 0.0);
        Self(if effective_zoom.is_finite() && effective_zoom > 0.0 {
            effective_zoom
        } else {
            1.0
        })
    }

    fn point(self, point: LayoutPoint) -> LayoutPoint {
        LayoutPoint::new(point.x / self.0, point.y / self.0)
    }

    fn scalar(self, value: f32) -> f32 {
        value / self.0
    }

    fn size(self, size: LayoutSize) -> LayoutSize {
        LayoutSize::new(size.width / self.0, size.height / self.0)
    }
}

impl<N> FrozenLayoutTree<N>
where
    N: Copy + Debug + Eq + Hash,
{
    /// Returns the untransformed CSSOM offset geometry for a flattened inline.
    ///
    /// CSSOM View defines `offsetLeft`/`offsetTop` from the first fragment and
    /// `offsetWidth`/`offsetHeight` from the bounding box of all non-empty
    /// border-box fragments. Mapping each fragment through its IFC owner's
    /// document-layout origin keeps that geometry in one physical coordinate
    /// system while intentionally excluding transforms and scrolling.
    fn inline_offset_geometry(
        &self,
        output: &LayoutNodeOutput,
        box_id: LayoutOutputBoxId,
    ) -> Option<InlineOffsetGeometry> {
        let mut first_fragment = None;
        let mut bounds = None::<LayoutRect>;

        for id in &output.fragments {
            let Some(fragment) = self.fragment(*id) else {
                continue;
            };
            let LayoutFragmentKind::InlineBox {
                box_id: fragment_box,
                ..
            } = fragment.kind
            else {
                continue;
            };
            if fragment_box != box_id {
                continue;
            }
            let Some(box_model) = fragment.box_model else {
                continue;
            };
            let Some(fragment_space) = self.coordinate_space(fragment.coordinate_space) else {
                continue;
            };
            let Some(owner) = fragment_space.owner else {
                continue;
            };
            let Some(owner_geometry) = self.box_geometry(owner) else {
                continue;
            };
            let border = box_model.border;
            let owner_origin = owner_geometry.layout_origin_in_document;
            let local_origin = LayoutPoint::new(border.x, border.y);
            let document_border = LayoutRect::new(
                owner_origin.x + border.x,
                owner_origin.y + border.y,
                border.width,
                border.height,
            );
            first_fragment.get_or_insert_with(|| {
                (
                    LayoutPoint::new(document_border.x, document_border.y),
                    fragment_space
                        .local_to_viewport_ignoring_css_transforms
                        .map_point(local_origin),
                )
            });

            // Blink's BoundingBoxRelativeToFirstFragment uses UniteIfNonZero,
            // which discards only a 0-by-0 fragment. A one-dimensional
            // fragment still contributes its non-zero axis to offset geometry.
            if document_border.width == 0.0 && document_border.height == 0.0 {
                continue;
            }
            bounds = Some(bounds.map_or(document_border, |current| current.union(document_border)));
        }

        let (layout_origin_in_document, border_origin_in_viewport_ignoring_css_transforms) =
            first_fragment?;
        let size = bounds.map_or(LayoutSize::ZERO, |rect| {
            LayoutSize::new(rect.width, rect.height)
        });
        Some(InlineOffsetGeometry {
            layout_origin_in_document,
            border_origin_in_viewport_ignoring_css_transforms,
            size,
        })
    }

    fn project_fragment_box_models(
        &self,
        models: &[(LayoutCoordinateSpaceId, LayoutFragmentBoxModel)],
    ) -> Option<LayoutBoxModel> {
        let first_space = models.first()?.0;
        if models.iter().all(|(space, _)| *space == first_space) {
            let mut combined = models.first()?.1;
            for (_, model) in &models[1..] {
                combined.content = combined.content.union(model.content);
                combined.padding = combined.padding.union(model.padding);
                combined.border = combined.border.union(model.border);
                combined.margin = combined.margin.union(model.margin);
            }
            return self.project_local_box_model(first_space, combined);
        }

        let mut projected = models
            .iter()
            .filter_map(|(space, model)| self.project_local_box_model(*space, *model));
        let first = projected.next()?;
        let combined = projected.fold(first, |mut combined, model| {
            combined.content = axis_aligned_union(combined.content, model.content);
            combined.padding = axis_aligned_union(combined.padding, model.padding);
            combined.border = axis_aligned_union(combined.border, model.border);
            combined.margin = axis_aligned_union(combined.margin, model.margin);
            combined
        });
        Some(combined)
    }

    fn offset_parent_box(
        &self,
        box_id: LayoutOutputBoxId,
        offset_parent_is_exposed: &mut impl FnMut(N) -> bool,
    ) -> Option<LayoutOutputBoxId> {
        let geometry = self.box_geometry(box_id)?;
        if box_id == self.root_box || geometry.is_body_element {
            return None;
        }
        let base_is_positioned = geometry.position != LayoutPosition::Static;
        let base_effective_zoom = geometry.effective_zoom;
        let mut in_fixed_position_chain = geometry.position == LayoutPosition::Fixed;
        let mut candidate = geometry.structural_parent;
        while let Some(id) = candidate {
            let parent = self.box_geometry(id)?;
            let source = self
                .boxes
                .get(id.index())
                .and_then(|layout_box| layout_box.geometry_source);
            let Some(source) = source else {
                candidate = parent.structural_parent;
                continue;
            };

            if !offset_parent_is_exposed(source) {
                if parent.establishes_fixed_containing_block {
                    in_fixed_position_chain = false;
                } else if parent.position == LayoutPosition::Fixed {
                    in_fixed_position_chain = true;
                }
                candidate = parent.structural_parent;
                continue;
            }

            if in_fixed_position_chain {
                if parent.establishes_fixed_containing_block {
                    return Some(id);
                }
            } else if parent.establishes_positioned_containing_block
                || parent.is_body_element
                || (!base_is_positioned && parent.is_table_offset_parent)
            {
                return Some(id);
            }
            // CSSOM View preserves WebKit/Blink's long-standing extension:
            // offsetParent stops at the first exposed layout ancestor whose
            // absolute zoom differs from the target. Resolve ordinary
            // containing-block candidates first, exactly as Blink does, then
            // admit this geometry-coordinate boundary.
            if base_effective_zoom != parent.effective_zoom {
                return Some(id);
            }
            in_fixed_position_chain |= parent.position == LayoutPosition::Fixed;
            candidate = parent.structural_parent;
        }
        None
    }

    fn project_local_box_model(
        &self,
        coordinate_space: LayoutCoordinateSpaceId,
        model: LayoutFragmentBoxModel,
    ) -> Option<LayoutBoxModel> {
        let transform = self.coordinate_space(coordinate_space)?.local_to_viewport;
        Some(LayoutBoxModel {
            content: transform.map_rect(model.content),
            padding: transform.map_rect(model.padding),
            border: transform.map_rect(model.border),
            margin: transform.map_rect(model.margin),
        })
    }
}

fn axis_aligned_union(left: LayoutQuad, right: LayoutQuad) -> LayoutQuad {
    LayoutTransform2D::IDENTITY.map_rect(left.bounding_rect().union(right.bounding_rect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_tree::{
        FrozenCoordinateSpace, FrozenLayoutBox, LayoutBoxGeometry, LayoutFragment,
        LayoutFragmentId, LayoutScrollExtent, LayoutViewport,
    };

    fn identity_space(owner: Option<LayoutOutputBoxId>) -> FrozenCoordinateSpace {
        FrozenCoordinateSpace {
            owner,
            local_to_viewport: LayoutTransform2D::IDENTITY,
            local_to_viewport_ignoring_css_transforms: LayoutTransform2D::IDENTITY,
        }
    }

    #[test]
    fn inline_offset_geometry_skips_unprojectable_fragments_and_keeps_one_dimensional_bounds() {
        let box_id = LayoutOutputBoxId::from_index(0);
        let coordinate_space = LayoutCoordinateSpaceId::from_index(1);
        let skipped_fragment = LayoutFragmentId::from_index(0);
        let valid_fragment = LayoutFragmentId::from_index(1);
        let fragment = |id, box_model| LayoutFragment {
            id,
            kind: LayoutFragmentKind::InlineBox {
                box_id,
                line_index: 0,
                has_start_edge: true,
                has_end_edge: true,
            },
            rect: LayoutRect::ZERO,
            box_model,
            coordinate_space,
            clip_chain: None,
            paint_order: None,
        };
        let valid_box_model = LayoutFragmentBoxModel {
            content: LayoutRect::new(3.0, 4.0, 0.0, 5.0),
            padding: LayoutRect::new(3.0, 4.0, 0.0, 5.0),
            border: LayoutRect::new(3.0, 4.0, 0.0, 5.0),
            margin: LayoutRect::new(3.0, 4.0, 0.0, 5.0),
        };
        let scroll_extent = LayoutScrollExtent {
            scrollport: LayoutRect::ZERO,
            scrollable_overflow: LayoutRect::ZERO,
            scroll_size: LayoutSize::ZERO,
            applied_offset: LayoutPoint::ZERO,
            minimum_offset: LayoutPoint::ZERO,
            maximum_offset: LayoutPoint::ZERO,
            is_scroll_container: false,
            allows_user_scroll_x: false,
            allows_user_scroll_y: false,
            clips_overflow: false,
            horizontal_scrollbar: None,
            vertical_scrollbar: None,
            scrollbar_corner: None,
            scrollbar_colors: None,
        };
        let tree = FrozenLayoutTree::new(
            0_u8,
            LayoutViewport::new(100, 100, 1.0),
            LayoutPoint::ZERO,
            LayoutSize::new(100.0, 100.0),
            box_id,
            vec![FrozenLayoutBox {
                geometry: LayoutBoxGeometry {
                    id: box_id,
                    effective_zoom: 1.0,
                    structural_parent: None,
                    parent: None,
                    layout_parent: None,
                    position: LayoutPosition::Static,
                    coordinate_space,
                    clip_chain: None,
                    content_box: LayoutRect::ZERO,
                    padding_box: LayoutRect::ZERO,
                    border_box: LayoutRect::ZERO,
                    margin_box: LayoutRect::ZERO,
                    fragments: vec![skipped_fragment, valid_fragment],
                    layout_origin_in_document: LayoutPoint::new(100.0, 200.0),
                    is_body_element: false,
                    is_table_offset_parent: false,
                    establishes_positioned_containing_block: false,
                    establishes_fixed_containing_block: false,
                    visible: true,
                    pointer_events: true,
                },
                scroll_extent,
                coordinate_space: identity_space(Some(box_id)),
                geometry_source: Some(1),
                principal_source: Some(1),
                hit_source: Some(1),
                resolved_grid_tracks: None,
                control_paint_order: None,
            }],
            vec![
                fragment(skipped_fragment, None),
                fragment(valid_fragment, Some(valid_box_model)),
            ],
            Vec::new(),
            identity_space(None),
            Vec::new(),
            Vec::new(),
            true,
        );
        let output = LayoutNodeOutput {
            principal_box: Some(box_id),
            fragments: vec![skipped_fragment, valid_fragment],
            scroll_proxy_boxes: Vec::new(),
        };

        let geometry = tree
            .inline_offset_geometry(&output, box_id)
            .expect("the valid fragment after the skipped entry must contribute geometry");

        assert_eq!(
            geometry.layout_origin_in_document,
            LayoutPoint::new(103.0, 204.0)
        );
        assert_eq!(
            geometry.border_origin_in_viewport_ignoring_css_transforms,
            LayoutPoint::new(3.0, 4.0)
        );
        assert_eq!(geometry.size, LayoutSize::new(0.0, 5.0));
    }
}
