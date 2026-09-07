use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::util::{get_private_value, set_private_value, v8str};
use moli_layout::{FontFaceData, WebFontFace, WebFontRegistration, WebFontStyle};

const RESOLVER: &str = "__moliFontFaceResolver";
const DATA: &str = "__moliFontFaceData";
const NEXT_SOURCE: &str = "__moliFontFaceNextSource";
const REGISTRATION: &str = "__moliFontFaceRegistration";
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

pub(super) fn initialize_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) {
    set_private_value(scope, face, RESOLVER, resolver.into());
    let id = NEXT_REGISTRATION.fetch_add(1, Ordering::Relaxed);
    let registration =
        v8_string(scope, &format!("js-font-face-{id}")).expect("font registration id");
    set_private_value(scope, face, REGISTRATION, registration.into());
}

pub(super) fn set_status<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    status: &'static str,
) {
    set_private_value(
        scope,
        face,
        FONT_FACE_STATUS_SLOT,
        v8str(scope, status).into(),
    );
}

pub(super) fn settle_font<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    result: Result<FontFaceData, (&str, &str)>,
) {
    let Some(resolver) = resolver_from_slot(scope, face, RESOLVER) else {
        return;
    };
    let was_loading = string_slot(scope, face, FONT_FACE_STATUS_SLOT) == "loading";
    match result {
        Ok(data) => {
            let backing =
                v8::ArrayBuffer::new_backing_store_from_vec(data.bytes().to_vec()).make_shared();
            let buffer = v8::ArrayBuffer::with_backing_store(scope, &backing);
            set_private_value(scope, face, DATA, buffer.into());
            set_status(scope, face, "loaded");
            sync_owners(scope, face);
            let _ = resolver.resolve(scope, face.into());
        }
        Err((name, message)) => {
            set_status(scope, face, "error");
            let error = new_dom_exception_value(scope, message, name);
            let _ = resolver.reject(scope, error);
        }
    }
    if was_loading {
        super::events::notify_font_face_set_owners_of_load(scope, face);
    }
}

pub(super) fn load_font<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let promise = get_private_value(scope, face, FONT_FACE_LOADED_SLOT)
        .and_then(|v| v8::Local::<v8::Promise>::try_from(v).ok())?;
    if string_slot(scope, face, FONT_FACE_STATUS_SLOT) != "unloaded" {
        return Some(promise);
    }
    let context = face.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    set_status(scope, face, "loading");
    super::events::font_face_loading_started(scope, face);
    try_next_source(scope, face);
    Some(promise)
}

fn try_next_source<'s>(scope: &mut v8::PinScope<'s, '_>, face: v8::Local<'s, v8::Object>) {
    let source = string_slot(scope, face, FONT_FACE_SOURCE_SLOT);
    let Some(sources) = moli_css_parse::parse_font_face_sources(&source) else {
        settle_font(
            scope,
            face,
            Err(("SyntaxError", "Invalid FontFace source.")),
        );
        return;
    };
    let next = get_private_value(scope, face, NEXT_SOURCE)
        .and_then(|v| v.uint32_value(scope))
        .unwrap_or(0) as usize;
    for (index, source) in sources.into_iter().enumerate().skip(next) {
        set_private_value(
            scope,
            face,
            NEXT_SOURCE,
            v8::Integer::new_from_unsigned(scope, index as u32 + 1).into(),
        );
        match source {
            moli_css_parse::CssFontSource::Local(name) => {
                if let Some(data) =
                    with_font_services(scope, |services| services.local_font_source(&name))
                {
                    settle_font(scope, face, Ok(data));
                    return;
                }
            }
            moli_css_parse::CssFontSource::Url(url) => {
                match crate::network_host::start_font_face_fetch(scope, face, &url) {
                    Ok(crate::network_host::FontFaceFetchStart::Pending) => return,
                    Ok(crate::network_host::FontFaceFetchStart::Local(bytes)) => {
                        if let Ok(data) = FontFaceData::from_bytes(&bytes) {
                            settle_font(scope, face, Ok(data));
                            return;
                        }
                    }
                    Err(_) => {}
                }
            }
        }
    }
    settle_font(
        scope,
        face,
        Err((
            "NetworkError",
            "A network error occurred while loading the font.",
        )),
    );
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
    if let Some(data) = bytes.and_then(|bytes| FontFaceData::from_bytes(bytes).ok()) {
        settle_font(scope, face, Ok(data));
    } else {
        try_next_source(scope, face);
    }
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
    let slot = string_slot(scope, face, REGISTRATION);
    if slot.is_empty() {
        return;
    }
    let context = owner.get_creation_context(scope);
    let Some(context) = context else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    // SAFETY: the document's realm owns this bridge; registration performs no JS calls.
    let host = unsafe { &*host_ptr };
    if !present {
        host.with_font_services(document, |services| services.remove_web_font(&slot));
        return;
    }
    let Some(buffer) = get_private_value(scope, face, DATA)
        .and_then(|v| v8::Local::<v8::ArrayBuffer>::try_from(v).ok())
    else {
        return;
    };
    let backing = buffer.get_backing_store();
    let bytes = backing.iter().map(|byte| byte.get()).collect::<Vec<_>>();
    let family = string_slot(scope, face, FONT_FACE_FAMILY_SLOT);
    let weight = string_slot(scope, face, FONT_FACE_WEIGHT_SLOT);
    let weight = if weight == "bold" {
        700.0
    } else {
        weight.parse().unwrap_or(400.0)
    };
    let stretch = string_slot(scope, face, FONT_FACE_STRETCH_SLOT);
    let stretch = stretch
        .strip_suffix('%')
        .and_then(|s| s.parse().ok())
        .unwrap_or(100.0);
    let style = match string_slot(scope, face, FONT_FACE_STYLE_SLOT).as_str() {
        "italic" => WebFontStyle::Italic,
        value if value.starts_with("oblique") => WebFontStyle::Oblique(Some(14.0)),
        _ => WebFontStyle::Normal,
    };
    let registration = WebFontRegistration::new(
        slot,
        WebFontFace::new(moli_css_parse::unquote_css_string(&family))
            .with_weight(weight)
            .with_stretch(stretch)
            .with_style(style),
        bytes,
    );
    host.with_font_services(document, |services| {
        services.register_web_font(registration)
    })
    .expect("FontFace stored validated font data and descriptors");
}
