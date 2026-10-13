use std::{fmt::Debug, hash::Hash, sync::Arc};

use super::RetainedWork;
use crate::{LayoutBox, LayoutListMarkerType, LayoutNamespace};

pub(super) fn allocation<T>(capacity: usize) -> usize {
    capacity.saturating_mul(std::mem::size_of::<T>())
}

impl<N> RetainedWork<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub(super) fn prune_and_estimate(&mut self) -> usize {
        let mut bytes = std::mem::size_of::<Self>()
            .saturating_add(allocation::<LayoutBox<N>>(self.boxes.capacity()))
            .saturating_add(self.fonts.estimated_metadata_bytes());
        for layout_box in &mut self.boxes {
            // These buffers are reconstructed from fresh source input or
            // numeric layout. Keeping them would pin old image resources and
            // duplicate the already-published geometry without saving work.
            layout_box.source_label = String::new();
            layout_box.owner_label = None;
            layout_box.capability_diagnostics = Vec::new();
            layout_box.layout_parent = None;
            layout_box.layout_children = Vec::new();
            layout_box.positioned_containing_block = None;
            layout_box.inline_static_position = None;
            layout_box.inline_context_owner = None;
            layout_box.inline_flattened = false;
            layout_box.replaced_context = None;
            layout_box.replaced_image = None;
            layout_box.css_images = Default::default();
            layout_box.collapsed_table_borders = None;
            layout_box.collapsed_table_border_part = false;
            layout_box.resolved_grid_tracks = None;
            layout_box.cache = taffy::Cache::new();
            layout_box.table_measure_cache = None;
            layout_box.unrounded_layout = taffy::Layout::with_order(0);
            layout_box.final_layout = taffy::Layout::with_order(0);

            // Float is the only Taffy projection used by the retained inline
            // input comparison. The remaining numeric style is rebuilt and
            // can release its Grid vectors and borrowed calc pointers now.
            layout_box.style.taffy = taffy::Style {
                float: layout_box.style.taffy.float,
                ..taffy::Style::default()
            };

            bytes = bytes
                .saturating_add(allocation::<crate::LayoutBoxId>(
                    layout_box.children.capacity(),
                ))
                .saturating_add(layout_box.text.as_deref().map_or(0, str::len))
                .saturating_add(layout_box.style.generated_text().map_or(0, str::len));
            if let Some(semantics) = &layout_box.element_semantics {
                bytes = bytes.saturating_add(semantics.local_name.len());
                if let LayoutNamespace::Other(namespace) = &semantics.namespace {
                    bytes = bytes.saturating_add(namespace.len());
                }
                if let Some(control) = &semantics.metadata.form_control {
                    bytes = bytes
                        .saturating_add(control.value.len())
                        .saturating_add(control.placeholder.len())
                        .saturating_add(allocation::<Arc<str>>(control.option_labels.capacity()));
                    bytes = control
                        .option_labels
                        .iter()
                        .fold(bytes, |bytes, label| bytes.saturating_add(label.len()));
                }
            }
            match layout_box.style.list_marker_type() {
                LayoutListMarkerType::String(text) => bytes = bytes.saturating_add(text.len()),
                LayoutListMarkerType::Symbols(symbols) => {
                    bytes = bytes.saturating_add(allocation::<Arc<str>>(symbols.capacity()));
                    bytes = symbols
                        .iter()
                        .fold(bytes, |bytes, symbol| bytes.saturating_add(symbol.len()));
                }
                _ => {}
            }
            if let Some(inline) = &mut layout_box.inline_layout {
                inline.prepare_for_retention();
                bytes = bytes.saturating_add(inline.estimated_retained_preparation_bytes());
            }
        }
        bytes
    }
}
