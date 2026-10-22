use super::storage::{font_face_set_owner_snapshot, set_font_face_set_status};
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
    initialize_simple_event_target(scope, object);
    for handler in FONT_FACE_SET_EVENT_HANDLERS {
        set_private_value(scope, object, handler.slot_name, v8::null(scope).into());
    }
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
    simple_object_event_set_handler(
        scope,
        args.this(),
        handler.event_type,
        handler.slot_name,
        args.get(0),
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
    let values = values.into_iter().collect::<Vec<_>>();
    let array = v8::Array::new_with_elements(scope, &values);
    let _ = array.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    array
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "FontFaceSetLoadEventInit")]
pub(in crate::context_bootstrap) struct FontFaceSetLoadEventInit<'s> {
    #[webidl(inherit)]
    base: crate::context_bootstrap::events::EventInit,
    #[webidl(sequence, interface = web_api_interfaces::FontFace, default = Vec::new())]
    fontfaces: Vec<v8::Local<'s, v8::Object>>,
}

impl<'s> FontFaceSetLoadEventInit<'s> {
    pub(in crate::context_bootstrap) fn event_flags(&self) -> (bool, bool, bool) {
        (self.base.bubbles, self.base.cancelable, self.base.composed)
    }
    pub(in crate::context_bootstrap) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        let fontfaces = frozen_font_face_array(scope, self.fontfaces.into_iter().map(Into::into));
        set_private_value(
            scope,
            event,
            FONT_FACE_SET_LOAD_EVENT_FONTFACES_SLOT,
            fontfaces.into(),
        );
    }
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
    dispatch_simple_event_target_event(scope, object, event_type, event)
}

const READY_RESOLVER: &str = "__moliFontFaceSetReadyResolver";
const LOADED_FACES: &str = "__moliFontFaceSetLoadedFaces";
const FAILED_FACES: &str = "__moliFontFaceSetFailedFaces";
const FINISH_QUEUED: &str = "__moliFontFaceSetFinishQueued";

fn queue_font_face_set_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    finish: bool,
) {
    let Some(context) = owner.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let data = v8::Array::new_with_elements(
        scope,
        &[owner.into(), v8::Boolean::new(scope, finish).into()],
    );
    let callback = v8::Function::builder(run_font_face_set_task)
        .data(data.into())
        .build(scope)
        .expect("font loading task");
    // The internal task scheduler captures this realm's exact Window/Document
    // lifetime and discards the callback when that owner retires.
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host_ptr }.queue_internal_task(scope, callback);
    } else {
        crate::worker::queue_worker_font_task(scope, callback);
    }
}

fn run_font_face_set_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let data = v8::Local::<v8::Array>::try_from(args.data()).expect("font loading task data");
    let owner = data
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("font loading task owner");
    if data
        .get_index(scope, 1)
        .is_some_and(|value| value.is_true())
    {
        set_private_value(scope, owner, FINISH_QUEUED, v8::undefined(scope).into());
        finish_font_set_task(scope, owner);
    } else {
        let _ = dispatch_font_face_set_event(scope, owner, "loading", None);
    }
}

pub(super) fn font_face_loading_started<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) {
    for owner in font_face_set_owner_snapshot(scope, face) {
        font_face_set_loading_started(scope, owner);
    }
}

/// CSS resource discovery and JS FontFace.load share one loading cycle and
/// one ready promise. Additional arrivals must not replace a pending promise.
pub(super) fn font_face_set_loading_started<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) {
    if super::loading::string_slot(scope, owner, FONT_FACE_SET_STATUS_SLOT) == "loading" {
        return;
    }
    set_font_face_set_status(scope, owner, "loading");
    let resolver = v8::PromiseResolver::new(scope).expect("FontFaceSet ready resolver");
    set_private_value(scope, owner, READY_RESOLVER, resolver.into());
    set_private_value(
        scope,
        owner,
        FONT_FACE_SET_READY_SLOT,
        resolver.get_promise(scope).into(),
    );
    for slot in [LOADED_FACES, FAILED_FACES] {
        let array = v8::Array::new(scope, 0);
        set_private_value(scope, owner, slot, array.into());
    }
    queue_font_face_set_task(scope, owner, false);
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
    let slot = if font_face_load_failed(scope, face) {
        FAILED_FACES
    } else {
        LOADED_FACES
    };
    for owner in owners {
        if let Some(array) = get_private_value(scope, owner, slot)
            .and_then(|v| v8::Local::<v8::Array>::try_from(v).ok())
        {
            let _ = array.set_index(scope, array.length(), face.into());
        }
        finish_font_set_if_idle(scope, owner);
    }
}

pub(super) fn finish_font_set_if_idle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) {
    if !font_set_is_loading_and_idle(scope, owner)
        || get_private_value(scope, owner, FINISH_QUEUED).is_some_and(|value| value.is_true())
    {
        return;
    }
    set_private_value(
        scope,
        owner,
        FINISH_QUEUED,
        v8::Boolean::new(scope, true).into(),
    );
    queue_font_face_set_task(scope, owner, true);
}

fn font_set_is_loading_and_idle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    if super::loading::string_slot(scope, owner, FONT_FACE_SET_STATUS_SLOT) != "loading" {
        return false;
    }
    // Native CSS requests can be pending before a JS wrapper is projected.
    // Conversely, the native CSS queue does not contain JS-created faces.
    if super::storage::document_font_face_set_has_pending_requests(scope, owner) {
        return false;
    }
    let pending = super::storage::font_face_set_faces_array(scope, owner).is_some_and(|faces| {
        (0..faces.length()).any(|index| {
            faces
                .get_index(scope, index)
                .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
                .is_some_and(|face| {
                    super::loading::status(scope, face)
                        == crate::font_loading::FontFaceStatus::Loading
                })
        })
    });
    !pending
}

fn finish_font_set_task<'s>(scope: &mut v8::PinScope<'s, '_>, owner: v8::Local<'s, v8::Object>) {
    // A completed face can expose another source (for example after a missing
    // glyph or a load error). Settle resource selection before resolving ready.
    super::automatic::discover_requests(scope);
    super::loading::start_queued_font_set_loads(scope, owner);
    // A synchronously completed fallback can queue another finish task and
    // expose more demand without leaving a face Loading. Let that task settle
    // selection before resolving the same ready promise and loading cycle.
    if !font_set_is_loading_and_idle(scope, owner)
        || get_private_value(scope, owner, FINISH_QUEUED).is_some_and(|value| value.is_true())
    {
        return;
    }
    let events = [
        (LOADED_FACES, "loadingdone"),
        (FAILED_FACES, "loadingerror"),
    ]
    .map(|(slot, event)| {
        let faces = get_private_value(scope, owner, slot)
            .and_then(|v| v8::Local::<v8::Array>::try_from(v).ok());
        set_private_value(scope, owner, slot, v8::undefined(scope).into());
        (faces, event)
    });
    set_font_face_set_status(scope, owner, "loaded");
    if let Some(resolver) = super::loading::resolver_from_slot(scope, owner, READY_RESOLVER) {
        set_private_value(scope, owner, READY_RESOLVER, v8::undefined(scope).into());
        let _ = resolver.resolve(scope, owner.into());
    }
    // A loadingdone handler may begin another cycle and replace its arrays.
    for (faces, event) in events {
        if let Some(array) = faces
            && (event == "loadingdone" || array.length() > 0)
        {
            let _ = dispatch_font_face_set_event(scope, owner, event, Some(array));
        }
    }
}

fn font_face_load_failed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) -> bool {
    super::loading::status(scope, face) == crate::font_loading::FontFaceStatus::Error
}
