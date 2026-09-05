//! Lazy, synchronous CSSOM size projections. These belong to a style read,
//! never to the Document or the frozen layout tree. Separate JavaScript
//! getters start separate reads; there is no cache spanning a script turn.
//! The first size query samples the available geometry (including absence)
//! for this synchronous read. Nothing here creates or refreshes layout, and
//! the next observation starts again from the latest published snapshot.

use std::{cell::OnceCell, collections::HashMap};

use moli_layout::LayoutUsedSize;

use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

pub(super) enum LayoutSizeReads {
    /// A single element's property batch needs one lookup, not a whole index.
    Element(DomHandle, OnceCell<Option<LayoutUsedSize>>),
    /// A multi-element observation derives one lookup per Document, lazily.
    Document(OnceCell<HashMap<DomHandle, LayoutUsedSize>>),
}

impl LayoutSizeReads {
    pub(super) fn new(target: Option<DomHandle>) -> Self {
        match target {
            Some(target) => Self::Element(target, OnceCell::new()),
            None => Self::Document(OnceCell::new()),
        }
    }

    pub(super) fn get(
        &self,
        runtime: &JsContextHost,
        source_document: Option<DomHandle>,
        handle: DomHandle,
    ) -> Option<LayoutUsedSize> {
        let document = size_document(runtime, handle)?;
        // Recursive compatibility reads may ask about a different Document.
        // Never answer those from this observation's document-local table.
        if Some(document) != source_document {
            return lookup_size(runtime, document, handle);
        }
        match self {
            Self::Element(target, size) if *target == handle => {
                *size.get_or_init(|| lookup_size(runtime, document, handle))
            }
            Self::Element(..) => lookup_size(runtime, document, handle),
            Self::Document(sizes) => sizes
                .get_or_init(|| {
                    runtime
                        .with_latest_layout_tree_for_document(document, |tree| {
                            #[cfg(test)]
                            note_index_build(tree.boxes.len());
                            tree.used_sizes().collect()
                        })
                        .unwrap_or_default()
                })
                .get(&handle)
                .copied(),
        }
    }
}

pub(super) fn used_size_from_layout_snapshot(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<LayoutUsedSize> {
    lookup_size(runtime, size_document(runtime, handle)?, handle)
}

fn size_document(runtime: &JsContextHost, handle: DomHandle) -> Option<DomHandle> {
    (runtime.layout_policy().uses_real_layout() && runtime.dom_host().is_connected(handle))
        .then(|| runtime.layout_document_for_source(handle))
        .flatten()
}

fn lookup_size(
    runtime: &JsContextHost,
    document: DomHandle,
    handle: DomHandle,
) -> Option<LayoutUsedSize> {
    runtime
        .with_latest_layout_tree_for_document(document, |tree| {
            #[cfg(test)]
            note_source_query();
            tree.used_size_for_source(handle)
        })
        .flatten()
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct SizeQueryCounts {
    pub source_queries: usize,
    pub index_builds: usize,
    pub indexed_boxes: usize,
}

#[cfg(test)]
thread_local! {
    static QUERY_COUNTS: std::cell::Cell<SizeQueryCounts> = const {
        std::cell::Cell::new(SizeQueryCounts { source_queries: 0, index_builds: 0, indexed_boxes: 0 })
    };
}

#[cfg(test)]
fn note_source_query() {
    let mut counts = QUERY_COUNTS.get();
    counts.source_queries += 1;
    QUERY_COUNTS.set(counts);
}

#[cfg(test)]
fn note_index_build(boxes: usize) {
    let mut counts = QUERY_COUNTS.get();
    counts.index_builds += 1;
    counts.indexed_boxes += boxes;
    QUERY_COUNTS.set(counts);
}

#[cfg(test)]
pub(super) fn take_query_counts() -> SizeQueryCounts {
    QUERY_COUNTS.replace(SizeQueryCounts::default())
}
