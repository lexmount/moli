use super::super::super::super::node::{
    node_is_document, node_runtime_and_handle_from_object_or_detached,
};
use super::super::super::{
    build_detached_document_all, detached_native_handle_for_runtime, is_html_document,
};
use super::builder::build_document_all_collection;

pub(crate) fn document_all_value_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let (runtime_ptr, handle) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver).ok()?;
    let runtime = unsafe { &*runtime_ptr };
    if !node_is_document(runtime, handle) {
        return None;
    }
    let context = receiver.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    if detached_native_handle_for_runtime(scope, runtime_ptr, receiver).is_some() {
        return build_detached_document_all(scope, receiver).map(Into::into);
    }
    if !is_html_document(runtime, handle) {
        return None;
    }
    build_document_all_collection(scope, runtime_ptr, handle).map(Into::into)
}
