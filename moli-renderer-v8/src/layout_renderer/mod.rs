mod display_lock;
mod inline_svg;
mod remembered_size;
mod source_view;
mod style_resolver;

pub(crate) use display_lock::AutoDisplayLockState;
pub(crate) use remembered_size::IntrinsicSizeObserverState;
pub(crate) use source_view::native_element_bypasses_display_lock_display_type_check;

use std::{collections::HashMap, time::Duration};

use moli_layout::{
    DocumentLayoutServices, EmbeddedFrameRenderer, LayoutPassRequest, LayoutPassResult,
    LayoutSource, LayoutViewport, PaintSnapshot, build_layout_pass_with_embedded_frames,
};
use style::values::generics::image::GenericImage;

use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

pub(crate) fn current_native_stylesheet_web_font_resources(
    runtime: &JsContextHost,
    root: DomHandle,
) -> Vec<crate::css_resource_urls::StylesheetLoadBlockingResource> {
    let mut reads = crate::native_bridge::element::ComputedStyleReadScope::new(runtime);
    let sources = reads.read(root).stylesheet_source_snapshots();
    let mut resources = std::collections::BTreeMap::new();
    for (css_text, base_url) in sources {
        for resource in crate::css_resource_urls::stylesheet_load_blocking_resources(
            &css_text,
            &base_url,
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        ) {
            let Some(font) = resource.web_font() else {
                continue;
            };
            resources.entry(font.slot().to_owned()).or_insert(resource);
        }
    }
    resources.into_values().collect()
}

/// Resolves the direct `url()` layers visible to the next paint pass.
///
/// Initial stylesheet discovery starts most requests earlier. This computed
/// walk closes the gap for inline styles, CSSOM mutations, and pseudo styles
/// while using the exact absolute URLs produced by Stylo.
pub(crate) fn current_native_css_image_resources(
    runtime: &JsContextHost,
    root: DomHandle,
) -> Vec<crate::css_resource_urls::StylesheetLoadBlockingResource> {
    let source = source_view::NativeLayoutSourceView::new(runtime, root);
    let mut reads = crate::native_bridge::element::ComputedStyleReadScope::new(runtime);
    let mut urls = std::collections::BTreeMap::new();
    let mut stack = vec![root];
    let mut seen = std::collections::HashSet::new();
    while let Some(node) = stack.pop() {
        if !seen.insert(node) {
            continue;
        }
        let children = source.flat_children(node).collect::<Vec<_>>();
        stack.extend(children.into_iter().rev());
        if source.node_kind(node) != moli_layout::LayoutSourceKind::Element {
            continue;
        }

        let read = reads.read(node);
        if let Some(computed) = read.computed_values() {
            collect_computed_css_image_urls(computed.as_ref(), &mut urls);
        }
        for pseudo in ["marker", "before", "after"] {
            if let Some(computed) = read.pseudo_computed_values(pseudo) {
                collect_computed_css_image_urls(computed.as_ref(), &mut urls);
            }
        }
    }
    urls.into_values()
        .map(crate::css_resource_urls::StylesheetLoadBlockingResource::image)
        .collect()
}

fn collect_computed_css_image_urls(
    computed: &style::properties::ComputedValues,
    output: &mut std::collections::BTreeMap<String, url::Url>,
) {
    let images = computed
        .get_background()
        .background_image
        .0
        .iter()
        .chain(computed.get_svg().mask_image.0.iter());
    for image in images {
        let GenericImage::Url(computed_url) = image else {
            continue;
        };
        let Some(url) = computed_url.url() else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https" | "data" | "blob") {
            continue;
        }
        // Blink removes fragments for the shared fetch but retains them on
        // StyleFetchedImage for SVG view/element selection. We defer those
        // views until that second semantic layer exists.
        if url.fragment().is_some() {
            continue;
        }
        output
            .entry(url.as_str().to_owned())
            .or_insert_with(|| url.as_ref().clone());
    }
}

pub(crate) fn build_native_layout_pass(
    runtime: &JsContextHost,
    root: DomHandle,
    services: &mut DocumentLayoutServices,
    embedded_document_services: &mut HashMap<DomHandle, DocumentLayoutServices>,
    auto_display_locks: &mut AutoDisplayLockState,
    intrinsic_size_observer: &mut IntrinsicSizeObserverState,
    request: LayoutPassRequest,
) -> Result<LayoutPassResult<DomHandle>, moli_layout::LayoutError> {
    let mut working_auto_display_locks = auto_display_locks.clone();
    working_auto_display_locks.retain_connected(runtime);
    let mut working_intrinsic_size_observer = intrinsic_size_observer.clone();
    working_intrinsic_size_observer.retain_connected(runtime);
    let mut delivered_display_lock_observations = std::collections::HashSet::new();
    let mut total_elapsed = Duration::ZERO;

    loop {
        // Every document and embedded frame in this epoch consumes one
        // immutable lock snapshot. Post-layout observations update the
        // working state only for the next epoch, so sibling frames cannot
        // observe a partially advanced lifecycle.
        let display_lock_snapshot = working_auto_display_locks.clone();
        let (result, display_lock_changed) = {
            let mut epoch = NativeLayoutEpoch {
                runtime,
                embedded_document_services,
                display_lock_snapshot: &display_lock_snapshot,
                auto_display_locks: &mut working_auto_display_locks,
                intrinsic_size_observer: &mut working_intrinsic_size_observer,
                delivered_display_lock_observations: &mut delivered_display_lock_observations,
                document_stack: Vec::new(),
                display_lock_changed: false,
            };
            let result = build_native_layout_pass_recursive(&mut epoch, root, services, request);
            (result, epoch.display_lock_changed)
        };
        let mut pass = result?;
        total_elapsed = total_elapsed.saturating_add(pass.metrics.elapsed);
        if display_lock_changed {
            continue;
        }

        pass.metrics.elapsed = total_elapsed;
        *auto_display_locks = working_auto_display_locks;
        *intrinsic_size_observer = working_intrinsic_size_observer;
        return Ok(pass);
    }
}

/// Mutable state shared by every document participating in one atomic layout
/// epoch. The immutable lock snapshot is the sole input to box construction;
/// observations accumulate separately and are visible only to the next epoch.
struct NativeLayoutEpoch<'a> {
    runtime: &'a JsContextHost,
    embedded_document_services: &'a mut HashMap<DomHandle, DocumentLayoutServices>,
    display_lock_snapshot: &'a AutoDisplayLockState,
    auto_display_locks: &'a mut AutoDisplayLockState,
    intrinsic_size_observer: &'a mut IntrinsicSizeObserverState,
    delivered_display_lock_observations: &'a mut std::collections::HashSet<DomHandle>,
    document_stack: Vec<DomHandle>,
    display_lock_changed: bool,
}

fn build_native_layout_pass_recursive(
    epoch: &mut NativeLayoutEpoch<'_>,
    root: DomHandle,
    services: &mut DocumentLayoutServices,
    request: LayoutPassRequest,
) -> Result<LayoutPassResult<DomHandle>, moli_layout::LayoutError> {
    let runtime = epoch.runtime;
    let document = runtime
        .dom_host()
        .owner_document_handle(root)
        .unwrap_or_else(|| runtime.document_handle());
    style_resolver::prepare_layout_style_inputs(runtime, root, document, request.viewport);
    style_resolver::reconcile_remembered_size_policies(
        runtime,
        document,
        request.viewport,
        epoch.intrinsic_size_observer,
    );
    epoch.document_stack.push(document);
    let source = source_view::NativeLayoutSourceView::with_paint_resources(
        runtime,
        root,
        request.requests_paint(),
    );
    let mut styles = style_resolver::NativeLayoutStyleResolver::new(
        runtime,
        document,
        request.viewport,
        epoch.display_lock_snapshot,
        epoch.intrinsic_size_observer,
    );
    let result = {
        let mut frames = NativeEmbeddedFrameRenderer {
            epoch,
            reason: request.reason,
            include_backgrounds: request.includes_backgrounds(),
        };
        build_layout_pass_with_embedded_frames(&source, &mut styles, services, request, &mut frames)
    };
    let policies = styles.into_pass_policies();
    if let Ok(pass) = &result {
        epoch
            .intrinsic_size_observer
            .observe_layout(&pass.tree, &policies.remembered_sizes);
        epoch.display_lock_changed |= epoch.auto_display_locks.observe_layout(
            runtime,
            root,
            document,
            &pass.tree,
            &policies.display_locks,
            epoch.delivered_display_lock_observations,
        );
    }
    epoch.document_stack.pop();
    result
}

struct NativeEmbeddedFrameRenderer<'epoch, 'state> {
    epoch: &'epoch mut NativeLayoutEpoch<'state>,
    reason: moli_layout::LayoutFlushReason,
    include_backgrounds: bool,
}

impl EmbeddedFrameRenderer<DomHandle> for NativeEmbeddedFrameRenderer<'_, '_> {
    fn render_embedded_frame(
        &mut self,
        frame: DomHandle,
        viewport: LayoutViewport,
    ) -> Result<Option<PaintSnapshot>, moli_layout::LayoutError> {
        const MAX_EMBEDDED_DOCUMENT_DEPTH: usize = 32;

        let Some(document) = self
            .epoch
            .runtime
            .child_browsing_context_document_handle(frame)
        else {
            return Ok(None);
        };
        if self.epoch.document_stack.len() >= MAX_EMBEDDED_DOCUMENT_DEPTH
            || self.epoch.document_stack.contains(&document)
        {
            return Ok(None);
        }
        let Some(root) = self
            .epoch
            .runtime
            .dom_host()
            .dom()
            .document_element_handle_for_document(document)
        else {
            return Ok(None);
        };
        let mut services = self
            .epoch
            .embedded_document_services
            .remove(&document)
            .unwrap_or_default();
        let mut capture = moli_layout::PaintCaptureRequest::viewport();
        capture.include_backgrounds = self.include_backgrounds;
        capture.base_background_color = moli_layout::PaintColor::TRANSPARENT;
        let result = build_native_layout_pass_recursive(
            self.epoch,
            root,
            &mut services,
            LayoutPassRequest::with_capture(viewport, self.reason, capture),
        );
        self.epoch
            .embedded_document_services
            .insert(document, services);
        let output = result?;
        output.into_paint_snapshot().map(Some)
    }
}

#[cfg(test)]
pub(crate) fn build_normalized_native_box_tree_for_test(
    runtime: &JsContextHost,
    root: DomHandle,
) -> Result<moli_layout::NormalizedBoxTree, moli_layout::LayoutError> {
    let document = runtime
        .dom_host()
        .owner_document_handle(root)
        .unwrap_or_else(|| runtime.document_handle());
    let viewport = runtime.layout_viewport_for_document(document);
    style_resolver::prepare_layout_style_inputs(runtime, root, document, viewport);
    let source = source_view::NativeLayoutSourceView::new(runtime, root);
    let remembered_sizes = IntrinsicSizeObserverState::default();
    let display_locks = AutoDisplayLockState::default();
    let mut styles = style_resolver::NativeLayoutStyleResolver::new(
        runtime,
        document,
        viewport,
        &display_locks,
        &remembered_sizes,
    );
    moli_layout::build_layout_world(&source, &mut styles).map(|world| world.normalized_tree())
}
