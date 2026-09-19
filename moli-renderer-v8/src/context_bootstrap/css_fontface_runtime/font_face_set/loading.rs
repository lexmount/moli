use super::*;
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
    let _ = (&parsed.font, &parsed.text);
    rv.set(v8::Boolean::new(scope, true).into());
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
    if font_load_query_contains_css_wide_keyword(&parsed.font) {
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
    let mut failed = false;
    for index in 0..matching_faces.length() {
        let Some(face) = matching_faces
            .get_index(scope, index)
            .and_then(|face| v8::Local::<v8::Object>::try_from(face).ok())
        else {
            continue;
        };
        start_font_face_load(scope, face);
        failed |= font_face_load_failed(scope, face);
    }
    if failed {
        rv.set(
            make_rejected_dom_exception_promise(
                scope,
                "NetworkError",
                "One or more matching FontFace objects failed to load.",
            )
            .into(),
        );
        return;
    }
    let faces_value = matching_faces.into();
    match resolved_promise(scope, faces_value) {
        Some(promise) => rv.set(v8::Local::<v8::Value>::from(promise)),
        None => rv.set(v8::undefined(scope).into()),
    }
}
