use super::storage::{
    font_face_set_owner_snapshot, is_font_face_value, replace_font_face_set_ready_promise,
    set_font_face_set_status,
};
use super::*;
use crate::context_bootstrap::events::initialize_event_object;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, global_constructor_prototype,
    set_private_value,
};
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
#[webapi(name = "FontFaceSet")]
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
#[webapi(name = "FontFaceSetLoadEvent", enumerable)]
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
    let active = v8::Local::<v8::Object>::try_from(value)
        .ok()
        .is_some_and(|callback| callback.is_callable());
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
    let value = get_private_value(scope, args.this(), FONT_FACE_SET_LOAD_EVENT_FONTFACES_SLOT)
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
    let event = v8::Object::new(scope);
    initialize_event_object(scope, event, event_type, false, false);
    if let Some(prototype) = global_constructor_prototype(scope, "FontFaceSetLoadEvent") {
        let _ = event.set_prototype(scope, prototype.into());
    }
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
    event
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

pub(super) fn notify_font_face_set_owners_of_load<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) {
    if get_private_value(scope, face, FONT_FACE_LOAD_NOTIFICATION_SENT_SLOT)
        .is_some_and(|value| value.is_true())
    {
        return;
    }
    let notification_sent = v8::Boolean::new(scope, true);
    set_private_value(
        scope,
        face,
        FONT_FACE_LOAD_NOTIFICATION_SENT_SLOT,
        notification_sent.into(),
    );

    let owners = font_face_set_owner_snapshot(scope, face);
    if owners.is_empty() {
        return;
    }
    let loaded_faces = v8::Array::new(scope, 1);
    let _ = loaded_faces.set_index(scope, 0, face.into());
    let completion_event_type = if super::font_face::font_face_load_failed(scope, face) {
        "loadingerror"
    } else {
        "loadingdone"
    };
    for owner in owners {
        set_font_face_set_status(scope, owner, "loading");
        replace_font_face_set_ready_promise(scope, owner);
        let _ = dispatch_font_face_set_event(scope, owner, "loading", None);
        set_font_face_set_status(scope, owner, "loaded");
        let _ =
            dispatch_font_face_set_event(scope, owner, completion_event_type, Some(loaded_faces));
    }
}
