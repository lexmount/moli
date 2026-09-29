use super::*;

pub(in crate::native_bridge::document) fn detached_text_content<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<String> {
    match detached_node_type(scope, node) {
        Some(9 | 10) => None,
        Some(3 | 4 | 8 | 7) => Some(detached_character_data_value(scope, node)),
        Some(_) => Some(detached_child_node_objects(scope, node).into_iter().fold(
            String::new(),
            |mut out, child| {
                append_detached_text_content(scope, child, &mut out);
                out
            },
        )),
        None => Some(String::new()),
    }
}

fn append_detached_text_content<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    out: &mut String,
) {
    match detached_node_type(scope, node) {
        Some(3 | 4) => out.push_str(&detached_character_data_value(scope, node)),
        Some(7..=10) | None => {}
        Some(_) => {
            for child in detached_child_node_objects(scope, node) {
                append_detached_text_content(scope, child, out);
            }
        }
    }
}
