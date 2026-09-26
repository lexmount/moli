use super::storage::font_face_set_faces_array;
use super::*;
use crate::util::serialize_v8_iter_array;

pub(super) fn font_load_query_is_valid(query: &str) -> bool {
    moli_css_parse::font_load_query_is_valid(query)
}

fn font_face_matches_query(
    scope: &mut v8::PinScope<'_, '_>,
    face: v8::Local<'_, v8::Object>,
    query: &str,
) -> bool {
    let Some(query_family) = moli_css_parse::font_load_query_family(query) else {
        return false;
    };
    object_string_property(scope, face, "family").is_some_and(|family| family == query_family)
}

pub(super) fn font_face_set_matching_faces_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    query: &str,
) -> Option<v8::Local<'s, v8::Array>> {
    let faces = font_face_set_faces_array(scope, object)?;
    let mut matching = Vec::new();
    for index in 0..faces.length() {
        let Some(face) = faces.get_index(scope, index) else {
            continue;
        };
        let Ok(face_object) = v8::Local::<v8::Object>::try_from(face) else {
            continue;
        };
        if !font_face_matches_query(scope, face_object, query) {
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
