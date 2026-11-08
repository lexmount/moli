use super::super::JsContextHost;
use crate::{document_runtime::DomHandle, native_bridge::node};
use url::Url;

pub(crate) fn document_cookie_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let (runtime_ptr, handle) =
        node::node_runtime_and_handle_from_object_or_detached(scope, receiver).ok()?;
    let runtime = unsafe { &*runtime_ptr };
    if !node::node_is_document(runtime, handle) {
        return None;
    }
    let Some(url) = document_cookie_url_for_handle(runtime, handle) else {
        return Some(String::new());
    };
    match runtime.host_document().cookie_for_url(&url) {
        Ok(cookie) => Some(cookie),
        Err(error) => {
            let message = v8::String::new(scope, &error.to_string())?;
            let exception = v8::Exception::error(scope, message);
            scope.throw_exception(exception);
            None
        }
    }
}

pub(crate) fn set_document_cookie_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    cookie: &str,
) -> bool {
    let Ok((runtime_ptr, handle)) =
        node::node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        return false;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    if !node::node_is_document(runtime, handle) {
        return false;
    }
    let Some(url) = document_cookie_url_for_handle(runtime, handle) else {
        return true;
    };
    match runtime.host_document_mut().set_cookie_for_url(&url, cookie) {
        Ok(()) => true,
        Err(error) => {
            if let Some(message) = v8::String::new(scope, &error.to_string()) {
                let exception = v8::Exception::error(scope, message);
                scope.throw_exception(exception);
            }
            false
        }
    }
}

fn document_cookie_url_for_handle(runtime: &JsContextHost, handle: DomHandle) -> Option<Url> {
    if handle == runtime.document_handle() {
        return Some(runtime.document_url().clone());
    }
    let child_handle = runtime.child_browsing_context_host_for_document_handle(handle)?;
    if runtime.child_browsing_context_inherits_parent_origin(child_handle) {
        return Some(runtime.document_url().clone());
    }
    runtime.child_browsing_context_current_url(child_handle)
}
