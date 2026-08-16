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
    if !font_face_set_has_document_load_cycle(scope, this) {
        set_font_face_set_status(scope, this, "loading");
        let _ = dispatch_font_face_set_event(scope, this, "loading", None);
        let _ = replace_font_face_set_ready_promise(scope, this);
        set_font_face_set_status(scope, this, "loaded");
        let _ = dispatch_font_face_set_event(scope, this, "loadingdone", Some(matching_faces));
    }
    let faces_value = matching_faces.into();
    match resolved_promise(scope, faces_value) {
        Some(promise) => rv.set(v8::Local::<v8::Value>::from(promise)),
        None => rv.set(v8::undefined(scope).into()),
    }
}
