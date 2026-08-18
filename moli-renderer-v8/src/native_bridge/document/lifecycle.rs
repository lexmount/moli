use super::super::node::{
    node_is_document, node_or_existing_detached_arg_handle,
    node_runtime_and_handle_from_args_or_detached, node_runtime_and_handle_from_object_or_detached,
    remove_child_in_reaction_scope,
};
use super::{
    JsContextHost, detached_document_content_type_value, detached_native_handle_for_runtime,
    is_html_document, throw_dom_exception,
};
use crate::native_bridge::element::{
    char_offset_to_byte_index, contenteditable_editing_host, dispatch_text_control_event,
    is_text_control, queue_text_control_document_selection_change_event,
    replace_contenteditable_selection, replace_text_control_selection, text_control_value,
};
use crate::{
    context_bootstrap::WINDOW_EVENT_HANDLER_PROPERTIES,
    custom_elements,
    document_runtime::DomHandle,
    util::{call_object_method, node_wrapper_from_handle, v8str},
    webidl,
};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.write")]
struct DocumentWriteArgs {
    #[webidl(variadic)]
    text: Vec<String>,
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
    let Some(parsed) = webidl::parse_args::<DocumentWriteArgs>(scope, &args) else {
        return;
    };
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
    if detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
        let mut html = parsed.text.concat();
        if append_newline {
            html.push('\n');
        }
        append_detached_html_document_body_html(scope, runtime_ptr, handle, &html);
        rv.set_undefined();
        return;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let implicit_replacement_session = !runtime.has_active_parser_write_insertion_point()
        && !runtime.host_document().replace_on_close();
    if implicit_replacement_session
        && current_script_ignores_document_write_without_parser_insertion_point(runtime)
    {
        rv.set_undefined();
        return;
    }
    if implicit_replacement_session {
        clear_window_event_handlers(scope);
        runtime.prepare_root_document_replacement(scope, runtime_ptr, handle);
    }
    for chunk in parsed.text {
        let _ = runtime.write_html(scope, runtime_ptr, handle, &chunk);
    }
    if append_newline {
        let _ = runtime.write_html(scope, runtime_ptr, handle, "\n");
    }
    rv.set_undefined();
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
        if detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
            set_detached_html_document_body_html(scope, runtime_ptr, handle, "");
            rv.set(args.this().into());
            return;
        }
        let runtime = unsafe { &mut *runtime_ptr };
        if !runtime.has_active_parser_write_insertion_point() {
            clear_window_event_handlers(scope);
            runtime.prepare_root_document_replacement(scope, runtime_ptr, handle);
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
    fn prepare_root_document_replacement(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
    ) {
        self.dispatch_document_open_descendant_frame_unload_lifecycle(scope, document_handle);
        self.clear_event_callbacks_for_document_replacement(document_handle, true);
        custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
            let _ = unsafe { &mut *host_ptr }.remove_all_children_for_document_replacement(
                scope,
                host_ptr,
                document_handle,
            );
        });
        self.open_root_document(scope);
    }

    /// Replaces the active root document through the native document stream.
    ///
    /// This is the internal equivalent of Blink's `Document::SetContent`: it
    /// deliberately bypasses the page-visible `document.open/write/close`
    /// properties, which may have been replaced by page script.
    pub(crate) fn set_root_document_content(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        html: &str,
    ) {
        let document_handle = self.document_handle();
        clear_window_event_handlers(scope);
        self.prepare_root_document_replacement(scope, host_ptr, document_handle);
        let _ = self.write_html(scope, host_ptr, document_handle, html);
        self.close_document(scope, host_ptr);
    }
}

fn redirect_document_open_to_window_open(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let document = args.this();
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
        if detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
            rv.set_undefined();
            return;
        }
        unsafe { &mut *runtime_ptr }.close_document(scope, runtime_ptr);
    }
    rv.set_undefined();
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
        let mut next = runtime.dom_host().inner_html(body).unwrap_or_default();
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

#[derive(Clone, Copy)]
enum EditingCommand {
    Copy,
    Delete,
    ForwardDelete,
    InsertText,
    SelectAll,
}

impl EditingCommand {
    fn parse(command: &str) -> Option<Self> {
        match command {
            "copy" => Some(Self::Copy),
            "delete" => Some(Self::Delete),
            "forwarddelete" => Some(Self::ForwardDelete),
            "inserttext" => Some(Self::InsertText),
            "selectall" => Some(Self::SelectAll),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Delete => "delete",
            Self::ForwardDelete => "forwarddelete",
            Self::InsertText => "inserttext",
            Self::SelectAll => "selectall",
        }
    }
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
            .is_some_and(|content_type| content_type.eq_ignore_ascii_case("application/xhtml+xml"))
        || detached_document_content_type_value(scope, args.this())
            .eq_ignore_ascii_case("application/xhtml+xml");
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
    let Some(command) = EditingCommand::parse(&command) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    match command {
        EditingCommand::SelectAll => {
            let selected =
                exec_command_select_all(scope, runtime_ptr, args.this(), document_handle);
            rv.set(v8::Boolean::new(scope, selected).into());
            return;
        }
        EditingCommand::Copy => {
            let active = current_protocol_user_gesture_activation(scope);
            rv.set(v8::Boolean::new(scope, active).into());
            return;
        }
        EditingCommand::InsertText => {
            let replacement = if args.length() > 2 {
                let Some(value) = args.get(2).to_string(scope) else {
                    return;
                };
                value.to_rust_string_lossy(scope)
            } else {
                String::new()
            };
            let inserted = exec_command_insert_text(scope, runtime_ptr, &replacement);
            rv.set(v8::Boolean::new(scope, inserted).into());
            return;
        }
        EditingCommand::Delete | EditingCommand::ForwardDelete => {}
    }
    let removed = exec_command_delete_selection(scope, runtime_ptr, args.this(), command.name());
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
        return replace_text_control_selection(scope, runtime_ptr, active, replacement);
    }
    let Some(editing_host) = contenteditable_editing_host(unsafe { &*runtime_ptr }, active) else {
        return false;
    };
    replace_contenteditable_selection(scope, runtime_ptr, editing_host, replacement)
}

pub(in crate::native_bridge) fn node_document_query_command_supported_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let command = normalized_editing_command(scope, &args);
    let Some((_runtime_ptr, _document_handle)) = editing_command_document(
        scope,
        &args,
        &mut rv,
        "queryCommandSupported is only supported on HTML documents.",
    ) else {
        return;
    };
    rv.set(v8::Boolean::new(scope, EditingCommand::parse(&command).is_some()).into());
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
    let enabled = match EditingCommand::parse(&command) {
        Some(
            EditingCommand::Delete | EditingCommand::ForwardDelete | EditingCommand::InsertText,
        ) => {
            runtime.document_design_mode_enabled(document_handle)
                || runtime.active_element_handle().is_some_and(|active| {
                    is_text_control(runtime, active)
                        || contenteditable_editing_host(runtime, active).is_some()
                })
        }
        Some(EditingCommand::SelectAll) => {
            exec_command_select_all_target(runtime, document_handle).is_some()
        }
        Some(EditingCommand::Copy) => current_protocol_user_gesture_activation(scope),
        None => false,
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
    let active_modal_dialog = current_modal_dialog(runtime);
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

fn current_modal_dialog(runtime: &JsContextHost) -> Option<DomHandle> {
    runtime
        .dom_host()
        .dom()
        .nodes()
        .iter()
        .rev()
        .find_map(|node| {
            if !node.is_connected() {
                return None;
            }
            let element = node.as_element()?;
            (element.is_html_element("dialog")
                && element.dialog_modal()
                && element.attribute("open").is_some())
            .then_some(node.id())
        })
}

fn current_protocol_user_gesture_activation(scope: &mut v8::PinScope<'_, '_>) -> bool {
    crate::util::context_host_ptr_from_global_bridge(scope)
        .is_some_and(|host_ptr| unsafe { (&*host_ptr).protocol_user_gesture_activation() })
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
    let value_len = value.chars().count() as u32;
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
    let (from, to, caret) = if start != end {
        (start, end, start)
    } else if command == "forwarddelete" {
        if start >= value_len {
            return Some(true);
        }
        (start, start + 1, start)
    } else {
        if start == 0 {
            return Some(true);
        }
        (start - 1, start, start - 1)
    };

    let next_value = format!(
        "{}{}",
        &value[..char_offset_to_byte_index(&value, from)],
        &value[char_offset_to_byte_index(&value, to)..]
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
