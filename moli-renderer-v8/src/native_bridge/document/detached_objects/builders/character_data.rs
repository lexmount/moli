use super::*;
use crate::util::context_host_ptr_from_global_bridge;

pub(in crate::native_bridge::document) fn build_detached_text_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_document: v8::Local<'s, v8::Object>,
    data: impl Into<moli_dom::native::DomStringValue>,
) -> Option<v8::Local<'s, v8::Object>> {
    let data = data.into();
    let object =
        new_detached_object_with_prototype(scope, "__detachedTextPrototype", Some("Text"))?;
    let state = new_detached_state_object(scope, "text", 3, "#text")?;
    let _ = state.set(
        scope,
        v8str(scope, "ownerDocument").into(),
        owner_document.into(),
    );
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(owner_document_handle) = detached_native_handle(scope, owner_document)
    {
        define_detached_state(scope, object, state);
        let handle =
            unsafe { &mut *runtime_ptr }.create_text_node_for_document(owner_document_handle, data);
        define_detached_native_handle(scope, object, handle);
    } else {
        let data = crate::util::v8_string_from_utf16_units(scope, &data.utf16_units())?;
        let _ = state.set(scope, v8str(scope, "data").into(), data.into());
        define_detached_state(scope, object, state);
    }
    install_detached_character_data_instance_properties(scope, object);
    Some(object)
}

pub(in crate::native_bridge::document) fn build_detached_comment_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_document: v8::Local<'s, v8::Object>,
    data: impl Into<moli_dom::native::DomStringValue>,
) -> Option<v8::Local<'s, v8::Object>> {
    let data = data.into();
    let object =
        new_detached_object_with_prototype(scope, "__detachedCommentPrototype", Some("Comment"))?;
    let state = new_detached_state_object(scope, "comment", 8, "#comment")?;
    let _ = state.set(
        scope,
        v8str(scope, "ownerDocument").into(),
        owner_document.into(),
    );
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(owner_document_handle) = detached_native_handle(scope, owner_document)
    {
        define_detached_state(scope, object, state);
        let handle =
            unsafe { &mut *runtime_ptr }.create_comment_for_document(owner_document_handle, data);
        define_detached_native_handle(scope, object, handle);
    } else {
        let data = crate::util::v8_string_from_utf16_units(scope, &data.utf16_units())?;
        let _ = state.set(scope, v8str(scope, "data").into(), data.into());
        define_detached_state(scope, object, state);
    }
    install_detached_character_data_instance_properties(scope, object);
    Some(object)
}

pub(crate) fn build_detached_cdata_section_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_document: v8::Local<'s, v8::Object>,
    data: impl Into<moli_dom::native::DomStringValue>,
) -> Option<v8::Local<'s, v8::Object>> {
    let data = data.into();
    let object = new_detached_object_with_prototype(
        scope,
        "__detachedCDATASectionPrototype",
        Some("CDATASection"),
    )?;
    let state = new_detached_state_object(scope, "cdataSection", 4, "#cdata-section")?;
    let _ = state.set(
        scope,
        v8str(scope, "ownerDocument").into(),
        owner_document.into(),
    );
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(owner_document_handle) = detached_native_handle(scope, owner_document)
    {
        define_detached_state(scope, object, state);
        let handle = unsafe { &mut *runtime_ptr }
            .create_cdata_section_for_document(owner_document_handle, data);
        define_detached_native_handle(scope, object, handle);
    } else {
        let data = crate::util::v8_string_from_utf16_units(scope, &data.utf16_units())?;
        let _ = state.set(scope, v8str(scope, "data").into(), data.into());
        define_detached_state(scope, object, state);
    }
    install_detached_character_data_instance_properties(scope, object);
    Some(object)
}

pub(in crate::native_bridge::document) fn build_detached_processing_instruction_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_document: v8::Local<'s, v8::Object>,
    target: &str,
    data: impl Into<moli_dom::native::DomStringValue>,
) -> Option<v8::Local<'s, v8::Object>> {
    let data = data.into();
    let object = new_detached_object_with_prototype(
        scope,
        "__detachedProcessingInstructionPrototype",
        Some("ProcessingInstruction"),
    )?;
    let state = new_detached_state_object(scope, "processingInstruction", 7, target)?;
    let _ = state.set(
        scope,
        v8str(scope, "ownerDocument").into(),
        owner_document.into(),
    );
    let _ = state.set(
        scope,
        v8str(scope, "target").into(),
        v8_string(scope, target)?.into(),
    );
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(owner_document_handle) = detached_native_handle(scope, owner_document)
    {
        define_detached_state(scope, object, state);
        let handle = unsafe { &mut *runtime_ptr }.create_processing_instruction_for_document(
            owner_document_handle,
            target,
            data,
        );
        define_detached_native_handle(scope, object, handle);
    } else {
        let data = crate::util::v8_string_from_utf16_units(scope, &data.utf16_units())?;
        let _ = state.set(scope, v8str(scope, "data").into(), data.into());
        define_detached_state(scope, object, state);
    }
    install_detached_processing_instruction_instance_properties(scope, object);
    Some(object)
}
