use super::storage::font_face_set_faces_array;
use super::*;
use crate::util::serialize_v8_iter_array;

fn font_face_matches_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    families: &[String],
) -> bool {
    let family = super::loading::string_slot(scope, face, FONT_FACE_FAMILY_SLOT);
    let family = moli_css_parse::unquote_css_string(&family);
    families
        .iter()
        .any(|query| query.eq_ignore_ascii_case(&family))
}

pub(super) fn font_face_set_matching_faces_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    query: &str,
) -> Option<v8::Local<'s, v8::Array>> {
    let query = moli_css_parse::parse_font_shorthand(query)?;
    let faces = font_face_set_faces_array(scope, object)?;
    let mut matching = Vec::new();
    for index in 0..faces.length() {
        let Some(face) = faces.get_index(scope, index) else {
            continue;
        };
        let Ok(face_object) = v8::Local::<v8::Object>::try_from(face) else {
            continue;
        };
        if !font_face_matches_query(scope, face_object, &query.families) {
            continue;
        }
        matching.push(face);
    }
    serialize_v8_iter_array(scope, matching)
}

pub(super) fn make_rejected_dom_exception_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    message: &str,
) -> v8::Local<'s, v8::Promise> {
    let resolver = v8::PromiseResolver::new(scope).expect("resolver");
    let exception = crate::context_bootstrap::new_dom_exception_value(scope, message, name);
    let _ = resolver.reject(scope, exception);
    resolver.get_promise(scope)
}
