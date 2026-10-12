use crate::{
    context_bootstrap::dispatch_window_error_event_with_details,
    document_runtime::DomHandle,
    dom::native::DomStringValue,
    exception_reporting::build_exception_report_without_stack,
    host::{EventHandlerCspAdmission, UncompiledEventHandler},
    native_bridge::{JsContextHost, OwnerDispatchScope},
    util::{create_function_origin_with_base_url, v8_string_from_dom_string_value},
};

pub(super) struct EventAttributeHandlerContext<'a> {
    pub(super) owner: OwnerDispatchScope,
    pub(super) element: Option<DomHandle>,
    pub(super) base_url: &'a url::Url,
}

pub(super) fn compile_event_attribute_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    handler: &UncompiledEventHandler,
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
        EventAttributeHandlerContext {
            owner,
            element: Some(handle),
            base_url: &base_url,
        },
        handler,
        arguments,
        context_extensions,
    )
}

pub(super) fn compile_event_attribute_handler_for_owner_with_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    context: EventAttributeHandlerContext<'_>,
    handler: &UncompiledEventHandler,
    arguments: &[v8::Local<'s, v8::String>],
    context_extensions: &[v8::Local<'s, v8::Object>],
) -> Option<v8::Local<'s, v8::Function>> {
    let host = unsafe { &*host_ptr };
    if !host.owner_scripting_enabled(context.owner) {
        return None;
    }
    if handler.csp_admission == EventHandlerCspAdmission::Pending
        && !unsafe { &mut *host_ptr }.allows_inline_event_handler_by_csp(
            scope,
            context.owner,
            context.element,
            handler.source.as_str_lossy(),
        )
    {
        return None;
    }
    compile_event_attribute_function(
        scope,
        host_ptr,
        context.base_url,
        &handler.source,
        arguments,
        context_extensions,
    )
}

fn compile_event_attribute_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    base_url: &url::Url,
    source: &DomStringValue,
    arguments: &[v8::Local<'s, v8::String>],
    context_extensions: &[v8::Local<'s, v8::Object>],
) -> Option<v8::Local<'s, v8::Function>> {
    let source_text = v8_string_from_dom_string_value(scope, source)?;
    let source_url = unsafe { &*host_ptr }.document_url().to_string();
    let origin = create_function_origin_with_base_url(
        scope,
        &source_url,
        base_url,
        context_extensions.len(),
    )?;
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
    // Reporting a parse error must not invoke an author's Error.prepareStackTrace
    // hook. ErrorEvent only needs the exception and its native source location.
    let report = build_exception_report_without_stack(&mut scope, exception, message);
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
