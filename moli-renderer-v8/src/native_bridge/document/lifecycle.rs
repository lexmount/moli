use super::super::node::{
    node_is_document, node_or_existing_detached_arg_handle,
    node_runtime_and_handle_from_args_or_detached, node_runtime_and_handle_from_object_or_detached,
    remove_child_in_reaction_scope,
};
use super::{
    JsContextHost, document_has_browsing_context,
    is_html_document, throw_dom_exception,
};
use crate::native_bridge::element::{
    TextEditInputType, contenteditable_editing_host, dispatch_text_control_event, document_copy_command_supported,
    form_control_is_effectively_disabled, is_text_control,
    queue_text_control_document_selection_change_event, replace_contenteditable_selection,
    replace_text_control_selection, run_document_copy_command, text_control_value,
};
use crate::{
    context_bootstrap::WINDOW_EVENT_HANDLER_PROPERTIES,
    custom_elements,
    document_runtime::{DomHandle, EventTargetHandle},
    dom::native::{DocumentReadyState, NativeDom, NodeData},
    parser::HtmlParser,
    util::{
        call_object_method, node_wrapper_from_handle, utf16_next_scalar_boundary,
        utf16_previous_scalar_boundary, utf16_replace_units_range_lossy,
        utf16_scalar_boundary_at_or_after, utf16_units, v8_string, v8str,
    },
    webidl,
};

struct DocumentWriteInput {
    text: String,
    is_trusted: bool,
}

pub(in crate::native_bridge) fn node_document_write_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    node_document_write_or_writeln_callback(scope, args, rv, false);
}

pub(in crate::native_bridge) fn node_document_writeln_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    node_document_write_or_writeln_callback(scope, args, rv, true);
}

fn node_document_write_or_writeln_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    append_newline: bool,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_undefined();
        return;
    };
    if !node_is_document(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let api_prefix = if append_newline {
        "Document.writeln"
    } else {
        "Document.write"
    };
    let input = match document_write_input(scope, &args, api_prefix) {
        Ok(input) => input,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let sink = if append_newline {
        "Document writeln"
    } else {
        "Document write"
    };
    let api_name = if append_newline { "writeln" } else { "write" };
    let mut html = input.text;
    if !input.is_trusted {
        let Some(value) = v8_string(scope, &html) else {
            return;
        };
        let context = crate::native_bridge::node_relevant_context(scope, args.this())
            .unwrap_or_else(|| scope.get_current_context());
        let global = context.global(scope);
        let requirements = unsafe { &*runtime_ptr }
            .trusted_types_for_script_requirements_for_global(scope, global);
        let Some(compliant) = crate::context_bootstrap::trusted_type_string_or_throw(
            scope,
            value.into(),
            crate::context_bootstrap::TrustedTypeKind::Html,
            requirements,
            sink,
            api_name,
            Some(global),
        ) else {
            return;
        };
        html = compliant;
    }
    if append_newline {
        html.push('\n');
    }
    if !is_html_document(unsafe { &*runtime_ptr }, handle) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "Only HTML documents support write().",
        );
        return;
    }
    if unsafe { &*runtime_ptr }.has_throw_on_dynamic_markup_insertion_counter(handle) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The object is in an invalid state.",
        );
        return;
    }
    if let Some(child_handle) =
        unsafe { &*runtime_ptr }.child_browsing_context_host_for_document_handle(handle)
    {
        unsafe { &mut *runtime_ptr }.write_child_document_stream(
            scope,
            runtime_ptr,
            child_handle,
            handle,
            html,
        );
        rv.set_undefined();
        return;
    }
    if !document_has_browsing_context(unsafe { &*runtime_ptr }, handle) {
        let stream_was_open = unsafe { &*runtime_ptr }.has_windowless_document_parser(handle);
        if !stream_was_open && unsafe { &*runtime_ptr }.has_document_unload_counter(handle) {
            rv.set_undefined();
            return;
        }
        if !stream_was_open {
            unsafe { &mut *runtime_ptr }.prepare_windowless_document_replacement(
                scope,
                runtime_ptr,
                handle,
            );
        }
        let _ = unsafe { &mut *runtime_ptr }.write_windowless_document(
            scope,
            runtime_ptr,
            handle,
            &html,
        );
        rv.set_undefined();
        return;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let implicit_replacement_session = !runtime.has_active_parser_write_insertion_point()
        && !runtime.host_document().replace_on_close();
    if implicit_replacement_session
        && (runtime.has_document_unload_counter(handle)
            || runtime.has_ignore_destructive_writes_counter(handle)
            || current_script_ignores_document_write_without_parser_insertion_point(runtime))
    {
        rv.set_undefined();
        return;
    }
    if implicit_replacement_session {
        let entry_document = runtime.document_open_entry_document(scope);
        clear_window_event_handlers(scope);
        JsContextHost::prepare_root_document_replacement(scope, runtime_ptr, handle, entry_document);
    }
    let _ = unsafe { &mut *runtime_ptr }.write_html(scope, runtime_ptr, handle, &html);
    rv.set_undefined();
}

fn document_write_input<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    api_prefix: &'static str,
) -> Result<DocumentWriteInput, webidl::WebIdlError> {
    let mut text = String::new();
    let mut is_trusted = true;
    for index in 0..args.length() {
        let value = args.get(index);
        if let Some(value) = crate::context_bootstrap::trusted_html_value_string(scope, value) {
            text.push_str(&value);
            continue;
        }
        is_trusted = false;
        let value = webidl::convert::<webidl::DomString>(
            scope,
            value,
            webidl::Context::argument(api_prefix, (index + 1) as usize),
        )?;
        text.push_str(&value.0);
    }
    Ok(DocumentWriteInput { text, is_trusted })
}

fn current_script_ignores_document_write_without_parser_insertion_point(
    runtime: &JsContextHost,
) -> bool {
    let Some(current_script) = runtime.current_script_handle() else {
        return false;
    };
    let Some(node) = runtime.dom_host().node(current_script) else {
        return false;
    };
    let Some(element) = node.as_element() else {
        return false;
    };
    if !node.is_script_element() {
        return false;
    }

    let script_type = runtime
        .dom_host()
        .get_attribute(current_script, "type")
        .unwrap_or_default();
    if script_type.trim().eq_ignore_ascii_case("module") {
        return true;
    }

    let has_src = element
        .script_source_attribute()
        .is_some_and(|src| !src.is_empty());
    has_src
        && (element.script_async() || (element.is_html_script() && element.has_attribute("defer")))
}

pub(in crate::native_bridge) fn node_document_open_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.length() >= 3 {
        redirect_document_open_to_window_open(scope, &args, &mut rv);
        return;
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_undefined();
        return;
    };
    if node_is_document(unsafe { &*runtime_ptr }, handle) {
        if !is_html_document(unsafe { &*runtime_ptr }, handle) {
            throw_dom_exception(
                scope,
                "InvalidStateError",
                11,
                "Only HTML documents support open().",
            );
            return;
        }
        if unsafe { &*runtime_ptr }.has_throw_on_dynamic_markup_insertion_counter(handle) {
            throw_dom_exception(
                scope,
                "InvalidStateError",
                11,
                "The object is in an invalid state.",
            );
            return;
        }
        if unsafe { &*runtime_ptr }.has_document_unload_counter(handle) {
            rv.set(args.this().into());
            return;
        }
        if let Some(child_handle) =
            unsafe { &*runtime_ptr }.child_browsing_context_host_for_document_handle(handle)
        {
            let entry_document = unsafe { &*runtime_ptr }.document_open_entry_document(scope);
            let _ = JsContextHost::begin_child_document_stream_replacement(
                scope,
                runtime_ptr,
                child_handle,
                handle,
                entry_document,
            );
            rv.set(args.this().into());
            return;
        }
        if !document_has_browsing_context(unsafe { &*runtime_ptr }, handle) {
            unsafe { &mut *runtime_ptr }.prepare_windowless_document_replacement(
                scope,
                runtime_ptr,
                handle,
            );
            rv.set(args.this().into());
            return;
        }
        let runtime = unsafe { &mut *runtime_ptr };
        if !runtime.has_active_parser_write_insertion_point() {
            let entry_document = runtime.document_open_entry_document(scope);
            clear_window_event_handlers(scope);
            JsContextHost::prepare_root_document_replacement(scope, runtime_ptr, handle, entry_document);
        }
    }
    rv.set(args.this().into());
}

fn clear_window_event_handlers(scope: &mut v8::PinScope<'_, '_>) {
    let global = scope.get_current_context().global(scope);
    let null = v8::null(scope).into();
    for name in WINDOW_EVENT_HANDLER_PROPERTIES {
        let _ = global.set(scope, v8str(scope, name).into(), null);
    }
}

impl JsContextHost {
    pub(in crate::native_bridge) fn document_open_entry_document(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<DomHandle> {
        if let Some(popup_id) = crate::native_bridge::active_lightweight_popup_id(scope) {
            return self.lightweight_popup_document_handle(popup_id);
        }
        // Borrowed methods execute in their callee realm. HTML instead uses
        // the Window that entered this script or microtask.
        let context = scope.get_entered_or_microtask_context();
        let host_ptr = crate::util::context_host_ptr_from_context_slot(context)?;
        if !std::ptr::eq(host_ptr, self) {
            return None;
        }
        let window = context.global(scope);
        if let Some(document) = crate::util::get_private_value(
            scope,
            window,
            crate::context_bootstrap::WINDOW_DOCUMENT_SLOT,
        )
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            let (document_host, handle) =
                node_runtime_and_handle_from_object_or_detached(scope, document).ok()?;
            return std::ptr::eq(document_host, self).then_some(handle);
        }
        Some(self.document_handle())
    }

    pub(in crate::native_bridge) fn document_open_replacement_url(
        &self,
        document: DomHandle,
        entry_document: DomHandle,
    ) -> url::Url {
        let mut url = self.document_url_for_handle(entry_document);
        if document != entry_document {
            url.set_fragment(None);
        }
        url
    }

    fn prepare_windowless_document_replacement(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document: DomHandle,
    ) {
        self.clear_event_callbacks_for_document_replacement(document, false);
        custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
            let runtime = unsafe { &mut *host_ptr };
            runtime.remove_all_children_for_document_replacement(scope, host_ptr, document);
            let _ = runtime.start_windowless_document_parser(document);
        });
    }

    fn prepare_root_document_replacement(
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        entry_document: Option<DomHandle>,
    ) {
        let owner = unsafe { &*host_ptr }.current_main_document_task_owner();
        Self::dispatch_document_open_descendant_frame_unload_lifecycle(
            scope,
            host_ptr,
            document_handle,
        );
        if unsafe { &*host_ptr }.current_main_document_task_owner() != owner {
            return;
        }
        unsafe { &mut *host_ptr }
            .clear_event_callbacks_for_document_replacement(document_handle, true);
        custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
            let _ = unsafe { &mut *host_ptr }.remove_all_children_for_document_replacement(
                scope,
                host_ptr,
                document_handle,
            );
        });
        if unsafe { &*host_ptr }.current_main_document_task_owner() != owner {
            return;
        }
        let replacement_url =
            entry_document.map(|entry| unsafe { &mut *host_ptr }.document_open_replacement_url(document_handle, entry));
        let url_changed = replacement_url
            .as_ref()
            .is_some_and(|url| url != unsafe { &mut *host_ptr }.document_url());
        if let Some(url) = replacement_url.as_ref() {
            unsafe { &mut *host_ptr }.set_document_url(url.clone());
        }
        Self::open_root_document(scope, host_ptr);
        if let Some(url) = replacement_url
            && let Some(window) =
                super::document_associated_window_for_handle(scope, host_ptr, document_handle)
        {
            // Entry-change listeners can synchronously change the URL again.
            // Publish this change first so their later handoffs remain last.
            if url_changed {
                unsafe { &mut *host_ptr }.record_same_document_navigation(
                    &url,
                    "historyApi",
                    moli_page_types::SameDocumentHistoryUpdate::Replace,
                );
            }
            crate::context_bootstrap::update_history_for_document_open(scope, window, &url);
        }
    }

    /// Replaces the active root document through the native document stream.
    /// Bypasses page-visible document methods without retaining a host borrow
    /// across descendant retirement callbacks.
    pub(crate) fn set_root_document_content(
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        html: &str,
    ) {
        let document_handle = unsafe { &*host_ptr }.document_handle();
        clear_window_event_handlers(scope);
        Self::prepare_root_document_replacement(scope, host_ptr, document_handle, None);
        let _ = unsafe { &mut *host_ptr }.write_html(scope, host_ptr, document_handle, html);
        unsafe { &mut *host_ptr }.close_document(scope, host_ptr);
    }
}

fn redirect_document_open_to_window_open<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let document = args.this();
    if let Ok((runtime_ptr, document_handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, document)
        && let Some(child_handle) = unsafe { &*runtime_ptr }
            .child_browsing_context_host_for_document_handle(document_handle)
    {
        unsafe { &mut *runtime_ptr }.redirect_child_document_open_to_window_open(
            scope,
            child_handle,
            document,
            args,
            rv,
        );
        return;
    }
    let Some(default_view_value) = document.get(scope, v8str(scope, "defaultView").into()) else {
        return;
    };
    if default_view_value.is_null_or_undefined() {
        throw_dom_exception(
            scope,
            "InvalidAccessError",
            15,
            "Document has no associated window.",
        );
        return;
    }
    let Ok(default_view) = v8::Local::<v8::Object>::try_from(default_view_value) else {
        throw_dom_exception(
            scope,
            "InvalidAccessError",
            15,
            "Document has no associated window.",
        );
        return;
    };
    rv.set(default_view.into());
}

pub(in crate::native_bridge) fn node_document_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_undefined();
        return;
    };
    if node_is_document(unsafe { &*runtime_ptr }, handle) {
        if !is_html_document(unsafe { &*runtime_ptr }, handle) {
            throw_dom_exception(
                scope,
                "InvalidStateError",
                11,
                "Only HTML documents support close().",
            );
            return;
        }
        if unsafe { &*runtime_ptr }.has_throw_on_dynamic_markup_insertion_counter(handle) {
            throw_dom_exception(
                scope,
                "InvalidStateError",
                11,
                "The object is in an invalid state.",
            );
            return;
        }
        if let Some(child_handle) =
            unsafe { &*runtime_ptr }.child_browsing_context_host_for_document_handle(handle)
        {
            unsafe { &mut *runtime_ptr }.close_child_document_stream(
                scope,
                runtime_ptr,
                child_handle,
                handle,
            );
            rv.set_undefined();
            return;
        }
        if !document_has_browsing_context(unsafe { &*runtime_ptr }, handle) {
            close_windowless_document(scope, runtime_ptr, handle, args.this());
            rv.set_undefined();
            return;
        }
        unsafe { &mut *runtime_ptr }.close_document(scope, runtime_ptr);
    }
    rv.set_undefined();
}

fn close_windowless_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    document: v8::Local<'s, v8::Object>,
) {
    // Borrowing close() from another realm does not change the realm of the
    // Document's lifecycle events.
    let context = crate::native_bridge::node_relevant_context(scope, document)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(token) =
        unsafe { &mut *runtime_ptr }.finish_windowless_document_parser(scope, runtime_ptr, handle)
    else {
        return;
    };
    for (ready_state, event_type) in [
        (Some(DocumentReadyState::Interactive), "readystatechange"),
        (None, "DOMContentLoaded"),
        (Some(DocumentReadyState::Complete), "readystatechange"),
    ] {
        // A lifecycle listener can open a replacement stream on the same
        // Document. The old parser must not complete that replacement.
        if !unsafe { &*runtime_ptr }.windowless_document_parser_is_current(handle, &token) {
            return;
        }
        if let Some(state) = ready_state {
            unsafe { &mut *runtime_ptr }
                .dom_host_mut()
                .set_document_ready_state_for_handle(handle, state);
        }
        if let Ok(event) = crate::host::create_host_event(
            scope,
            event_type,
            document.into(),
            document.into(),
            event_type == "DOMContentLoaded",
            false,
        ) {
            let _ = unsafe { &mut *runtime_ptr }.dispatch_public_event_best_effort(
                scope,
                runtime_ptr,
                EventTargetHandle::Node(handle),
                event,
                "windowless document lifecycle event",
            );
        }
    }
    unsafe { &mut *runtime_ptr }.release_finished_windowless_document_parser(handle, &token);
}

fn detached_html_document_body_handle(
    runtime: &JsContextHost,
    document_handle: DomHandle,
) -> Option<DomHandle> {
    let dom = runtime.dom_host().dom();
    dom.node(document_handle)?
        .as_document()?
        .body_handle(dom, document_handle)
}

pub(in crate::native_bridge) fn set_detached_html_document_body_html(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    html: &str,
) -> bool {
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let runtime = unsafe { &mut *runtime_ptr };
        let Some(body) = detached_html_document_body_handle(runtime, document_handle) else {
            return false;
        };
        if html.is_empty() && runtime.dom_host().child_handles(body).next().is_none() {
            return true;
        }
        runtime.set_inner_html(scope, runtime_ptr, body, html)
    })
}

pub(in crate::native_bridge) fn append_detached_html_document_body_html(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    html: &str,
) -> bool {
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let runtime = unsafe { &mut *runtime_ptr };
        let Some(body) = detached_html_document_body_handle(runtime, document_handle) else {
            return false;
        };
        let scripting_enabled_for_node = |node| runtime.node_document_scripting_enabled(node);
        let mut next = runtime
            .dom_host()
            .get_html(body, &scripting_enabled_for_node, false, &[])
            .unwrap_or_default();
        next.push_str(html);
        runtime.set_inner_html(scope, runtime_ptr, body, &next)
    })
}

fn normalized_editing_command<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> String {
    args.get(0)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope).to_ascii_lowercase())
        .unwrap_or_default()
}

#[derive(Clone, Copy, strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
enum EditingCommand {
    Copy,
    Delete,
    ForwardDelete,
    InsertHtml,
    InsertText,
    SelectAll,
}

fn editing_command_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
    non_html_message: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, document_handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set(v8::Boolean::new(scope, false).into());
        return None;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !node_is_document(runtime, document_handle) {
        rv.set(v8::Boolean::new(scope, false).into());
        return None;
    }
    let supports_editing_commands = is_html_document(runtime, document_handle)
        || runtime
            .dom_host()
            .document_content_type_for_handle(document_handle)
            .is_some_and(|content_type| content_type.eq_ignore_ascii_case("application/xhtml+xml"));
    if !supports_editing_commands {
        throw_dom_exception(scope, "InvalidStateError", 11, non_html_message);
        return None;
    }
    Some((runtime_ptr, document_handle))
}

pub(in crate::native_bridge) fn node_document_exec_command_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let command = normalized_editing_command(scope, &args);
    let Some((runtime_ptr, document_handle)) = editing_command_document(
        scope,
        &args,
        &mut rv,
        "execCommand is only supported on HTML documents.",
    ) else {
        return;
    };
    let Ok(command) = command.parse::<EditingCommand>() else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    if !unsafe { &mut *runtime_ptr }.begin_document_editing_command(document_handle) {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    }
    execute_document_editing_command(scope, runtime_ptr, document_handle, &args, command, &mut rv);
    unsafe { &mut *runtime_ptr }.end_document_editing_command(document_handle);
}

fn execute_document_editing_command<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    args: &v8::FunctionCallbackArguments<'s>,
    command: EditingCommand,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
) {
    match command {
        EditingCommand::SelectAll => {
            let selected =
                exec_command_select_all(scope, runtime_ptr, args.this(), document_handle);
            rv.set(v8::Boolean::new(scope, selected).into());
            return;
        }
        EditingCommand::Copy => {
            let copied = run_document_copy_command(scope, runtime_ptr, document_handle, false);
            rv.set(v8::Boolean::new(scope, copied).into());
            return;
        }
        EditingCommand::InsertText => {
            let Some(value) = editing_command_value(scope, args) else {
                return;
            };
            let inserted = exec_command_insert_text(scope, runtime_ptr, &value);
            rv.set(v8::Boolean::new(scope, inserted).into());
            return;
        }
        EditingCommand::InsertHtml => {
            let Some(value) = editing_command_insert_html_value(scope, runtime_ptr, args) else {
                return;
            };
            let inserted = exec_command_insert_html(scope, runtime_ptr, &value);
            rv.set(v8::Boolean::new(scope, inserted).into());
            return;
        }
        EditingCommand::Delete | EditingCommand::ForwardDelete => {}
    }
    let command_name: &'static str = command.into();
    let removed = exec_command_delete_selection(scope, runtime_ptr, args.this(), command_name);
    rv.set(v8::Boolean::new(scope, removed).into());
}

fn exec_command_insert_text(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    replacement: &str,
) -> bool {
    let Some(active) = unsafe { &*runtime_ptr }.active_element_handle() else {
        return false;
    };
    if is_text_control(unsafe { &*runtime_ptr }, active) {
        return replace_text_control_selection(
            scope,
            runtime_ptr,
            active,
            replacement,
            TextEditInputType::InsertText,
        );
    }
    let Some(editing_host) = contenteditable_editing_host(unsafe { &*runtime_ptr }, active) else {
        return false;
    };
    replace_contenteditable_selection(
        scope,
        runtime_ptr,
        editing_host,
        replacement,
        TextEditInputType::InsertText,
    )
}

pub(in crate::native_bridge) fn node_document_query_command_supported_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let command = normalized_editing_command(scope, &args);
    let Some((runtime_ptr, document_handle)) = editing_command_document(
        scope,
        &args,
        &mut rv,
        "queryCommandSupported is only supported on HTML documents.",
    ) else {
        return;
    };
    let supported = match command.parse::<EditingCommand>() {
        Ok(EditingCommand::Copy) => {
            document_copy_command_supported(unsafe { &*runtime_ptr }, document_handle)
        }
        Ok(_) => true,
        Err(_) => false,
    };
    rv.set(v8::Boolean::new(scope, supported).into());
}

pub(in crate::native_bridge) fn node_document_query_command_enabled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let command = normalized_editing_command(scope, &args);
    let Some((runtime_ptr, document_handle)) = editing_command_document(
        scope,
        &args,
        &mut rv,
        "queryCommandEnabled is only supported on HTML documents.",
    ) else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let enabled = match command.parse::<EditingCommand>() {
        Ok(EditingCommand::Delete | EditingCommand::ForwardDelete | EditingCommand::InsertText) => {
            runtime.document_design_mode_enabled(document_handle)
                || runtime.active_element_handle().is_some_and(|active| {
                    is_text_control(runtime, active)
                        || contenteditable_editing_host(runtime, active).is_some()
                })
        }
        Ok(EditingCommand::InsertHtml) => exec_command_insert_html_target(runtime).is_some(),
        Ok(EditingCommand::SelectAll) => {
            exec_command_select_all_target(runtime, document_handle).is_some()
        }
        Ok(EditingCommand::Copy) => {
            run_document_copy_command(scope, runtime_ptr, document_handle, true)
        }
        Err(_) => false,
    };
    rv.set(v8::Boolean::new(scope, enabled).into());
}

fn query_command_constant_boolean<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
    non_html_message: &'static str,
) {
    let _command = normalized_editing_command(scope, args);
    if editing_command_document(scope, args, rv, non_html_message).is_some() {
        rv.set(v8::Boolean::new(scope, false).into());
    }
}

pub(in crate::native_bridge) fn node_document_query_command_indeterm_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    query_command_constant_boolean(
        scope,
        &args,
        &mut rv,
        "queryCommandIndeterm is only supported on HTML documents.",
    );
}

pub(in crate::native_bridge) fn node_document_query_command_state_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    query_command_constant_boolean(
        scope,
        &args,
        &mut rv,
        "queryCommandState is only supported on HTML documents.",
    );
}

pub(in crate::native_bridge) fn node_document_query_command_value_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let _command = normalized_editing_command(scope, &args);
    if editing_command_document(
        scope,
        &args,
        &mut rv,
        "queryCommandValue is only supported on HTML documents.",
    )
    .is_some()
    {
        rv.set(v8str(scope, "").into());
    }
}

fn exec_command_select_all<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    document: v8::Local<'s, v8::Object>,
    document_handle: DomHandle,
) -> bool {
    let Some(selection) = call_object_method(scope, document, "getSelection", &[])
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(target_handle) =
        exec_command_select_all_target(unsafe { &*runtime_ptr }, document_handle)
    else {
        return false;
    };
    let Some(target) = node_wrapper_from_handle(scope, target_handle) else {
        return false;
    };
    let _ = call_object_method(scope, selection, "selectAllChildren", &[target.into()]);
    true
}

fn exec_command_select_all_target(
    runtime: &JsContextHost,
    document_handle: DomHandle,
) -> Option<DomHandle> {
    let active_modal_dialog = current_modal_dialog(runtime, document_handle);
    if let Some(active) = runtime.active_element_handle()
        && let Some(editing_host) = contenteditable_editing_host(runtime, active)
        && active_modal_dialog
            .is_none_or(|dialog| dom_descendant_or_self(runtime, dialog, editing_host))
    {
        return Some(editing_host);
    }
    if let Some(dialog) = active_modal_dialog {
        return Some(dialog);
    }

    let dom = runtime.dom_host().dom();
    let document = dom.node(document_handle)?.as_document()?;
    document
        .body_handle(dom, document_handle)
        .or_else(|| document.document_element_handle(dom, document_handle))
        .or(Some(document_handle))
}

fn current_modal_dialog(runtime: &JsContextHost, document: DomHandle) -> Option<DomHandle> {
    runtime
        .dom_host()
        .dom()
        .nodes()
        .iter()
        .rev()
        .find_map(|node| {
            if !node.is_connected()
                || runtime.dom_host().owner_document_handle(node.id()) != Some(document)
            {
                return None;
            }
            let element = node.as_element()?;
            (element.is_html_element("dialog")
                && element.dialog_modal()
                && element.attribute("open").is_some())
            .then_some(node.id())
        })
}

fn editing_command_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<String> {
    if args.length() < 3 {
        return Some(String::new());
    }
    args.get(2)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

fn editing_command_insert_html_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<String> {
    if args.length() < 3 || args.get(2).is_undefined() {
        return Some(String::new());
    }
    let requirements = unsafe { &*runtime_ptr }.trusted_types_for_script_requirements(scope);
    crate::context_bootstrap::trusted_html_string_or_throw(
        scope,
        args.get(2),
        requirements,
        "Document execCommand",
        "execCommand",
    )
}

fn exec_command_insert_html(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    value: &str,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    let Some(target) = exec_command_insert_html_target(runtime) else {
        return false;
    };
    let insertion_text = input_text_from_html_fragment(runtime, value);
    replace_text_control_selection(scope, runtime_ptr, target, &insertion_text)
}

fn exec_command_insert_html_target(runtime: &JsContextHost) -> Option<DomHandle> {
    let handle = runtime.active_element_handle()?;
    let element = runtime.dom_host().node(handle)?.as_element()?;
    let accepts_plain_text = element.is_html_textarea()
        || (element.is_html_input() && element.input_type().supports_text_length_validation());
    if !accepts_plain_text
        || element.has_attribute("readonly")
        || form_control_is_effectively_disabled(runtime, handle)
    {
        return None;
    }
    Some(handle)
}

fn input_text_from_html_fragment(runtime: &JsContextHost, value: &str) -> String {
    let document_handle = runtime.dom_host().document_handle();
    let parsed =
        HtmlParser::with_scripting_enabled(runtime.document_scripting_enabled(document_handle))
            .parse_fragment_without_declarative_shadow_roots(
                runtime.host_document().url().clone(),
                "http://www.w3.org/1999/xhtml",
                "body",
                value.to_owned(),
            );
    let root = parsed
        .body_node_id()
        .unwrap_or_else(|| parsed.document_node_id());
    let mut text = String::new();
    for child in parsed.child_ids(root) {
        append_input_fragment_text(&parsed, child, &mut text);
    }
    text
}

fn append_input_fragment_text(dom: &NativeDom, handle: DomHandle, text: &mut String) {
    let Some(node) = dom.node(handle) else {
        return;
    };
    if node.is_html_element_named("br") {
        text.push('\n');
        return;
    }
    match node.data() {
        NodeData::Text(value) => text.push_str(value.data()),
        NodeData::CDataSection(value) => text.push_str(value.data()),
        NodeData::Document(_) | NodeData::Element(_) | NodeData::DocumentFragment(_) => {
            for child in dom.child_ids(handle) {
                append_input_fragment_text(dom, child, text);
            }
        }
        NodeData::DocumentType(_) | NodeData::Comment(_) | NodeData::ProcessingInstruction(_) => {}
    }
}

fn exec_command_delete_selection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    document: v8::Local<'s, v8::Object>,
    command: &str,
) -> bool {
    if let Some(active) = unsafe { &*runtime_ptr }.active_element_handle()
        && let Some(handled) = exec_command_delete_text_control(scope, runtime_ptr, active, command)
    {
        return handled;
    }
    let Some(selection) = call_object_method(scope, document, "getSelection", &[])
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    if !selection
        .get(scope, v8str(scope, "isCollapsed").into())
        .is_some_and(|value| value.boolean_value(scope))
    {
        if !exec_command_delete_non_collapsed_selection(scope, runtime_ptr, document, selection) {
            let _ = call_object_method(scope, selection, "deleteFromDocument", &[]);
        }
        return true;
    }
    let Some(anchor_node_value) = selection.get(scope, v8str(scope, "anchorNode").into()) else {
        return false;
    };
    if anchor_node_value.is_null_or_undefined() {
        return false;
    }
    let anchor_offset = selection
        .get(scope, v8str(scope, "anchorOffset").into())
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0) as usize;
    let Some(parent) = node_or_existing_detached_arg_handle(scope, runtime_ptr, anchor_node_value)
    else {
        return false;
    };
    let children = unsafe { &*runtime_ptr }
        .dom_host()
        .child_handles(parent)
        .collect::<Vec<_>>();
    let child = children.get(anchor_offset).copied().or_else(|| {
        anchor_offset
            .checked_sub(1)
            .and_then(|index| children.get(index).copied())
    });
    let Some(child) = child else {
        return false;
    };
    remove_child_in_reaction_scope(scope, runtime_ptr, parent, child)
}

fn exec_command_delete_text_control(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    command: &str,
) -> Option<bool> {
    let runtime = unsafe { &*runtime_ptr };
    if !is_text_control(runtime, handle) {
        return None;
    }
    let value = text_control_value(runtime, handle);
    let value_units = utf16_units(&value);
    let value_len = value_units.len() as u32;
    let (start, end) = runtime
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        .map(|element| {
            let start = element.selection_start().min(value_len);
            let end = element.selection_end().min(value_len);
            if start <= end {
                (start, end)
            } else {
                (end, start)
            }
        })
        .unwrap_or((value_len, value_len));
    // Selection APIs expose code units, but native editing first resolves both
    // endpoints to caret positions that cannot split a surrogate pair.
    let start = utf16_scalar_boundary_at_or_after(&value_units, start as usize) as u32;
    let end = utf16_scalar_boundary_at_or_after(&value_units, end as usize) as u32;
    let (from, to) = if start != end {
        (start, end)
    } else if command == "forwarddelete" {
        (
            start,
            utf16_next_scalar_boundary(&value_units, start as usize) as u32,
        )
    } else {
        (
            utf16_previous_scalar_boundary(&value_units, start as usize) as u32,
            start,
        )
    };
    if from == to {
        return Some(true);
    }
    let caret = from;

    let next_value = utf16_replace_units_range_lossy(
        &value_units,
        from as usize,
        to.saturating_sub(from) as usize,
        &[],
    );
    let runtime = unsafe { &mut *runtime_ptr };
    let changed = runtime.set_input_value_from_user_edit(handle, &next_value);
    if changed {
        runtime.mark_text_control_change_pending(handle, &value);
    }
    let selection_changed =
        runtime.set_selection_range_with_direction(handle, caret, caret, "none");
    if changed || selection_changed {
        dispatch_text_control_event(scope, runtime_ptr, handle, "input");
        queue_text_control_document_selection_change_event(scope, runtime_ptr, handle);
    }
    Some(changed || selection_changed)
}

fn exec_command_delete_non_collapsed_selection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    document: v8::Local<'s, v8::Object>,
    selection: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(anchor_node) = selection.get(scope, v8str(scope, "anchorNode").into()) else {
        return false;
    };
    if anchor_node.is_null_or_undefined() {
        return false;
    }
    let Some(focus_node) = selection.get(scope, v8str(scope, "focusNode").into()) else {
        return false;
    };
    if focus_node.is_null_or_undefined() {
        return false;
    }
    let Some(anchor_handle) = node_or_existing_detached_arg_handle(scope, runtime_ptr, anchor_node)
    else {
        return false;
    };
    let Some(focus_handle) = node_or_existing_detached_arg_handle(scope, runtime_ptr, focus_node)
    else {
        return false;
    };

    let runtime = unsafe { &*runtime_ptr };
    if inert_ancestor_or_self(runtime, anchor_handle).is_some() {
        return true;
    }
    let Some(protected_inert_root) = inert_ancestor_or_self(runtime, focus_handle) else {
        return false;
    };

    let Some(range) = call_object_method(
        scope,
        selection,
        "getRangeAt",
        &[v8::Integer::new(scope, 0).into()],
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok()) else {
        return false;
    };
    let Some(start_node) = range.get(scope, v8str(scope, "startContainer").into()) else {
        return false;
    };
    let Some(end_node) = range.get(scope, v8str(scope, "endContainer").into()) else {
        return false;
    };
    let Some(start_handle) = node_or_existing_detached_arg_handle(scope, runtime_ptr, start_node)
    else {
        return false;
    };
    let Some(end_handle) = node_or_existing_detached_arg_handle(scope, runtime_ptr, end_node)
    else {
        return false;
    };
    let start_offset = range
        .get(scope, v8str(scope, "startOffset").into())
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    let end_offset = range
        .get(scope, v8str(scope, "endOffset").into())
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);

    if dom_descendant_or_self(runtime, protected_inert_root, start_handle) {
        let Some((container_handle, offset)) =
            boundary_after_protected_inert_root(runtime, protected_inert_root)
        else {
            return true;
        };
        let Some(container) = node_wrapper_from_handle(scope, container_handle) else {
            return true;
        };
        return exec_command_delete_range_part(
            scope, document, container, offset, end_node, end_offset,
        );
    }

    if dom_descendant_or_self(runtime, protected_inert_root, end_handle) {
        let Some((container_handle, offset)) =
            boundary_before_protected_inert_root(runtime, protected_inert_root)
        else {
            return true;
        };
        let Some(container) = node_wrapper_from_handle(scope, container_handle) else {
            return true;
        };
        return exec_command_delete_range_part(
            scope,
            document,
            start_node,
            start_offset,
            container,
            offset,
        );
    }

    false
}

fn exec_command_delete_range_part<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    start_node: impl Into<v8::Local<'s, v8::Value>>,
    start_offset: u32,
    end_node: impl Into<v8::Local<'s, v8::Value>>,
    end_offset: u32,
) -> bool {
    let Some(range) = call_object_method(scope, document, "createRange", &[])
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return true;
    };
    let _ = call_object_method(
        scope,
        range,
        "setStart",
        &[
            start_node.into(),
            v8::Integer::new_from_unsigned(scope, start_offset).into(),
        ],
    );
    let _ = call_object_method(
        scope,
        range,
        "setEnd",
        &[
            end_node.into(),
            v8::Integer::new_from_unsigned(scope, end_offset).into(),
        ],
    );
    let _ = call_object_method(scope, range, "deleteContents", &[]);
    true
}

fn inert_ancestor_or_self(runtime: &JsContextHost, handle: DomHandle) -> Option<DomHandle> {
    let mut current = Some(handle);
    while let Some(handle) = current {
        if runtime
            .dom_host()
            .node(handle)
            .and_then(|node| node.as_element())
            .is_some_and(|element| element.attribute("inert").is_some())
        {
            return Some(handle);
        }
        current = runtime.dom_host().parent_node(handle);
    }
    None
}

fn dom_descendant_or_self(runtime: &JsContextHost, ancestor: DomHandle, handle: DomHandle) -> bool {
    let mut current = Some(handle);
    while let Some(handle) = current {
        if handle == ancestor {
            return true;
        }
        current = runtime.dom_host().parent_node(handle);
    }
    false
}

fn boundary_before_protected_inert_root(
    runtime: &JsContextHost,
    protected_inert_root: DomHandle,
) -> Option<(DomHandle, u32)> {
    let parent = runtime.dom_host().parent_node(protected_inert_root)?;
    let index = runtime
        .dom_host()
        .child_index(parent, protected_inert_root)?;
    let offset = u32::try_from(index).ok()?;
    Some((parent, offset))
}

fn boundary_after_protected_inert_root(
    runtime: &JsContextHost,
    protected_inert_root: DomHandle,
) -> Option<(DomHandle, u32)> {
    let parent = runtime.dom_host().parent_node(protected_inert_root)?;
    let index = runtime
        .dom_host()
        .child_index(parent, protected_inert_root)?;
    let offset = u32::try_from(index + 1).ok()?;
    Some((parent, offset))
}
