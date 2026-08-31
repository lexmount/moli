use super::super::geometry::observable_bounding_client_rect;
use super::super::{
    canonical_cross_origin_value, canonical_loading_value, canonical_preload_value,
    construct_simple_event, dispatch_public_event, element_attribute, element_has_attribute,
    html_element_getter_receiver, html_element_setter_receiver, html_media_element_getter_receiver,
    html_media_element_setter_receiver, parsed_url_like_attribute, property_dom_string_value,
    property_usv_string_value, remove_reflected_attribute, resolve_url_like_attribute,
    set_reflected_attribute, set_reflected_boolean_attribute,
    should_block_dangling_markup_subresource,
};
use crate::document_runtime::DocumentSubresourceCspKind;
use crate::native_bridge::{
    JsContextHost, MediaLoadSequenceId, PendingMediaCanPlayFollowup,
    PendingMediaLoadTerminalFollowup,
};
use crate::util::v8_string;

pub(in crate::native_bridge) fn media_autoplay_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    media_boolean_attribute_getter(scope, args.this(), "autoplay", rv);
}

pub(in crate::native_bridge) fn media_src_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = html_media_element_getter_receiver(scope, args.this(), "src")
    else {
        rv.set_null();
        return;
    };
    let value = resolve_url_like_attribute(unsafe { &*runtime_ptr }, handle, "src");
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge) fn media_src_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = html_media_element_setter_receiver(scope, args.this(), "src")
    else {
        rv.set_undefined();
        return;
    };
    let Some(value) = property_usv_string_value(scope, args.get(0), "HTMLMediaElement", "src")
    else {
        return;
    };
    let previous = element_attribute(unsafe { &*runtime_ptr }, handle, "src");
    set_reflected_attribute(scope, runtime_ptr, handle, "src", &value);
    if previous.as_deref() == Some(value.as_str()) {
        // The generic attribute mutation hook owns changed values. Preserve
        // HTMLMediaElement's same-value resource-selection restart without
        // starting a second sequence for ordinary IDL reflection.
        queue_media_load_if_source_or_loading_change(scope, runtime_ptr, handle, "src");
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn media_cross_origin_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_getter_receiver(scope, args.this(), "crossOrigin")
    else {
        rv.set_null();
        return;
    };
    match element_attribute(unsafe { &*runtime_ptr }, handle, "crossorigin") {
        Some(value) => {
            let Some(value) = v8_string(scope, canonical_cross_origin_value(&value)) else {
                rv.set_null();
                return;
            };
            rv.set(value.into());
        }
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn media_cross_origin_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_setter_receiver(scope, args.this(), "crossOrigin")
    else {
        rv.set_undefined();
        return;
    };
    let value = args.get(0);
    if value.is_null() || value.is_undefined() {
        remove_reflected_attribute(scope, runtime_ptr, handle, "crossorigin");
        rv.set_undefined();
        return;
    }
    let Some(value) = property_dom_string_value(scope, value, "HTMLMediaElement", "crossOrigin")
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "crossorigin", &value);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn media_loading_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_getter_receiver(scope, args.this(), "loading")
    else {
        rv.set_null();
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "loading").unwrap_or_default();
    let Some(value) = v8_string(scope, canonical_loading_value(&value)) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge) fn media_loading_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_setter_receiver(scope, args.this(), "loading")
    else {
        rv.set_undefined();
        return;
    };
    let Some(value) = property_dom_string_value(scope, args.get(0), "HTMLMediaElement", "loading")
    else {
        return;
    };
    let previous = element_attribute(unsafe { &*runtime_ptr }, handle, "loading");
    set_reflected_attribute(scope, runtime_ptr, handle, "loading", &value);
    if previous.as_deref() == Some(value.as_str()) {
        queue_media_load_if_source_or_loading_change(scope, runtime_ptr, handle, "loading");
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn media_preload_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_getter_receiver(scope, args.this(), "preload")
    else {
        rv.set_empty_string();
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "preload").unwrap_or_default();
    if let Some(value) = v8_string(scope, canonical_preload_value(&value)) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge) fn media_preload_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_setter_receiver(scope, args.this(), "preload")
    else {
        rv.set_undefined();
        return;
    };
    let Some(value) = property_dom_string_value(scope, args.get(0), "HTMLMediaElement", "preload")
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "preload", &value);
    rv.set_undefined();
}

pub(crate) fn queue_media_load_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: crate::document_runtime::DomHandle,
) {
    queue_media_load(scope, runtime_ptr, handle, MediaLoadTrigger::Automatic);
}

pub(crate) fn queue_media_load_for_explicit_request(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: crate::document_runtime::DomHandle,
) {
    queue_media_load(scope, runtime_ptr, handle, MediaLoadTrigger::Explicit);
}

fn queue_media_load(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: crate::document_runtime::DomHandle,
    trigger: MediaLoadTrigger,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    let current_owner_document = runtime
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::owner_document);
    if runtime
        .pending_media_load_sequence(handle)
        .is_some_and(|pending| current_owner_document != Some(pending.owner_document_handle()))
    {
        // Chromium restarts media selection after DidMoveToNewDocument. Retire
        // the old document's exact delay before accepting the new owner below;
        // the queued callback remains harmless because the new sequence gets a
        // different id.
        let _ = runtime.cancel_pending_media_load_sequence(handle);
        let _ = runtime.set_media_ready_state(handle, 0);
        let _ = runtime.set_media_network_state(handle, 0);
    }
    let Some(element) = runtime
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::as_element)
    else {
        return;
    };
    if !element.is_html_element("audio") && !element.is_html_element("video") {
        return;
    }
    if runtime.pending_media_load_sequence(handle).is_some() || element.media_ready_state() > 0 {
        return;
    }
    let selected_source = match selected_media_source(runtime, handle) {
        Ok(Some(selected_source)) => selected_source,
        Ok(None) => return,
        Err(()) => {
            start_media_load(scope, runtime_ptr, handle, None);
            return;
        }
    };
    if let Some(deferral) = media_load_deferral(runtime, handle, trigger) {
        if deferral == MediaLoadDeferral::Lazy {
            register_lazy_media_load_candidate_if_media(runtime, handle);
        }
        let _ = runtime.set_media_ready_state(handle, 0);
        let _ = runtime.set_media_network_state(handle, 1);
        return;
    }
    start_media_load(scope, runtime_ptr, handle, Some(selected_source));
}

pub(crate) fn queue_media_load_if_source_or_loading_change(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: crate::document_runtime::DomHandle,
    attribute_name: &str,
) {
    if !attribute_name.eq_ignore_ascii_case("src")
        && !attribute_name.eq_ignore_ascii_case("loading")
        && !attribute_name.eq_ignore_ascii_case("preload")
        && !attribute_name.eq_ignore_ascii_case("autoplay")
    {
        return;
    }
    let target = {
        let runtime = unsafe { &mut *runtime_ptr };
        media_load_target_for_source_change(runtime, handle, attribute_name)
    };
    let Some(target) = target else {
        return;
    };
    if attribute_name.eq_ignore_ascii_case("src") {
        let runtime = unsafe { &mut *runtime_ptr };
        let _ = runtime.cancel_pending_media_load_sequence(target);
        let _ = runtime.set_media_ready_state(target, 0);
        let _ = runtime.set_media_network_state(target, 0);
    }
    queue_media_load_if_needed(scope, runtime_ptr, target);
}

pub(crate) fn queue_revealed_lazy_media_loads(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
) {
    let candidates = unsafe { &*runtime_ptr }.lazy_media_load_candidates();
    let mut stale = Vec::new();
    let mut handles = Vec::new();
    {
        let runtime = unsafe { &*runtime_ptr };
        for handle in candidates {
            match lazy_media_load_candidate_state(runtime, handle) {
                LazyMediaLoadCandidateState::Stale => stale.push(handle),
                LazyMediaLoadCandidateState::Pending => {}
                LazyMediaLoadCandidateState::Revealed => handles.push(handle),
            }
        }
    }
    {
        let runtime = unsafe { &mut *runtime_ptr };
        for handle in stale.iter().chain(handles.iter()) {
            runtime.remove_lazy_media_load_candidate(*handle);
        }
    }
    for handle in handles {
        queue_media_load(scope, runtime_ptr, handle, MediaLoadTrigger::LazyReveal);
    }
}

pub(crate) fn register_lazy_media_load_candidate_if_media(
    runtime: &mut JsContextHost,
    handle: crate::document_runtime::DomHandle,
) {
    if runtime
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::as_element)
        .is_some_and(|element| element.is_html_element("audio") || element.is_html_element("video"))
    {
        runtime.register_lazy_media_load_candidate(handle);
    }
}

enum LazyMediaLoadCandidateState {
    Stale,
    Pending,
    Revealed,
}

fn lazy_media_load_candidate_state(
    runtime: &JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> LazyMediaLoadCandidateState {
    let Some(node) = runtime.dom_host().node(handle) else {
        return LazyMediaLoadCandidateState::Stale;
    };
    if !node.is_connected() {
        return LazyMediaLoadCandidateState::Stale;
    }
    let Some(element) = node.as_element() else {
        return LazyMediaLoadCandidateState::Stale;
    };
    if !element.is_html_element("audio") && !element.is_html_element("video") {
        return LazyMediaLoadCandidateState::Stale;
    }
    if element.media_ready_state() > 0 {
        return LazyMediaLoadCandidateState::Stale;
    }
    if !element
        .attribute("loading")
        .is_some_and(|loading| loading.eq_ignore_ascii_case("lazy"))
        || element.attribute("controls").is_none()
        || selected_media_source(runtime, handle)
            .ok()
            .flatten()
            .is_none()
    {
        return LazyMediaLoadCandidateState::Pending;
    }
    if media_is_inside_initial_viewport(runtime, handle) {
        LazyMediaLoadCandidateState::Revealed
    } else {
        LazyMediaLoadCandidateState::Pending
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MediaLoadTrigger {
    Automatic,
    LazyReveal,
    Explicit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MediaLoadDeferral {
    Lazy,
    PreloadNone,
}

fn media_load_deferral(
    runtime: &JsContextHost,
    handle: crate::document_runtime::DomHandle,
    trigger: MediaLoadTrigger,
) -> Option<MediaLoadDeferral> {
    let element = runtime
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::as_element)
        .filter(|element| element.is_html_element("audio") || element.is_html_element("video"))?;
    if trigger == MediaLoadTrigger::Automatic
        && element
            .attribute("loading")
            .is_some_and(|loading| loading.eq_ignore_ascii_case("lazy"))
    {
        return Some(MediaLoadDeferral::Lazy);
    }
    if trigger != MediaLoadTrigger::Explicit
        && element.attribute("autoplay").is_none()
        && element
            .attribute("preload")
            .is_some_and(|preload| preload.eq_ignore_ascii_case("none"))
    {
        return Some(MediaLoadDeferral::PreloadNone);
    }
    None
}

fn media_is_inside_initial_viewport(
    runtime: &JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> bool {
    let Ok(rect) = observable_bounding_client_rect(
        runtime,
        handle,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    ) else {
        return false;
    };
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return false;
    }
    let Some(document) = runtime.layout_document_for_source(handle) else {
        return false;
    };
    let viewport_height = f64::from(runtime.layout_viewport_for_document(document).css_height);
    rect.bottom > 0.0 && rect.top < viewport_height
}

fn start_media_load(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: crate::document_runtime::DomHandle,
    selected_source: Option<SelectedMediaSource>,
) {
    let pending = {
        let runtime = unsafe { &mut *runtime_ptr };
        let pending = runtime.register_pending_media_load_sequence(handle);
        runtime.remove_lazy_media_load_candidate(handle);
        pending
    };
    let Some(pending) = pending else {
        return;
    };
    super::apply_default_text_track_modes_for_media(scope, runtime_ptr, handle);
    super::queue_media_selection_text_track_loads(scope, runtime_ptr, handle, pending.id());
    let _ = unsafe { &mut *runtime_ptr }.set_media_error_code(handle, None);
    let _ = unsafe { &mut *runtime_ptr }.set_media_ready_state(handle, 0);
    let _ = unsafe { &mut *runtime_ptr }.set_media_network_state(handle, 2);
    if !queue_media_load_event_phase(
        scope,
        runtime_ptr,
        handle,
        pending.id(),
        MediaLoadEventPhase::LoadStart,
    ) {
        let _ = unsafe { &mut *runtime_ptr }
            .cancel_pending_media_load_sequence_if_matches(handle, pending.id());
        return;
    }
    let start = selected_source
        .ok_or_else(|| "media resource selection found no supported URL".to_owned())
        .and_then(|selected_source| {
            let request_url = selected_source.request_url;
            if unsafe { &mut *runtime_ptr }
                .check_top_document_subresource_csp(
                    scope,
                    &request_url,
                    DocumentSubresourceCspKind::Media,
                )
                .blocks_request()
            {
                return Ok(crate::network_host::MediaElementResourceFetchStart::Local {
                    successful: false,
                });
            }
            if should_block_dangling_markup_subresource(
                &request_url,
                &selected_source.original_input,
            ) {
                return Ok(crate::network_host::MediaElementResourceFetchStart::Local {
                    successful: false,
                });
            }
            crate::network_host::start_media_element_resource_fetch(
                scope,
                unsafe { &mut *runtime_ptr },
                handle,
                pending.id(),
                request_url,
            )
        });
    match start {
        Ok(crate::network_host::MediaElementResourceFetchStart::Pending) => {}
        Ok(crate::network_host::MediaElementResourceFetchStart::PolicySkipped) => {
            let followup = unsafe { &mut *runtime_ptr }
                .complete_pending_media_load_local_resource_if_matches(handle, pending.id(), true);
            queue_media_load_terminal_followup_if_ready(
                scope,
                runtime_ptr,
                handle,
                pending.id(),
                followup,
            );
        }
        Ok(crate::network_host::MediaElementResourceFetchStart::Local { successful }) => {
            let followup = unsafe { &mut *runtime_ptr }
                .complete_pending_media_load_local_resource_if_matches(
                    handle,
                    pending.id(),
                    successful,
                );
            queue_media_load_terminal_followup_if_ready(
                scope,
                runtime_ptr,
                handle,
                pending.id(),
                followup,
            );
        }
        Err(error) => {
            tracing::debug!(
                media = handle.index(),
                sequence = pending.id().get(),
                %error,
                "media resource selection failed"
            );
            let followup = unsafe { &mut *runtime_ptr }
                .complete_pending_media_load_local_resource_if_matches(handle, pending.id(), false);
            queue_media_load_terminal_followup_if_ready(
                scope,
                runtime_ptr,
                handle,
                pending.id(),
                followup,
            );
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MediaLoadEventPhase {
    LoadStart,
    LoadedMetadata,
    LoadedData,
    CanPlay,
    Error,
}

impl MediaLoadEventPhase {
    fn event_type(self) -> &'static str {
        match self {
            Self::LoadStart => "loadstart",
            Self::LoadedMetadata => "loadedmetadata",
            Self::LoadedData => "loadeddata",
            Self::CanPlay => "canplay",
            Self::Error => "error",
        }
    }

    fn next(self) -> Option<Self> {
        match self {
            Self::LoadStart => Some(Self::LoadedMetadata),
            Self::LoadedMetadata => Some(Self::LoadedData),
            Self::LoadedData => Some(Self::CanPlay),
            Self::CanPlay | Self::Error => None,
        }
    }
}

fn queue_media_load_event_phase(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: crate::document_runtime::DomHandle,
    sequence: MediaLoadSequenceId,
    phase: MediaLoadEventPhase,
) -> bool {
    if !unsafe { &*runtime_ptr }.pending_media_load_sequence_is_current(handle, sequence) {
        return false;
    }
    unsafe { &mut *runtime_ptr }.queue_media_load_event_phase(scope, handle, sequence, phase)
}

pub(crate) fn dispatch_media_load_event_phase(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: crate::document_runtime::DomHandle,
    sequence: MediaLoadSequenceId,
    phase: MediaLoadEventPhase,
) -> bool {
    if !unsafe { &*runtime_ptr }.pending_media_load_sequence_is_current(handle, sequence) {
        let _ = unsafe { &mut *runtime_ptr }
            .cancel_pending_media_load_sequence_if_matches(handle, sequence);
        return false;
    }
    match phase {
        MediaLoadEventPhase::LoadStart => {}
        MediaLoadEventPhase::LoadedMetadata => {
            let _ = unsafe { &mut *runtime_ptr }.set_media_ready_state(handle, 1);
        }
        MediaLoadEventPhase::LoadedData => {
            let _ = unsafe { &mut *runtime_ptr }.set_media_ready_state(handle, 2);
        }
        MediaLoadEventPhase::CanPlay => {
            let _ = unsafe { &mut *runtime_ptr }.set_media_ready_state(handle, 4);
            let _ = unsafe { &mut *runtime_ptr }.set_media_network_state(handle, 1);
        }
        MediaLoadEventPhase::Error => {
            let _ = unsafe { &mut *runtime_ptr }.set_media_error_code(handle, Some(4));
            let _ = unsafe { &mut *runtime_ptr }.set_media_ready_state(handle, 0);
            let _ = unsafe { &mut *runtime_ptr }.set_media_network_state(handle, 3);
        }
    }
    let dispatched = if let Some(event) =
        construct_simple_event(scope, phase.event_type(), false, false, false)
    {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
        true
    } else {
        false
    };
    if matches!(
        phase,
        MediaLoadEventPhase::LoadedData | MediaLoadEventPhase::Error
    ) {
        let _ = unsafe { &mut *runtime_ptr }
            .settle_pending_media_load_delay_if_matches(handle, sequence);
    }
    if !unsafe { &*runtime_ptr }.pending_media_load_sequence_is_current(handle, sequence) {
        let _ = unsafe { &mut *runtime_ptr }
            .cancel_pending_media_load_sequence_if_matches(handle, sequence);
        return dispatched;
    }
    if phase == MediaLoadEventPhase::LoadStart {
        let followup = unsafe { &mut *runtime_ptr }
            .mark_pending_media_loadstart_dispatched_if_matches(handle, sequence);
        queue_media_load_terminal_followup_if_ready(scope, runtime_ptr, handle, sequence, followup);
        return dispatched;
    }
    if phase == MediaLoadEventPhase::CanPlay {
        dispatch_media_autoplay_if_needed(scope, runtime_ptr, handle);
    }
    if phase == MediaLoadEventPhase::LoadedData
        && unsafe { &mut *runtime_ptr }
            .defer_pending_media_canplay_for_text_tracks(handle, sequence)
    {
        return dispatched;
    }
    let Some(next) = phase.next() else {
        let _ = unsafe { &mut *runtime_ptr }
            .finish_pending_media_load_sequence_if_matches(handle, sequence);
        return dispatched;
    };
    if !queue_media_load_event_phase(scope, runtime_ptr, handle, sequence, next) {
        let _ = unsafe { &mut *runtime_ptr }
            .cancel_pending_media_load_sequence_if_matches(handle, sequence);
    }
    dispatched
}

pub(crate) fn queue_media_canplay_after_text_tracks(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    followup: Option<PendingMediaCanPlayFollowup>,
) {
    let Some(followup) = followup else {
        return;
    };
    if !queue_media_load_event_phase(
        scope,
        runtime_ptr,
        followup.media_handle(),
        followup.media_sequence(),
        MediaLoadEventPhase::CanPlay,
    ) {
        let _ = unsafe { &mut *runtime_ptr }.cancel_pending_media_load_sequence_if_matches(
            followup.media_handle(),
            followup.media_sequence(),
        );
    }
}

pub(crate) fn queue_media_load_network_terminal_followup(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: crate::document_runtime::DomHandle,
    sequence: MediaLoadSequenceId,
    followup: Option<PendingMediaLoadTerminalFollowup>,
) {
    queue_media_load_terminal_followup_if_ready(scope, runtime_ptr, handle, sequence, followup);
}

fn queue_media_load_terminal_followup_if_ready(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: crate::document_runtime::DomHandle,
    sequence: MediaLoadSequenceId,
    followup: Option<PendingMediaLoadTerminalFollowup>,
) {
    let Some(followup) = followup else {
        return;
    };
    let phase = match followup {
        PendingMediaLoadTerminalFollowup::Ready => MediaLoadEventPhase::LoadedMetadata,
        PendingMediaLoadTerminalFollowup::Failed => MediaLoadEventPhase::Error,
    };
    if !queue_media_load_event_phase(scope, runtime_ptr, handle, sequence, phase) {
        let _ = unsafe { &mut *runtime_ptr }
            .cancel_pending_media_load_sequence_if_matches(handle, sequence);
    }
}

fn dispatch_media_autoplay_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: crate::document_runtime::DomHandle,
) {
    let should_autoplay = {
        let runtime = unsafe { &mut *runtime_ptr };
        let should_autoplay = runtime
            .dom_host()
            .node(handle)
            .and_then(crate::dom::native::Node::as_element)
            .is_some_and(|element| {
                element.attribute("autoplay").is_some() && element.media_paused()
            });
        should_autoplay && runtime.set_media_paused(handle, false)
    };
    if !should_autoplay {
        return;
    }
    if let Some(event) = construct_simple_event(scope, "play", false, false, false) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
    if let Some(event) = construct_simple_event(scope, "playing", false, false, false) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
}

fn media_load_target_for_source_change(
    runtime: &JsContextHost,
    handle: crate::document_runtime::DomHandle,
    attribute_name: &str,
) -> Option<crate::document_runtime::DomHandle> {
    let element = runtime.dom_host().node(handle)?.as_element()?;
    if element.is_html_element("audio") || element.is_html_element("video") {
        return Some(handle);
    }
    if !attribute_name.eq_ignore_ascii_case("src") || !element.is_html_element("source") {
        return None;
    }
    let parent = runtime.dom_host().parent_node(handle)?;
    runtime
        .dom_host()
        .node(parent)
        .and_then(crate::dom::native::Node::as_element)
        .filter(|parent| parent.is_html_element("audio") || parent.is_html_element("video"))
        .map(|_| parent)
}

struct SelectedMediaSource {
    request_url: url::Url,
    original_input: String,
}

fn selected_media_source(
    runtime: &JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> Result<Option<SelectedMediaSource>, ()> {
    let element = runtime
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::as_element)
        .ok_or(())?;
    if let Some(src) = element.attribute("src") {
        if src.trim().is_empty() {
            return Err(());
        }
        return parsed_url_like_attribute(runtime, handle, "src")
            .map(|request_url| {
                Some(SelectedMediaSource {
                    request_url,
                    original_input: src.to_owned(),
                })
            })
            .ok_or(());
    }
    for child in runtime.dom_host().child_handles(handle) {
        let Some(source) = runtime
            .dom_host()
            .node(child)
            .and_then(crate::dom::native::Node::as_element)
            .filter(|source| source.is_html_element("source"))
        else {
            continue;
        };
        let Some(src) = source.attribute("src") else {
            continue;
        };
        if src.trim().is_empty() {
            continue;
        }
        return parsed_url_like_attribute(runtime, child, "src")
            .map(|request_url| {
                Some(SelectedMediaSource {
                    request_url,
                    original_input: src.to_owned(),
                })
            })
            .ok_or(());
    }
    Ok(None)
}

pub(in crate::native_bridge) fn media_autoplay_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    media_boolean_attribute_setter(scope, args, rv, "autoplay");
}

pub(in crate::native_bridge) fn media_controls_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    media_boolean_attribute_getter(scope, args.this(), "controls", rv);
}

pub(in crate::native_bridge) fn media_controls_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    media_boolean_attribute_setter(scope, args, rv, "controls");
}

pub(in crate::native_bridge) fn media_default_muted_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    media_boolean_attribute_getter(scope, args.this(), "muted", rv);
}

pub(in crate::native_bridge) fn media_default_muted_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    media_boolean_attribute_setter(scope, args, rv, "muted");
}

pub(in crate::native_bridge) fn media_plays_inline_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = html_element_getter_receiver(
        scope,
        args.this(),
        "HTMLVideoElement",
        "playsInline",
        "video",
    ) else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(element_has_attribute(
        unsafe { &*runtime_ptr },
        handle,
        "playsinline",
    ));
}

pub(in crate::native_bridge) fn media_plays_inline_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = html_element_setter_receiver(
        scope,
        args.this(),
        "HTMLVideoElement",
        "playsInline",
        "video",
    ) else {
        rv.set_undefined();
        return;
    };
    set_reflected_boolean_attribute(
        scope,
        runtime_ptr,
        handle,
        "playsinline",
        args.get(0).boolean_value(scope),
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn media_loop_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    media_boolean_attribute_getter(scope, args.this(), "loop", rv);
}

pub(in crate::native_bridge) fn media_loop_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    media_boolean_attribute_setter(scope, args, rv, "loop");
}

fn media_boolean_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = html_media_element_getter_receiver(scope, object, name)
    else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(element_has_attribute(
        unsafe { &*runtime_ptr },
        handle,
        name,
    ));
}

fn media_boolean_attribute_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    name: &'static str,
) {
    let Some((runtime_ptr, handle)) = html_media_element_setter_receiver(scope, args.this(), name)
    else {
        rv.set_undefined();
        return;
    };
    set_reflected_boolean_attribute(
        scope,
        runtime_ptr,
        handle,
        name,
        args.get(0).boolean_value(scope),
    );
    rv.set_undefined();
}
