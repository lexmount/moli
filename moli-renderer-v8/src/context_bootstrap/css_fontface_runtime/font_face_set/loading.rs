use super::*;
use crate::util::{get_private_value, set_private_value};
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FontFaceSet.check")]
struct FontFaceSetCheckArgs {
    #[webidl(required)]
    font: String,
    #[webidl(default = " ")]
    text: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FontFaceSet.load")]
struct FontFaceSetLoadArgs {
    #[webidl(required)]
    font: String,
    #[webidl(default = " ")]
    text: String,
}

pub(in crate::context_bootstrap) fn font_face_set_check_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    apply_pending_stylesheet_source_css_projections(scope);
    let Some(parsed) = webidl::parse_args::<FontFaceSetCheckArgs>(scope, &args) else {
        return;
    };
    let _ = &parsed.text;
    if !font_load_query_is_valid(&parsed.font) {
        let error = new_dom_exception_value(
            scope,
            "The provided font shorthand is invalid.",
            "SyntaxError",
        );
        scope.throw_exception(error);
        return;
    }
    let faces = font_face_set_matching_faces_array(scope, args.this(), &parsed.font);
    let loaded = faces.is_none_or(|faces| {
        (0..faces.length()).all(|index| {
            faces
                .get_index(scope, index)
                .and_then(|face| v8::Local::<v8::Object>::try_from(face).ok())
                .is_none_or(|face| font_face_status(scope, face).as_deref() == Some("loaded"))
        })
    });
    rv.set_bool(loaded);
}

pub(in crate::context_bootstrap) fn font_face_set_load_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    apply_pending_stylesheet_source_css_projections(scope);
    let this = args.this();
    let Some(parsed) = webidl::parse_args::<FontFaceSetLoadArgs>(scope, &args) else {
        return;
    };
    let _ = &parsed.text;
    if !font_load_query_is_valid(&parsed.font) {
        rv.set(
            make_rejected_dom_exception_promise(
                scope,
                "SyntaxError",
                "The provided font shorthand is invalid.",
            )
            .into(),
        );
        return;
    }
    let matching_faces = font_face_set_matching_faces_array(scope, this, &parsed.font)
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    if matching_faces.length() == 0 {
        let _ = resolver.resolve(scope, matching_faces.into());
        return;
    }
    let state = v8::Object::new(scope);
    set_private_value(scope, state, LOAD_RESOLVER, resolver.into());
    set_private_value(scope, state, LOAD_FACES, matching_faces.into());
    let remaining = v8::Integer::new_from_unsigned(scope, matching_faces.length());
    set_private_value(scope, state, LOAD_REMAINING, remaining.into());
    let Some(fulfilled) = v8::Function::builder(font_set_load_fulfilled)
        .data(state.into())
        .build(scope)
    else {
        return;
    };
    let Some(rejected) = v8::Function::builder(font_set_load_rejected)
        .data(state.into())
        .build(scope)
    else {
        return;
    };
    for index in 0..matching_faces.length() {
        if let Some(face) = matching_faces
            .get_index(scope, index)
            .and_then(|face| v8::Local::<v8::Object>::try_from(face).ok())
        {
            if let Some(loaded) = ensure_font_face_loaded_promise(scope, face) {
                let _ = loaded.then2(scope, fulfilled, rejected);
            }
            start_font_face_load(scope, face);
        }
    }
}

const LOAD_RESOLVER: &str = "__moliFontSetLoadResolver";
const LOAD_FACES: &str = "__moliFontSetLoadFaces";
const LOAD_REMAINING: &str = "__moliFontSetLoadRemaining";

fn load_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    get_private_value(scope, state, LOAD_RESOLVER)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|value| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(value) })
}

fn font_set_load_fulfilled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(state) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    let remaining = get_private_value(scope, state, LOAD_REMAINING)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    if remaining == 0 {
        return;
    }
    let next = v8::Integer::new_from_unsigned(scope, remaining - 1);
    set_private_value(scope, state, LOAD_REMAINING, next.into());
    if remaining == 1
        && let Some(resolver) = load_resolver(scope, state)
        && let Some(faces) = get_private_value(scope, state, LOAD_FACES)
    {
        let _ = resolver.resolve(scope, faces);
    }
}

fn font_set_load_rejected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(state) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    let zero = v8::Integer::new(scope, 0);
    set_private_value(scope, state, LOAD_REMAINING, zero.into());
    if let Some(resolver) = load_resolver(scope, state) {
        let _ = resolver.reject(scope, args.get(0));
    }
}
