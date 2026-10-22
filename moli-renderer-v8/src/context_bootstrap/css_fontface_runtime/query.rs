use super::storage::font_face_set_faces_array;
use crate::util::serialize_v8_iter_array;

mod selection;

struct Candidate<'s> {
    face: v8::Local<'s, v8::Value>,
    descriptor: std::sync::Arc<moli_layout::WebFontFace>,
    selection: selection::Selection,
}

pub(super) fn font_face_set_matching_faces_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    query: &str,
    text: &str,
) -> Option<v8::Local<'s, v8::Array>> {
    if !moli_css_parse::font_load_query_is_valid(query) {
        return None;
    }
    // System font shorthands are valid queries, but Stylo's Servo configuration
    // does not expose their platform families for matching authored faces.
    let Some(query) = moli_css_parse::parse_font_shorthand(query) else {
        return serialize_v8_iter_array(scope, std::iter::empty::<v8::Local<'s, v8::Value>>());
    };
    let requested = selection::Selection::new(&query.weight, &query.stretch, &query.style);
    let faces = font_face_set_faces_array(scope, object)?;
    let mut candidates = Vec::new();
    for index in 0..faces.length() {
        let Some(face) = faces.get_index(scope, index) else {
            continue;
        };
        let Ok(face_object) = v8::Local::<v8::Object>::try_from(face) else {
            continue;
        };
        // CSS parsing replaces unpaired surrogates; such a literal FontFace
        // name must not alias the replacement character in a parsed query.
        if super::font_face::font_face_raw_family(scope, face_object)?
            .as_str()
            .is_none()
        {
            continue;
        }
        let load = super::state::get(scope, face_object).expect("FontFace has native state");
        let descriptor = load.borrow().descriptor().clone();
        // Constructors and successful setters update this same typed descriptor
        // for query matching and layout. Querying never reparses JS slots.
        let selection = descriptor
            .css_capabilities()
            .expect("FontFace has computed CSS descriptors");
        candidates.push(Candidate {
            face,
            descriptor,
            selection,
        });
    }
    let text = text.chars().collect::<Vec<_>>();
    let mut matching = Vec::new();
    for family in query.named_families {
        let family = moli_css_parse::font_family_name_key(&family);
        let family_faces = candidates
            .iter()
            .filter(|face| family == face.descriptor.family_name_key())
            .collect::<Vec<_>>();
        let Some(best) = family_faces
            .iter()
            .min_by(|a, b| a.selection.compare(b.selection, requested))
        else {
            continue;
        };
        for face in &family_faces {
            // Select capabilities from the entire family first. Filtering by
            // text or load state earlier could choose an unrelated fallback
            // weight/style instead of the CSS-matched segmented face.
            if face.selection.compare(best.selection, requested).is_eq()
                && text.iter().any(|&character| {
                    face.descriptor.unicode_ranges().is_empty()
                        || face
                            .descriptor
                            .unicode_ranges()
                            .iter()
                            .any(|range| range.contains(character))
                })
            {
                matching.push(face.face);
            }
        }
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
