use super::storage::{form_data_entries, form_data_is_object, push_form_data_entry};
use super::*;
use crate::custom_elements::is_form_associated_custom_element_handle;
use crate::dom::{
    forms::{
        InputType, OptionDisabledAncestorStep, apply_textarea_wrapping_transformation,
        option_disabled_ancestor_step,
    },
    native::Node,
};
use crate::native_bridge::{
    element::{
        element_attribute_for_object, element_internals_form_value_for_target,
        form_control_is_effectively_disabled, form_data_control_elements, text_control_value,
    },
    node_runtime_and_handle_from_object,
};
use moli_encoding::is_charset_sentinel_name;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct EmptyFileOptionsDeclaration {
    #[webapi(data_property, enumerable)]
    r#type: &'static str,
}

pub(super) fn serialize_form_data_controls<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: crate::document_runtime::DomHandle,
    submitter: Option<v8::Local<'s, v8::Object>>,
) -> Vec<(String, v8::Global<v8::Value>)> {
    let mut entries = Vec::new();
    let controls = form_data_control_elements(unsafe { &*runtime_ptr }, form_handle);
    for handle in controls {
        let Some(control) = form_data_control_object(scope, runtime_ptr, handle) else {
            continue;
        };
        append_form_data_entries_for_control(scope, &mut entries, control, submitter);
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

fn append_form_data_entries_for_control<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: &mut Vec<(String, v8::Global<v8::Value>)>,
    control: v8::Local<'s, v8::Object>,
    submitter: Option<v8::Local<'s, v8::Object>>,
) {
    if control_is_effectively_disabled(scope, control) {
        return;
    }
    if control_has_datalist_ancestor(scope, control) {
        return;
    }

    let tag = object_string_property_defined(scope, control, "tagName")
        .map(|tag| tag.to_ascii_lowercase())
        .unwrap_or_default();
    let control_type = object_string_property_defined(scope, control, "type")
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let name = form_control_name(scope, control);
    let is_submitter = submitter.is_some_and(|submitter| control.strict_equals(submitter.into()));

    match tag.as_str() {
        "input" => match InputType::from_attribute_value(Some(&control_type)) {
            InputType::Checkbox | InputType::Radio => {
                if !object_bool_property(scope, control, "checked").unwrap_or(false)
                    || name.is_empty()
                {
                    return;
                }
                let value = form_control_value(scope, control, "on");
                push_string_form_data_entry(scope, entries, &name, value);
            }
            InputType::Submit => {
                if is_submitter {
                    if !name.is_empty() {
                        let value = form_control_value(scope, control, "");
                        push_string_form_data_entry(scope, entries, &name, value);
                    }
                    append_dirname_form_data_entry(scope, entries, control);
                }
            }
            InputType::Image => {
                if is_submitter {
                    let (x, y) = image_submitter_coordinates(scope, control);
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
                if name.is_empty() {
                    return;
                }
                append_file_form_data_entries(scope, entries, control, &name);
            }
            InputType::Hidden if is_charset_control_name(&name) => {
                push_string_form_data_entry(scope, entries, &name, "UTF-8".to_owned());
                append_dirname_form_data_entry(scope, entries, control);
            }
            _ => {
                if name.is_empty() {
                    return;
                }
                let value = form_control_value(scope, control, "");
                push_string_form_data_entry(scope, entries, &name, value);
                if InputType::from_attribute_value(Some(&control_type)).supports_dirname() {
                    append_dirname_form_data_entry(scope, entries, control);
                }
            }
        },
        "textarea" => {
            if !name.is_empty() {
                let value = form_control_value(scope, control, "");
                push_string_form_data_entry(scope, entries, &name, value);
                append_dirname_form_data_entry(scope, entries, control);
            }
        }
        "select" => {
            if name.is_empty() {
                return;
            }
            let multiple = object_bool_property(scope, control, "multiple").unwrap_or(false);
            let Some(options) = object_property_as_object(scope, control, "options") else {
                return;
            };
            let length = object_number_property(scope, options, "length")
                .unwrap_or(0.0)
                .max(0.0) as u32;
            let mut selected = Vec::new();
            let mut first_enabled_option_value = None;
            let mut saw_selected_option = false;
            for index in 0..length {
                let Some(option) = options
                    .get_index(scope, index)
                    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                else {
                    continue;
                };
                let option_selected =
                    object_bool_property(scope, option, "selected").unwrap_or(false);
                saw_selected_option |= option_selected;
                if option_is_disabled(scope, option) {
                    continue;
                }
                let value = form_control_value(scope, option, "");
                if first_enabled_option_value.is_none() {
                    first_enabled_option_value = Some(value.clone());
                }
                if option_selected {
                    selected.push(value);
                }
            }
            if multiple {
                for value in selected {
                    push_string_form_data_entry(scope, entries, &name, value);
                }
            } else if let Some(value) = selected.into_iter().next().or_else(|| {
                (!saw_selected_option)
                    .then_some(first_enabled_option_value)
                    .flatten()
            }) {
                push_string_form_data_entry(scope, entries, &name, value);
            }
        }
        "button" => {
            if is_submitter && !name.is_empty() && matches!(control_type.as_str(), "" | "submit") {
                let value = form_control_value(scope, control, "");
                push_string_form_data_entry(scope, entries, &name, value);
            }
        }
        _ => {
            if native_form_associated_custom_element(scope, control) {
                let Some(value) = element_internals_form_value_for_target(scope, control) else {
                    return;
                };
                let value: v8::Local<'_, v8::Value> = value;
                if value.is_null_or_undefined() {
                    return;
                }
                if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
                    && form_data_is_object(scope, object)
                {
                    entries.extend(form_data_entries(scope, object));
                    return;
                }
                if name.is_empty() {
                    return;
                }
                push_form_data_entry(entries, &name, v8::Global::new(scope, value));
            }
        }
    }
}

fn form_control_name(
    scope: &mut v8::PinScope<'_, '_>,
    control: v8::Local<'_, v8::Object>,
) -> String {
    element_attribute_for_object(scope, control, "name").unwrap_or_default()
}

fn image_submitter_coordinates(
    scope: &mut v8::PinScope<'_, '_>,
    control: v8::Local<'_, v8::Object>,
) -> (u32, u32) {
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, control)
    else {
        return (0, 0);
    };
    unsafe { &*runtime_ptr }
        .active_image_submitter_coordinate(handle)
        .unwrap_or((0, 0))
}

fn is_charset_control_name(name: &str) -> bool {
    is_charset_sentinel_name(name)
}

fn append_dirname_form_data_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: &mut Vec<(String, v8::Global<v8::Value>)>,
    control: v8::Local<'s, v8::Object>,
) {
    let Some(dirname) = object_string_property_defined(scope, control, "dirName") else {
        return;
    };
    if dirname.is_empty() {
        return;
    }
    let direction = form_control_direction(scope, control);
    push_string_form_data_entry(scope, entries, &dirname, direction);
}

fn form_control_direction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    control: v8::Local<'s, v8::Object>,
) -> String {
    let mut current = Some(control);
    while let Some(element) = current {
        let dir = object_string_property_defined(scope, element, "dir")
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        match dir.as_str() {
            "rtl" => return "rtl".to_owned(),
            "ltr" => return "ltr".to_owned(),
            "auto" if element.strict_equals(control.into()) => {
                return moli_selector::first_strong_text_direction(&form_control_value(
                    scope, control, "",
                ))
                .map(|direction| direction.as_str())
                .unwrap_or("ltr")
                .to_owned();
            }
            _ => {}
        }
        current = object_property_as_object(scope, element, "parentElement");
    }
    "ltr".to_owned()
}

fn append_file_form_data_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: &mut Vec<(String, v8::Global<v8::Value>)>,
    control: v8::Local<'s, v8::Object>,
    name: &str,
) {
    let Some(files) = object_property_as_object(scope, control, "files") else {
        push_empty_file_form_data_entry(scope, entries, name);
        return;
    };
    let length = object_number_property(scope, files, "length")
        .unwrap_or(0.0)
        .max(0.0) as u32;
    if length == 0 {
        push_empty_file_form_data_entry(scope, entries, name);
        return;
    }
    for index in 0..length {
        let Some(file) = files.get_index(scope, index) else {
            continue;
        };
        if v8::Local::<v8::Object>::try_from(file)
            .ok()
            .is_some_and(|file| blob::blob_bytes_from_object(scope, file).is_some())
        {
            push_form_data_entry(entries, name, v8::Global::new(scope, file));
        }
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
    let constructor = global_constructor_object(scope, "File")
        .and_then(|constructor| v8::Local::<v8::Function>::try_from(constructor).ok())?;
    let file_bits = crate::util::serialize_v8_array(scope, [""])?;
    let file_name = v8str(scope, "");
    let options = EmptyFileOptionsDeclaration::new("application/octet-stream")
        .bind(scope)
        .expect("empty File options declaration should bind");
    constructor.new_instance(scope, &[file_bits.into(), file_name.into(), options.into()])
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

fn form_control_value(
    scope: &mut v8::PinScope<'_, '_>,
    control: v8::Local<'_, v8::Object>,
    default: &str,
) -> String {
    if let Some(value) = native_form_control_value(scope, control) {
        return value;
    }
    object_string_property_defined(scope, control, "value").unwrap_or_else(|| default.to_owned())
}

fn native_form_control_value(
    scope: &mut v8::PinScope<'_, '_>,
    control: v8::Local<'_, v8::Object>,
) -> Option<String> {
    let (runtime_ptr, handle) = node_runtime_and_handle_from_object(scope, control).ok()?;
    let runtime = unsafe { &*runtime_ptr };
    let element = runtime.dom_host().node(handle).and_then(Node::as_element)?;
    if element.is_html_input() {
        return Some(text_control_value(runtime, handle));
    }
    if element.is_html_textarea() {
        return Some(apply_textarea_wrapping_transformation(
            text_control_value(runtime, handle),
            element.attribute_ns("", "wrap"),
            element.attribute_ns("", "cols"),
        ));
    }
    if element.is_html_option() {
        return Some(element.option_value(runtime.dom_host().dom(), handle));
    }
    if element.is_html_button() {
        return Some(
            element
                .attribute_ns("", "value")
                .map(str::to_owned)
                .unwrap_or_default(),
        );
    }
    None
}

fn control_is_effectively_disabled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    control: v8::Local<'s, v8::Object>,
) -> bool {
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, control) {
        return form_control_is_effectively_disabled(unsafe { &*runtime_ptr }, handle);
    }
    if object_bool_property(scope, control, "disabled").unwrap_or(false) {
        return true;
    }
    let mut current = object_property_as_object(scope, control, "parentElement");
    while let Some(element) = current {
        let tag = object_string_property_defined(scope, element, "tagName")
            .map(|tag| tag.to_ascii_lowercase())
            .unwrap_or_default();
        if tag == "fieldset"
            && object_bool_property(scope, element, "disabled").unwrap_or(false)
            && !control_is_in_first_legend(scope, control, element)
        {
            return true;
        }
        current = object_property_as_object(scope, element, "parentElement");
    }
    false
}

fn native_form_associated_custom_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    control: v8::Local<'s, v8::Object>,
) -> bool {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, control) else {
        return false;
    };
    is_form_associated_custom_element_handle(unsafe { &*runtime_ptr }, handle)
}

fn control_is_in_first_legend<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    control: v8::Local<'s, v8::Object>,
    fieldset: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(children) = object_property_as_object(scope, fieldset, "children") else {
        return false;
    };
    let length = object_number_property(scope, children, "length")
        .unwrap_or(0.0)
        .max(0.0) as u32;
    for index in 0..length {
        let Some(child) = children
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        let tag = object_string_property_defined(scope, child, "tagName")
            .map(|tag| tag.to_ascii_lowercase())
            .unwrap_or_default();
        if tag != "legend" {
            continue;
        }
        return dom_node_contains_internal(scope, child, control);
    }
    false
}

fn control_has_datalist_ancestor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    control: v8::Local<'s, v8::Object>,
) -> bool {
    let mut current = object_property_as_object(scope, control, "parentElement");
    while let Some(element) = current {
        let tag = object_string_property_defined(scope, element, "tagName")
            .map(|tag| tag.to_ascii_lowercase())
            .unwrap_or_default();
        if tag == "datalist" {
            return true;
        }
        current = object_property_as_object(scope, element, "parentElement");
    }
    false
}

fn option_is_disabled(scope: &mut v8::PinScope<'_, '_>, option: v8::Local<'_, v8::Object>) -> bool {
    if object_bool_property(scope, option, "disabled").unwrap_or(false) {
        return true;
    }

    let mut current = object_property_as_object(scope, option, "parentElement");
    while let Some(element) = current {
        let namespace =
            object_string_property_defined(scope, element, "namespaceURI").unwrap_or_default();
        let local_name =
            object_string_property_defined(scope, element, "localName").unwrap_or_default();
        let has_disabled_attribute =
            object_bool_property(scope, element, "disabled").unwrap_or(false);
        match option_disabled_ancestor_step(&namespace, &local_name, has_disabled_attribute) {
            OptionDisabledAncestorStep::Continue => {}
            OptionDisabledAncestorStep::Disabled(disabled) => return disabled,
        }
        current = object_property_as_object(scope, element, "parentElement");
    }
    false
}
