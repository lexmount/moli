//! IndexedDB's sorted UTF-16 name-list projection onto the shared DOMStringList.
pub(in crate::context_bootstrap::indexed_db) use moli_webapi::dom_string_list::install_dom_string_list_template_bindings;

pub(in crate::context_bootstrap::indexed_db) fn new_idb_name_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    names: &[moli_indexeddb::IndexedDbName],
) -> v8::Local<'s, v8::Object> {
    let mut names = names.to_vec();
    names.sort();
    let elements = names
        .iter()
        .map(|name| {
            crate::util::v8_string_from_utf16_units(scope, name.as_utf16())
                .expect("IndexedDB name fits in a V8 string")
                .into()
        })
        .collect::<Vec<v8::Local<v8::Value>>>();
    let values = v8::Array::new_with_elements(scope, &elements);
    moli_webapi::dom_string_list::new_dom_string_list(scope, values)
}
