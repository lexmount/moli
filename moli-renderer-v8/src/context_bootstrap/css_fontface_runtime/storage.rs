use super::events::{dispatch_font_face_set_event, initialize_font_face_set_event_target};
use super::*;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, serialize_v8_iter_array,
    set_private_value,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

#[derive(Default, WebApiObject)]
#[webapi(interface = "FontFaceSet")]
struct FontFaceSetObjectDeclaration {
    #[webapi(slot = FONT_FACE_SET_FACES_SLOT, init = "array")]
    faces: (),
    #[webapi(slot = FONT_FACE_SET_MANUAL_FACES_SLOT, init = "array")]
    manual_faces: (),
    #[webapi(slot = FONT_FACE_SET_CONNECTED_FACES_SLOT, init = "array")]
    connected_faces: (),
    #[webapi(slot = FONT_FACE_SET_LISTENERS_SLOT, init = "null_object")]
    listeners: (),
    #[webapi(slot = FONT_FACE_SET_STATUS_SLOT, init = string("loaded"))]
    status: (),
    #[webapi(slot = FONT_FACE_SET_SIZE_SLOT, init = 0)]
    size: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FontFaceSet")]
struct FontFaceSetPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = font_face_set_attribute_getter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    status: (),
    #[webapi(
        accessor_property,
        getter = font_face_set_attribute_getter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    ready: (),
    #[webapi(
        accessor_property,
        getter = font_face_set_attribute_getter_callback,
        data = callback_data_index_value(scope, 2),
        enumerable
    )]
    size: (),
}

#[derive(Clone, Copy)]
enum FontFaceSetAttribute {
    Status,
    Ready,
    Size,
}

const FONT_FACE_SET_ATTRIBUTES: &[FontFaceSetAttribute] = &[
    FontFaceSetAttribute::Status,
    FontFaceSetAttribute::Ready,
    FontFaceSetAttribute::Size,
];

pub(in crate::context_bootstrap) fn install_font_face_set_template_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    FontFaceSetPrototypeAccessorsDeclaration::initialize_prototype_template(scope, prototype);
}

fn font_face_set_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(attribute) = callback_data_item(
        scope,
        &args,
        FONT_FACE_SET_ATTRIBUTES,
        "FontFaceSet attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    match attribute {
        FontFaceSetAttribute::Ready => {
            apply_pending_stylesheet_source_css_projections(scope);
            crate::native_bridge::document::synchronize_font_face_set_load_state_for_attribute(
                scope,
                args.this(),
                true,
            );
        }
        FontFaceSetAttribute::Status => {
            crate::native_bridge::document::synchronize_font_face_set_load_state_for_attribute(
                scope,
                args.this(),
                false,
            );
        }
        FontFaceSetAttribute::Size => {}
    }
    match attribute {
        FontFaceSetAttribute::Status => rv.set(
            font_face_set_slot_value(scope, args.this(), FONT_FACE_SET_STATUS_SLOT)
                .unwrap_or_else(|| v8::undefined(scope).into()),
        ),
        FontFaceSetAttribute::Ready => rv.set(
            font_face_set_slot_value(scope, args.this(), FONT_FACE_SET_READY_SLOT)
                .unwrap_or_else(|| v8::undefined(scope).into()),
        ),
        FontFaceSetAttribute::Size => {
            apply_pending_stylesheet_source_css_projections(scope);
            let size = font_face_set_faces_array(scope, args.this())
                .map(|faces| faces.length() as i32)
                .unwrap_or(0);
            rv.set(v8::Integer::new(scope, size).into());
        }
    }
}

pub(super) fn initialize_font_face_set_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    FontFaceSetObjectDeclaration::default()
        .initialize(scope, object)
        .expect("FontFaceSet declaration should initialize object");
    initialize_font_face_set_event_target(scope, object);
    let _ = replace_font_face_set_ready_promise(scope, object);
}

pub(super) fn replace_font_face_set_ready_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    if font_face_set_document_load_cycle(scope, object).is_some() {
        return false;
    }
    if let Some(ready) = resolved_promise(scope, object.into()) {
        set_font_face_set_slot_value(scope, object, FONT_FACE_SET_READY_SLOT, ready.into());
        set_font_face_set_slot_value(
            scope,
            object,
            FONT_FACE_SET_READY_RESOLVER_SLOT,
            v8::undefined(scope).into(),
        );
        return true;
    }
    false
}

/// Projects one native document-font load cycle into its JS `FontFaceSet`.
/// The native side owns request membership and task scheduling; this object
/// owns only the stable promise identity and observable events for that cycle.
pub(crate) fn begin_document_font_face_set_load_cycle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    cycle: crate::script_vm::web_fonts::DocumentWebFontLoadCycleId,
) -> bool {
    if font_face_set_document_load_cycle(scope, object) == Some(cycle.as_u64()) {
        return true;
    }
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return false;
    };
    let promise = resolver.get_promise(scope);
    set_font_face_set_slot_value(scope, object, FONT_FACE_SET_READY_SLOT, promise.into());
    set_font_face_set_slot_value(
        scope,
        object,
        FONT_FACE_SET_READY_RESOLVER_SLOT,
        resolver.into(),
    );
    set_font_face_set_slot_value(
        scope,
        object,
        FONT_FACE_SET_DOCUMENT_LOAD_CYCLE_SLOT,
        v8::BigInt::new_from_u64(scope, cycle.as_u64()).into(),
    );
    set_font_face_set_status(scope, object, "loading");
    let _ = dispatch_font_face_set_event(scope, object, "loading", None);
    true
}

/// Settles the exact JS cycle only after native layout consumed its terminal
/// font collection. Clearing the native-cycle slots before event dispatch
/// allows a `loadingdone` listener to start a distinct, non-overwritten cycle;
/// the captured old resolver is resolved afterwards like Blink's ready
/// property.
pub(crate) fn settle_document_font_face_set_load_cycle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    cycle: crate::script_vm::web_fonts::DocumentWebFontLoadCycleId,
) -> bool {
    if font_face_set_document_load_cycle(scope, object) != Some(cycle.as_u64()) {
        return false;
    }
    let resolver = get_private_value(scope, object, FONT_FACE_SET_READY_RESOLVER_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|object| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) });
    set_font_face_set_slot_value(
        scope,
        object,
        FONT_FACE_SET_READY_RESOLVER_SLOT,
        v8::undefined(scope).into(),
    );
    set_font_face_set_slot_value(
        scope,
        object,
        FONT_FACE_SET_DOCUMENT_LOAD_CYCLE_SLOT,
        v8::undefined(scope).into(),
    );
    set_font_face_set_status(scope, object, "loaded");
    let _ = dispatch_font_face_set_event(scope, object, "loadingdone", None);
    if let Some(resolver) = resolver {
        let _ = resolver.resolve(scope, object.into());
    }
    true
}

pub(super) fn font_face_set_has_document_load_cycle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    font_face_set_document_load_cycle(scope, object).is_some()
}

fn font_face_set_document_load_cycle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    let cycle = get_private_value(scope, object, FONT_FACE_SET_DOCUMENT_LOAD_CYCLE_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())?;
    let (cycle, lossless) = cycle.u64_value();
    lossless.then_some(cycle)
}

pub(super) fn font_face_set_faces_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    font_face_set_array_for_slot(scope, object, FONT_FACE_SET_FACES_SLOT)
}

pub(super) fn font_face_set_manual_faces_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    font_face_set_array_for_slot(scope, object, FONT_FACE_SET_MANUAL_FACES_SLOT)
}

fn font_face_set_connected_faces_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    font_face_set_array_for_slot(scope, object, FONT_FACE_SET_CONNECTED_FACES_SLOT)
}

fn font_face_set_array_for_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Array>> {
    font_face_set_slot_value(scope, object, slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

pub(crate) fn rebuild_font_face_set_faces<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    let previous =
        font_face_set_faces_array(scope, object).unwrap_or_else(|| v8::Array::new(scope, 0));
    let mut combined = Vec::new();
    if let Some(connected) = font_face_set_connected_faces_array(scope, object) {
        for index in 0..connected.length() {
            let Some(face) = connected.get_index(scope, index) else {
                continue;
            };
            combined.push(face);
        }
    }
    if let Some(manual) = font_face_set_manual_faces_array(scope, object) {
        for index in 0..manual.length() {
            let Some(face) = manual.get_index(scope, index) else {
                continue;
            };
            combined.push(face);
        }
    }
    let combined =
        serialize_v8_iter_array(scope, combined).unwrap_or_else(|| v8::Array::new(scope, 0));
    sync_font_face_set_owners(scope, object, previous, combined);
    set_font_face_set_slot_value(scope, object, FONT_FACE_SET_FACES_SLOT, combined.into());
    sync_font_face_set_size(scope, object);
}

pub(super) fn font_face_set_owner_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let Some(owners) = font_face_set_owner_array(scope, face.into(), false) else {
        return Vec::new();
    };
    let mut snapshot = Vec::with_capacity(owners.length() as usize);
    for index in 0..owners.length() {
        let Some(owner) = owners.get_index(scope, index) else {
            continue;
        };
        let Ok(owner) = v8::Local::<v8::Object>::try_from(owner) else {
            continue;
        };
        snapshot.push(owner);
    }
    snapshot
}

fn sync_font_face_set_owners<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    previous: v8::Local<'s, v8::Array>,
    current: v8::Local<'s, v8::Array>,
) {
    for index in 0..previous.length() {
        let Some(face) = previous.get_index(scope, index) else {
            continue;
        };
        if !array_contains_value(scope, current, face) {
            remove_font_face_set_owner(scope, face, owner);
        }
    }
    for index in 0..current.length() {
        let Some(face) = current.get_index(scope, index) else {
            continue;
        };
        if !array_contains_value(scope, previous, face) {
            add_font_face_set_owner(scope, face, owner);
        }
    }
}

fn font_face_set_owner_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Value>,
    create: bool,
) -> Option<v8::Local<'s, v8::Array>> {
    let face = v8::Local::<v8::Object>::try_from(face).ok()?;
    if let Some(owners) = get_private_value(scope, face, FONT_FACE_SET_OWNERS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    {
        return Some(owners);
    }
    if !create {
        return None;
    }
    let owners = v8::Array::new(scope, 0);
    set_private_value(scope, face, FONT_FACE_SET_OWNERS_SLOT, owners.into());
    Some(owners)
}

fn add_font_face_set_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Value>,
    owner: v8::Local<'s, v8::Object>,
) {
    let Some(owners) = font_face_set_owner_array(scope, face, true) else {
        return;
    };
    if !array_contains_value(scope, owners, owner.into()) {
        let _ = owners.set_index(scope, owners.length(), owner.into());
    }
}

fn remove_font_face_set_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Value>,
    owner: v8::Local<'s, v8::Object>,
) {
    let Some(owners) = font_face_set_owner_array(scope, face, false) else {
        return;
    };
    let next = v8::Array::new(scope, 0);
    for index in 0..owners.length() {
        let Some(candidate) = owners.get_index(scope, index) else {
            continue;
        };
        if !candidate.strict_equals(owner.into()) {
            let _ = next.set_index(scope, next.length(), candidate);
        }
    }
    let Ok(face) = v8::Local::<v8::Object>::try_from(face) else {
        return;
    };
    set_private_value(scope, face, FONT_FACE_SET_OWNERS_SLOT, next.into());
}

pub(in crate::context_bootstrap) fn set_font_face_set_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: v8::Local<'s, v8::Value>,
) {
    set_private_value(scope, object, slot, value);
}

pub(super) fn set_font_face_set_status<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    status: &'static str,
) {
    let value = v8_string(scope, status).unwrap_or_else(|| v8::String::empty(scope));
    set_font_face_set_slot_value(scope, object, FONT_FACE_SET_STATUS_SLOT, value.into());
}

pub(super) fn is_font_face_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return false;
    };
    if global_constructor_prototype(scope, "FontFace").is_some_and(|prototype| {
        object
            .get_prototype(scope)
            .is_some_and(|candidate| candidate.strict_equals(prototype.into()))
    }) {
        return true;
    }
    object_has_string_property(scope, object, "family")
        && object_has_string_property(scope, object, "status")
}

pub(super) fn array_contains_value(
    scope: &mut v8::PinScope<'_, '_>,
    array: v8::Local<'_, v8::Array>,
    candidate: v8::Local<'_, v8::Value>,
) -> bool {
    for index in 0..array.length() {
        let Some(existing) = array.get_index(scope, index) else {
            continue;
        };
        if existing.strict_equals(candidate) {
            return true;
        }
    }
    false
}

fn object_has_string_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> bool {
    object
        .get(
            scope,
            v8_string(scope, key)
                .map(Into::into)
                .unwrap_or_else(|| v8::String::empty(scope).into()),
        )
        .is_some_and(|value| value.is_string())
}

fn sync_font_face_set_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    let size = font_face_set_faces_array(scope, object)
        .map(|faces| faces.length() as i32)
        .unwrap_or(0);
    let value = v8::Integer::new(scope, size);
    set_font_face_set_slot_value(scope, object, FONT_FACE_SET_SIZE_SLOT, value.into());
}

fn font_face_set_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, object, slot)
}
