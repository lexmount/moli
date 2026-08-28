use crate::{
    context_bootstrap::dispatch_window_error_event_with_details,
    document_runtime::DomHandle,
    exception_reporting::build_event_handler_exception_report,
    native_bridge::{JsContextHost, OwnerDispatchScope},
    util::{create_script_origin_with_base_url, v8_string},
};

#[derive(Clone, Copy)]
pub(crate) enum EventAttributeHandlerScope {
    Element,
    ChildWindow,
}

pub(super) fn compile_event_attribute_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    source: &str,
    arguments: &[v8::Local<'s, v8::String>],
    context_extensions: &[v8::Local<'s, v8::Object>],
) -> Option<v8::Local<'s, v8::Function>> {
    let host = unsafe { &*host_ptr };
    let owner = host.owner_dispatch_scope_for_node(handle)?;
    let document = host.dom_host().owner_document_handle(handle)?;
    let base_url = host.document_base_url_for_handle(document);
    compile_event_attribute_handler_for_owner_with_context(
        scope,
        host_ptr,
        owner,
        &base_url,
        source,
        EventAttributeHandlerScope::Element,
        arguments,
        context_extensions,
    )
}

pub(crate) fn compile_event_attribute_handler_for_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    owner: OwnerDispatchScope,
    source: &str,
    handler_scope: EventAttributeHandlerScope,
) -> Option<v8::Local<'s, v8::Function>> {
    let host = unsafe { &*host_ptr };
    let base_url = match owner {
        OwnerDispatchScope::Top => host.document_base_url_for_handle(host.document_handle()),
        OwnerDispatchScope::Child(handle) => host
            .child_browsing_context_base_url(handle)
            .unwrap_or_else(|| host.document_url().clone()),
    };
    let event_argument = v8_string(scope, "event")?;
    compile_event_attribute_handler_for_owner_with_context(
        scope,
        host_ptr,
        owner,
        &base_url,
        source,
        handler_scope,
        &[event_argument],
        &[],
    )
}

fn compile_event_attribute_handler_for_owner_with_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    owner: OwnerDispatchScope,
    base_url: &url::Url,
    source: &str,
    handler_scope: EventAttributeHandlerScope,
    arguments: &[v8::Local<'s, v8::String>],
    context_extensions: &[v8::Local<'s, v8::Object>],
) -> Option<v8::Local<'s, v8::Function>> {
    let host = unsafe { &*host_ptr };
    if !host.owner_scripting_enabled(owner) {
        return None;
    }
    if !unsafe { &mut *host_ptr }.allows_inline_event_handler_by_csp(scope, owner, source) {
        return None;
    }
    let body = match handler_scope {
        EventAttributeHandlerScope::Element => source.to_owned(),
        EventAttributeHandlerScope::ChildWindow => format!("with (this) {{\n{source}\n}}"),
    };
    compile_event_attribute_function(
        scope,
        host_ptr,
        base_url,
        &body,
        arguments,
        context_extensions,
    )
}

fn compile_event_attribute_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    base_url: &url::Url,
    source: &str,
    arguments: &[v8::Local<'s, v8::String>],
    context_extensions: &[v8::Local<'s, v8::Object>],
) -> Option<v8::Local<'s, v8::Function>> {
    let source_text = v8_string(scope, source)?;
    let source_url = unsafe { &*host_ptr }.document_url().to_string();
    let origin = create_script_origin_with_base_url(scope, &source_url, 0, Some(base_url));
    let mut compiler_source = v8::script_compiler::Source::new(source_text, Some(&origin));

    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let handler = v8::script_compiler::compile_function(
        &scope,
        &mut compiler_source,
        arguments,
        context_extensions,
        v8::script_compiler::CompileOptions::NoCompileOptions,
        v8::script_compiler::NoCacheReason::NoReason,
    );
    if handler.is_some() || !scope.has_caught() {
        return handler;
    }

    let exception = scope.exception();
    let message = scope.message();
    let stack_trace = scope.stack_trace();
    let report = build_event_handler_exception_report(&mut scope, exception, message, stack_trace);
    scope.reset();

    let error_value = report
        .exception
        .as_ref()
        .map(|exception| v8::Local::new(&scope, exception));
    let _ = dispatch_window_error_event_with_details(
        &mut scope,
        host_ptr,
        &report.summary,
        report.source.as_deref().unwrap_or(""),
        report.line.unwrap_or(0) as u32,
        report.column.unwrap_or(0) as u32,
        error_value,
    );
    None
}
