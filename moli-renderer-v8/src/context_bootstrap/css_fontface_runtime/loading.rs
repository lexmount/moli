use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::font_loading::{FontFaceLoad, FontFaceResource, FontFaceStatus};
use crate::util::{get_private_value, set_private_value};

const RESOLVER: &str = "__moliFontFaceResolver";
const OWNER_DOCUMENT: &str = "__moliFontFaceSetOwnerDocument";
static NEXT_REGISTRATION: AtomicU64 = AtomicU64::new(1);
const QUERY_RESOLVER: &str = "__moliFontQueryResolver";
const QUERY_FACES: &str = "__moliFontQueryFaces";
const QUERY_REMAINING: &str = "__moliFontQueryRemaining";

pub(super) fn load_matching_faces<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    faces: v8::Local<'s, v8::Array>,
) -> v8::Local<'s, v8::Promise> {
    let resolver = v8::PromiseResolver::new(scope).expect("FontFaceSet.load resolver");
    let promise = resolver.get_promise(scope);
    if faces.length() == 0 {
        let _ = resolver.resolve(scope, faces.into());
        return promise;
    }
    let state = v8::Object::new(scope);
    set_private_value(scope, state, QUERY_RESOLVER, resolver.into());
    set_private_value(scope, state, QUERY_FACES, faces.into());
    set_private_value(
        scope,
        state,
        QUERY_REMAINING,
        v8::Integer::new_from_unsigned(scope, faces.length()).into(),
    );
    let fulfilled = v8::Function::builder(query_face_loaded)
        .data(state.into())
        .build(scope)
        .expect("font loaded callback");
    let rejected = v8::Function::builder(query_face_failed)
        .data(state.into())
        .build(scope)
        .expect("font failed callback");
    for index in 0..faces.length() {
        let face = faces
            .get_index(scope, index)
            .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
            .expect("matched FontFace");
        if let Some(loaded) = load_font(scope, face) {
            let _ = loaded.then2(scope, fulfilled, rejected);
        }
    }
    promise
}

fn query_face_loaded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let state = v8::Local::<v8::Object>::try_from(args.data()).expect("font query state");
    let remaining = get_private_value(scope, state, QUERY_REMAINING)
        .and_then(|v| v.uint32_value(scope))
        .unwrap_or(0);
    if remaining == 0 {
        return;
    }
    set_private_value(
        scope,
        state,
        QUERY_REMAINING,
        v8::Integer::new_from_unsigned(scope, remaining - 1).into(),
    );
    if remaining == 1 {
        let resolver =
            resolver_from_slot(scope, state, QUERY_RESOLVER).expect("font query resolver");
        let faces = get_private_value(scope, state, QUERY_FACES).expect("font query faces");
        let _ = resolver.resolve(scope, faces);
    }
}

fn query_face_failed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let state = v8::Local::<v8::Object>::try_from(args.data()).expect("font query state");
    set_private_value(
        scope,
        state,
        QUERY_REMAINING,
        v8::Integer::new(scope, 0).into(),
    );
    let resolver = resolver_from_slot(scope, state, QUERY_RESOLVER).expect("font query resolver");
    let _ = resolver.reject(scope, args.get(0));
}

pub(super) fn resolver_from_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    let value = get_private_value(scope, object, slot)?;
    // SAFETY: these private slots are populated only by PromiseResolver::new
    // above (and FontFaceSet.ready). They are never writable by page code.
    Some(unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(value) })
}

pub(super) fn string_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    slot: &str,
) -> String {
    get_private_value(scope, face, slot)
        .and_then(|v| v8::Local::<v8::String>::try_from(v).ok())
        .map(|v| v.to_rust_string_lossy(scope))
        .unwrap_or_default()
}

pub(super) fn status<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) -> FontFaceStatus {
    super::state::get(scope, face)
        .expect("FontFace has native state")
        .borrow()
        .status()
}

pub(super) fn new_resource<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    sources: Vec<moli_css_parse::CssFontSource>,
) -> FontFaceLoad {
    let id = NEXT_REGISTRATION.fetch_add(1, Ordering::Relaxed);
    FontFaceResource::new(
        format!("js-font-face-{id}"),
        descriptor(scope, face),
        sources,
    )
}

fn descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) -> moli_layout::WebFontFace {
    crate::font_loading::script_font_descriptor(
        &string_slot(scope, face, FONT_FACE_FAMILY_SLOT),
        &string_slot(scope, face, FONT_FACE_WEIGHT_SLOT),
        &string_slot(scope, face, FONT_FACE_STRETCH_SLOT),
        &string_slot(scope, face, FONT_FACE_STYLE_SLOT),
    )
}

pub(super) fn update_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) {
    let load = super::state::get(scope, face).expect("FontFace has native state");
    load.borrow_mut().set_descriptor(descriptor(scope, face));
    sync_owners(scope, face);
}

pub(super) fn initialize_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    resource: FontFaceLoad,
) {
    set_private_value(scope, face, RESOLVER, resolver.into());
    super::state::initialize(scope, face, resource);
    publish_state(scope, face);
}

/// Only the observer layer creates JS errors, settles promises and dispatches
/// events. Native CSS completions update the same resource without touching V8.
fn publish_state<'s>(scope: &mut v8::PinScope<'s, '_>, face: v8::Local<'s, v8::Object>) {
    let Some((previous, load)) = super::state::take_change(scope, face) else {
        return;
    };
    let (current, started, error) = {
        let load = load.borrow();
        (load.status(), load.was_started(), load.error())
    };
    if previous == FontFaceStatus::Unloaded && started {
        super::events::font_face_loading_started(scope, face);
    }
    match current {
        FontFaceStatus::Loaded => {
            sync_owners(scope, face);
            if let Some(resolver) = resolver_from_slot(scope, face, RESOLVER) {
                let _ = resolver.resolve(scope, face.into());
            }
        }
        FontFaceStatus::Error => {
            if let Some(resolver) = resolver_from_slot(scope, face, RESOLVER) {
                let error = error.expect("native failed font has an error");
                let exception = new_dom_exception_value(scope, error.message, error.name);
                let _ = resolver.reject(scope, exception);
            }
        }
        FontFaceStatus::Unloaded | FontFaceStatus::Loading => return,
    }
    if started {
        super::events::notify_font_face_set_owners_of_load(scope, face);
    }
}

pub(crate) fn publish_font_face_load_changes(scope: &mut v8::PinScope<'_, '_>) {
    for face in super::state::changed_wrappers(scope) {
        if let Some(context) = face.get_creation_context(scope) {
            let scope = &mut v8::ContextScope::new(scope, context);
            publish_state(scope, face);
        }
    }
}

pub(crate) fn bind_stylesheet_font_face<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    load: FontFaceLoad,
) -> bool {
    if !super::state::bind(scope, face, load) {
        return false;
    }
    publish_state(scope, face);
    true
}

pub(super) fn load_font<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let promise = get_private_value(scope, face, FONT_FACE_LOADED_SLOT)
        .and_then(|v| v8::Local::<v8::Promise>::try_from(v).ok())?;
    let load = super::state::get(scope, face)?;
    let context = face.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let start = load.borrow_mut().begin();
    publish_state(scope, face);
    if start {
        advance_sources(scope, face, &load);
    }
    Some(promise)
}

fn advance_sources<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    load: &FontFaceLoad,
) {
    loop {
        let url = with_font_services(scope, |services| load.borrow_mut().next_url(services));
        let Some(url) = url else {
            break;
        };
        match crate::network_host::start_font_face_fetch(scope, face, &url) {
            Ok(crate::network_host::FontFaceFetchStart::Pending) => return,
            Ok(crate::network_host::FontFaceFetchStart::Local(bytes)) => {
                let _ = load.borrow_mut().accept_response(Some(&bytes));
            }
            Err(_) => {}
        }
    }
    publish_font_face_load_changes(scope);
}

pub(crate) fn finish_font_face_url_load<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    bytes: Option<&[u8]>,
) {
    let Some(context) = face.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let load = super::state::get(scope, face).expect("pending font retains native state");
    let _ = load.borrow_mut().accept_response(bytes);
    advance_sources(scope, face, &load);
}

pub(super) fn sync_owners<'s>(scope: &mut v8::PinScope<'s, '_>, face: v8::Local<'s, v8::Object>) {
    for owner in super::storage::font_face_set_owner_snapshot(scope, face) {
        sync_registration(scope, owner, face, true);
    }
}

pub(super) fn sync_registration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    face: v8::Local<'s, v8::Object>,
    present: bool,
) {
    let Some(document) = get_private_value(scope, owner, OWNER_DOCUMENT)
        .and_then(|value| crate::native_bridge::callback_value_dom_handle(scope, value))
    else {
        return;
    };
    let Some(load) = super::state::get(scope, face) else {
        return;
    };
    let Some(context) = owner.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    // SAFETY: this is the owner document's live bridge; the native font service
    // does not invoke V8 or page code while borrowed.
    let host = unsafe { &*host_ptr };
    host.with_font_services(document, |services| {
        let load = load.borrow();
        if present {
            if let Err(error) = load.register(services) {
                tracing::warn!(%error, "failed to register FontFace");
            }
        } else {
            load.unregister(services);
        }
    });
}
