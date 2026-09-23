use super::storage::{font_face_set_owner_snapshot, is_font_face_value, set_font_face_set_status};
use super::*;
use crate::context_bootstrap::events::initialize_event_object;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, set_private_value,
};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::WebApiFunctionTemplate;

const FONT_FACE_SET_LOAD_EVENT_FONTFACES_SLOT: &str = "__moliFontFaceSetLoadEventFontfaces";
const FONT_FACE_SET_ONLOADING_SLOT: &str = "__moliFontFaceSetOnloading";
const FONT_FACE_SET_ONLOADINGDONE_SLOT: &str = "__moliFontFaceSetOnloadingdone";
const FONT_FACE_SET_ONLOADINGERROR_SLOT: &str = "__moliFontFaceSetOnloadingerror";

#[derive(Clone, Copy)]
struct FontFaceSetEventHandler {
    event_type: &'static str,
    slot_name: &'static str,
}

const FONT_FACE_SET_EVENT_HANDLERS: &[FontFaceSetEventHandler] = &[
    FontFaceSetEventHandler {
        event_type: "loading",
        slot_name: FONT_FACE_SET_ONLOADING_SLOT,
    },
    FontFaceSetEventHandler {
        event_type: "loadingdone",
        slot_name: FONT_FACE_SET_ONLOADINGDONE_SLOT,
    },
    FontFaceSetEventHandler {
        event_type: "loadingerror",
        slot_name: FONT_FACE_SET_ONLOADINGERROR_SLOT,
    },
];

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::FontFaceSet, receiver)]
struct FontFaceSetEventHandlerAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = font_face_set_event_handler_getter,
        setter = font_face_set_event_handler_setter,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    onloading: (),

    #[webapi(
        accessor_property,
        getter = font_face_set_event_handler_getter,
        setter = font_face_set_event_handler_setter,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    onloadingdone: (),

    #[webapi(
        accessor_property,
        getter = font_face_set_event_handler_getter,
        setter = font_face_set_event_handler_setter,
        data = callback_data_index_value(scope, 2),
        enumerable
    )]
    onloadingerror: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::FontFaceSetLoadEvent, enumerable, receiver)]
struct FontFaceSetLoadEventPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = font_face_set_load_event_fontfaces_getter
    )]
    fontfaces: (),
}

pub(in crate::context_bootstrap) fn install_font_face_set_load_event_template_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    FontFaceSetLoadEventPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

pub(in crate::context_bootstrap) fn install_font_face_set_event_handler_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    // The callback-data indexes must remain aligned with
    // FONT_FACE_SET_EVENT_HANDLERS. The shared setter publishes each handler
    // into the same ordered registration list as addEventListener, so replacing
    // an active handler preserves its registration position.
    FontFaceSetEventHandlerAccessorsDeclaration::initialize_prototype_template(scope, prototype);
}

pub(super) fn initialize_font_face_set_event_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    mark_simple_event_target_slot(scope, object, FONT_FACE_SET_LISTENERS_SLOT);
    for handler in FONT_FACE_SET_EVENT_HANDLERS {
        set_private_value(scope, object, handler.slot_name, v8::null(scope).into());
    }
    install_simple_event_target_ordered_handlers(scope, object);
}

fn font_face_set_event_handler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(handler) = callback_data_item(
        scope,
        &args,
        FONT_FACE_SET_EVENT_HANDLERS,
        "FontFaceSet event handlers",
    ) else {
        rv.set_null();
        return;
    };
    rv.set(
        get_private_value(scope, args.this(), handler.slot_name)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn font_face_set_event_handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(handler) = callback_data_item(
        scope,
        &args,
        FONT_FACE_SET_EVENT_HANDLERS,
        "FontFaceSet event handlers",
    ) else {
        return;
    };
    let value = args.get(0);
    let active = value.is_object();
    let stored = if active {
        value
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, args.this(), handler.slot_name, stored);
    simple_object_event_set_ordered_handler(
        scope,
        args.this(),
        FONT_FACE_SET_LISTENERS_SLOT,
        handler.event_type,
        handler.slot_name,
        active,
    );
}

fn font_face_set_load_event_fontfaces_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let value = crate::context_bootstrap::event_private_value(
        scope,
        args.this(),
        FONT_FACE_SET_LOAD_EVENT_FONTFACES_SLOT,
    )
    .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

fn frozen_font_face_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: impl IntoIterator<Item = v8::Local<'s, v8::Value>>,
) -> v8::Local<'s, v8::Array> {
    let array = v8::Array::new(scope, 0);
    for value in values {
        let _ = array.set_index(scope, array.length(), value);
    }
    let _ = array.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    array
}

fn font_face_values_from_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Option<Vec<v8::Local<'s, v8::Value>>> {
    let Some(init) = init else {
        return Some(Vec::new());
    };
    let values = match webidl::optional_member::<webidl::Sequence<v8::Local<'s, v8::Value>>>(
        scope,
        init,
        "fontfaces",
        webidl::Context::member("FontFaceSetLoadEventInit", "fontfaces"),
    ) {
        Ok(values) => values.map(|values| values.0).unwrap_or_default(),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    if values
        .iter()
        .copied()
        .any(|value| !is_font_face_value(scope, value))
    {
        throw_type_error(
            scope,
            "Failed to construct 'FontFaceSetLoadEvent': member fontfaces is not a sequence of FontFace objects.",
        );
        return None;
    }
    Some(values)
}

pub(in crate::context_bootstrap) fn initialize_font_face_set_load_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let Some(values) = font_face_values_from_init(scope, init) else {
        return false;
    };
    let fontfaces = frozen_font_face_array(scope, values);
    set_private_value(
        scope,
        event,
        FONT_FACE_SET_LOAD_EVENT_FONTFACES_SLOT,
        fontfaces.into(),
    );
    true
}

fn dispatched_font_face_set_load_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    fontfaces: Option<v8::Local<'s, v8::Array>>,
) -> v8::Local<'s, v8::Object> {
    let event = crate::context_bootstrap::new_event_state(scope);
    initialize_event_object(scope, event, event_type, false, false);
    web_api_interfaces::FontFaceSetLoadEvent::DESCRIPTOR
        .initialize(scope, event)
        .expect("font loading events should carry their native interface brand");
    let mut values = Vec::new();
    if let Some(fontfaces) = fontfaces {
        for index in 0..fontfaces.length() {
            if let Some(value) = fontfaces.get_index(scope, index) {
                values.push(value);
            }
        }
    }
    let fontfaces = frozen_font_face_array(scope, values);
    set_private_value(
        scope,
        event,
        FONT_FACE_SET_LOAD_EVENT_FONTFACES_SLOT,
        fontfaces.into(),
    );
    crate::context_bootstrap::new_event_wrapper(scope, event).expect("FontFaceSetLoadEvent wrapper")
}

pub(super) fn dispatch_font_face_set_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    event_type: &str,
    fontfaces: Option<v8::Local<'s, v8::Array>>,
) -> bool {
    let event = dispatched_font_face_set_load_event(scope, event_type, fontfaces);
    dispatch_simple_event_target_event(
        scope,
        object,
        FONT_FACE_SET_LISTENERS_SLOT,
        event_type,
        event,
    )
}

const LOADING_FONTS: &str = "__moliFontFaceSetLoadingFonts";
const LOADED_FONTS: &str = "__moliFontFaceSetLoadedFonts";
const FAILED_FONTS: &str = "__moliFontFaceSetFailedFonts";
const READY_RESOLVER: &str = "__moliFontFaceSetReadyResolver";

fn font_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> v8::Local<'s, v8::Array> {
    if let Some(array) = get_private_value(scope, owner, slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    {
        return array;
    }
    let array = v8::Array::new(scope, 0);
    set_private_value(scope, owner, slot, array.into());
    array
}

fn remove_from_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    slot: &'static str,
    face: v8::Local<'s, v8::Value>,
) -> bool {
    let array = font_list(scope, owner, slot);
    let mut values = Vec::new();
    let mut removed = false;
    for index in 0..array.length() {
        if let Some(value) = array.get_index(scope, index) {
            if value.strict_equals(face) {
                removed = true;
            } else {
                values.push(value);
            }
        }
    }
    let next = v8::Array::new_with_elements(scope, &values);
    set_private_value(scope, owner, slot, next.into());
    removed
}

fn queue_set_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    completion: bool,
) {
    let Some(context) = owner.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let callback = if completion {
        v8::Function::builder(font_set_completed_task)
            .data(owner.into())
            .build(scope)
    } else {
        v8::Function::builder(font_set_loading_task)
            .data(owner.into())
            .build(scope)
    };
    if let Some(callback) = callback {
        super::font_loading::queue_font_task(scope, callback);
    }
}

pub(super) fn add_loading_font<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    face: v8::Local<'s, v8::Object>,
) {
    let Some(context) = owner.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let loading = font_list(scope, owner, LOADING_FONTS);
    if super::storage::array_contains_value(scope, loading, face.into()) {
        return;
    }
    if loading.length() == 0 {
        set_font_face_set_status(scope, owner, "loading");
        if let Some(resolver) = v8::PromiseResolver::new(scope) {
            let ready = resolver.get_promise(scope);
            set_private_value(scope, owner, FONT_FACE_SET_READY_SLOT, ready.into());
            set_private_value(scope, owner, READY_RESOLVER, resolver.into());
        }
        queue_set_event(scope, owner, false);
    }
    let _ = loading.set_index(scope, loading.length(), face.into());
}

pub(super) fn notify_font_face_set_owners_loading<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) {
    for owner in font_face_set_owner_snapshot(scope, face) {
        add_loading_font(scope, owner, face);
    }
}

fn finish_if_idle<'s>(scope: &mut v8::PinScope<'s, '_>, owner: v8::Local<'s, v8::Object>) {
    if font_list(scope, owner, LOADING_FONTS).length() != 0 {
        return;
    }
    set_font_face_set_status(scope, owner, "loaded");
    if let Some(resolver) = get_private_value(scope, owner, READY_RESOLVER)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        let resolver = unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(resolver) };
        let _ = resolver.resolve(scope, owner.into());
    }
    queue_set_event(scope, owner, true);
}

pub(super) fn remove_font_from_loading_set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    face: v8::Local<'s, v8::Value>,
) {
    remove_from_list(scope, owner, LOADED_FONTS, face);
    remove_from_list(scope, owner, FAILED_FONTS, face);
    if remove_from_list(scope, owner, LOADING_FONTS, face) {
        finish_if_idle(scope, owner);
    }
}

pub(super) fn notify_font_face_set_owners_finished<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    succeeded: bool,
) {
    for owner in font_face_set_owner_snapshot(scope, face) {
        if !remove_from_list(scope, owner, LOADING_FONTS, face.into()) {
            continue;
        }
        let completed = font_list(
            scope,
            owner,
            if succeeded {
                LOADED_FONTS
            } else {
                FAILED_FONTS
            },
        );
        let _ = completed.set_index(scope, completed.length(), face.into());
        finish_if_idle(scope, owner);
    }
}

fn font_set_loading_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Ok(owner) = v8::Local::<v8::Object>::try_from(args.data()) {
        let _ = dispatch_font_face_set_event(scope, owner, "loading", None);
    }
}

fn take_event_faces<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> v8::Local<'s, v8::Array> {
    let faces = font_list(scope, owner, slot);
    let empty = v8::Array::new(scope, 0);
    set_private_value(scope, owner, slot, empty.into());
    let current = super::storage::font_face_set_faces_array(scope, owner);
    let mut values = Vec::new();
    for index in 0..faces.length() {
        if let Some(face) = faces.get_index(scope, index)
            && current
                .is_some_and(|current| super::storage::array_contains_value(scope, current, face))
        {
            values.push(face);
        }
    }
    v8::Array::new_with_elements(scope, &values)
}

fn font_set_completed_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Ok(owner) = v8::Local::<v8::Object>::try_from(args.data()) {
        let loaded = take_event_faces(scope, owner, LOADED_FONTS);
        let failed = take_event_faces(scope, owner, FAILED_FONTS);
        let _ = dispatch_font_face_set_event(scope, owner, "loadingdone", Some(loaded));
        if failed.length() != 0 {
            let _ = dispatch_font_face_set_event(scope, owner, "loadingerror", Some(failed));
        }
    }
}
