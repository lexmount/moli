// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use accesskit::{Node, NodeId, Role, TextAlign, TextDirection, TreeUpdate};
use hashbrown::{HashMap, HashSet};
use skrifa::{
    FontRef,
    raw::{TableProvider, types::NameId},
};

use crate::style::Brush;
use crate::{Alignment, ClusterPath, FontStyle, Layout, LineMetrics, Run, Style};

fn link_spans(prev_id: NodeId, prev: &mut Node, next_id: NodeId, next: &mut Node) {
    prev.set_next_on_line(next_id);
    next.set_previous_on_line(prev_id);
}

fn finish_span(
    node: &mut Node,
    x_offset: f64,
    y_offset: f64,
    metrics: &LineMetrics,
    run_offset: f32,
    span_offset: f32,
    span_advance: f32,
    span_text: String,
    character_lengths: Vec<u8>,
    character_positions: Vec<f32>,
    character_widths: Vec<f32>,
    word_starts: Vec<u8>,
) {
    node.set_bounds(accesskit::Rect {
        x0: x_offset + (run_offset + span_offset) as f64,
        y0: y_offset + metrics.block_min_coord as f64,
        x1: x_offset + (run_offset + span_offset + span_advance) as f64,
        y1: y_offset + metrics.block_max_coord as f64,
    });
    node.set_value(span_text);
    node.set_character_lengths(character_lengths);
    node.set_character_positions(character_positions);
    node.set_character_widths(character_widths);
    node.set_word_starts(word_starts);
}

fn add_span(update: &mut TreeUpdate, parent_node: &mut Node, id: NodeId, node: Node) {
    update.nodes.push((id, node));
    parent_node.push_child(id);
}

#[derive(Clone, Default, Debug)]
pub struct LayoutAccessibility {
    // We define a span as a sequence of clusters, in logical order, that all
    // have an identical style. For each span we create an AccessKit node
    // with the `TextRun` role, and these nodes are in logical order.
    // The following two fields maintain a two-way mapping between spans
    // and AccessKit node IDs, where each span is identified by the path to
    // its first cluster, or a span path for short. These maps are maintained by
    // `LayoutAccess::build_nodes`, which ensures that removed spans are removed
    // from the maps on the next accessibility pass.
    pub(crate) access_ids_by_span_path: HashMap<ClusterPath, NodeId>,
    pub(crate) span_paths_by_access_id: HashMap<NodeId, ClusterPath>,
    // Map from cluster path to span path. This allows `Cursor::to_access_position`
    // to complete in O(1), rather than worst-case O(n) where n is the length
    // of the run. It also means that the logic for when to start a new span,
    // including the limitation on the number of characters per span,
    // only needs to live in `LayoutAccess::build_nodes`.
    pub(crate) span_paths_by_cluster_path: HashMap<ClusterPath, ClusterPath>,
}

impl LayoutAccessibility {
    fn span_id_and_node<B: Brush>(
        &mut self,
        next_node_id: &mut impl FnMut() -> NodeId,
        ids: &mut HashSet<NodeId>,
        run: &Run<'_, B>,
        span_path: ClusterPath,
    ) -> (NodeId, Node) {
        // If we encountered this same span path in the previous
        // accessibility pass, reuse the same AccessKit ID. Otherwise,
        // allocate a new one. This enables stable node IDs when merely
        // updating the content of existing spans.
        let id = self
            .access_ids_by_span_path
            .get(&span_path)
            .copied()
            .unwrap_or_else(|| {
                let id = (*next_node_id)();
                self.access_ids_by_span_path.insert(span_path, id);
                self.span_paths_by_access_id.insert(id, span_path);
                id
            });
        ids.insert(id);
        let mut node = Node::new(Role::TextRun);
        node.set_text_direction(if run.is_rtl() {
            TextDirection::RightToLeft
        } else {
            TextDirection::LeftToRight
        });

        let font = run.font();
        if let Ok(font_ref) = FontRef::from_index(font.data.as_ref(), font.index) {
            if let Ok(name) = font_ref.name() {
                for n in name.name_record().iter() {
                    if n.name_id.get() == NameId::FAMILY_NAME {
                        if let Ok(string) = n.string(name.string_data()) {
                            node.set_font_family(string.to_string());
                        }
                        break;
                    }
                }
            }
        }
        node.set_font_size(run.font_size());
        let attrs = run.font_attrs();
        node.set_font_weight(attrs.weight.value());
        if matches!(attrs.style, FontStyle::Italic) {
            node.set_italic();
        }
        if let Some(align) = run.layout.data.alignment {
            node.set_text_align(match align {
                Alignment::Start => {
                    if run.is_rtl() {
                        TextAlign::Right
                    } else {
                        TextAlign::Left
                    }
                }
                Alignment::End => {
                    if run.is_rtl() {
                        TextAlign::Left
                    } else {
                        TextAlign::Right
                    }
                }
                Alignment::Left => TextAlign::Left,
                Alignment::Center => TextAlign::Center,
                Alignment::Right => TextAlign::Right,
                Alignment::Justify => TextAlign::Justify,
            });
        }

        (id, node)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build_nodes<B: Brush>(
        &mut self,
        text: &str,
        layout: &Layout<B>,
        update: &mut TreeUpdate,
        parent_node: &mut Node,
        mut next_node_id: impl FnMut() -> NodeId,
        x_offset: f64,
        y_offset: f64,
        set_brush_properties: impl Fn(&mut Node, &Style<B>),
    ) {
        self.span_paths_by_cluster_path.clear();
        // Build a set of node IDs for the runs encountered in this pass.
        let mut ids = HashSet::<NodeId>::new();
        // Reuse scratch space for storing a sorted list of runs.
        let mut runs = Vec::new();

        for (line_index, line) in layout.lines().enumerate() {
            let metrics = line.metrics();
            // Defer adding each run node until we reach either the next run
            // or the end of the line. That way, we can set relations between
            // runs in a line and do anything special that might be required
            // for the last run in a line.
            let mut last_node: Option<(NodeId, Node)> = None;

            // Iterate over the runs from left to right, computing their offsets,
            // then sort them into text order.
            runs.clear();
            runs.reserve(line.len());
            {
                let mut run_offset = metrics.offset;
                for run in line.runs() {
                    let advance = run.advance();
                    runs.push((run, run_offset));
                    run_offset += advance;
                }
            }
            runs.sort_by_key(|(r, _)| r.text_range().start);

            for (run, run_offset) in runs.drain(..) {
                let mut span_path = ClusterPath::new(line_index as u32, run.index() as u32, 0);
                let (mut id, mut node) =
                    self.span_id_and_node(&mut next_node_id, &mut ids, &run, span_path);

                if let Some((last_id, mut last_node)) = last_node.take() {
                    link_spans(last_id, &mut last_node, id, &mut node);
                    add_span(update, parent_node, last_id, last_node);
                }

                let mut prev_style_index: Option<u16> = None;
                let mut span_text = String::new();
                let mut character_lengths = Vec::new();
                let mut span_offset = 0.0;
                let mut span_advance = 0.0;
                let mut character_positions = Vec::new();
                let mut character_widths = Vec::new();
                let mut word_starts = Vec::new();

                for cluster in run.clusters() {
                    let style_index = cluster.data.style_index;
                    if let Some(prev_index) = prev_style_index {
                        // Limit spans to 256 characters because `word_starts`
                        // consists of `u8`s.
                        if prev_index != style_index || character_lengths.len() >= 256 {
                            prev_style_index = None;
                            finish_span(
                                &mut node,
                                x_offset,
                                y_offset,
                                metrics,
                                run_offset,
                                span_offset,
                                span_advance,
                                span_text.clone(),
                                character_lengths.clone(),
                                character_positions.clone(),
                                character_widths.clone(),
                                word_starts.clone(),
                            );
                            span_offset += span_advance;
                            span_advance = 0.0;
                            span_text.clear();
                            character_lengths.clear();
                            character_positions.clear();
                            character_widths.clear();
                            word_starts.clear();
                            (id, node) = {
                                let (old_id, mut old_node) = (id, node);
                                span_path = cluster.path();
                                let (new_id, mut new_node) = self.span_id_and_node(
                                    &mut next_node_id,
                                    &mut ids,
                                    &run,
                                    span_path,
                                );
                                link_spans(old_id, &mut old_node, new_id, &mut new_node);
                                add_span(update, parent_node, old_id, old_node);
                                (new_id, new_node)
                            };
                        }
                    }

                    if prev_style_index.is_none() {
                        prev_style_index = Some(style_index);
                        let style = cluster.first_style();
                        set_brush_properties(&mut node, style);
                        if let Some(locale) = &style.locale {
                            node.set_language(locale.as_str());
                        }
                    }

                    let cluster_text = &text[cluster.text_range()];
                    span_text.push_str(cluster_text);
                    if cluster.is_word_boundary() && !cluster.is_space_or_nbsp() {
                        word_starts.push(character_lengths.len() as _);
                    }
                    character_lengths.push(cluster_text.len() as _);
                    character_positions.push(span_advance);
                    character_widths.push(cluster.advance());
                    span_advance += cluster.advance();
                    self.span_paths_by_cluster_path
                        .insert(cluster.path(), span_path);
                }

                finish_span(
                    &mut node,
                    x_offset,
                    y_offset,
                    metrics,
                    run_offset,
                    span_offset,
                    span_advance,
                    span_text,
                    character_lengths,
                    character_positions,
                    character_widths,
                    word_starts,
                );
                last_node = Some((id, node));
            }

            if let Some((id, node)) = last_node {
                add_span(update, parent_node, id, node);
            }
        }

        // Remove mappings for spans that no longer exist.
        self.span_paths_by_access_id.retain(|access_id, span_path| {
            let keep = ids.contains(access_id);
            if !keep {
                self.access_ids_by_span_path.remove(span_path);
            }
            keep
        });
    }
}
