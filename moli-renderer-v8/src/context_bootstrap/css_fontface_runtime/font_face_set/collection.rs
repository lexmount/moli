use super::*;

pub(in crate::context_bootstrap) fn font_face_set_add_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    apply_pending_stylesheet_source_css_projections(scope);
    let this = args.this();
    let candidate = args.get(0);
    if !is_font_face_value(scope, candidate) {
        throw_type_error(scope, "Expected FontFace");
        return;
    }
    if font_face_set_faces_array(scope, this)
        .is_some_and(|faces| array_contains_value(scope, faces, candidate))
    {
        rv.set(this.into());
        return;
    }
    let Some(faces) = font_face_set_manual_faces_array(scope, this) else {
        rv.set(this.into());
        return;
    };
    let _ = faces.set_index(scope, faces.length(), candidate);
    rebuild_font_face_set_faces(scope, this);
    rv.set(this.into());
}

pub(in crate::context_bootstrap) fn font_face_set_has_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    apply_pending_stylesheet_source_css_projections(scope);
    let candidate = args.get(0);
    if !is_font_face_value(scope, candidate) {
        throw_type_error(scope, "Expected FontFace");
        return;
    }
    let present = font_face_set_faces_array(scope, args.this())
        .map(|faces| array_contains_value(scope, faces, candidate))
        .unwrap_or(false);
    rv.set(v8::Boolean::new(scope, present).into());
}

pub(in crate::context_bootstrap) fn font_face_set_delete_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    apply_pending_stylesheet_source_css_projections(scope);
    let candidate = args.get(0);
    if !is_font_face_value(scope, candidate) {
        throw_type_error(scope, "Expected FontFace");
        return;
    }
    let Some(faces) = font_face_set_manual_faces_array(scope, args.this()) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let next = v8::Array::new(scope, 0);
    let mut removed = false;
    for index in 0..faces.length() {
        let Some(existing) = faces.get_index(scope, index) else {
            continue;
        };
        if existing.strict_equals(candidate) {
            removed = true;
            continue;
        }
        let _ = next.set_index(scope, next.length(), existing);
    }
    if removed {
        set_font_face_set_slot_value(
            scope,
            args.this(),
            FONT_FACE_SET_MANUAL_FACES_SLOT,
            next.into(),
        );
        rebuild_font_face_set_faces(scope, args.this());
    }
    rv.set(v8::Boolean::new(scope, removed).into());
}

pub(in crate::context_bootstrap) fn font_face_set_clear_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    apply_pending_stylesheet_source_css_projections(scope);
    let manual_faces = v8::Array::new(scope, 0);
    set_font_face_set_slot_value(
        scope,
        args.this(),
        FONT_FACE_SET_MANUAL_FACES_SLOT,
        manual_faces.into(),
    );
    rebuild_font_face_set_faces(scope, args.this());
    rv.set_undefined();
}
