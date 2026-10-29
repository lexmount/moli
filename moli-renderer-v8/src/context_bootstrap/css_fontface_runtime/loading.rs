use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::font_loading::{FontFaceLoad, FontFaceResource, FontFaceStatus};
use crate::util::{get_private_value, set_private_value};

const RESOLVER: &str = "__moliFontFaceResolver";
const PROMISE_OWNER: &str = "__moliFontFacePromiseOwner";
const REQUEST_QUEUED: &str = "__moliFontFaceRequestQueued";
const OWNER_DOCUMENT: &str = "__moliFontFaceSetOwnerDocument";
static NEXT_REGISTRATION: AtomicU64 = AtomicU64::new(1);
const QUERY_RESOLVER: &str = "__moliFontQueryResolver";
const QUERY_FACES: &str = "__moliFontQueryFaces";
const QUERY_REMAINING: &str = "__moliFontQueryRemaining";
const QUERY_STATE: &str = "__moliFontQueryState";

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
    // Keep the direct promise's state in the JS heap as well. Native pending
    // font loads root each face until completion or Window retirement, so a
    // downstream .then()/await promise need not retain this original promise.
    set_private_value(scope, promise.into(), QUERY_STATE, state.into());
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
        release_query_state(scope, state, resolver);
        let _ = resolver.resolve(scope, faces);
    }
}

fn query_face_failed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let state = v8::Local::<v8::Object>::try_from(args.data()).expect("font query state");
    if get_private_value(scope, state, QUERY_REMAINING)
        .and_then(|v| v.uint32_value(scope))
        .unwrap_or(0)
        == 0
    {
        return;
    }
    let resolver = resolver_from_slot(scope, state, QUERY_RESOLVER).expect("font query resolver");
    release_query_state(scope, state, resolver);
    let _ = resolver.reject(scope, args.get(0));
}

fn release_query_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) {
    set_private_value(
        scope,
        state,
        QUERY_REMAINING,
        v8::Integer::new(scope, 0).into(),
    );
    set_private_value(
        scope,
        resolver.get_promise(scope).into(),
        QUERY_STATE,
        v8::undefined(scope).into(),
    );
    for slot in [QUERY_FACES, QUERY_RESOLVER] {
        set_private_value(scope, state, slot, v8::undefined(scope).into());
    }
}

pub(super) fn resolver_from_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    let value = get_private_value(scope, object, slot).filter(|value| value.is_object())?;
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
        &string_slot(scope, face, FONT_FACE_UNICODE_RANGE_SLOT),
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

pub(super) fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    resource: FontFaceLoad,
    request_context: Option<crate::native_bridge::WindowExecutionContextIdentity>,
) {
    super::state::initialize(scope, face, resource, request_context);
    publish_state(scope, face);
}

pub(super) fn loaded_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Promise>> {
    if let Some(promise) = get_private_value(scope, face, FONT_FACE_LOADED_SLOT)
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())
    {
        return Some(promise);
    }
    let load = super::state::get(scope, face)?;
    let context = face.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    // Native errors can exist before page code observes .loaded. Creating a
    // rejected promise in the constructor would report an unhandled rejection
    // even when the page never obtains that promise.
    let resolver = v8::PromiseResolver::new(scope).expect("FontFace loaded resolver");
    let promise = resolver.get_promise(scope);
    set_private_value(scope, face, FONT_FACE_LOADED_SLOT, promise.into());
    set_private_value(scope, face, RESOLVER, resolver.into());
    // A retained pending .loaded promise also retains its settlement endpoint.
    // Loading faces have a Window-owned native root, released on completion
    // or retirement; unloaded JS-only cycles remain collectible.
    set_private_value(scope, promise.into(), PROMISE_OWNER, face.into());
    settle_loaded_promise(scope, face, &load);
    Some(promise)
}

/// Only the observer layer creates JS errors, settles promises and dispatches
/// events. Native CSS completions update the same resource without touching V8.
fn publish_state<'s>(scope: &mut v8::PinScope<'s, '_>, face: v8::Local<'s, v8::Object>) {
    // Synchronize before running any reentrant loading handler, including when
    // rebinding replaced the resource without changing its published status.
    super::state::sync_loading_root(scope, face);
    let Some((previous, load)) = super::state::take_change(scope, face) else {
        return;
    };
    let (current, started) = {
        let load = load.borrow();
        (load.status(), load.was_started())
    };
    if previous == FontFaceStatus::Unloaded && started {
        super::events::font_face_loading_started(scope, face);
    }
    match current {
        FontFaceStatus::Loaded => {
            sync_owners(scope, face);
        }
        FontFaceStatus::Error => {}
        FontFaceStatus::Unloaded | FontFaceStatus::Loading => return,
    }
    settle_loaded_promise(scope, face, &load);
    if started {
        super::events::notify_font_face_set_owners_of_load(scope, face);
    }
}

fn settle_loaded_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    load: &FontFaceLoad,
) {
    let Some(resolver) = resolver_from_slot(scope, face, RESOLVER) else {
        return;
    };
    let (status, error) = {
        let load = load.borrow();
        (load.status(), load.error())
    };
    match status {
        FontFaceStatus::Loaded => {
            release_promise_owner(scope, face, resolver);
            let _ = resolver.resolve(scope, face.into());
        }
        FontFaceStatus::Error => {
            let error = error.expect("native failed font has an error");
            let exception = new_dom_exception_value(scope, error.message, error.name);
            release_promise_owner(scope, face, resolver);
            let _ = resolver.reject(scope, exception);
        }
        FontFaceStatus::Unloaded | FontFaceStatus::Loading => {}
    }
}

fn release_promise_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) {
    set_private_value(
        scope,
        resolver.get_promise(scope).into(),
        PROMISE_OWNER,
        v8::undefined(scope).into(),
    );
    set_private_value(scope, face, RESOLVER, v8::undefined(scope).into());
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
    let promise = loaded_promise(scope, face)?;
    start_font_load(scope, face);
    Some(promise)
}

fn start_font_load<'s>(scope: &mut v8::PinScope<'s, '_>, face: v8::Local<'s, v8::Object>) {
    let Some(load) = super::state::get(scope, face) else {
        return;
    };
    let Some(context) = face.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let start = load.borrow_mut().begin();
    publish_state(scope, face);
    if start {
        advance_sources(scope, face, &load);
    }
}

pub(super) fn queue_requested_font_loads(
    scope: &mut v8::PinScope<'_, '_>,
    requests: &std::collections::BTreeSet<String>,
) {
    if requests.is_empty() {
        return;
    }
    for face in super::state::wrappers_for_slots(scope, requests) {
        if status(scope, face) != FontFaceStatus::Unloaded {
            continue;
        }
        let Some(context) = face.get_creation_context(scope) else {
            continue;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) else {
            continue;
        };
        // An accepted task can also be discarded if its Window retires before
        // dispatch. Check the creation lifetime before honoring the queue flag.
        let current = super::state::request_context(scope, face).is_some_and(|identity| {
            unsafe { &*host_ptr }.window_execution_context_identity_is_current(identity)
        });
        if !current {
            let unqueued = v8::undefined(scope);
            set_private_value(scope, face, REQUEST_QUEUED, unqueued.into());
            start_font_load(scope, face);
            continue;
        }
        if get_private_value(scope, face, REQUEST_QUEUED).is_some_and(|value| value.is_true()) {
            continue;
        }
        let callback = v8::Function::builder(run_requested_font_load)
            .data(face.into())
            .build(scope)
            .expect("font resource task");
        if unsafe { &mut *host_ptr }.queue_internal_task(scope, callback) {
            let queued = v8::Boolean::new(scope, true);
            set_private_value(scope, face, REQUEST_QUEUED, queued.into());
        } else {
            // Use the normal terminal-error and owner-notification path so
            // another face can satisfy demand without a FontFaceSet getter.
            start_font_load(scope, face);
        }
    }
}

fn run_requested_font_load<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let face = v8::Local::<v8::Object>::try_from(args.data()).expect("font resource face");
    start_queued_font_load(scope, face);
}

pub(super) fn start_queued_font_load<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) {
    if get_private_value(scope, face, REQUEST_QUEUED).is_some_and(|value| value.is_true()) {
        set_private_value(scope, face, REQUEST_QUEUED, v8::undefined(scope).into());
        start_font_load(scope, face);
    }
}

pub(super) fn start_queued_font_set_loads<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    fonts: v8::Local<'s, v8::Object>,
) {
    if let Some(faces) = super::storage::font_face_set_faces_array(scope, fonts) {
        for index in 0..faces.length() {
            if let Some(face) = faces
                .get_index(scope, index)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            {
                start_queued_font_load(scope, face);
            }
        }
    }
}

fn advance_sources<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    load: &FontFaceLoad,
) {
    let request_context = super::state::request_context(scope, face);
    if let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope)
        && !request_context.is_some_and(|identity| {
            unsafe { &*host_ptr }.window_execution_context_identity_is_current(identity)
        })
    {
        load.borrow_mut().fail(
            "NetworkError",
            "FontFace creation Window is no longer available.",
        );
        publish_font_face_load_changes(scope);
        return;
    }
    loop {
        let url = with_font_services(scope, |services| load.borrow_mut().next_url(services));
        let Some(url) = url else {
            break;
        };
        match crate::network_host::start_font_face_fetch(scope, face, request_context, &url) {
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
        sync_registration(scope, owner, face);
    }
}

pub(super) fn sync_registration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    face: v8::Local<'s, v8::Object>,
) {
    let Some(document) = get_private_value(scope, owner, OWNER_DOCUMENT)
        .and_then(|value| crate::native_bridge::callback_value_dom_handle(scope, value))
    else {
        return;
    };
    let Some(load) = super::state::get(scope, face) else {
        return;
    };
    let registration_scope = super::storage::font_face_set_registration_scope(scope, owner, face);
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
    let changed = host.with_font_services(document, |services| {
        let load = load.borrow();
        match load.register(services, registration_scope) {
            Ok(Some(
                moli_layout::WebFontRegistrationOutcome::Added
                | moli_layout::WebFontRegistrationOutcome::Replaced,
            )) => true,
            Ok(_) => false,
            Err(error) => {
                tracing::warn!(%error, "failed to register FontFace");
                false
            }
        }
    });
    if changed {
        host.invalidate_document_font_layout(document);
    }
}
