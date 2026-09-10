use crate::context_bootstrap::{
    dispatch_simple_event_target_event, mark_simple_event_target_slot, set_text_track_cue_track,
};
use crate::document_runtime::DomHandle;
use crate::dom::native::Node;
use crate::native_bridge::bridge::throw_dom_exception;
use crate::native_bridge::{
    JsContextHost, MediaLoadSequenceId, PendingMediaTextTrackGateRegistration,
    PendingTextTrackLoadTerminal, PendingTextTrackLoadTerminalFollowup, TextTrackLoadSequenceId,
};
use crate::page_task_queue::{RendererPageTextTrackLoadTaskId, RendererPageTextTrackLoadTaskKind};
use crate::util::{
    callback_arg_string, context_host_ptr_from_global_bridge, get_private_value,
    node_wrapper_from_handle, serialize_v8_array, set_private_value, throw_type_error, v8_string,
    v8str,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use super::super::super::node::node_runtime_and_handle_from_object_or_detached;
use super::super::{
    construct_event, construct_simple_event, dispatch_public_event, element_attribute,
    html_media_element_getter_receiver, html_media_element_method_receiver,
};

const TEXT_TRACK_CACHE_SLOT: &str = "__moliTextTrack";
const TEXT_TRACKS_CACHE_SLOT: &str = "__moliTextTracks";
const TEXT_TRACKS_MANUAL_SLOT: &str = "__moliTextTracksManual";
const TEXT_TRACK_LIST_LISTENERS_SLOT: &str = "__moliTextTrackListListeners";
const TEXT_TRACK_OWNER_HANDLE_SLOT: &str = "__moliTextTrackOwnerHandle";
const TEXT_TRACK_MEDIA_HANDLE_SLOT: &str = "__moliTextTrackMediaHandle";
const TEXT_TRACK_KIND_SLOT: &str = "__moliTextTrackKind";
const TEXT_TRACK_LABEL_SLOT: &str = "__moliTextTrackLabel";
const TEXT_TRACK_LANGUAGE_SLOT: &str = "__moliTextTrackLanguage";
const TEXT_TRACK_MODE_SLOT: &str = "__moliTextTrackMode";
const TEXT_TRACK_CUES_SLOT: &str = "__moliTextTrackCues";
const TEXT_TRACK_ACTIVE_CUES_SLOT: &str = "__moliTextTrackActiveCues";
const TEXT_TRACK_INSERTION_COUNTER_SLOT: &str = "__moliTextTrackInsertionCounter";
const TEXT_TRACK_CUE_ORDER_SLOT: &str = "__moliTextTrackCueOrder";
const TEXT_TRACK_LIST_ITEMS_SLOT: &str = "__moliTextTrackListItems";
const TEXT_TRACK_ONCUECHANGE_SLOT: &str = "__moliTextTrackOnCueChange";
const TEXT_TRACK_LIST_ONADDTRACK_SLOT: &str = "__moliTextTrackListOnAddTrack";
const TEXT_TRACK_LIST_ONREMOVETRACK_SLOT: &str = "__moliTextTrackListOnRemoveTrack";
const TRACK_READY_STATE_SLOT: &str = "__moliTrackReadyState";

const TRACK_READY_STATE_NONE: u32 = 0;
const TRACK_READY_STATE_LOADING: u32 = 1;
const TRACK_READY_STATE_LOADED: u32 = 2;
const TRACK_READY_STATE_ERROR: u32 = 3;

#[derive(WebApiObject)]
#[webapi(interface = "TextTrackCueList")]
struct TextTrackCueListObjectDeclaration<'scope> {
    #[webapi(slot = TEXT_TRACK_LIST_ITEMS_SLOT)]
    items: Vec<v8::Local<'scope, v8::Value>>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "TextTrackCueList", enumerable)]
struct TextTrackCueListTemplateDeclaration {
    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),

    #[webapi(accessor_property, getter = text_track_cue_list_length_getter)]
    length: (),

    #[webapi(
        method,
        length = 1,
        enumerable,
        callback = text_track_cue_list_get_cue_by_id_callback
    )]
    get_cue_by_id: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "TextTrackList")]
struct TextTrackListObjectDeclaration<'scope> {
    #[webapi(slot = TEXT_TRACK_LIST_ITEMS_SLOT)]
    items: Vec<v8::Local<'scope, v8::Value>>,

    #[webapi(slot = TEXT_TRACK_LIST_ONADDTRACK_SLOT, init = "null")]
    _onaddtrack: (),

    #[webapi(slot = TEXT_TRACK_LIST_ONREMOVETRACK_SLOT, init = "null")]
    _onremovetrack: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "TextTrackList", enumerable)]
struct TextTrackListTemplateDeclaration {
    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),

    #[webapi(accessor_property, getter = text_track_list_length_getter)]
    length: (),

    #[webapi(
        accessor_property,
        getter = text_track_list_onaddtrack_getter,
        setter = text_track_list_onaddtrack_setter
    )]
    onaddtrack: (),

    #[webapi(
        accessor_property,
        getter = text_track_list_onremovetrack_getter,
        setter = text_track_list_onremovetrack_setter
    )]
    onremovetrack: (),

    #[webapi(
        method,
        length = 1,
        enumerable,
        callback = text_track_list_get_track_by_id_callback
    )]
    get_track_by_id: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "TextTrack")]
struct TextTrackObjectDeclaration {
    #[webapi(slot = TEXT_TRACK_KIND_SLOT)]
    kind: String,

    #[webapi(slot = TEXT_TRACK_LABEL_SLOT)]
    label: String,

    #[webapi(slot = TEXT_TRACK_LANGUAGE_SLOT)]
    language: String,

    #[webapi(slot = TEXT_TRACK_MODE_SLOT)]
    mode: String,

    #[webapi(slot = TEXT_TRACK_INSERTION_COUNTER_SLOT)]
    insertion_counter: f64,

    #[webapi(slot = TEXT_TRACK_OWNER_HANDLE_SLOT)]
    owner_handle: Option<u32>,

    #[webapi(slot = TEXT_TRACK_MEDIA_HANDLE_SLOT)]
    media_handle: Option<u32>,

    #[webapi(slot = TEXT_TRACK_ONCUECHANGE_SLOT, init = "null")]
    _oncuechange: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "TextTrack", enumerable)]
struct TextTrackTemplateDeclaration {
    #[webapi(accessor_property, getter = text_track_kind_getter)]
    kind: (),

    #[webapi(accessor_property, getter = text_track_id_getter)]
    id: (),

    #[webapi(accessor_property, getter = text_track_label_getter)]
    label: (),

    #[webapi(accessor_property, getter = text_track_language_getter)]
    language: (),

    #[webapi(
        accessor_property,
        getter = text_track_mode_getter,
        setter = text_track_mode_setter
    )]
    mode: (),

    #[webapi(accessor_property, getter = text_track_cues_getter)]
    cues: (),

    #[webapi(accessor_property = "activeCues", getter = text_track_active_cues_getter)]
    active_cues: (),

    #[webapi(
        accessor_property,
        getter = text_track_oncuechange_getter,
        setter = text_track_oncuechange_setter
    )]
    oncuechange: (),

    #[webapi(method, length = 1, enumerable, callback = text_track_add_cue_callback)]
    add_cue: (),

    #[webapi(method, length = 1, enumerable, callback = text_track_remove_cue_callback)]
    remove_cue: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct TrackEventInitDeclaration<'scope> {
    track: v8::Local<'scope, v8::Object>,
}

pub(crate) fn install_text_track_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "TextTrack" => {
            TextTrackTemplateDeclaration::initialize_prototype_template(scope, prototype)
        }
        "TextTrackList" => {
            TextTrackListTemplateDeclaration::initialize_prototype_template(scope, prototype)
        }
        "TextTrackCueList" => {
            TextTrackCueListTemplateDeclaration::initialize_prototype_template(scope, prototype)
        }
        _ => {}
    }
}

fn text_track_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let receiver = args.this();
    if moli_webapi_declare::implements_interface(scope, receiver, "TextTrack") {
        return Some(receiver);
    }
    throw_type_error(scope, "Illegal invocation");
    None
}

fn text_track_list_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let receiver = args.this();
    if moli_webapi_declare::implements_interface(scope, receiver, "TextTrackList") {
        return Some(receiver);
    }
    throw_type_error(scope, "Illegal invocation");
    None
}

fn text_track_cue_list_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let receiver = args.this();
    if moli_webapi_declare::implements_interface(scope, receiver, "TextTrackCueList") {
        return Some(receiver);
    }
    throw_type_error(scope, "Illegal invocation");
    None
}

fn valid_text_track_kind(value: &str) -> bool {
    matches!(
        value,
        "subtitles" | "captions" | "descriptions" | "chapters" | "metadata"
    )
}

fn canonical_text_track_kind(value: &str) -> &'static str {
    for keyword in [
        "subtitles",
        "captions",
        "descriptions",
        "chapters",
        "metadata",
    ] {
        if value.eq_ignore_ascii_case(keyword) {
            return keyword;
        }
    }
    "metadata"
}

fn private_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<String> {
    get_private_value(scope, object, slot)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn set_private_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
    value: &str,
) {
    if let Some(value) = v8_string(scope, value) {
        set_private_value(scope, object, slot, value.into());
    }
}

fn private_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<f64> {
    get_private_value(scope, object, slot).and_then(|value| value.number_value(scope))
}

fn set_private_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
    value: f64,
) {
    set_private_value(scope, object, slot, v8::Number::new(scope, value).into());
}

fn owner_track_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    track: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    private_dom_handle(scope, track, TEXT_TRACK_OWNER_HANDLE_SLOT)
}

fn private_dom_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<DomHandle> {
    let raw = get_private_value(scope, object, slot)?.uint32_value(scope)?;
    Some(DomHandle::new(raw as usize))
}

fn owner_track_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    track: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<String> {
    let handle = owner_track_handle(scope, track)?;
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    element_attribute(unsafe { &*runtime_ptr }, handle, name)
}

fn owner_track_attribute_or_default<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    track: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<String> {
    let handle = owner_track_handle(scope, track)?;
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    Some(element_attribute(unsafe { &*runtime_ptr }, handle, name).unwrap_or_default())
}

fn make_cue_list<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Object>> {
    let list = TextTrackCueListObjectDeclaration::new(Vec::new())
        .bind(scope)
        .ok()?;
    update_indexed_list(scope, list, &[]);
    Some(list)
}

fn make_text_track_list<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Object>> {
    let list = TextTrackListObjectDeclaration::new(Vec::new())
        .bind(scope)
        .ok()?;
    mark_simple_event_target_slot(scope, list, TEXT_TRACK_LIST_LISTENERS_SLOT);
    update_indexed_list(scope, list, &[]);
    Some(list)
}

fn indexed_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Value>> {
    let Some(items) = get_private_value(scope, list, TEXT_TRACK_LIST_ITEMS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        return Vec::new();
    };
    (0..items.length())
        .filter_map(|index| items.get_index(scope, index))
        .collect()
}

fn text_track_list_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_list_receiver(scope, &args) else {
        return;
    };
    let length = get_private_value(scope, receiver, TEXT_TRACK_LIST_ITEMS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .map_or(0, |items| items.length());
    rv.set_uint32(length);
}

fn text_track_cue_list_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_list_receiver(scope, &args) else {
        return;
    };
    let length = get_private_value(scope, receiver, TEXT_TRACK_LIST_ITEMS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .map_or(0, |items| items.length());
    rv.set_uint32(length);
}

fn text_track_list_onaddtrack_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_list_receiver(scope, &args) else {
        return;
    };
    text_track_event_handler_getter(scope, receiver, rv, TEXT_TRACK_LIST_ONADDTRACK_SLOT);
}

fn text_track_list_onaddtrack_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_list_receiver(scope, &args) else {
        return;
    };
    text_track_event_handler_setter(
        scope,
        receiver,
        args.get(0),
        TEXT_TRACK_LIST_ONADDTRACK_SLOT,
    );
}

fn text_track_list_onremovetrack_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_list_receiver(scope, &args) else {
        return;
    };
    text_track_event_handler_getter(scope, receiver, rv, TEXT_TRACK_LIST_ONREMOVETRACK_SLOT);
}

fn text_track_list_onremovetrack_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_list_receiver(scope, &args) else {
        return;
    };
    text_track_event_handler_setter(
        scope,
        receiver,
        args.get(0),
        TEXT_TRACK_LIST_ONREMOVETRACK_SLOT,
    );
}

fn update_indexed_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    items: &[v8::Local<'s, v8::Value>],
) {
    let old_len = list
        .get(scope, v8str(scope, "length").into())
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    for index in 0..old_len.max(items.len() as u32) {
        let Some(key) = v8_string(scope, &index.to_string()) else {
            continue;
        };
        let _ = list.delete(scope, key.into());
    }
    let array = serialize_v8_array(scope, items).unwrap_or_else(|| v8::Array::new(scope, 0));
    for (index, item) in items.iter().enumerate() {
        let Some(key) = v8_string(scope, &index.to_string()) else {
            continue;
        };
        let _ =
            list.define_own_property(scope, key.into(), *item, v8::PropertyAttribute::READ_ONLY);
    }
    if let Some(key) = v8_string(scope, &items.len().to_string()) {
        let _ = list.define_own_property(
            scope,
            key.into(),
            v8::undefined(scope).into(),
            v8::PropertyAttribute::READ_ONLY,
        );
    }
    set_private_value(scope, list, TEXT_TRACK_LIST_ITEMS_SLOT, array.into());
}

fn make_text_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: &str,
    label: &str,
    language: &str,
    mode: &str,
    owner: Option<DomHandle>,
    media_owner: Option<DomHandle>,
) -> Option<v8::Local<'s, v8::Object>> {
    let owner_handle = owner.map(DomHandle::index_u32);
    let media_handle = media_owner.map(DomHandle::index_u32);
    let track = TextTrackObjectDeclaration::new(
        kind.to_owned(),
        label.to_owned(),
        language.to_owned(),
        mode.to_owned(),
        0.0,
        owner_handle,
        media_handle,
    )
    .bind(scope)
    .ok()?;
    mark_simple_event_target_slot(scope, track, "__moliTextTrackListeners");
    let cues = make_cue_list(scope)?;
    let active_cues = make_cue_list(scope)?;
    set_private_value(scope, track, TEXT_TRACK_CUES_SLOT, cues.into());
    set_private_value(
        scope,
        track,
        TEXT_TRACK_ACTIVE_CUES_SLOT,
        active_cues.into(),
    );
    Some(track)
}

pub(in crate::native_bridge) fn track_text_track_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let holder = args.this();
    if let Some(cached) = get_private_value(scope, holder, TEXT_TRACK_CACHE_SLOT) {
        rv.set(cached);
        return;
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, holder)
    else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let Some(track) = ensure_track_element_text_track(scope, runtime, holder, handle) else {
        rv.set_undefined();
        return;
    };
    rv.set(track.into());
}

fn ensure_track_element_text_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime: &JsContextHost,
    wrapper: v8::Local<'s, v8::Object>,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(cached) = get_private_value(scope, wrapper, TEXT_TRACK_CACHE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Some(cached);
    }
    let kind = element_attribute(runtime, handle, "kind")
        .map(|value| canonical_text_track_kind(&value).to_owned())
        .unwrap_or_else(|| "subtitles".to_owned());
    let label = element_attribute(runtime, handle, "label").unwrap_or_default();
    let language = element_attribute(runtime, handle, "srclang").unwrap_or_default();
    let track = make_text_track(
        scope,
        &kind,
        &label,
        &language,
        "disabled",
        Some(handle),
        track_media_parent(runtime, handle),
    )?;
    set_private_value(scope, wrapper, TEXT_TRACK_CACHE_SLOT, track.into());
    Some(track)
}

pub(in crate::native_bridge) fn media_text_tracks_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let holder = args.this();
    if html_media_element_getter_receiver(scope, holder, "textTracks").is_none() {
        rv.set_undefined();
        return;
    }
    let list = if let Some(cached) = get_private_value(scope, holder, TEXT_TRACKS_CACHE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        cached
    } else {
        let Some(list) = make_text_track_list(scope) else {
            rv.set_undefined();
            return;
        };
        set_private_value(scope, holder, TEXT_TRACKS_CACHE_SLOT, list.into());
        list
    };
    update_media_text_track_list(scope, holder, list);
    rv.set(list.into());
}

pub(in crate::native_bridge) fn media_add_text_track_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((_, media_handle)) =
        html_media_element_method_receiver(scope, args.this(), "addTextTrack")
    else {
        rv.set_undefined();
        return;
    };
    let Some(kind) = callback_arg_string(scope, &args, 0) else {
        return;
    };
    if !valid_text_track_kind(&kind) {
        throw_type_error(
            scope,
            "Failed to execute 'addTextTrack' on 'HTMLMediaElement': invalid TextTrack kind.",
        );
        return;
    }
    let label = optional_callback_string(scope, &args, 1);
    let language = optional_callback_string(scope, &args, 2);
    let Some(track) = make_text_track(
        scope,
        &kind,
        &label,
        &language,
        "hidden",
        None,
        Some(media_handle),
    ) else {
        rv.set_undefined();
        return;
    };
    let media = args.this();
    let manual = manual_text_track_array(scope, media);
    let index = manual.length();
    let _ = manual.set_index(scope, index, track.into());
    if let Some(list) = get_private_value(scope, media, TEXT_TRACKS_CACHE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        update_media_text_track_list(scope, media, list);
    }
    rv.set(track.into());
}

fn optional_callback_string(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
    index: i32,
) -> String {
    let value = args.get(index);
    if value.is_undefined() {
        String::new()
    } else {
        callback_arg_string(scope, args, index).unwrap_or_default()
    }
}

fn manual_text_track_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    media: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    if let Some(array) = get_private_value(scope, media, TEXT_TRACKS_MANUAL_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    {
        return array;
    }
    let array = v8::Array::new(scope, 0);
    set_private_value(scope, media, TEXT_TRACKS_MANUAL_SLOT, array.into());
    array
}

fn update_media_text_track_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    media: v8::Local<'s, v8::Object>,
    list: v8::Local<'s, v8::Object>,
) {
    let previous_items = indexed_items(scope, list);
    let mut items = Vec::new();
    let media_target = node_runtime_and_handle_from_object_or_detached(scope, media).ok();
    if let Some((runtime_ptr, handle)) = media_target {
        let runtime = unsafe { &*runtime_ptr };
        for child in runtime.dom_host().child_handles(handle) {
            if !runtime.dom_host().is_html_element_named(child, "track") {
                continue;
            }
            let Some(wrapper) = node_wrapper_from_handle(scope, child) else {
                continue;
            };
            let track = ensure_track_element_text_track(scope, runtime, wrapper, child)
                .map(|track| track.into());
            if let Some(track) = track {
                items.push(track);
            }
            queue_text_track_load_if_needed(scope, runtime_ptr, child);
        }
    }
    let manual = manual_text_track_array(scope, media);
    for index in 0..manual.length() {
        if let Some(track) = manual.get_index(scope, index) {
            items.push(track);
        }
    }
    update_indexed_list(scope, list, &items);
    for item in items {
        if previous_items
            .iter()
            .any(|previous| previous.strict_equals(item))
        {
            continue;
        }
        if let Ok(track) = v8::Local::<v8::Object>::try_from(item)
            && let Some((runtime_ptr, media_handle)) = media_target
        {
            queue_text_track_list_track_event(
                scope,
                runtime_ptr,
                media_handle,
                list,
                track,
                "addtrack",
            );
        }
    }
}

fn text_track_kind_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_receiver(scope, &args) else {
        return;
    };
    let value = owner_track_attribute(scope, receiver, "kind")
        .map(|value| canonical_text_track_kind(&value).to_owned())
        .or_else(|| private_string(scope, receiver, TEXT_TRACK_KIND_SLOT))
        .unwrap_or_else(|| "subtitles".to_owned());
    rv.set(
        v8_string(scope, &value)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
}

fn text_track_id_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_receiver(scope, &args) else {
        return;
    };
    let value = owner_track_attribute(scope, receiver, "id").unwrap_or_default();
    rv.set(
        v8_string(scope, &value)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
}

fn text_track_label_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_receiver(scope, &args) else {
        return;
    };
    let value = owner_track_attribute_or_default(scope, receiver, "label")
        .or_else(|| private_string(scope, receiver, TEXT_TRACK_LABEL_SLOT))
        .unwrap_or_default();
    rv.set(
        v8_string(scope, &value)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
}

fn text_track_language_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_receiver(scope, &args) else {
        return;
    };
    let value = owner_track_attribute_or_default(scope, receiver, "srclang")
        .or_else(|| private_string(scope, receiver, TEXT_TRACK_LANGUAGE_SLOT))
        .unwrap_or_default();
    rv.set(
        v8_string(scope, &value)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
}

fn text_track_mode_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_receiver(scope, &args) else {
        return;
    };
    let value = private_string(scope, receiver, TEXT_TRACK_MODE_SLOT)
        .unwrap_or_else(|| "disabled".to_owned());
    rv.set(
        v8_string(scope, &value)
            .unwrap_or_else(|| v8str(scope, "disabled"))
            .into(),
    );
}

fn text_track_mode_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_receiver(scope, &args) else {
        return;
    };
    let Some(value) = args
        .get(0)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        return;
    };
    if matches!(value.as_str(), "disabled" | "hidden" | "showing") {
        set_private_string(scope, receiver, TEXT_TRACK_MODE_SLOT, &value);
        refresh_text_track_active_cues(scope, receiver);
        if value != "disabled"
            && let Some(handle) = owner_track_handle(scope, receiver)
            && let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        {
            queue_text_track_load_if_needed(scope, runtime_ptr, handle);
        }
    }
}

fn text_track_cues_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_receiver(scope, &args) else {
        return;
    };
    if private_string(scope, receiver, TEXT_TRACK_MODE_SLOT).as_deref() == Some("disabled") {
        rv.set_null();
        return;
    }
    rv.set(
        get_private_value(scope, receiver, TEXT_TRACK_CUES_SLOT)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn text_track_active_cues_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_receiver(scope, &args) else {
        return;
    };
    if private_string(scope, receiver, TEXT_TRACK_MODE_SLOT).as_deref() == Some("disabled") {
        rv.set_null();
        return;
    }
    rv.set(
        get_private_value(scope, receiver, TEXT_TRACK_ACTIVE_CUES_SLOT)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn text_track_oncuechange_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_receiver(scope, &args) else {
        return;
    };
    text_track_event_handler_getter(scope, receiver, rv, TEXT_TRACK_ONCUECHANGE_SLOT);
}

fn text_track_oncuechange_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_receiver(scope, &args) else {
        return;
    };
    text_track_event_handler_setter(scope, receiver, args.get(0), TEXT_TRACK_ONCUECHANGE_SLOT);
}

fn text_track_event_handler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    slot: &'static str,
) {
    rv.set(get_private_value(scope, receiver, slot).unwrap_or_else(|| v8::null(scope).into()));
}

fn text_track_event_handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    slot: &'static str,
) {
    set_private_value(
        scope,
        receiver,
        slot,
        if value.is_function() {
            value
        } else {
            v8::null(scope).into()
        },
    );
}

fn text_track_add_cue_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(track) = text_track_receiver(scope, &args) else {
        return;
    };
    let Ok(cue) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_type_error(
            scope,
            "Failed to execute 'addCue' on 'TextTrack': cue is required.",
        );
        return;
    };
    if let Some(previous_track) = cue
        .get(scope, v8str(scope, "track").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        remove_cue_from_track(scope, previous_track, cue);
    }
    add_cue_to_track(scope, track, cue);
    set_text_track_cue_track(scope, cue, track.into());
    refresh_text_track_active_cues(scope, track);
}

fn text_track_remove_cue_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(track) = text_track_receiver(scope, &args) else {
        return;
    };
    let Ok(cue) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_dom_exception(scope, "NotFoundError", 8, "The cue was not found.");
        return;
    };
    if !remove_cue_from_track(scope, track, cue) {
        throw_dom_exception(scope, "NotFoundError", 8, "The cue was not found.");
        return;
    }
    set_text_track_cue_track(scope, cue, v8::null(scope).into());
    refresh_text_track_active_cues(scope, track);
}

fn add_cue_to_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    track: v8::Local<'s, v8::Object>,
    cue: v8::Local<'s, v8::Object>,
) {
    let Some(cues) = get_private_value(scope, track, TEXT_TRACK_CUES_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let mut items = indexed_items(scope, cues);
    if !items.iter().any(|item| item.strict_equals(cue.into())) {
        let order = next_text_track_cue_order(scope, track);
        set_private_number(scope, cue, TEXT_TRACK_CUE_ORDER_SLOT, order);
        items.push(cue.into());
        items = sorted_cue_items(scope, items);
    }
    update_indexed_list(scope, cues, &items);
}

fn next_text_track_cue_order<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    track: v8::Local<'s, v8::Object>,
) -> f64 {
    let value = private_number(scope, track, TEXT_TRACK_INSERTION_COUNTER_SLOT).unwrap_or(0.0);
    set_private_number(scope, track, TEXT_TRACK_INSERTION_COUNTER_SLOT, value + 1.0);
    value
}

fn sorted_cue_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    items: Vec<v8::Local<'s, v8::Value>>,
) -> Vec<v8::Local<'s, v8::Value>> {
    let mut keyed = items
        .into_iter()
        .map(|item| (cue_sort_key(scope, item), item))
        .collect::<Vec<_>>();
    keyed.sort_by(|left, right| {
        left.0
            .0
            .total_cmp(&right.0.0)
            .then_with(|| right.0.1.total_cmp(&left.0.1))
            .then_with(|| left.0.2.total_cmp(&right.0.2))
    });
    keyed.into_iter().map(|(_, item)| item).collect()
}

pub(crate) fn resort_text_track_cues_for_cue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cue: v8::Local<'s, v8::Object>,
) {
    let Some(track) = cue
        .get(scope, v8str(scope, "track").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(cues) = get_private_value(scope, track, TEXT_TRACK_CUES_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let items = indexed_items(scope, cues);
    let items = sorted_cue_items(scope, items);
    update_indexed_list(scope, cues, &items);
    refresh_text_track_active_cues(scope, track);
}

fn cue_sort_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cue: v8::Local<'s, v8::Value>,
) -> (f64, f64, f64) {
    let Ok(cue) = v8::Local::<v8::Object>::try_from(cue) else {
        return (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY);
    };
    let start = cue
        .get(scope, v8str(scope, "startTime").into())
        .and_then(|value| value.number_value(scope))
        .unwrap_or(f64::INFINITY);
    let end = cue
        .get(scope, v8str(scope, "endTime").into())
        .and_then(|value| value.number_value(scope))
        .unwrap_or(f64::NEG_INFINITY);
    let order = private_number(scope, cue, TEXT_TRACK_CUE_ORDER_SLOT).unwrap_or(f64::INFINITY);
    (start, end, order)
}

fn remove_cue_from_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    track: v8::Local<'s, v8::Object>,
    cue: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(cues) = get_private_value(scope, track, TEXT_TRACK_CUES_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let mut removed = false;
    let items = indexed_items(scope, cues)
        .into_iter()
        .filter(|item| {
            let keep = !item.strict_equals(cue.into());
            removed |= !keep;
            keep
        })
        .collect::<Vec<_>>();
    if removed {
        update_indexed_list(scope, cues, &items);
    }
    removed
}

pub(in crate::native_bridge) fn refresh_media_active_text_track_cues(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    media_handle: DomHandle,
) {
    let Some(media) = node_wrapper_from_handle(scope, media_handle) else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    for child in runtime.dom_host().child_handles(media_handle) {
        if !runtime.dom_host().is_html_element_named(child, "track") {
            continue;
        }
        let Some(wrapper) = node_wrapper_from_handle(scope, child) else {
            continue;
        };
        if let Some(track) = ensure_track_element_text_track(scope, runtime, wrapper, child) {
            refresh_text_track_active_cues_with_runtime(scope, runtime, track);
        }
    }
    let manual = manual_text_track_array(scope, media);
    for index in 0..manual.length() {
        let Some(track) = manual
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        refresh_text_track_active_cues_with_runtime(scope, runtime, track);
    }
}

fn refresh_text_track_active_cues<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    track: v8::Local<'s, v8::Object>,
) {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    refresh_text_track_active_cues_with_runtime(scope, unsafe { &*runtime_ptr }, track);
}

fn refresh_text_track_active_cues_with_runtime<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime: &JsContextHost,
    track: v8::Local<'s, v8::Object>,
) {
    let Some(active_cues) = get_private_value(scope, track, TEXT_TRACK_ACTIVE_CUES_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(media_handle) = text_track_media_handle(scope, runtime, track) else {
        update_indexed_list(scope, active_cues, &[]);
        return;
    };
    let Some(media) = runtime
        .dom_host()
        .node(media_handle)
        .and_then(Node::as_element)
    else {
        update_indexed_list(scope, active_cues, &[]);
        return;
    };
    if private_string(scope, track, TEXT_TRACK_MODE_SLOT).as_deref() == Some("disabled")
        || media.media_paused()
    {
        update_indexed_list(scope, active_cues, &[]);
        return;
    }
    let current_time = media.media_current_time();
    let Some(cues) = get_private_value(scope, track, TEXT_TRACK_CUES_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        update_indexed_list(scope, active_cues, &[]);
        return;
    };
    let items = indexed_items(scope, cues)
        .into_iter()
        .filter(|cue| cue_is_active_at(scope, *cue, current_time))
        .collect::<Vec<_>>();
    update_indexed_list(scope, active_cues, &items);
}

fn text_track_media_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime: &JsContextHost,
    track: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    if let Some(handle) = private_dom_handle(scope, track, TEXT_TRACK_MEDIA_HANDLE_SLOT) {
        return Some(handle);
    }
    owner_track_handle(scope, track).and_then(|owner| track_media_parent(runtime, owner))
}

fn cue_is_active_at<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cue: v8::Local<'s, v8::Value>,
    current_time: f64,
) -> bool {
    let Ok(cue) = v8::Local::<v8::Object>::try_from(cue) else {
        return false;
    };
    let start = cue
        .get(scope, v8str(scope, "startTime").into())
        .and_then(|value| value.number_value(scope))
        .unwrap_or(f64::INFINITY);
    let end = cue
        .get(scope, v8str(scope, "endTime").into())
        .and_then(|value| value.number_value(scope))
        .unwrap_or(f64::NEG_INFINITY);
    start <= current_time && current_time < end
}

fn text_track_cue_list_get_cue_by_id_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(list) = text_track_cue_list_receiver(scope, &args) else {
        return;
    };
    let Some(id) = callback_arg_string(scope, &args, 0) else {
        rv.set_null();
        return;
    };
    if id.is_empty() {
        rv.set_null();
        return;
    }
    for item in indexed_items(scope, list) {
        let Ok(cue) = v8::Local::<v8::Object>::try_from(item) else {
            continue;
        };
        let cue_id = cue
            .get(scope, v8str(scope, "id").into())
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_default();
        if cue_id == id {
            rv.set(cue.into());
            return;
        }
    }
    rv.set_null();
}

fn text_track_list_get_track_by_id_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(list) = text_track_list_receiver(scope, &args) else {
        return;
    };
    let Some(id) = callback_arg_string(scope, &args, 0) else {
        rv.set_null();
        return;
    };
    for item in indexed_items(scope, list) {
        let Ok(track) = v8::Local::<v8::Object>::try_from(item) else {
            continue;
        };
        let track_id = track
            .get(scope, v8str(scope, "id").into())
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_default();
        if track_id == id {
            rv.set(track.into());
            return;
        }
    }
    rv.set_null();
}

pub(in crate::native_bridge) fn apply_default_text_track_modes_for_media(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    media_handle: DomHandle,
) {
    let runtime = unsafe { &*runtime_ptr };
    let handles = runtime
        .dom_host()
        .child_handles(media_handle)
        .filter(|handle| runtime.dom_host().is_html_element_named(*handle, "track"))
        .collect::<Vec<_>>();
    for handle in handles {
        apply_default_text_track_mode_for_track(scope, runtime_ptr, handle);
    }
}

pub(crate) fn queue_default_text_track_mode_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    let runtime = unsafe { &*runtime_ptr };
    if !track_has_media_parent(runtime, handle)
        || element_attribute(runtime, handle, "default").is_none()
    {
        return;
    }
    let _ = unsafe { &mut *runtime_ptr }.queue_text_track_default_mode_task(scope, handle);
}

pub(crate) fn apply_default_text_track_mode_for_track(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    if !track_has_media_parent(runtime, handle)
        || element_attribute(runtime, handle, "default").is_none()
    {
        return false;
    }
    let Some(wrapper) = node_wrapper_from_handle(scope, handle) else {
        return false;
    };
    let Some(track) = ensure_track_element_text_track(scope, runtime, wrapper, handle) else {
        return false;
    };
    if private_string(scope, track, TEXT_TRACK_MODE_SLOT).as_deref() != Some("disabled") {
        return false;
    }
    set_private_string(scope, track, TEXT_TRACK_MODE_SLOT, "showing");
    refresh_text_track_active_cues(scope, track);
    queue_text_track_load_if_needed(scope, runtime_ptr, handle);
    true
}

pub(crate) fn queue_text_track_load_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    queue_text_track_load_with_media_gate(scope, runtime_ptr, handle, None);
}

pub(crate) fn queue_media_selection_text_track_loads(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    media_handle: DomHandle,
    media_sequence: MediaLoadSequenceId,
) {
    let track_handles = unsafe { &*runtime_ptr }
        .dom_host()
        .child_handles(media_handle)
        .filter(|handle| {
            unsafe { &*runtime_ptr }
                .dom_host()
                .is_html_element_named(*handle, "track")
        })
        .collect::<Vec<_>>();
    for track_handle in track_handles {
        let Some(wrapper) = node_wrapper_from_handle(scope, track_handle) else {
            continue;
        };
        let Some(track) =
            ensure_track_element_text_track(scope, unsafe { &*runtime_ptr }, wrapper, track_handle)
        else {
            continue;
        };
        if private_string(scope, track, TEXT_TRACK_MODE_SLOT).as_deref() == Some("disabled") {
            continue;
        }
        queue_text_track_load_with_media_gate(
            scope,
            runtime_ptr,
            track_handle,
            Some((media_handle, media_sequence)),
        );
    }
}

fn queue_text_track_load_with_media_gate(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    media_gate: Option<(DomHandle, MediaLoadSequenceId)>,
) {
    let runtime = unsafe { &*runtime_ptr };
    if !track_has_media_parent(runtime, handle) {
        return;
    }
    let src = element_attribute(runtime, handle, "src").unwrap_or_default();
    let Some(wrapper) = node_wrapper_from_handle(scope, handle) else {
        return;
    };
    let ready_state = track_ready_state(scope, wrapper);
    if matches!(
        ready_state,
        TRACK_READY_STATE_LOADED | TRACK_READY_STATE_ERROR
    ) {
        return;
    }
    let Some(track) = ensure_track_element_text_track(scope, runtime, wrapper, handle) else {
        return;
    };
    if private_string(scope, track, TEXT_TRACK_MODE_SLOT).as_deref() == Some("disabled") {
        return;
    }
    if let Some(existing) = unsafe { &*runtime_ptr }.pending_text_track_load_sequence(handle)
        && unsafe { &*runtime_ptr }
            .pending_text_track_load_sequence_is_current(handle, existing.id())
        && existing.source() == src
    {
        if let Some((media_handle, media_sequence)) = media_gate {
            register_media_text_track_gate(
                scope,
                runtime_ptr,
                media_handle,
                media_sequence,
                handle,
            );
        }
        return;
    }
    if ready_state == TRACK_READY_STATE_LOADING {
        reset_track_ready_state(scope, handle);
    }
    let _ = unsafe { &mut *runtime_ptr }.cancel_pending_text_track_load_sequence(handle, false);
    let Some(pending) =
        unsafe { &mut *runtime_ptr }.register_pending_text_track_load_sequence(handle, src.clone())
    else {
        return;
    };
    if let Some((media_handle, media_sequence)) = media_gate {
        register_media_text_track_gate(scope, runtime_ptr, media_handle, media_sequence, handle);
    }
    if !queue_track_load_start(runtime_ptr, handle, pending.id()) {
        cancel_text_track_sequence_and_settle_media(scope, runtime_ptr, handle);
    }
}

fn register_media_text_track_gate(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    media_handle: DomHandle,
    media_sequence: MediaLoadSequenceId,
    track_handle: DomHandle,
) {
    let registration: Option<PendingMediaTextTrackGateRegistration> = unsafe { &mut *runtime_ptr }
        .register_pending_media_text_track_gate(media_handle, media_sequence, track_handle);
    let followup = registration.and_then(|registration| registration.displaced_canplay_followup());
    super::queue_media_canplay_after_text_tracks(scope, runtime_ptr, followup);
}

fn queue_track_load_start(
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    sequence: TextTrackLoadSequenceId,
) -> bool {
    unsafe { &*runtime_ptr }
        .send_text_track_load_task(
            RendererPageTextTrackLoadTaskId::new(handle, sequence),
            RendererPageTextTrackLoadTaskKind::Start,
        )
        .is_ok()
}

fn apply_track_load_start(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    task_id: RendererPageTextTrackLoadTaskId,
) -> bool {
    let handle = task_id.track();
    let sequence = task_id.sequence();
    let runtime = unsafe { &*runtime_ptr };
    let Some(src) = runtime
        .pending_text_track_load_sequence(handle)
        .filter(|pending| pending.id() == sequence)
        .map(|pending| pending.source().to_owned())
    else {
        return false;
    };
    if !runtime.pending_text_track_load_sequence_is_current(handle, sequence)
        || !track_source_still_matches(runtime, handle, &src)
        || !track_has_media_parent(runtime, handle)
    {
        let followup =
            unsafe { &mut *runtime_ptr }.cancel_pending_text_track_load_sequence(handle, true);
        super::queue_media_canplay_after_text_tracks(scope, runtime_ptr, followup);
        return false;
    }
    let Some(wrapper) = node_wrapper_from_handle(scope, handle) else {
        cancel_text_track_sequence_and_settle_media(scope, runtime_ptr, handle);
        return false;
    };
    if track_ready_state(scope, wrapper) != TRACK_READY_STATE_NONE {
        cancel_text_track_sequence_and_settle_media(scope, runtime_ptr, handle);
        return false;
    }
    if ensure_track_element_text_track(scope, runtime, wrapper, handle).is_none() {
        cancel_text_track_sequence_and_settle_media(scope, runtime_ptr, handle);
        return false;
    }
    set_track_ready_state(scope, wrapper, TRACK_READY_STATE_LOADING);
    let start = crate::network_host::start_text_track_resource_fetch(
        scope,
        unsafe { &mut *runtime_ptr },
        handle,
        sequence,
    );
    match start {
        Ok(crate::network_host::TextTrackResourceFetchStart::Pending) => {}
        Ok(crate::network_host::TextTrackResourceFetchStart::PolicySkipped) => {
            let followup = unsafe { &mut *runtime_ptr }
                .complete_pending_text_track_local_if_matches(handle, sequence, Ok(String::new()));
            queue_text_track_terminal_followup(scope, runtime_ptr, handle, sequence, followup);
        }
        Ok(crate::network_host::TextTrackResourceFetchStart::Local(result)) => {
            let followup = unsafe { &mut *runtime_ptr }
                .complete_pending_text_track_local_if_matches(handle, sequence, result);
            queue_text_track_terminal_followup(scope, runtime_ptr, handle, sequence, followup);
        }
        Err(error) => {
            let followup = unsafe { &mut *runtime_ptr }
                .complete_pending_text_track_local_if_matches(handle, sequence, Err(error));
            queue_text_track_terminal_followup(scope, runtime_ptr, handle, sequence, followup);
        }
    }
    true
}

pub(crate) fn queue_text_track_terminal_followup(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    sequence: TextTrackLoadSequenceId,
    followup: Option<PendingTextTrackLoadTerminalFollowup>,
) {
    let kind = match followup {
        Some(PendingTextTrackLoadTerminalFollowup::Ready) => {
            RendererPageTextTrackLoadTaskKind::NetworkTerminal
        }
        Some(PendingTextTrackLoadTerminalFollowup::FetchFailed) => {
            RendererPageTextTrackLoadTaskKind::FetchFailureTerminal
        }
        None => return,
    };
    let task_id = RendererPageTextTrackLoadTaskId::new(handle, sequence);
    if unsafe { &*runtime_ptr }
        .send_text_track_load_task(task_id, kind)
        .is_err()
    {
        let media_followup =
            unsafe { &mut *runtime_ptr }.cancel_pending_text_track_load_sequence(handle, true);
        super::queue_media_canplay_after_text_tracks(scope, runtime_ptr, media_followup);
    }
}

fn apply_track_terminal(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    task_id: RendererPageTextTrackLoadTaskId,
    kind: RendererPageTextTrackLoadTaskKind,
) -> bool {
    let handle = task_id.track();
    let sequence = task_id.sequence();
    if !unsafe { &*runtime_ptr }.pending_text_track_load_sequence_is_current(handle, sequence) {
        let media_followup =
            unsafe { &mut *runtime_ptr }.cancel_pending_text_track_load_sequence(handle, true);
        super::queue_media_canplay_after_text_tracks(scope, runtime_ptr, media_followup);
        return false;
    }
    let Some(terminal) =
        unsafe { &mut *runtime_ptr }.take_pending_text_track_terminal_if_matches(handle, sequence)
    else {
        return false;
    };
    let Some(wrapper) = node_wrapper_from_handle(scope, handle) else {
        let media_followup =
            unsafe { &mut *runtime_ptr }.settle_pending_media_text_track_gate(handle);
        super::queue_media_canplay_after_text_tracks(scope, runtime_ptr, media_followup);
        return false;
    };
    let Some(track) =
        ensure_track_element_text_track(scope, unsafe { &*runtime_ptr }, wrapper, handle)
    else {
        let media_followup =
            unsafe { &mut *runtime_ptr }.settle_pending_media_text_track_gate(handle);
        super::queue_media_canplay_after_text_tracks(scope, runtime_ptr, media_followup);
        return false;
    };
    clear_track_cues(scope, track);
    let event_type = match (kind, terminal) {
        (
            RendererPageTextTrackLoadTaskKind::NetworkTerminal,
            PendingTextTrackLoadTerminal::Ready(body),
        ) => {
            for cue in parse_vtt_cues(&body) {
                if let Some(cue_object) = build_vtt_cue(scope, &cue) {
                    add_cue_to_track(scope, track, cue_object);
                    set_text_track_cue_track(scope, cue_object, track.into());
                }
            }
            refresh_text_track_active_cues(scope, track);
            set_track_ready_state(scope, wrapper, TRACK_READY_STATE_LOADED);
            "load"
        }
        (
            RendererPageTextTrackLoadTaskKind::FetchFailureTerminal,
            PendingTextTrackLoadTerminal::FetchFailed,
        ) => {
            set_track_ready_state(scope, wrapper, TRACK_READY_STATE_ERROR);
            "error"
        }
        (unexpected_kind, unexpected_terminal) => {
            tracing::error!(
                ?unexpected_kind,
                ?unexpected_terminal,
                track = handle.index(),
                sequence = sequence.get(),
                "text-track terminal task classification disagreed with its durable state"
            );
            let media_followup =
                unsafe { &mut *runtime_ptr }.settle_pending_media_text_track_gate(handle);
            super::queue_media_canplay_after_text_tracks(scope, runtime_ptr, media_followup);
            return false;
        }
    };
    let media_followup = unsafe { &mut *runtime_ptr }.settle_pending_media_text_track_gate(handle);
    if let Some(event) = construct_simple_event(scope, event_type, false, false, false) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
    super::queue_media_canplay_after_text_tracks(scope, runtime_ptr, media_followup);
    true
}

pub(crate) fn apply_text_track_load_task(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    task_id: RendererPageTextTrackLoadTaskId,
    kind: RendererPageTextTrackLoadTaskKind,
) -> bool {
    match kind {
        RendererPageTextTrackLoadTaskKind::Start => {
            apply_track_load_start(scope, runtime_ptr, task_id)
        }
        RendererPageTextTrackLoadTaskKind::NetworkTerminal
        | RendererPageTextTrackLoadTaskKind::FetchFailureTerminal => {
            apply_track_terminal(scope, runtime_ptr, task_id, kind)
        }
    }
}

fn cancel_text_track_sequence_and_settle_media(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    let followup =
        unsafe { &mut *runtime_ptr }.cancel_pending_text_track_load_sequence(handle, true);
    super::queue_media_canplay_after_text_tracks(scope, runtime_ptr, followup);
}

fn track_source_still_matches(runtime: &JsContextHost, handle: DomHandle, src: &str) -> bool {
    element_attribute(runtime, handle, "src").unwrap_or_default() == src
}

fn queue_text_track_list_track_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    media_handle: DomHandle,
    list: v8::Local<'s, v8::Object>,
    track: v8::Local<'s, v8::Object>,
    event_type: &str,
) {
    let _ = unsafe { &mut *runtime_ptr }.queue_text_track_list_event(
        scope,
        media_handle,
        list,
        track,
        event_type.to_owned(),
    );
}

pub(crate) fn dispatch_text_track_list_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    track: v8::Local<'s, v8::Object>,
    event_type: &str,
) -> bool {
    let Some(init) = TrackEventInitDeclaration::new(track).bind(scope).ok() else {
        return false;
    };
    if let Some(event) = construct_event(scope, "TrackEvent", event_type, init) {
        let _ = dispatch_simple_event_target_event(
            scope,
            list,
            TEXT_TRACK_LIST_LISTENERS_SLOT,
            event_type,
            event,
        );
        true
    } else {
        false
    }
}

fn track_has_media_parent(runtime: &JsContextHost, handle: DomHandle) -> bool {
    track_media_parent(runtime, handle).is_some()
}

fn track_media_parent(runtime: &JsContextHost, handle: DomHandle) -> Option<DomHandle> {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node)
        .filter(|parent| {
            runtime.dom_host().is_html_element_named(*parent, "audio")
                || runtime.dom_host().is_html_element_named(*parent, "video")
        })
}

pub(in crate::native_bridge) fn queue_text_track_load_if_source(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
) {
    if name.eq_ignore_ascii_case("src") || name.eq_ignore_ascii_case("default") {
        if name.eq_ignore_ascii_case("src") {
            let _ =
                unsafe { &mut *runtime_ptr }.cancel_pending_text_track_load_sequence(handle, false);
        }
        reset_track_ready_state(scope, handle);
        clear_track_cues_for_handle(scope, handle);
        if name.eq_ignore_ascii_case("default") {
            queue_default_text_track_mode_if_needed(scope, runtime_ptr, handle);
        }
        queue_text_track_load_if_needed(scope, runtime_ptr, handle);
    }
}

fn clear_track_cues<'s>(scope: &mut v8::PinScope<'s, '_>, track: v8::Local<'s, v8::Object>) {
    let Some(cues) = get_private_value(scope, track, TEXT_TRACK_CUES_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    update_indexed_list(scope, cues, &[]);
    refresh_text_track_active_cues(scope, track);
}

fn clear_track_cues_for_handle(scope: &mut v8::PinScope<'_, '_>, handle: DomHandle) {
    let Some(wrapper) = node_wrapper_from_handle(scope, handle) else {
        return;
    };
    let Some(track) = get_private_value(scope, wrapper, TEXT_TRACK_CACHE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    clear_track_cues(scope, track);
}

fn track_ready_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
) -> u32 {
    get_private_value(scope, wrapper, TRACK_READY_STATE_SLOT)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(TRACK_READY_STATE_NONE)
}

fn set_track_ready_state(
    scope: &mut v8::PinScope<'_, '_>,
    wrapper: v8::Local<'_, v8::Object>,
    state: u32,
) {
    set_private_value(
        scope,
        wrapper,
        TRACK_READY_STATE_SLOT,
        v8::Integer::new_from_unsigned(scope, state).into(),
    );
}

fn reset_track_ready_state(scope: &mut v8::PinScope<'_, '_>, handle: DomHandle) {
    if let Some(wrapper) = node_wrapper_from_handle(scope, handle) {
        set_track_ready_state(scope, wrapper, TRACK_READY_STATE_NONE);
    }
}

pub(in crate::native_bridge) fn track_ready_state_for_handle(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
) -> u32 {
    node_wrapper_from_handle(scope, handle)
        .map(|wrapper| track_ready_state(scope, wrapper))
        .unwrap_or(TRACK_READY_STATE_NONE)
}

struct ParsedCue {
    id: String,
    start: f64,
    end: f64,
    text: String,
    line: ParsedCueLine,
    position: Option<f64>,
    size: Option<f64>,
    align: Option<&'static str>,
    vertical: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ParsedCueLine {
    Auto,
    Number(f64),
    Percent(f64),
}

fn parse_vtt_cues(text: &str) -> Vec<ParsedCue> {
    let mut cues = Vec::new();
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines = normalized.lines().collect::<Vec<_>>();
    let mut index = 0usize;
    if lines
        .first()
        .map(|line| {
            line.trim_start_matches('\u{feff}')
                .trim_start()
                .starts_with("WEBVTT")
        })
        .unwrap_or(false)
    {
        index = 1;
    }

    while index < lines.len() {
        while index < lines.len() && lines[index].trim().is_empty() {
            index += 1;
        }
        if index >= lines.len() {
            break;
        }

        let first = lines[index];
        if first
            .trim_start_matches('\u{feff}')
            .trim_start()
            .starts_with("WEBVTT")
        {
            index += 1;
            continue;
        };

        let mut id = String::new();
        let timing_index = if is_vtt_timing_line(first) {
            Some(index)
        } else if !first.contains("-->")
            && index + 1 < lines.len()
            && is_vtt_timing_line(lines[index + 1])
        {
            id = first.to_owned();
            Some(index + 1)
        } else {
            let mut scan = index + 1;
            let mut found = None;
            while scan < lines.len() && !lines[scan].trim().is_empty() {
                if is_vtt_timing_line(lines[scan]) {
                    found = Some(scan);
                    break;
                }
                scan += 1;
            }
            if found.is_none() {
                index = if scan < lines.len() { scan + 1 } else { scan };
                continue;
            }
            found
        };

        let Some(timing_index) = timing_index else {
            index += 1;
            continue;
        };
        let Some((start, end, settings)) = parse_vtt_timing_line(lines[timing_index]) else {
            index = timing_index + 1;
            continue;
        };
        index = timing_index + 1;

        let mut text_lines = Vec::new();
        while index < lines.len() {
            if lines[index].trim().is_empty() || is_vtt_timing_line(lines[index]) {
                break;
            }
            text_lines.push(lines[index]);
            index += 1;
        }

        if start >= end {
            continue;
        }
        let text = text_lines.join("\n");
        let settings = parse_vtt_cue_settings(settings);
        cues.push(ParsedCue {
            id,
            start,
            end,
            text,
            line: settings.line,
            position: settings.position,
            size: settings.size,
            align: settings.align,
            vertical: settings.vertical,
        });

        while index < lines.len() && lines[index].trim().is_empty() {
            index += 1;
        }
    }
    cues
}

fn is_vtt_timing_line(line: &str) -> bool {
    parse_vtt_timing_line(line).is_some()
}

fn parse_vtt_timing_line(line: &str) -> Option<(f64, f64, &str)> {
    let (start, rest) = line.split_once("-->")?;
    let rest = rest.trim_start();
    let end_len = rest
        .find(|ch: char| ch.is_ascii_whitespace())
        .unwrap_or(rest.len());
    let end = &rest[..end_len];
    let settings = &rest[end_len..];
    Some((
        parse_vtt_time(start.trim())?,
        parse_vtt_time(end.trim())?,
        settings,
    ))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ParsedCueSettings {
    line: ParsedCueLine,
    position: Option<f64>,
    size: Option<f64>,
    align: Option<&'static str>,
    vertical: Option<&'static str>,
}

impl Default for ParsedCueSettings {
    fn default() -> Self {
        Self {
            line: ParsedCueLine::Auto,
            position: None,
            size: None,
            align: None,
            vertical: None,
        }
    }
}

fn parse_vtt_cue_settings(settings: &str) -> ParsedCueSettings {
    let mut parsed = ParsedCueSettings::default();
    for token in settings.split_ascii_whitespace() {
        let Some((name, value)) = token.split_once(':') else {
            continue;
        };
        match name {
            "line" => {
                if let Some(value) = parse_vtt_line_setting(value) {
                    parsed.line = value;
                }
            }
            "position" => {
                if let Some(value) = parse_vtt_percentage_setting(value) {
                    parsed.position = Some(value);
                }
            }
            "size" => {
                if let Some(value) = parse_vtt_percentage_setting(value) {
                    parsed.size = Some(value);
                }
            }
            "align" => match value {
                "start" => parsed.align = Some("start"),
                "middle" | "center" => parsed.align = Some("center"),
                "end" => parsed.align = Some("end"),
                "left" => parsed.align = Some("left"),
                "right" => parsed.align = Some("right"),
                _ => {}
            },
            "vertical" => match value {
                "rl" => parsed.vertical = Some("rl"),
                "lr" => parsed.vertical = Some("lr"),
                _ => {}
            },
            _ => {}
        }
    }
    parsed
}

fn parse_vtt_line_setting(value: &str) -> Option<ParsedCueLine> {
    if let Some(percent) = value.strip_suffix('%') {
        return parse_vtt_percentage_number(percent).map(ParsedCueLine::Percent);
    }
    parse_vtt_signed_integer(value).map(|value| ParsedCueLine::Number(value as f64))
}

fn parse_vtt_percentage_setting(value: &str) -> Option<f64> {
    parse_vtt_percentage_number(value.strip_suffix('%')?)
}

fn parse_vtt_percentage_number(value: &str) -> Option<f64> {
    if value.is_empty()
        || value.starts_with('+')
        || value.starts_with('-')
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let value = value.parse::<u32>().ok()?;
    (value <= 100).then_some(f64::from(value))
}

fn parse_vtt_signed_integer(value: &str) -> Option<i32> {
    if value.is_empty() || value.starts_with('+') {
        return None;
    }
    let digits = value.strip_prefix('-').unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn parse_vtt_time(value: &str) -> Option<f64> {
    let parts = value.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        [minutes, seconds] => {
            let minutes = parse_vtt_two_digit_range(minutes, 59)?;
            let seconds = parse_vtt_seconds_millis(seconds)?;
            Some(minutes as f64 * 60.0 + seconds)
        }
        [hours, minutes, seconds] => {
            let hours = parse_vtt_digits(hours)?;
            let minutes = parse_vtt_two_digit_range(minutes, 59)?;
            let seconds = parse_vtt_seconds_millis(seconds)?;
            Some(hours as f64 * 3600.0 + minutes as f64 * 60.0 + seconds)
        }
        _ => None,
    }
}

fn parse_vtt_digits(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn parse_vtt_two_digit_range(value: &str, max: u64) -> Option<u64> {
    if value.len() != 2 {
        return None;
    }
    let value = parse_vtt_digits(value)?;
    (value <= max).then_some(value)
}

fn parse_vtt_seconds_millis(value: &str) -> Option<f64> {
    let (seconds, milliseconds) = value.split_once('.')?;
    let seconds = parse_vtt_two_digit_range(seconds, 59)?;
    if milliseconds.len() != 3 || !milliseconds.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let milliseconds = milliseconds.parse::<u16>().ok()?;
    Some(seconds as f64 + f64::from(milliseconds) / 1000.0)
}

fn build_vtt_cue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: &ParsedCue,
) -> Option<v8::Local<'s, v8::Object>> {
    let ctor = scope
        .get_current_context()
        .global(scope)
        .get(scope, v8str(scope, "VTTCue").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let text = v8_string(scope, &parsed.text)?;
    let cue = ctor.new_instance(
        scope,
        &[
            v8::Number::new(scope, parsed.start).into(),
            v8::Number::new(scope, parsed.end).into(),
            text.into(),
        ],
    )?;
    if !parsed.id.is_empty()
        && let Some(id_value) = v8_string(scope, &parsed.id)
    {
        let _ = cue.set(scope, v8str(scope, "id").into(), id_value.into());
    }
    apply_parsed_vtt_cue_settings(scope, cue, parsed);
    Some(cue)
}

fn apply_parsed_vtt_cue_settings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cue: v8::Local<'s, v8::Object>,
    parsed: &ParsedCue,
) {
    match parsed.line {
        ParsedCueLine::Auto => {}
        ParsedCueLine::Number(value) => {
            let _ = cue.set(
                scope,
                v8str(scope, "line").into(),
                v8::Number::new(scope, value).into(),
            );
        }
        ParsedCueLine::Percent(value) => {
            let _ = cue.set(
                scope,
                v8str(scope, "line").into(),
                v8::Number::new(scope, value).into(),
            );
            let _ = cue.set(
                scope,
                v8str(scope, "snapToLines").into(),
                v8::Boolean::new(scope, false).into(),
            );
        }
    }
    if let Some(value) = parsed.position {
        let _ = cue.set(
            scope,
            v8str(scope, "position").into(),
            v8::Number::new(scope, value).into(),
        );
    }
    if let Some(value) = parsed.size {
        let _ = cue.set(
            scope,
            v8str(scope, "size").into(),
            v8::Number::new(scope, value).into(),
        );
    }
    if let Some(value) = parsed.align
        && let Some(value) = v8_string(scope, value)
    {
        let _ = cue.set(scope, v8str(scope, "align").into(), value.into());
    }
    if let Some(value) = parsed.vertical
        && let Some(value) = v8_string(scope, value)
    {
        let _ = cue.set(scope, v8str(scope, "vertical").into(), value.into());
    }
}

#[cfg(test)]
mod tests {
    use super::{ParsedCueLine, parse_vtt_cues, parse_vtt_time};

    #[test]
    fn parse_vtt_time_rejects_malformed_timestamps() {
        assert_eq!(parse_vtt_time("00:30.500"), Some(30.5));
        assert_eq!(parse_vtt_time("00:01:00.500"), Some(60.5));
        assert_eq!(parse_vtt_time("100:20:00.500"), Some(361200.5));
        assert_eq!(parse_vtt_time("00:00.000"), Some(0.0));

        assert_eq!(parse_vtt_time("00.00.000"), None);
        assert_eq!(parse_vtt_time("01:00:500"), None);
        assert_eq!(parse_vtt_time("120:00.500"), None);
        assert_eq!(parse_vtt_time("00:120:00.500"), None);
        assert_eq!(parse_vtt_time("03m:00.500"), None);
    }

    #[test]
    fn parse_vtt_cues_skips_invalid_or_non_positive_ranges() {
        let cues = parse_vtt_cues(
            "\u{feff}WEBVTT\n\n\
             valid\n\
             00:00.000 --> 00:01.500\n\
             text\n\n\
             invalid\n\
             00:31.000 --> 01:00:500\n\
             ignored\n\n\
             negative duration\n\
             00:03.000 --> 00:01.000\n\
             ignored\n\n\
             zero\n\
             00:02.000 --> 00:02.000\n\
             ignored\n",
        );

        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].id, "valid");
        assert_eq!(cues[0].start, 0.0);
        assert_eq!(cues[0].end, 1.5);
        assert_eq!(cues[0].text, "text");
    }

    #[test]
    fn parse_vtt_cue_settings_from_timing_line() {
        let cues = parse_vtt_cues(
            "WEBVTT\n\n\
             00:00.000 --> 00:01.000 line:15% position:40% size:10% align:middle vertical:rl\n\
             settings\n\n\
             00:02.000 --> 00:03.000 line:-1 position:150% align: start size:\t50% vertical:bad\n\
             bad settings\n",
        );

        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].line, ParsedCueLine::Percent(15.0));
        assert_eq!(cues[0].position, Some(40.0));
        assert_eq!(cues[0].size, Some(10.0));
        assert_eq!(cues[0].align, Some("center"));
        assert_eq!(cues[0].vertical, Some("rl"));
        assert_eq!(cues[1].line, ParsedCueLine::Number(-1.0));
        assert_eq!(cues[1].position, None);
        assert_eq!(cues[1].size, None);
        assert_eq!(cues[1].align, None);
        assert_eq!(cues[1].vertical, None);
    }

    #[test]
    fn parse_vtt_recovers_timing_lines_without_blank_separator() {
        let cues = parse_vtt_cues(
            "WEBVTT\n\
             00:00.000 --> 00:01.000\n\
             Valid cue 1\n\
             label kept as text\n\
             00:02.000 --> 00:03.000\n\
             Valid cue 2\n\n\
             --> bad id\n\
             00:04.000 --> 00:05.000\n\
             Recovered cue\n\n\
             00:06.000 --> 00:07.000\n\
             00:08.000 --> 00:09.000\n",
        );

        assert_eq!(cues.len(), 5);
        assert_eq!(cues[0].text, "Valid cue 1\nlabel kept as text");
        assert_eq!(cues[1].id, "");
        assert_eq!(cues[1].text, "Valid cue 2");
        assert_eq!(cues[2].id, "");
        assert_eq!(cues[2].text, "Recovered cue");
        assert_eq!(cues[3].start, 6.0);
        assert_eq!(cues[3].end, 7.0);
        assert_eq!(cues[3].text, "");
        assert_eq!(cues[4].start, 8.0);
        assert_eq!(cues[4].end, 9.0);
        assert_eq!(cues[4].text, "");
    }
}
