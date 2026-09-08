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
    let _ = &parsed.text;
    let Some(faces) = font_face_set_matching_faces_array(scope, args.this(), &parsed.font) else {
        webidl::throw_dom_exception(
            scope,
            "SyntaxError",
            "The provided font shorthand is invalid.",
        );
        return;
    };
    let loaded = (0..faces.length()).all(|index| {
        faces
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .is_some_and(|face| {
                super::super::loading::string_slot(scope, face, FONT_FACE_STATUS_SLOT) == "loaded"
            })
    });
    rv.set(v8::Boolean::new(scope, loaded).into());
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
    let Some(matching_faces) = font_face_set_matching_faces_array(scope, this, &parsed.font) else {
        rv.set(
            make_rejected_dom_exception_promise(
                scope,
                "SyntaxError",
                "The provided font shorthand is invalid.",
            )
            .into(),
        );
        return;
    };
    rv.set(super::super::loading::load_matching_faces(scope, matching_faces).into());
}
