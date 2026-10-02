use super::storage::{form_data_entries, form_data_is_object, push_form_data_entry};
use super::*;
use crate::custom_elements::is_form_associated_custom_element_handle;
use crate::document_runtime::DomHandle;
use crate::dom::{
    forms::{InputType, apply_textarea_wrapping_transformation},
    native::{Node, SelectedFile},
};
use crate::native_bridge::{
    element::{
        control_has_datalist_ancestor, element_internals_form_value_for_target,
        form_control_is_effectively_disabled, form_data_control_elements, text_control_value,
    },
    node_relevant_context_for_handle, node_runtime_and_handle_from_object_or_detached,
};
use moli_encoding::is_charset_sentinel_name;
pub(super) fn serialize_form_data_controls<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: crate::document_runtime::DomHandle,
    submitter: Option<v8::Local<'s, v8::Object>>,
) -> Vec<(String, v8::Global<v8::Value>)> {
    let context = node_relevant_context_for_handle(scope, runtime_ptr, form_handle)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let submitter = submitter.and_then(|object| {
        let (owner, handle) =
            node_runtime_and_handle_from_object_or_detached(scope, object).ok()?;
        (owner == runtime_ptr).then_some(handle)
    });
    let mut entries = Vec::new();
    let controls = form_data_control_elements(unsafe { &*runtime_ptr }, form_handle);
    for handle in controls {
        if control_has_datalist_ancestor(unsafe { &*runtime_ptr }, handle) {
            continue;
        }
        append_form_data_entries_for_control(scope, &mut entries, runtime_ptr, handle, submitter);
    }
    entries
}

fn form_data_control_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(control) = crate::native_bridge::document::paired_detached_native_object_for_handle(
        scope,
        runtime_ptr,
        handle,
    ) {
        return Some(control);
    }
    if unsafe { &*runtime_ptr }.dom_host().is_connected(handle) {
        return unsafe { &mut *runtime_ptr }
            .native_bridge_mut()
            .wrap_handle(scope, runtime_ptr, handle);
    }
    crate::native_bridge::document::detached_native_object_for_handle(scope, runtime_ptr, handle)
}

pub(crate) fn form_data_entries_to_string_pairs<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: &[(String, v8::Global<v8::Value>)],
) -> Vec<(String, String)> {
    entries
        .iter()
        .filter_map(|(name, value)| {
            let value = v8::Local::new(scope, value);
            form_data_entry_value_as_name_value_string(scope, value)
                .map(|value| (name.clone(), value))
        })
        .collect()
}

fn append_form_data_entries_for_control(
    scope: &mut v8::PinScope<'_, '_>,
    entries: &mut Vec<(String, v8::Global<v8::Value>)>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    submitter: Option<DomHandle>,
) {
    let runtime = unsafe { &*runtime_ptr };
    if form_control_is_effectively_disabled(runtime, handle) {
        return;
    }
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return;
    };
    let name = element
        .attribute_ns("", "name")
        .unwrap_or_default()
        .to_owned();
    let is_submitter = submitter == Some(handle);

    if element.is_html_input() {
        let input_type = element.input_type();
        match input_type {
            InputType::Checkbox | InputType::Radio => {
                if element.checked() && !name.is_empty() {
                    push_string_form_data_entry(
                        scope,
                        entries,
                        &name,
                        text_control_value(runtime, handle),
                    );
                }
            }
            InputType::Submit => {
                if is_submitter && !name.is_empty() {
                    push_string_form_data_entry(
                        scope,
                        entries,
                        &name,
                        text_control_value(runtime, handle),
                    );
                    append_dirname_form_data_entry(scope, entries, runtime, handle);
                }
            }
            InputType::Image => {
                if is_submitter {
                    let (x, y) = runtime
                        .active_image_submitter_coordinate(handle)
                        .unwrap_or((0, 0));
                    let prefix = if name.is_empty() {
                        String::new()
                    } else {
                        format!("{name}.")
                    };
                    push_string_form_data_entry(
                        scope,
                        entries,
                        &format!("{prefix}x"),
                        x.to_string(),
                    );
                    push_string_form_data_entry(
                        scope,
                        entries,
                        &format!("{prefix}y"),
                        y.to_string(),
                    );
                }
            }
            InputType::Button | InputType::Reset => {}
            InputType::File => {
                if !name.is_empty()
                    && let Some(control) = form_data_control_object(scope, runtime_ptr, handle)
                {
                    append_file_form_data_entries(scope, entries, control, &name);
                }
            }
            _ => {
                if name.is_empty() {
                    return;
                }
                let value = if input_type == InputType::Hidden && is_charset_control_name(&name) {
                    "UTF-8".to_owned()
                } else {
                    text_control_value(runtime, handle)
                };
                push_string_form_data_entry(scope, entries, &name, value);
                if input_type.supports_dirname() {
                    append_dirname_form_data_entry(scope, entries, runtime, handle);
                }
            }
        }
    } else if element.is_html_textarea() {
        if !name.is_empty() {
            let value = apply_textarea_wrapping_transformation(
                text_control_value(runtime, handle),
                element.attribute_ns("", "wrap"),
                element.attribute_ns("", "cols"),
            );
            push_string_form_data_entry(scope, entries, &name, value);
            append_dirname_form_data_entry(scope, entries, runtime, handle);
        }
    } else if element.is_html_select() {
        if !name.is_empty() {
            for option in runtime.dom_host().select_selected_option_elements(handle) {
                if !runtime.dom_host().option_is_disabled(option)
                    && let Some(option_element) =
                        runtime.dom_host().node(option).and_then(Node::as_element)
                {
                    push_string_form_data_entry(
                        scope,
                        entries,
                        &name,
                        option_element.option_value(runtime.dom_host().dom(), option),
                    );
                }
            }
        }
    } else if element.is_html_button() {
        if is_submitter && !name.is_empty() && runtime.dom_host().button_is_submit_button(handle) {
            push_string_form_data_entry(
                scope,
                entries,
                &name,
                element
                    .attribute_ns("", "value")
                    .unwrap_or_default()
                    .to_owned(),
            );
        }
    } else if is_form_associated_custom_element_handle(runtime, handle) {
        let Some(control) = form_data_control_object(scope, runtime_ptr, handle) else {
            return;
        };
        let Some(value) = element_internals_form_value_for_target(scope, control) else {
            return;
        };
        if value.is_null_or_undefined() {
            return;
        }
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
            && form_data_is_object(scope, object)
        {
            entries.extend(form_data_entries(scope, object));
        } else if !name.is_empty() {
            push_form_data_entry(entries, &name, v8::Global::new(scope, value));
        }
    }
}

fn is_charset_control_name(name: &str) -> bool {
    is_charset_sentinel_name(name)
}

fn append_dirname_form_data_entry(
    scope: &mut v8::PinScope<'_, '_>,
    entries: &mut Vec<(String, v8::Global<v8::Value>)>,
    runtime: &JsContextHost,
    handle: DomHandle,
) {
    let Some(dirname) = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .and_then(|element| element.attribute_ns("", "dirname"))
        .filter(|name| !name.is_empty())
    else {
        return;
    };
    let direction = moli_selector::html_directionality(runtime.dom_host(), handle);
    push_string_form_data_entry(scope, entries, dirname, direction.as_str().to_owned());
}

fn append_file_form_data_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: &mut Vec<(String, v8::Global<v8::Value>)>,
    control: v8::Local<'s, v8::Object>,
    name: &str,
) {
    let Some(files) = crate::native_bridge::element::input_files_for_object(scope, control)
        .and_then(|files| file_api::file_list_files_from_object(scope, files))
    else {
        push_empty_file_form_data_entry(scope, entries, name);
        return;
    };
    if files.is_empty() {
        push_empty_file_form_data_entry(scope, entries, name);
        return;
    }
    for file in files {
        let file: v8::Local<'_, v8::Value> = file.into();
        push_form_data_entry(entries, name, v8::Global::new(scope, file));
    }
}

fn push_empty_file_form_data_entry(
    scope: &mut v8::PinScope<'_, '_>,
    entries: &mut Vec<(String, v8::Global<v8::Value>)>,
    name: &str,
) {
    let Some(file) = empty_file_object(scope) else {
        return;
    };
    let file: v8::Local<'_, v8::Value> = file.into();
    push_form_data_entry(entries, name, v8::Global::new(scope, file));
}

fn empty_file_object<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Object>> {
    file_api::build_file_object(
        scope,
        &SelectedFile {
            bytes: Vec::new(),
            mime_type: "application/octet-stream".to_owned(),
            name: String::new(),
            last_modified: unix_epoch_millis(),
        },
    )
}

fn form_data_entry_value_as_name_value_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<String> {
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && blob::blob_bytes_from_object(scope, object).is_some()
    {
        return Some(
            file_api::file_name_from_object(scope, object).unwrap_or_else(|| "blob".into()),
        );
    }
    callback_value_string(scope, value)
}

fn push_string_form_data_entry(
    scope: &mut v8::PinScope<'_, '_>,
    entries: &mut Vec<(String, v8::Global<v8::Value>)>,
    name: &str,
    value: String,
) {
    let value: v8::Local<'_, v8::Value> = v8_string(scope, &value)
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    push_form_data_entry(entries, name, v8::Global::new(scope, value));
}
