use super::*;
use crate::native_bridge::document::detached_install::install_detached_parent_node_instance_properties;
use crate::util::context_host_ptr_from_global_bridge;

pub(in crate::native_bridge::document) fn build_detached_document_type_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &moli_dom::native::DomStringValue,
    public_id: &moli_dom::native::DomStringValue,
    system_id: &moli_dom::native::DomStringValue,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = new_detached_object_with_prototype(
        scope,
        "__detachedDocumentTypePrototype",
        Some("DocumentType"),
    )?;
    let state = new_detached_state_object(scope, "doctype", 10, name)?;
    let _ = state.set(
        scope,
        v8str(scope, "name").into(),
        crate::util::v8_string_from_dom_string_value(scope, name)?.into(),
    );
    let _ = state.set(
        scope,
        v8str(scope, "publicId").into(),
        crate::util::v8_string_from_dom_string_value(scope, public_id)?.into(),
    );
    let _ = state.set(
        scope,
        v8str(scope, "systemId").into(),
        crate::util::v8_string_from_dom_string_value(scope, system_id)?.into(),
    );
    define_detached_state(scope, object, state);
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) {
        let handle = unsafe { &mut *runtime_ptr }.create_document_type(name, public_id, system_id);
        define_detached_native_handle(scope, object, handle);
    }
    install_detached_document_type_instance_properties(scope, object);
    Some(object)
}

pub(in crate::native_bridge::document) fn build_detached_document_fragment_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_document: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = new_detached_object_with_prototype(
        scope,
        "__detachedDocumentFragmentPrototype",
        Some("DocumentFragment"),
    )?;
    let state = new_detached_state_object(scope, "fragment", 11, "#document-fragment")?;
    let _ = state.set(
        scope,
        v8str(scope, "ownerDocument").into(),
        owner_document.into(),
    );
    define_detached_state(scope, object, state);
    if let Some((runtime_ptr, owner_document_handle)) =
        detached_native_handle(scope, owner_document)
            .map(|handle| (context_host_ptr_from_global_bridge(scope), handle))
            .and_then(|(runtime_ptr, handle)| runtime_ptr.map(|runtime_ptr| (runtime_ptr, handle)))
            .or_else(|| node_runtime_and_handle_from_object(scope, owner_document).ok())
    {
        let handle = unsafe { &mut *runtime_ptr }
            .create_document_fragment_for_document(owner_document_handle);
        define_detached_native_handle(scope, object, handle);
    }
    install_detached_node_core_instance_properties(scope, object);
    install_detached_parent_node_instance_properties(scope, object);
    Some(object)
}
