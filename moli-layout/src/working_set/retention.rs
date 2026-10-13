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
                    .boxes
                    .get(owner.index())
                    .and_then(|layout_box| layout_box.inline_layout.as_ref())
                    .is_some_and(|inline| inline.laid_out.is_some());
                has_retained_output |= keep;
                keep
            });
            if !has_retained_output {
                inline_numeric_reuse = None;
            }
        }
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
                let owner = LayoutBoxId::from_index(index);
                let preserve_numeric = inline_numeric_reuse
                    .as_ref()
                    .is_some_and(|numeric| numeric.contains(owner));
                inline.prepare_for_retention(preserve_numeric);
                inline_contexts.push((owner, inline));
            }
            // Per-box numeric caches/results, complete box styles, image
            // resources and diagnostic strings drop with the old box. Only
            // eligible IFC keys/results remain in the optional sidecar.
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

    pub(super) fn estimated_retention(&self) -> RetentionEstimate {
        let construction = self
            .construction_inputs
            .as_ref()
            .map_or(0, |inputs| inputs.estimated_retained_bytes());
        let mut graph = std::mem::size_of::<Self>()
            .saturating_add(allocation::<RetainedBox<N>>(self.boxes.capacity()))
            .saturating_add(construction);
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
