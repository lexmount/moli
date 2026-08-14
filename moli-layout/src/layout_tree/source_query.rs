//! CSSOM box, range, element-metric, and source-provenance projections.

use std::{fmt::Debug, hash::Hash, ops::Range};

use crate::LayoutPosition;

use super::{
    model::{
        LayoutBoxModel, LayoutCoordinateSpaceId, LayoutFragmentBoxModel, LayoutFragmentKind,
        LayoutOutputBoxId, LayoutPoint, LayoutQuad, LayoutRect, LayoutSize, LayoutTransform2D,
    },
    query::{LayoutElementMetrics, LayoutNodeOutput},
    tree::FrozenLayoutTree,
};

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
        let layout_origin = output
            .fragments
            .iter()
            .find_map(|id| {
                let fragment = self.fragment(*id)?;
                let LayoutFragmentKind::InlineBox {
                    box_id: fragment_box,
                    ..
                } = fragment.kind
                else {
                    return None;
                };
                if fragment_box != box_id {
                    return None;
                }
                let border = fragment.box_model?.border;
                let owner = self.coordinate_space(fragment.coordinate_space)?.owner?;
                let owner_origin = self.box_geometry(owner)?.layout_origin_in_document;
                Some(LayoutPoint::new(
                    owner_origin.x + border.x,
                    owner_origin.y + border.y,
                ))
            })
            .unwrap_or(geometry.layout_origin_in_document);
        let client_size = if is_root {
            LayoutSize::new(
                self.viewport.css_width as f32,
                self.viewport.css_height as f32,
            )
        } else {
            LayoutSize::new(geometry.padding_box.width, geometry.padding_box.height)
        };
        let scroll_size = if is_root {
            LayoutSize::new(
                extent.scroll_size.width.max(self.content_size.width),
                extent.scroll_size.height.max(self.content_size.height),
            )
        } else {
            extent.scroll_size
        };
        Some(LayoutElementMetrics {
            offset_parent,
            offset_position: LayoutPoint::new(
                layout_origin.x - offset_parent_origin.x,
                layout_origin.y - offset_parent_origin.y,
            ),
            offset_size: LayoutSize::new(geometry.border_box.width, geometry.border_box.height),
            content_size: LayoutSize::new(geometry.content_box.width, geometry.content_box.height),
            client_size,
            client_border: LayoutPoint::new(
                geometry.padding_box.x - geometry.border_box.x,
                geometry.padding_box.y - geometry.border_box.y,
            ),
            scroll_size,
            scroll_offset: extent.applied_offset,
            minimum_scroll_offset: extent.minimum_offset,
            maximum_scroll_offset: extent.maximum_offset,
            scrollport: coordinate_space
                .local_to_viewport
                .map_rect(extent.scrollport),
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

impl<N> FrozenLayoutTree<N>
where
    N: Copy + Debug + Eq + Hash,
{
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
        let mut in_fixed_position_chain = geometry.position == LayoutPosition::Fixed;
        let mut candidate = geometry.parent;
        while let Some(id) = candidate {
            let parent = self.box_geometry(id)?;
            let source = self
                .boxes
                .get(id.index())
                .and_then(|layout_box| layout_box.geometry_source);
            let Some(source) = source else {
                candidate = parent.parent;
                continue;
            };

            if !offset_parent_is_exposed(source) {
                if parent.establishes_fixed_containing_block {
                    in_fixed_position_chain = false;
                } else if parent.position == LayoutPosition::Fixed {
                    in_fixed_position_chain = true;
                }
                candidate = parent.parent;
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
            in_fixed_position_chain |= parent.position == LayoutPosition::Fixed;
            candidate = parent.parent;
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
