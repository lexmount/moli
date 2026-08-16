//! CSSOM box, range, element-metric, and source-provenance projections.

use std::{fmt::Debug, hash::Hash, ops::Range};

use crate::LayoutPosition;

use super::{
    model::{
        LayoutBoxModel, LayoutCoordinateSpaceId, LayoutFragmentBoxModel, LayoutFragmentKind,
        LayoutOutputBoxId, LayoutPhysicalAxis, LayoutPoint, LayoutQuad, LayoutRect, LayoutSize,
        LayoutTransform2D,
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
            .map(|geometry| geometry.0)
            .unwrap_or(geometry.layout_origin_in_document);
        let offset_size = inline_offset_geometry
            .map(|geometry| geometry.1)
            .unwrap_or_else(|| {
                LayoutSize::new(geometry.border_box.width, geometry.border_box.height)
            });
        let client_size = if geometry.uses_viewport_client_metrics {
            LayoutSize::new(
                self.viewport.css_width as f32,
                self.viewport.css_height as f32,
            )
        } else {
            LayoutSize::new(geometry.padding_box.width, geometry.padding_box.height)
        };
        let scroll_size = if box_id == self.root_box {
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
            offset_size,
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
                    LayoutFragmentKind::Box { .. }
                        | LayoutFragmentKind::InlineBox { .. }
                        | LayoutFragmentKind::LineBreak { .. }
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
                        | LayoutFragmentKind::LineBreak { .. }
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
            inline_axis: LayoutPhysicalAxis,
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
                    source_span,
                    is_forced_line_break,
                    inline_axis,
                    rtl,
                    ..
                } = &fragment.kind
                else {
                    return None;
                };
                // Blink skips a forced-break FragmentItem for collapsed
                // ranges. Adjacent text then supplies the upstream or
                // downstream caret quad instead of exposing both lines.
                if utf16_range.is_empty() && *is_forced_line_break {
                    return None;
                }
                let (selected_start, selected_end) = source_span.selected_edges(&utf16_range)?;
                let start_fraction = selected_start.visual_fraction(*rtl);
                let end_fraction = selected_end.visual_fraction(*rtl);
                let visual_start_fraction = start_fraction.min(end_fraction);
                let selected_fraction = (end_fraction - start_fraction).abs();
                let rect = match inline_axis {
                    LayoutPhysicalAxis::Horizontal => LayoutRect::new(
                        fragment.rect.x + fragment.rect.width * visual_start_fraction,
                        fragment.rect.y,
                        fragment.rect.width * selected_fraction,
                        fragment.rect.height,
                    ),
                    LayoutPhysicalAxis::Vertical => LayoutRect::new(
                        fragment.rect.x,
                        fragment.rect.y + fragment.rect.height * visual_start_fraction,
                        fragment.rect.width,
                        fragment.rect.height * selected_fraction,
                    ),
                };
                Some(SelectedTextRect {
                    box_id: *box_id,
                    line_index: *line_index,
                    rtl: *rtl,
                    inline_axis: *inline_axis,
                    coordinate_space: fragment.coordinate_space,
                    rect,
                })
            })
            .collect::<Vec<_>>();

        // Parley exposes cluster-level source fragments, including separate
        // font-fallback clusters, while Blink exposes one FragmentItem rect
        // per contiguous directional text fragment on a line. Group in
        // physical inline order and union the cross-axis font bounds. Requiring
        // equal ascent/descent here would leak fallback-run boundaries as
        // extra DOMRects and make a Range over one Text node non-contiguous.
        selected.sort_by(|left, right| {
            left.coordinate_space
                .index()
                .cmp(&right.coordinate_space.index())
                .then_with(|| left.box_id.index().cmp(&right.box_id.index()))
                .then_with(|| left.line_index.cmp(&right.line_index))
                .then_with(|| left.rtl.cmp(&right.rtl))
                .then_with(|| match left.inline_axis {
                    LayoutPhysicalAxis::Horizontal => left
                        .rect
                        .x
                        .total_cmp(&right.rect.x)
                        .then_with(|| left.rect.y.total_cmp(&right.rect.y)),
                    LayoutPhysicalAxis::Vertical => left
                        .rect
                        .y
                        .total_cmp(&right.rect.y)
                        .then_with(|| left.rect.x.total_cmp(&right.rect.x)),
                })
        });
        let mut merged: Vec<SelectedTextRect> = Vec::with_capacity(selected.len());
        for fragment in selected {
            let can_merge = merged.last().is_some_and(|previous| {
                let (previous_inline_size, fragment_inline_size) = match fragment.inline_axis {
                    LayoutPhysicalAxis::Horizontal => (previous.rect.width, fragment.rect.width),
                    LayoutPhysicalAxis::Vertical => (previous.rect.height, fragment.rect.height),
                };
                let tolerance = previous_inline_size
                    .abs()
                    .max(fragment_inline_size.abs())
                    .max(1.0)
                    * f32::EPSILON
                    * 16.0;
                previous.box_id == fragment.box_id
                    && previous.line_index == fragment.line_index
                    && previous.rtl == fragment.rtl
                    && previous.inline_axis == fragment.inline_axis
                    && previous.coordinate_space == fragment.coordinate_space
                    && (match fragment.inline_axis {
                        LayoutPhysicalAxis::Horizontal => {
                            fragment.rect.x <= previous.rect.right() + tolerance
                        }
                        LayoutPhysicalAxis::Vertical => {
                            fragment.rect.y <= previous.rect.bottom() + tolerance
                        }
                    })
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
    ) -> Option<(LayoutPoint, LayoutSize)> {
        let mut first_origin = None;
        let mut bounds = None::<LayoutRect>;

        for id in &output.fragments {
            let fragment = self.fragment(*id)?;
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
            let mut border = fragment.box_model?.border;
            let owner = self.coordinate_space(fragment.coordinate_space)?.owner?;
            let owner_origin = self.box_geometry(owner)?.layout_origin_in_document;
            border.x += owner_origin.x;
            border.y += owner_origin.y;
            first_origin.get_or_insert(LayoutPoint::new(border.x, border.y));

            // Blink's BoundingBoxRelativeToFirstFragment uses UniteIfNonZero:
            // an empty fragment supplies the offset anchor but cannot stretch
            // the size union across lines by its zero-area position alone.
            if border.width <= 0.0 || border.height <= 0.0 {
                continue;
            }
            bounds = Some(bounds.map_or(border, |current| current.union(border)));
        }

        let origin = first_origin?;
        let size = bounds.map_or(LayoutSize::ZERO, |rect| {
            LayoutSize::new(rect.width, rect.height)
        });
        Some((origin, size))
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
