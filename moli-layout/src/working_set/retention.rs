use std::{fmt::Debug, hash::Hash, sync::Arc};

use super::{InlineStyleInputs, RetainedBox, RetainedWork};
use crate::{
    LayoutBoxId, LayoutNamespace, LayoutWorld, inline::InlineFormattingContext,
    text::LayoutFontInputs,
};

fn allocation<T>(capacity: usize) -> usize {
    capacity.saturating_mul(std::mem::size_of::<T>())
}

pub(super) struct RetentionEstimate {
    pub(super) graph: usize,
    pub(super) inline: usize,
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
    pub(super) fn from_world(mut world: LayoutWorld<N>, fonts: LayoutFontInputs) -> Self {
        let inline_count = world
            .boxes
            .iter()
            .filter(|layout_box| layout_box.inline_layout.is_some())
            .count();
        let mut inline_contexts = Vec::with_capacity(inline_count);
        // Do not collect a mapped Vec::IntoIter here: in-place collection may
        // reuse the much larger LayoutBox allocation as excess graph capacity.
        let mut boxes = Vec::with_capacity(world.boxes.len());
        for (index, mut layout_box) in std::mem::take(&mut world.boxes).into_iter().enumerate() {
            if let Some(mut inline) = layout_box.inline_layout.take() {
                inline.prepare_for_retention();
                inline_contexts.push((LayoutBoxId::from_index(index), inline));
            }
            // Numeric caches/results, complete numeric styles, image
            // resources and diagnostic strings drop with the old box.
            boxes.push(RetainedBox {
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
        Self {
            boxes,
            inline_contexts,
            root: world.root,
            root_is_document_element: world.root_is_document_element,
            quirks_mode: world.quirks_mode,
            fonts,
        }
    }

    pub(super) fn estimated_retention(&self) -> RetentionEstimate {
        let mut graph = std::mem::size_of::<Self>()
            .saturating_add(allocation::<RetainedBox<N>>(self.boxes.capacity()));
        for layout_box in &self.boxes {
            graph = graph
                .saturating_add(allocation::<LayoutBoxId>(layout_box.children.capacity()))
                .saturating_add(layout_box.text.as_deref().map_or(0, str::len));
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
        let inline = self.inline_contexts.iter().fold(
            allocation::<(LayoutBoxId, InlineFormattingContext)>(self.inline_contexts.capacity()),
            |bytes, (_, inline)| {
                bytes.saturating_add(inline.estimated_retained_preparation_bytes())
            },
        );
        RetentionEstimate {
            graph,
            inline,
            font_metadata: self.fonts.estimated_metadata_bytes(),
        }
    }
}
