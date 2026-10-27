use std::{fmt::Debug, hash::Hash, sync::Arc};

use super::{InlineStyleInputs, RetainedBox, RetainedGraph, RetainedWork};
use crate::{
    LayoutBoxId, LayoutNamespace, LayoutWorld, inline::InlineFormattingContext,
    text::LayoutFontInputs,
};

fn allocation<T>(capacity: usize) -> usize {
    capacity.saturating_mul(std::mem::size_of::<T>())
}

pub(super) struct RetentionEstimate {
    pub(super) graph: usize,
    pub(super) construction: usize,
    pub(super) inline: usize,
    pub(super) inline_numeric: usize,
    pub(super) font_metadata: usize,
}

impl RetentionEstimate {
    pub(super) fn total(&self) -> usize {
        self.graph
            .saturating_add(self.inline)
            .saturating_add(self.font_metadata)
    }
}

impl<N> RetainedWork<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub(super) fn discard_inline_measurements(&mut self) {
        let Some(numeric) = &mut self.inline_numeric_reuse else {
            return;
        };
        numeric.discard_measurements();
        // Measurement-only owners may have kept accepted lines for the next
        // pass. Once this layer is evicted, retain their shaping alone.
        for (owner, inline) in &mut self.inline_contexts {
            if !numeric.contains(*owner) {
                inline.prepare_for_retention(false);
            }
        }
    }

    pub(super) fn from_world(
        mut world: LayoutWorld<N>,
        fonts: LayoutFontInputs,
        pass_box_count: usize,
    ) -> Self {
        let mut inline_numeric_reuse = world.inline_numeric_reuse.take();
        if let Some(numeric) = &mut inline_numeric_reuse {
            let mut has_retained_output = false;
            numeric.retain_owners(|owner| {
                let keep = world
                    .inline_layouts
                    .get(owner.index())
                    .and_then(Option::as_ref)
                    .is_some_and(|inline| inline.laid_out.is_some());
                has_retained_output |= keep;
                keep
            });
            if !has_retained_output {
                inline_numeric_reuse = None;
            }
        }
        let inline_count = world
            .inline_layouts
            .iter()
            .filter(|inline| inline.is_some())
            .count();
        let mut inline_contexts = Vec::with_capacity(inline_count);
        for (index, inline) in world.inline_layouts.iter_mut().enumerate() {
            if let Some(mut inline) = inline.take() {
                let owner = LayoutBoxId::from_index(index);
                let preserve_numeric = inline_numeric_reuse
                    .as_ref()
                    .is_some_and(|numeric| numeric.contains(owner));
                inline.prepare_for_retention(preserve_numeric);
                inline_contexts.push((owner, inline));
            }
        }
        let graph = if world.construction_inputs.is_some() {
            for layout_box in &mut world.boxes {
                // Retain construction ownership, not derived numeric results
                // or paint resources. Clearing a length would still pin its
                // backing allocation, so release pass-local containers here.
                layout_box.clear_numeric_cache_slot();
                layout_box.layout_parent = None;
                layout_box.layout_children = Vec::new();
                layout_box.positioned_containing_block = None;
                layout_box.inline_static_position = None;
                layout_box.inline_context_owner = None;
                layout_box.inline_flattened = false;
                layout_box.scroll_offset = crate::LayoutPoint::ZERO;
                layout_box.replaced_context = None;
                layout_box.replaced_image = None;
                layout_box.css_images = Default::default();
                layout_box.collapsed_table_border_part = false;
                layout_box.table_measure_cache = None;
                layout_box.unrounded_layout = taffy::Layout::with_order(0);
                layout_box.final_layout = taffy::Layout::with_order(0);
                layout_box.capability_diagnostics = Vec::new();
                layout_box.style.prepare_for_construction_retention();
            }
            RetainedGraph::Owned {
                boxes: std::mem::take(&mut world.boxes),
                source_mapping: std::mem::take(&mut world.source_mapping),
            }
        } else {
            RetainedGraph::Compact(compact_boxes(std::mem::take(&mut world.boxes)))
        };
        Self {
            graph,
            prepared_inline_inputs: None,
            construction_inputs: world.construction_inputs.take(),
            pass_box_count,
            inline_contexts,
            inline_numeric_reuse,
            root: world.root,
            root_is_document_element: world.root_is_document_element,
            quirks_mode: world.quirks_mode,
            fonts,
        }
    }

    /// Release excess growth capacity only under pressure. Keep space for
    /// the previous pass's numeric-only suffix: truncating construction boxes
    /// must not force a new doubling allocation on every static-position pass.
    pub(super) fn trim_owned_box_capacity(&mut self) -> usize {
        let RetainedGraph::Owned { boxes, .. } = &mut self.graph else {
            return 0;
        };
        let before = boxes.capacity();
        boxes.shrink_to(self.pass_box_count);
        before.saturating_sub(boxes.capacity())
    }

    /// A budget fallback releases the full object allocation before its
    /// smaller charge is published. IFCs and exact comparison inputs survive.
    pub(super) fn compact_graph(&mut self) {
        let old = std::mem::replace(&mut self.graph, RetainedGraph::Compact(Vec::new()));
        self.graph = match old {
            RetainedGraph::Owned { boxes, .. } => RetainedGraph::Compact(compact_boxes(boxes)),
            compact => compact,
        };
    }

    pub(super) fn estimated_retention(&self) -> RetentionEstimate {
        let construction = self
            .construction_inputs
            .as_ref()
            .map_or(0, |inputs| inputs.estimated_retained_bytes());
        let storage = match &self.graph {
            RetainedGraph::Compact(boxes) => allocation::<RetainedBox<N>>(boxes.capacity()),
            RetainedGraph::Owned {
                boxes,
                source_mapping,
            } => {
                // HashMap exposes usable capacity, not allocator bucket bytes.
                // Conservatively allow two bucket slots per usable entry plus
                // controls; this is an admission model, not physical memory.
                let buckets = source_mapping.capacity().saturating_mul(2);
                let mapping = allocation::<(N, LayoutBoxId)>(buckets)
                    .saturating_add(buckets)
                    .saturating_add(16);
                let labels = boxes.iter().fold(0usize, |bytes, layout_box| {
                    bytes
                        .saturating_add(layout_box.source_label.capacity())
                        .saturating_add(layout_box.owner_label.as_ref().map_or(0, String::capacity))
                });
                allocation::<crate::LayoutBox<N>>(boxes.capacity())
                    .saturating_add(mapping)
                    .saturating_add(labels)
            }
        };
        let mut graph = std::mem::size_of::<Self>()
            .saturating_add(storage)
            .saturating_add(construction);
        let child_capacity = match &self.graph {
            RetainedGraph::Compact(boxes) => boxes.iter().map(|b| b.children.capacity()).sum(),
            RetainedGraph::Owned { boxes, .. } => boxes.iter().map(|b| b.children.capacity()).sum(),
        };
        graph = graph.saturating_add(allocation::<LayoutBoxId>(child_capacity));
        for layout_box in self.construction_boxes() {
            graph = graph.saturating_add(layout_box.text.map_or(0, |text| text.len()));
            if let Some(semantics) = &layout_box.element_semantics {
                graph = graph.saturating_add(semantics.local_name.len());
                if let LayoutNamespace::Other(namespace) = &semantics.namespace {
                    graph = graph.saturating_add(namespace.len());
                }
                if let Some(control) = &semantics.metadata.form_control {
                    graph = graph
                        .saturating_add(control.value.len())
                        .saturating_add(control.placeholder.len())
                        .saturating_add(allocation::<Arc<str>>(control.option_labels.capacity()));
                    graph = control
                        .option_labels
                        .iter()
                        .fold(graph, |bytes, label| bytes.saturating_add(label.len()));
                }
            }
        }
        // The IFC estimator excludes the struct itself. Charge its actual
        // vector slot here, only for real IFC owners, alongside owned buffers
        // and the existing model for opaque Parley paragraph storage.
        let inline_preparation = self.inline_contexts.iter().fold(
            allocation::<(LayoutBoxId, InlineFormattingContext)>(self.inline_contexts.capacity()),
            |bytes, (_, inline)| {
                bytes.saturating_add(inline.estimated_retained_preparation_bytes())
            },
        );
        // Final output vectors remain owned by eligible IFCs. The sidecar
        // accounts for its own allocation and key storage; neither estimate
        // substitutes source length for visible output-buffer capacities.
        let inline_numeric = self.inline_contexts.iter().fold(
            self.inline_numeric_reuse
                .as_ref()
                .map_or(0, |numeric| numeric.estimated_retained_bytes()),
            |bytes, (_, inline)| bytes.saturating_add(inline.estimated_retained_numeric_bytes()),
        );
        RetentionEstimate {
            graph,
            construction,
            inline: inline_preparation.saturating_add(inline_numeric),
            inline_numeric,
            font_metadata: self.fonts.estimated_metadata_bytes(),
        }
    }
}

fn compact_boxes<N>(boxes: Vec<crate::LayoutBox<N>>) -> Vec<RetainedBox<N>> {
    // Never use an in-place IntoIter collect: the large LayoutBox allocation
    // must actually be released before charging the compact representation.
    let mut compact = Vec::with_capacity(boxes.len());
    for mut layout_box in boxes {
        compact.push(RetainedBox {
            source: layout_box.source,
            owner: layout_box.owner,
            pseudo: layout_box.pseudo,
            kind: layout_box.kind,
            anonymous_reason: layout_box.anonymous_reason,
            structural_parent: layout_box.structural_parent,
            parent: layout_box.parent,
            children: std::mem::take(&mut layout_box.children),
            inline_formatting_context: layout_box.inline_formatting_context,
            outside_list_marker: layout_box.outside_list_marker,
            inline_style: InlineStyleInputs::from_style(&layout_box.style),
            computed: layout_box.style.computed.take(),
            element_semantics: layout_box.element_semantics.take(),
            text: layout_box.text.take(),
            text_selection: layout_box.text_selection,
        });
    }
    compact
}
