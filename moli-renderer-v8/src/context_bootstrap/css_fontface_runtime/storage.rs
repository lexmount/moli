use super::events::initialize_font_face_set_event_target;
use super::*;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, serialize_v8_iter_array,
    set_private_value,
};
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

#[derive(Default, WebApiObject)]
#[webapi(interface = web_api_interfaces::FontFaceSet)]
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
#[webapi(interface = web_api_interfaces::FontFaceSet)]
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
    replace_font_face_set_ready_promise(scope, object);
}

pub(super) fn replace_font_face_set_ready_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    if let Some(ready) = resolved_promise(scope, object.into()) {
        set_font_face_set_slot_value(scope, object, FONT_FACE_SET_READY_SLOT, ready.into());
    }
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
    super::events::finish_font_set_if_idle(scope, object);
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
            if let Ok(face) = v8::Local::<v8::Object>::try_from(face) {
                super::loading::sync_registration(scope, owner, face, false);
            }
            remove_font_face_set_owner(scope, face, owner);
        }
    }
    for index in 0..current.length() {
        let Some(face) = current.get_index(scope, index) else {
            continue;
        };
        if !array_contains_value(scope, previous, face) {
            add_font_face_set_owner(scope, face, owner);
            if let Ok(face) = v8::Local::<v8::Object>::try_from(face) {
                super::loading::sync_registration(scope, owner, face, true);
                if super::loading::string_slot(scope, face, FONT_FACE_STATUS_SLOT) == "loading" {
                    super::events::font_face_loading_started(scope, face);
                }
            }
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

pub(super) fn is_font_face_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return false;
    };
    web_api_interfaces::FontFace::is_instance(scope, object)
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
