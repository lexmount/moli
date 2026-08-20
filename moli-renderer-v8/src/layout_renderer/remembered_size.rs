use std::collections::HashMap;

use moli_layout::{
    FrozenLayoutTree, LayoutLastRememberedSize, LayoutLastRememberedSizePolicy, LayoutSize,
};

use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

/// Browser-owned state behind `contain-intrinsic-size: auto`.
///
/// Taffy consumes only the used containment size for one layout epoch. The
/// state that survives layout-object destruction belongs here, alongside the
/// document's other retained layout sidecars, and is keyed by the DOM element
/// rather than by an ephemeral layout box.
#[derive(Clone, Default)]
pub(crate) struct IntrinsicSizeObserverState {
    entries: HashMap<DomHandle, LayoutLastRememberedSize>,
}

impl IntrinsicSizeObserverState {
    pub(crate) fn get(&self, element: DomHandle) -> LayoutLastRememberedSize {
        self.entries.get(&element).copied().unwrap_or_default()
    }

    pub(crate) fn handles(&self) -> impl Iterator<Item = DomHandle> + '_ {
        self.entries.keys().copied()
    }

    /// Paint-clean lifecycle processing clears values that still belong to
    /// disconnected elements. An element removed and reinserted before the
    /// next rendering update therefore retains its value, matching Blink's
    /// deferred disconnected-element processing.
    pub(crate) fn retain_connected(&mut self, runtime: &JsContextHost) {
        self.entries
            .retain(|element, _| runtime.dom_host().is_connected(*element));
    }

    /// Reconciles state with the element's current computed auto axes.
    /// Losing `auto` clears that logical axis even if the element currently
    /// owns no layout box (for example because it is `display:none`).
    pub(crate) fn reconcile_policy(
        &mut self,
        element: DomHandle,
        policy: LayoutLastRememberedSizePolicy,
    ) {
        let Some(size) = self.entries.get_mut(&element) else {
            return;
        };
        if !policy.records_inline_size() {
            size.inline_size = None;
        }
        if !policy.records_block_size() {
            size.block_size = None;
        }
        if size.is_empty() {
            self.entries.remove(&element);
        }
    }

    /// Runs the document's intrinsic-size observation step over one completed
    /// frozen layout. Only visible/unlocked principal CSS boxes publish a new
    /// unzoomed content-box size. Skipped boxes preserve the previous value.
    pub(crate) fn observe_layout(
        &mut self,
        tree: &FrozenLayoutTree<DomHandle>,
        policies: &HashMap<DomHandle, LayoutLastRememberedSizePolicy>,
    ) {
        for layout_box in &tree.boxes {
            let Some(element) = layout_box.principal_source else {
                continue;
            };
            let Some(policy) = policies.get(&element).copied() else {
                continue;
            };
            if !policy.records_any_size()
                || layout_box.contents_skipped
                || layout_box.used_values.is_none()
            {
                continue;
            }

            let zoom = layout_box.effective_zoom;
            if !zoom.is_finite() || zoom <= 0.0 {
                continue;
            }
            let physical_size = LayoutSize::new(
                layout_box.content_box.width / zoom,
                layout_box.content_box.height / zoom,
            );
            if !physical_size.width.is_finite()
                || !physical_size.height.is_finite()
                || physical_size.width < 0.0
                || physical_size.height < 0.0
            {
                continue;
            }

            let observed = policy.observe(physical_size);
            let retained = self.entries.entry(element).or_default();
            if policy.records_inline_size() {
                retained.inline_size = observed.inline_size;
            }
            if policy.records_block_size() {
                retained.block_size = observed.block_size;
            }
        }
    }
}
