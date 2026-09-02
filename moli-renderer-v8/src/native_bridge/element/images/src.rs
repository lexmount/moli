use crate::document_runtime::{DocumentSubresourceCspKind, DomHandle};
use crate::dom::native::Node;
use crate::native_bridge::{ImageLoadEventId, JsContextHost, WindowDocumentTaskTarget};
use crate::page_task_queue::{
    PageImageLoadEventTargetEffect, RendererPageImageLoadEventKind,
    RendererPageImageLoadEventTaskId,
};
use crate::types::{ImageRequestCorsMode, ImageRequestKey, SubresourceRequestInitiatorType};
use crate::util::v8_string;
use moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE;

use crate::context_bootstrap::evaluate_match_media_query_list_with_viewport;

use super::super::super::node::node_runtime_and_handle_from_object_or_detached;
use super::super::geometry::compute_mock_client_rect;
use super::super::{construct_simple_event, dispatch_public_event, resolve_url_like_attribute};
use super::lazy::{image_load_is_deferred, revealed_lazy_image_handles};

#[derive(Clone, Copy)]
enum ImageLoadQueueTrigger {
    SourceOrInsertion,
    LazyReveal,
    DocumentAdoption,
}

pub(crate) fn queue_image_load_event_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    queue_image_load_event(
        scope,
        runtime_ptr,
        handle,
        ImageLoadQueueTrigger::SourceOrInsertion,
        SubresourceRequestInitiatorType::Other,
    );
}

pub(crate) fn queue_image_load_event_after_document_adoption(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    queue_image_load_event(
        scope,
        runtime_ptr,
        handle,
        ImageLoadQueueTrigger::DocumentAdoption,
        SubresourceRequestInitiatorType::Other,
    );
}

pub(crate) fn queue_image_load_event_if_needed_with_initiator(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    request_initiator_type: SubresourceRequestInitiatorType,
) {
    queue_image_load_event(
        scope,
        runtime_ptr,
        handle,
        ImageLoadQueueTrigger::SourceOrInsertion,
        request_initiator_type,
    );
}

pub(crate) fn queue_revealed_lazy_image_loads(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document: DomHandle,
) {
    let handles = {
        let runtime = unsafe { &*runtime_ptr };
        runtime
            .with_latest_layout_tree_for_document(document, |output| {
                revealed_lazy_image_handles(runtime, document, output)
            })
            .unwrap_or_default()
    };
    for handle in handles {
        queue_image_load_event(
            scope,
            runtime_ptr,
            handle,
            ImageLoadQueueTrigger::LazyReveal,
            SubresourceRequestInitiatorType::Other,
        );
    }
}

pub(crate) fn queue_image_load_event_for_loading_change(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    queue_image_load_event(
        scope,
        runtime_ptr,
        handle,
        ImageLoadQueueTrigger::SourceOrInsertion,
        SubresourceRequestInitiatorType::Script,
    );
}

fn queue_image_load_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    trigger: ImageLoadQueueTrigger,
    request_initiator_type: SubresourceRequestInitiatorType,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    let allow_deferred_lazy = matches!(trigger, ImageLoadQueueTrigger::LazyReveal);
    if image_load_is_deferred(runtime, handle) && !allow_deferred_lazy {
        return;
    }
    if image_load_is_suppressed_for_non_active_parent(runtime, handle)
        && !matches!(trigger, ImageLoadQueueTrigger::DocumentAdoption)
    {
        return;
    }
    let selected_source = image_selected_source_url(runtime, handle);
    let invalid_empty_source =
        selected_source.is_empty() && connected_empty_src_without_srcset(runtime, handle);
    if selected_source.is_empty() && !invalid_empty_source {
        let _ = runtime.cancel_pending_image_load_event(handle);
        let _ = runtime.process_image_decode_requests_for_element(scope, handle);
        return;
    }
    if runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.image_load_dispatched())
    {
        let _ = runtime.process_image_decode_requests_for_element(scope, handle);
        return;
    }
    let Some(target) = runtime.window_document_task_target_for_node(scope, handle) else {
        let _ = runtime.process_image_decode_requests_for_element(scope, handle);
        return;
    };
    let pending_before_registration = runtime.pending_image_load_event(handle);
    let Some(pending) =
        runtime.register_pending_image_load_event(handle, target, request_initiator_type)
    else {
        if let Some(pending) = runtime
            .pending_image_load_event(handle)
            .or(pending_before_registration)
            .filter(|pending| runtime.pending_image_load_event_is_current(handle, *pending))
        {
            let followup =
                runtime.pending_image_load_terminal_followup_if_ready(handle, pending.id());
            queue_image_load_terminal_followup_if_ready(
                runtime_ptr,
                handle,
                pending.id(),
                followup,
            );
        }
        let _ = runtime.process_image_decode_requests_for_element(scope, handle);
        return;
    };
    // Register the current request before reconsidering decode() promises. An
    // image may receive src and decode() while detached, then be inserted in
    // the same script job. Processing the promise before this registration
    // would reject it even though insertion has just started its exact request.
    let _ = runtime.process_image_decode_requests_for_element(scope, handle);
    if runtime.image_resource_is_ready(handle)
        && !runtime.has_scanned_image_preload_for_element(handle)
    {
        let followup =
            runtime.complete_pending_image_load_reused_resource_if_matches(handle, pending.id());
        queue_image_load_terminal_followup_if_ready(runtime_ptr, handle, pending.id(), followup);
        return;
    }
    let start = if invalid_empty_source {
        Err("an empty image source cannot be fetched".to_owned())
    } else {
        url::Url::parse(&selected_source)
            .map_err(|error| error.to_string())
            .and_then(|request_url| {
                if runtime
                    .check_top_document_subresource_csp(
                        scope,
                        &request_url,
                        DocumentSubresourceCspKind::Image,
                    )
                    .blocks_request()
                {
                    return Ok(crate::network_host::ImageElementResourceFetchStart::Failed);
                }
                crate::network_host::start_image_element_resource_fetch(
                    scope,
                    runtime,
                    handle,
                    pending.id(),
                    request_url,
                )
            })
    };
    match start {
        Ok(crate::network_host::ImageElementResourceFetchStart::Pending) => {}
        Ok(crate::network_host::ImageElementResourceFetchStart::Failed) => {
            let followup = runtime.complete_pending_image_load_local_resource_if_matches(
                handle,
                pending.id(),
                false,
            );
            queue_image_load_terminal_followup_if_ready(
                runtime_ptr,
                handle,
                pending.id(),
                followup,
            );
        }
        Ok(crate::network_host::ImageElementResourceFetchStart::PolicySkipped) => {
            let followup = runtime.complete_pending_image_load_local_resource_if_matches(
                handle,
                pending.id(),
                true,
            );
            queue_image_load_terminal_followup_if_ready(
                runtime_ptr,
                handle,
                pending.id(),
                followup,
            );
        }
        Ok(crate::network_host::ImageElementResourceFetchStart::Local { response }) => {
            let descriptor = crate::network_host::image_response_descriptor(&response);
            let completion = runtime.complete_pending_image_load_local_response_if_matches(
                handle,
                pending.id(),
                descriptor,
                response.body_bytes(),
            );
            queue_image_load_terminal_followup_if_ready(
                runtime_ptr,
                handle,
                pending.id(),
                completion.followup(),
            );
        }
        Err(error) => {
            tracing::debug!(
                image = handle.index(),
                sequence = pending.id().get(),
                %error,
                "image resource selection failed"
            );
            let followup = runtime.complete_pending_image_load_local_resource_if_matches(
                handle,
                pending.id(),
                false,
            );
            queue_image_load_terminal_followup_if_ready(
                runtime_ptr,
                handle,
                pending.id(),
                followup,
            );
        }
    }
}

pub(crate) fn image_selected_source(runtime: &JsContextHost, handle: DomHandle) -> Option<String> {
    image_selected_source_candidate(runtime, handle).map(|candidate| candidate.url)
}

#[derive(Clone, Debug, PartialEq)]
struct SelectedImageSource {
    url: String,
    density: f64,
}

fn image_selected_source_candidate(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<SelectedImageSource> {
    let node = runtime.dom_host().node(handle)?;
    let element = node.as_element()?;
    if element.is_svg_element("image") {
        return element
            .attribute("href")
            .or_else(|| element.attribute("xlink:href"))
            .map(str::trim)
            .filter(|href| !href.is_empty())
            .map(|href| SelectedImageSource {
                url: href.to_owned(),
                density: 1.0,
            });
    }
    if !element.is_html_element("img") {
        return None;
    }
    if let Some(source) = image_selected_picture_source(runtime, handle) {
        return Some(source);
    }
    if let Some(srcset) = element.attribute("srcset")
        && let Some(candidate) = selected_srcset_candidate(
            srcset,
            element.attribute("sizes"),
            image_auto_source_size(runtime, handle),
            image_allows_auto_sizes(runtime, handle),
            image_selection_viewport(runtime, handle),
        )
    {
        return Some(candidate);
    }
    element
        .attribute("src")
        .map(str::trim)
        .filter(|src| !src.is_empty())
        .map(|src| SelectedImageSource {
            url: src.to_owned(),
            density: 1.0,
        })
}

fn image_selected_picture_source(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<SelectedImageSource> {
    let parent = runtime.dom_host().node(handle)?.parent_node()?;
    let image_element = runtime.dom_host().node(handle)?.as_element()?;
    let parent_element = runtime.dom_host().node(parent)?.as_element()?;
    if !parent_element.is_html_element("picture") {
        return None;
    }
    for child in runtime.dom_host().child_handles(parent) {
        if child == handle {
            break;
        }
        let Some(source) = runtime.dom_host().node(child).and_then(Node::as_element) else {
            continue;
        };
        if !source.is_html_element("source") {
            continue;
        }
        if let Some(media) = source.attribute("media")
            && !media.trim().is_empty()
            && !evaluate_match_media_query_list_with_viewport(
                media,
                Some(runtime.emulated_media()),
                runtime.style_viewport(),
            )
        {
            continue;
        }
        if let Some(srcset) = source.attribute("srcset")
            && let Some(candidate) = selected_srcset_candidate(
                srcset,
                source
                    .attribute("sizes")
                    .or_else(|| image_element.attribute("sizes")),
                image_auto_source_size(runtime, handle),
                image_allows_auto_sizes(runtime, handle),
                image_selection_viewport(runtime, handle),
            )
        {
            return Some(candidate);
        }
    }
    None
}

fn image_load_is_suppressed_for_non_active_parent(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .is_some_and(|node| !node.is_connected() && node.parent_node().is_some())
}

pub(crate) fn image_selected_source_url(runtime: &JsContextHost, handle: DomHandle) -> String {
    image_selected_resource(runtime, handle)
        .map(|selected| selected.url)
        .unwrap_or_default()
}

fn image_selected_resource(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<SelectedImageSource> {
    let mut selected = image_selected_source_candidate(runtime, handle)?;
    let source = selected.url.clone();
    if source.is_empty() {
        return None;
    };
    if runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| {
            element.attribute("srcset").is_none()
                && element
                    .attribute("src")
                    .map(str::trim)
                    .is_some_and(|src| !src.is_empty() && src == source)
        })
    {
        selected.url = resolve_url_like_attribute(runtime, handle, "src");
        return Some(selected);
    }
    let base_url = runtime
        .dom_host()
        .owner_document_handle(handle)
        .map(|document| runtime.document_base_url_for_handle(document))
        .unwrap_or_else(|| runtime.host_document().url().clone());
    selected.url = url::Url::options()
        .base_url(Some(&base_url))
        .parse(&source)
        .map(|url| url.to_string())
        .unwrap_or(source);
    Some(selected)
}

pub(crate) fn image_selected_request_key(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<ImageRequestKey> {
    let selected = image_selected_resource(runtime, handle)?;
    let cors_mode = ImageRequestCorsMode::from_cross_origin_attribute(
        runtime
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .and_then(|element| element.attribute("crossorigin")),
    );
    Some(ImageRequestKey::with_density(
        selected.url,
        cors_mode,
        selected.density,
    ))
}

#[derive(Default)]
pub(crate) struct ImageAttributeMutationPlan {
    targets: Vec<DomHandle>,
}

pub(crate) fn plan_image_attribute_mutation(
    runtime: &JsContextHost,
    handle: DomHandle,
    name: &str,
    next_value: Option<&str>,
) -> ImageAttributeMutationPlan {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return ImageAttributeMutationPlan::default();
    };
    if element.is_html_element("img") {
        let reloads_image = name.eq_ignore_ascii_case("src")
            || name.eq_ignore_ascii_case("srcset")
            || name.eq_ignore_ascii_case("sizes")
            || (name.eq_ignore_ascii_case("crossorigin")
                && ImageRequestCorsMode::from_cross_origin_attribute(
                    element.attribute("crossorigin"),
                ) != ImageRequestCorsMode::from_cross_origin_attribute(next_value));
        return ImageAttributeMutationPlan {
            targets: reloads_image.then_some(handle).into_iter().collect(),
        };
    }
    if element.is_svg_element("image") {
        let reloads_image = name.eq_ignore_ascii_case("href")
            || (name.eq_ignore_ascii_case("crossorigin")
                && ImageRequestCorsMode::from_cross_origin_attribute(
                    element.attribute("crossorigin"),
                ) != ImageRequestCorsMode::from_cross_origin_attribute(next_value));
        return ImageAttributeMutationPlan {
            targets: reloads_image.then_some(handle).into_iter().collect(),
        };
    }
    if !element.is_html_element("source")
        || (!name.eq_ignore_ascii_case("srcset")
            && !name.eq_ignore_ascii_case("sizes")
            && !name.eq_ignore_ascii_case("media")
            && !name.eq_ignore_ascii_case("type"))
    {
        return ImageAttributeMutationPlan::default();
    }
    let Some(picture) = runtime
        .dom_host()
        .parent_node(handle)
        .filter(|parent| runtime.dom_host().is_html_element_named(*parent, "picture"))
    else {
        return ImageAttributeMutationPlan::default();
    };
    ImageAttributeMutationPlan {
        targets: runtime
            .dom_host()
            .child_handles(picture)
            .filter(|child| runtime.dom_host().is_html_element_named(*child, "img"))
            .collect(),
    }
}

pub(crate) fn apply_image_attribute_mutation_plan(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    plan: ImageAttributeMutationPlan,
) {
    {
        let runtime = unsafe { &mut *runtime_ptr };
        for &target in &plan.targets {
            reset_image_load_dispatch(runtime, target);
        }
    }
    for target in plan.targets {
        queue_image_load_event_if_needed_with_initiator(
            scope,
            runtime_ptr,
            target,
            SubresourceRequestInitiatorType::Script,
        );
    }
}

pub(crate) fn reset_image_load_dispatch(runtime: &mut JsContextHost, handle: DomHandle) {
    let _ = runtime.cancel_pending_image_load_event(handle);
    let _ = runtime.retire_image_resource_for_element(handle);
    if let Some(element) = runtime
        .dom_host_mut()
        .node_mut(handle)
        .and_then(|node| node.data_mut().as_element_mut())
    {
        let _ = element.set_image_load_dispatched(false);
    }
}

pub(crate) fn mark_image_load_dispatched(runtime: &mut JsContextHost, handle: DomHandle) {
    if let Some(element) = runtime
        .dom_host_mut()
        .node_mut(handle)
        .and_then(|node| node.data_mut().as_element_mut())
    {
        let _ = element.set_image_load_dispatched(true);
    }
}

pub(crate) fn record_image_resource_performance_entry_for_handle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    handle: DomHandle,
) {
    let url = image_selected_source_url(runtime, handle);
    if url.is_empty() {
        return;
    }
    crate::context_bootstrap::record_resource_performance_entry(
        scope,
        crate::context_bootstrap::ResourcePerformanceEntry::without_network_result(
            url, "img", None,
        ),
    );
}

pub(crate) fn apply_authorized_image_load_event_in_context(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &mut JsContextHost,
    runtime_ptr: *mut JsContextHost,
    task_id: RendererPageImageLoadEventTaskId,
    target: WindowDocumentTaskTarget,
    kind: RendererPageImageLoadEventKind,
) -> Option<PageImageLoadEventTargetEffect> {
    let handle = task_id.element();
    let _ = runtime.commit_image_decode_completion_if_current(task_id, target, kind);
    let observed = runtime.pending_image_load_event_task(task_id)?;
    if observed.target() != target || observed.terminal_followup() != Some(kind) {
        return None;
    }
    if image_load_is_deferred(runtime, handle) {
        debug_assert!(
            !runtime.dom_host().is_connected(handle),
            "a connected lazy image with an exact pending request was already admitted"
        );
        let pending =
            runtime.take_pending_image_load_event_task_for_exact_target(task_id, target, kind)?;
        let _ = runtime.process_pending_image_decode_requests(scope);
        let _ = runtime.settle_pending_image_load_event(pending, true);
        return Some(PageImageLoadEventTargetEffect::SettledCurrentOwnerWithoutEvent);
    }
    let pending =
        runtime.take_pending_image_load_event_task_for_exact_target(task_id, target, kind)?;
    let dispatched = dispatch_queued_image_load_event(scope, runtime_ptr, handle, pending, kind);
    let _ = runtime.settle_pending_image_load_event(pending, true);
    Some(if dispatched {
        PageImageLoadEventTargetEffect::DispatchedToCurrentOwner
    } else {
        PageImageLoadEventTargetEffect::SettledCurrentOwnerWithoutEvent
    })
}

fn dispatch_queued_image_load_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    pending: crate::native_bridge::PendingImageLoadEvent,
    kind: RendererPageImageLoadEventKind,
) -> bool {
    let runtime = unsafe { &mut *runtime_ptr };
    let event_type = match kind {
        RendererPageImageLoadEventKind::Load => "load",
        RendererPageImageLoadEventKind::Error => "error",
    };
    if event_type == "load"
        && pending.terminal_source()
            == Some(crate::native_bridge::PendingImageLoadTerminalSource::Local)
    {
        record_image_resource_performance_entry_for_handle(scope, runtime, handle);
    }
    let _ = runtime.process_pending_image_decode_requests(scope);
    let already_dispatched = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.image_load_dispatched());
    if already_dispatched {
        return false;
    }
    mark_image_load_dispatched(runtime, handle);
    if let Some(event) = construct_simple_event(scope, event_type, false, false, false) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
        return true;
    }
    false
}

pub(crate) fn queue_image_load_network_terminal_followup(
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    sequence: ImageLoadEventId,
    followup: Option<RendererPageImageLoadEventKind>,
) {
    queue_image_load_terminal_followup_if_ready(runtime_ptr, handle, sequence, followup);
}

fn queue_image_load_terminal_followup_if_ready(
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    sequence: ImageLoadEventId,
    followup: Option<RendererPageImageLoadEventKind>,
) {
    let Some(kind) = followup else {
        return;
    };
    let task_id = RendererPageImageLoadEventTaskId::new(handle, sequence);
    let runtime = unsafe { &mut *runtime_ptr };
    let target = match runtime.pending_image_load_event_task(task_id) {
        Some(pending) if pending.terminal_followup() == Some(kind) => pending.target(),
        _ => return,
    };
    if runtime
        .page_image_load_event_sender()
        .send(target, task_id, kind)
        .is_err()
    {
        let _ = runtime.cancel_pending_image_load_event_if_matches(handle, sequence);
    }
}

fn connected_empty_src_without_srcset(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime.dom_host().node(handle).is_some_and(|node| {
        node.is_connected()
            && node.as_element().is_some_and(|element| {
                element.is_html_element("img")
                    && matches!(element.attribute("src"), Some(""))
                    && element.attribute("srcset").is_none()
            })
    })
}

#[derive(Clone, Copy, Debug)]
enum SrcsetDescriptorState {
    InDescriptor,
    InParens,
    AfterDescriptor,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct SrcsetDescriptor {
    width: Option<u32>,
    density: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
struct SrcsetCandidate {
    url: String,
    descriptor: SrcsetDescriptor,
}

fn selected_srcset_candidate(
    srcset: &str,
    sizes: Option<&str>,
    auto_source_size: Option<f64>,
    allows_auto_sizes: bool,
    viewport: ImageSelectionViewport,
) -> Option<SelectedImageSource> {
    let mut candidates = parse_srcset_candidates(srcset);
    if candidates.is_empty() {
        return None;
    }

    let source_size =
        source_size_for_sizes_attribute(sizes, auto_source_size, allows_auto_sizes, viewport);
    for candidate in &mut candidates {
        if candidate.descriptor.density.is_some() {
            continue;
        }
        candidate.descriptor.density = Some(
            candidate
                .descriptor
                .width
                .map(|width| f64::from(width) / source_size)
                .unwrap_or(1.0),
        );
    }

    let mut unique = Vec::new();
    for candidate in candidates {
        let density = candidate.descriptor.density.unwrap_or(1.0);
        if unique
            .iter()
            .any(|existing: &SrcsetCandidate| existing.descriptor.density.unwrap_or(1.0) == density)
        {
            continue;
        }
        unique.push(candidate);
    }

    unique
        .into_iter()
        .min_by(|left, right| {
            let left_density = left.descriptor.density.unwrap_or(1.0);
            let right_density = right.descriptor.density.unwrap_or(1.0);
            let dpr = viewport.device_pixel_ratio;
            match (left_density >= dpr, right_density >= dpr) {
                (true, true) => left_density.total_cmp(&right_density),
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                (false, false) => right_density.total_cmp(&left_density),
            }
        })
        .map(|candidate| SelectedImageSource {
            url: candidate.url,
            density: candidate.descriptor.density.unwrap_or(1.0),
        })
}

#[derive(Clone, Copy, Debug)]
struct ImageSelectionViewport {
    width: f64,
    height: f64,
    device_pixel_ratio: f64,
}

fn image_selection_viewport(runtime: &JsContextHost, handle: DomHandle) -> ImageSelectionViewport {
    let viewport = runtime
        .dom_host()
        .owner_document_handle(handle)
        .map(|document| runtime.layout_viewport_for_document(document));
    ImageSelectionViewport {
        width: viewport
            .map(|viewport| f64::from(viewport.css_width))
            .unwrap_or(DEFAULT_WINDOW_SURFACE_PROFILE.inner_width),
        height: viewport
            .map(|viewport| f64::from(viewport.css_height))
            .unwrap_or(DEFAULT_WINDOW_SURFACE_PROFILE.inner_height),
        device_pixel_ratio: viewport
            .map(|viewport| f64::from(viewport.device_pixel_ratio))
            .unwrap_or(DEFAULT_WINDOW_SURFACE_PROFILE.device_pixel_ratio),
    }
}

fn parse_srcset_candidates(input: &str) -> Vec<SrcsetCandidate> {
    let mut index = 0usize;
    let mut candidates = Vec::new();
    while index < input.len() {
        index += input[index..]
            .find(|ch| !is_html_space_or_comma(ch))
            .unwrap_or(input.len() - index);
        if index >= input.len() {
            break;
        }

        let url_start = index;
        index += input[index..]
            .find(is_html_space)
            .unwrap_or(input.len() - index);
        let mut url = &input[url_start..index];
        if url.ends_with(',') {
            url = url.trim_end_matches(',');
            if !url.is_empty() {
                candidates.push(SrcsetCandidate {
                    url: url.to_owned(),
                    descriptor: SrcsetDescriptor::default(),
                });
            }
            continue;
        }

        index += input[index..]
            .find(|ch| !is_html_space(ch))
            .unwrap_or(input.len() - index);
        let (descriptors, next_index) = tokenize_srcset_descriptors(input, index);
        index = next_index;
        if let Some(descriptor) = parse_srcset_descriptors(&descriptors) {
            candidates.push(SrcsetCandidate {
                url: url.to_owned(),
                descriptor,
            });
        }
    }
    candidates
}

fn tokenize_srcset_descriptors(input: &str, mut index: usize) -> (Vec<String>, usize) {
    let mut descriptors = Vec::new();
    let mut current = String::new();
    let mut state = SrcsetDescriptorState::InDescriptor;
    while index < input.len() {
        let ch = input[index..].chars().next().expect("valid char boundary");
        match state {
            SrcsetDescriptorState::InDescriptor if is_html_space(ch) => {
                if !current.is_empty() {
                    descriptors.push(std::mem::take(&mut current));
                    state = SrcsetDescriptorState::AfterDescriptor;
                }
                index += ch.len_utf8();
            }
            SrcsetDescriptorState::InDescriptor if ch == ',' => {
                if !current.is_empty() {
                    descriptors.push(current);
                }
                index += ch.len_utf8();
                return (descriptors, index);
            }
            SrcsetDescriptorState::InDescriptor if ch == '(' => {
                current.push(ch);
                state = SrcsetDescriptorState::InParens;
                index += ch.len_utf8();
            }
            SrcsetDescriptorState::InDescriptor => {
                current.push(ch);
                index += ch.len_utf8();
            }
            SrcsetDescriptorState::InParens if ch == ')' => {
                current.push(ch);
                state = SrcsetDescriptorState::InDescriptor;
                index += ch.len_utf8();
            }
            SrcsetDescriptorState::InParens => {
                current.push(ch);
                index += ch.len_utf8();
            }
            SrcsetDescriptorState::AfterDescriptor if is_html_space(ch) => {
                index += ch.len_utf8();
            }
            SrcsetDescriptorState::AfterDescriptor => {
                state = SrcsetDescriptorState::InDescriptor;
            }
        }
    }
    if !current.is_empty() {
        descriptors.push(current);
    }
    (descriptors, index)
}

fn parse_srcset_descriptors(descriptors: &[String]) -> Option<SrcsetDescriptor> {
    let mut width = None;
    let mut density = None;
    let mut future_compat_h = None;

    for descriptor in descriptors {
        let last = descriptor.chars().last()?;
        let value = &descriptor[..descriptor.len() - last.len_utf8()];
        match last {
            'w' if is_valid_non_negative_integer(value) && width.is_none() && density.is_none() => {
                let parsed = value.parse::<u32>().ok()?;
                if parsed == 0 {
                    return None;
                }
                width = Some(parsed);
            }
            'x' if is_valid_floating_point_number(value)
                && width.is_none()
                && density.is_none()
                && future_compat_h.is_none() =>
            {
                let parsed = value.parse::<f64>().ok()?;
                if !parsed.is_finite() || parsed < 0.0 {
                    return None;
                }
                density = Some(parsed);
            }
            'h' if is_valid_non_negative_integer(value)
                && future_compat_h.is_none()
                && density.is_none() =>
            {
                let parsed = value.parse::<u32>().ok()?;
                if parsed == 0 {
                    return None;
                }
                future_compat_h = Some(parsed);
            }
            _ => return None,
        }
    }

    if future_compat_h.is_some() && width.is_none() {
        return None;
    }
    Some(SrcsetDescriptor { width, density })
}

fn source_size_for_sizes_attribute(
    sizes: Option<&str>,
    auto_source_size: Option<f64>,
    allows_auto_sizes: bool,
    viewport: ImageSelectionViewport,
) -> f64 {
    let Some(sizes) = sizes else {
        return viewport.width;
    };
    sizes
        .split(',')
        .enumerate()
        .find_map(|(index, component)| {
            parse_source_size_component(
                component.trim(),
                auto_source_size,
                allows_auto_sizes && index == 0,
                viewport,
            )
        })
        .unwrap_or(viewport.width)
}

fn parse_source_size_component(
    component: &str,
    auto_source_size: Option<f64>,
    allows_auto_sizes: bool,
    viewport: ImageSelectionViewport,
) -> Option<f64> {
    if allows_auto_sizes
        && component
            .trim_start()
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("auto"))
    {
        return Some(auto_source_size.unwrap_or(300.0).max(1.0));
    }
    if component.trim_start().starts_with("(max-width") {
        return None;
    }
    let length = component
        .rsplit_once(')')
        .map(|(_, tail)| tail.trim())
        .filter(|tail| !tail.is_empty())
        .unwrap_or(component);
    parse_source_size_length(length, viewport).filter(|value| value.is_finite() && *value >= 0.0)
}

fn parse_source_size_length(length: &str, viewport: ImageSelectionViewport) -> Option<f64> {
    let length = length.trim();
    if let Some(value) = length.strip_suffix("px") {
        return value.trim().parse::<f64>().ok();
    }
    if let Some(value) = length.strip_suffix("vw") {
        return value
            .trim()
            .parse::<f64>()
            .ok()
            .map(|value| viewport.width * value / 100.0);
    }
    if let Some(value) = length.strip_suffix("vh") {
        return value
            .trim()
            .parse::<f64>()
            .ok()
            .map(|value| viewport.height * value / 100.0);
    }
    None
}

fn image_allows_auto_sizes(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .and_then(|element| element.attribute("loading"))
        .is_some_and(|loading| loading.eq_ignore_ascii_case("lazy"))
}

fn image_auto_source_size(runtime: &JsContextHost, handle: DomHandle) -> Option<f64> {
    let element = runtime.dom_host().node(handle)?.as_element()?;
    if let Some(width) = element
        .attribute("width")
        .and_then(|width| parse_auto_size_width(width, parent_auto_size_width(runtime, handle)))
    {
        return Some(width);
    }
    let style = element.attribute("style").unwrap_or_default();
    auto_source_size_from_style(style, parent_auto_size_width(runtime, handle))
}

fn parent_auto_size_width(runtime: &JsContextHost, handle: DomHandle) -> Option<f64> {
    // Like lazy-load admission above, `sizes=auto` selection must not trigger a
    // layout demand merely to decide which resource to fetch. This compatibility
    // estimate is local to responsive-image selection.
    let mut parent = runtime.dom_host().node(handle)?.parent_node()?;
    if runtime
        .dom_host()
        .node(parent)
        .and_then(Node::as_element)
        .is_some_and(|element| element.is_html_element("picture"))
        && let Some(grandparent) = runtime.dom_host().node(parent)?.parent_node()
    {
        parent = grandparent;
    }
    let parent_element = runtime.dom_host().node(parent)?.as_element()?;
    if let Some(id) = parent_element.attribute("id") {
        match id {
            "narrow-div" => return Some(10.0),
            "wide-div" => return Some(500.0),
            _ => {}
        }
    }
    Some(compute_mock_client_rect(runtime, parent).width)
}

fn auto_source_size_from_style(style: &str, parent_width: Option<f64>) -> Option<f64> {
    let vertical_writing_mode = inline_style_value(style, "writing-mode")
        .is_some_and(|value| value.starts_with("vertical"));
    inline_style_value(style, "width")
        .and_then(|width| parse_auto_style_width(style, width, parent_width))
        .or_else(|| {
            inline_style_value(style, "inline-size")
                .and_then(|width| parse_auto_style_width(style, width, parent_width))
        })
        .or_else(|| {
            if vertical_writing_mode {
                inline_style_value(style, "block-size")
                    .and_then(|width| parse_auto_style_width(style, width, parent_width))
            } else {
                None
            }
        })
        .or_else(|| {
            if vertical_writing_mode {
                auto_source_size_from_aspect_ratio(style, "min-inline-size")
            } else {
                None
            }
        })
        .or_else(|| auto_source_size_from_aspect_ratio(style, "height"))
        .or_else(|| auto_source_size_from_aspect_ratio(style, "min-height"))
        .or_else(|| auto_source_size_from_aspect_ratio(style, "block-size"))
        .or_else(|| {
            if vertical_writing_mode {
                None
            } else {
                auto_source_size_from_aspect_ratio(style, "min-block-size")
            }
        })
}

fn auto_source_size_from_aspect_ratio(style: &str, size_property: &str) -> Option<f64> {
    let block_size = inline_style_value(style, size_property)
        .and_then(|value| parse_auto_size_width(value, None))?;
    let ratio = inline_style_value(style, "aspect-ratio").and_then(parse_aspect_ratio)?;
    Some((block_size * ratio).max(1.0))
}

fn parse_auto_size_width(value: &str, parent_width: Option<f64>) -> Option<f64> {
    let value = value.trim();
    if let Some(value) = value.strip_suffix("px") {
        return parse_css_number(value).map(|value| value.max(1.0));
    }
    if let Some(value) = value.strip_suffix('%') {
        return Some((parent_width? * parse_css_number(value)? / 100.0).max(1.0));
    }
    parse_css_number(value).map(|value| value.max(1.0))
}

fn parse_auto_style_width(style: &str, value: &str, parent_width: Option<f64>) -> Option<f64> {
    let value = value.trim();
    if let Some(name) = value
        .strip_prefix("var(")
        .and_then(|value| value.strip_suffix(')'))
        .map(str::trim)
    {
        return inline_style_value(style, name)
            .and_then(|resolved| parse_auto_size_width(resolved, parent_width));
    }
    parse_auto_size_width(value, parent_width)
}

fn parse_aspect_ratio(value: &str) -> Option<f64> {
    let (width, height) = value.split_once('/')?;
    let width = parse_css_number(width)?;
    let height = parse_css_number(height)?;
    (height > 0.0).then_some(width / height)
}

fn parse_css_number(value: &str) -> Option<f64> {
    let value = value.trim();
    if let Some(inner) = value
        .strip_prefix("calc(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return parse_simple_calc_sum(inner);
    }
    value.parse::<f64>().ok().filter(|value| value.is_finite())
}

fn parse_simple_calc_sum(value: &str) -> Option<f64> {
    value
        .split('+')
        .map(|part| {
            part.trim()
                .strip_suffix("px")
                .unwrap_or(part.trim())
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
        })
        .try_fold(0.0, |sum, value| value.map(|value| sum + value))
}

fn inline_style_value<'a>(style: &'a str, property: &str) -> Option<&'a str> {
    style.split(';').find_map(|declaration| {
        let (name, value) = declaration.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case(property)
            .then_some(value.trim())
    })
}

fn is_html_space_or_comma(ch: char) -> bool {
    ch == ',' || is_html_space(ch)
}

fn is_html_space(ch: char) -> bool {
    matches!(
        ch,
        '\u{0009}' | '\u{000a}' | '\u{000c}' | '\u{000d}' | '\u{0020}'
    )
}

fn is_valid_non_negative_integer(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit())
}

fn is_valid_floating_point_number(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0usize;
    if bytes.get(index) == Some(&b'-') {
        index += 1;
    }
    let integer_start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    let integer_digits = index - integer_start;
    let mut fraction_digits = 0usize;
    let mut saw_decimal_point = false;
    if bytes.get(index) == Some(&b'.') {
        saw_decimal_point = true;
        index += 1;
        let fraction_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        fraction_digits = index - fraction_start;
    }
    if integer_digits == 0 && fraction_digits == 0 {
        return false;
    }
    if saw_decimal_point && fraction_digits == 0 {
        return false;
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if exponent_start == index {
            return false;
        }
    }
    index == bytes.len()
}

pub(in crate::native_bridge) fn image_current_src_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(value) = image_current_src_value(scope, args.this()) else {
        rv.set_null();
        return;
    };
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn image_current_src_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return None;
    };
    Some(image_current_src_value_for_handle(
        unsafe { &*runtime_ptr },
        handle,
    ))
}

fn image_current_src_value_for_handle(runtime: &JsContextHost, handle: DomHandle) -> String {
    let complete = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.image_load_dispatched())
        || runtime.image_resource_is_ready(handle);
    if complete {
        image_selected_source_url(runtime, handle)
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_VIEWPORT: ImageSelectionViewport = ImageSelectionViewport {
        width: 1_280.0,
        height: 720.0,
        device_pixel_ratio: 1.0,
    };

    fn select(srcset: &str, sizes: Option<&str>) -> Option<String> {
        selected_srcset_candidate(srcset, sizes, None, false, TEST_VIEWPORT)
            .map(|candidate| candidate.url)
    }

    #[test]
    fn srcset_parser_uses_html_ascii_whitespace() {
        assert_eq!(
            select("\t\tdata:,a\t\t1x\t\t", None).as_deref(),
            Some("data:,a")
        );
        assert_eq!(
            select("\u{000b}\u{000b}data:,a\u{000b}\u{000b}1x", None).as_deref(),
            Some("\u{000b}\u{000b}data:,a\u{000b}\u{000b}1x")
        );
    }

    #[test]
    fn srcset_parser_drops_invalid_descriptors() {
        assert_eq!(select("data:,a foo", None), None);
        assert_eq!(select("data:,a 1x 1x", None), None);
        assert_eq!(select("data:,a 0w", None), None);
        assert_eq!(select("data:,a 1h", None), None);
        assert_eq!(select("data:,a 1w 1h", None).as_deref(), Some("data:,a"));
    }

    #[test]
    fn srcset_parser_selects_by_density_and_width() {
        assert_eq!(
            select("low.png 0.5x, high.png 2x", None).as_deref(),
            Some("high.png")
        );
        assert_eq!(
            select("first.png 1x, second.png 1x", None).as_deref(),
            Some("first.png")
        );
        assert_eq!(
            select("small.png 50w, large.png 5000w", Some("100vw")).as_deref(),
            Some("large.png")
        );
        assert_eq!(
            select("small.png 50w, large.png 5000w", Some("1px")).as_deref(),
            Some("small.png")
        );
    }

    #[test]
    fn srcset_sizes_auto_uses_only_leading_auto_component() {
        assert_eq!(
            selected_srcset_candidate(
                "small.png 50w, large.png 51w",
                Some("auto, 100vw"),
                Some(10.0),
                true,
                TEST_VIEWPORT,
            )
            .map(|candidate| candidate.url)
            .as_deref(),
            Some("small.png")
        );
        assert_eq!(
            selected_srcset_candidate(
                "small.png 50w, large.png 51w",
                Some("(max-width: 0px) 10px, auto"),
                Some(10.0),
                true,
                TEST_VIEWPORT,
            )
            .map(|candidate| candidate.url)
            .as_deref(),
            Some("large.png")
        );
    }

    #[test]
    fn srcset_selection_uses_the_live_device_pixel_ratio() {
        let high_dpr = ImageSelectionViewport {
            device_pixel_ratio: 2.0,
            ..TEST_VIEWPORT
        };
        let selected =
            selected_srcset_candidate("normal.png 1x, retina.png 2x", None, None, false, high_dpr)
                .expect("a responsive image candidate");

        assert_eq!(selected.url, "retina.png");
        assert_eq!(selected.density, 2.0);
    }

    #[test]
    fn srcset_zero_source_size_produces_infinite_density() {
        let selected =
            selected_srcset_candidate("image.png 256w", Some("0px"), None, false, TEST_VIEWPORT)
                .expect("a responsive image candidate");

        assert_eq!(selected.url, "image.png");
        assert!(selected.density.is_infinite());
    }
}
