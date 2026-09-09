use std::{collections::HashMap, ops::Range, sync::Arc};

use moli_layout::{LayoutSvgText, LayoutSvgTextElement};

use crate::{document_runtime::DomHandle, dom::native::DomHost};

/// Resolve XML provenance while the source DOM still exists. Authored IDs are
/// deliberately irrelevant: duplicate/missing IDs must not change text queries.
pub(super) fn source_layout(
    host: &DomHost,
    elements: &[DomHandle],
    source_ids: &[u32],
    svg: &moli_image::SvgImage,
) -> Option<Arc<LayoutSvgText<DomHandle>>> {
    if elements.len() != source_ids.len() {
        tracing::debug!("inline SVG element provenance does not match serialized XML");
        return None;
    }
    let sources = source_ids
        .iter()
        .copied()
        .zip(elements.iter().copied())
        .collect::<HashMap<_, _>>();
    let mut result = LayoutSvgText::default();
    for resource in svg.text_resources() {
        let Some(&root) = sources.get(&resource.source_node_id) else {
            continue;
        };
        let mut ranges_by_element: HashMap<DomHandle, Vec<Range<usize>>> = HashMap::new();
        for range in resource.sources {
            let Some(&source) = sources.get(&range.source_node_id) else {
                continue;
            };
            let mut current = Some(source);
            let mut ancestors = Vec::new();
            while let Some(handle) = current {
                let Some(node) = host.node(handle) else { break };
                if node.namespace() == Some(moli_layout::LayoutNamespace::SVG_URI)
                    && matches!(node.local_name(), Some("text" | "tspan" | "textPath"))
                {
                    ancestors.push(handle);
                }
                if handle == root {
                    for ancestor in ancestors {
                        ranges_by_element
                            .entry(ancestor)
                            .or_default()
                            .push(range.code_units.clone());
                    }
                    break;
                }
                current = node.parent_node_id();
            }
        }
        let layout_index = result.layouts.len();
        result.layouts.push(resource.layout);
        for (source, mut ranges) in ranges_by_element {
            ranges.sort_by_key(|range| (range.start, range.end));
            let mut merged: Vec<Range<usize>> = Vec::new();
            for range in ranges {
                if let Some(previous) = merged.last_mut()
                    && range.start <= previous.end
                {
                    previous.end = previous.end.max(range.end);
                } else {
                    merged.push(range);
                }
            }
            result.elements.push(LayoutSvgTextElement {
                source,
                layout_index,
                ranges: merged,
            });
        }
    }
    result
        .elements
        .sort_by_key(|element| (element.layout_index, element.source.index()));
    (!result.elements.is_empty()).then(|| Arc::new(result))
}
