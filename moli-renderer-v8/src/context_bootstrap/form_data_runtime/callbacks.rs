use super::iterators::{FormDataIteratorKind, live_form_data_iterator};
use super::serialize::serialize_form_data_controls;
use super::storage::{
    form_data_entries, initialize_form_data_entries, mutate_form_data_entries,
    normalize_form_data_value, push_form_data_entry, set_form_data_entries,
};
use super::*;
use crate::{
    callback_invocation::invoke_synchronous_webidl_callback_function,
    native_bridge::{
        JsContextHost,
        element::{form_associated_form_owner, is_valid_submit_button},
        node_runtime_and_handle_from_object, throw_dom_exception,
    },
    util::serialize_v8_iter_array,
    webidl,
};
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(record, data_properties, enumerable)]
struct FormDataEventInitDeclaration<'scope> {
    form_data: v8::Local<'scope, v8::Object>,
    bubbles: bool,
    cancelable: bool,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FormData")]
struct FormDataConstructorArgs<'s> {
    #[webidl(converter = "raw")]
    form: Option<v8::Local<'s, v8::Value>>,
    #[webidl(index = 1, converter = "raw")]
    submitter: Option<v8::Local<'s, v8::Value>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FormData")]
struct FormDataNameArgs {
    #[webidl(required, converter = "usv_string")]
    name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FormData")]
struct FormDataMutationArgs<'s> {
    #[webidl(required, converter = "usv_string")]
    name: String,
    #[webidl(required)]
    value: v8::Local<'s, v8::Value>,
    #[webidl(index = 2, converter = "raw")]
    filename: Option<v8::Local<'s, v8::Value>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FormData.forEach")]
struct FormDataForEachArgs<'s> {
    #[webidl(
        required,
        converter = "callback_function",
        missing_message = "FormData.forEach requires a callback"
    )]
    callback: webidl::WebIdlCallbackFunction,
    this_arg: Option<v8::Local<'s, v8::Value>>,
}

fn require_form_data_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    method: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = args.this();
    if !form_data_is_object(scope, object) {
        throw_type_error(
            scope,
            &format!("Failed to execute '{method}' on 'FormData': Illegal invocation."),
        );
        return None;
    }
    Some(object)
}

fn form_data_value_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    filename: Option<v8::Local<'s, v8::Value>>,
    method: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    normalize_form_data_value(
        scope,
        value,
        filename,
        webidl::Context::argument(method, 2),
        webidl::Context::argument(method, 3),
    )
    .map_or_else(
        |error| {
            webidl::throw_error(scope, &error);
            None
        },
        Some,
    )
}

pub(super) fn form_data_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'FormData': Please use the 'new' operator.",
        );
        return;
    }

    let mut entries = Vec::new();
    let Some(parsed) = webidl::parse_args::<FormDataConstructorArgs<'s>>(scope, &args) else {
        return;
    };
    if let Some(form) = parsed.form {
        let Ok(form) = v8::Local::<v8::Object>::try_from(form) else {
            throw_type_error(scope, "FormData constructor requires an HTMLFormElement");
            return;
        };
        if object_string_property_defined(scope, form, "tagName")
            .is_none_or(|tag| !tag.eq_ignore_ascii_case("form"))
        {
            throw_type_error(scope, "FormData constructor requires an HTMLFormElement");
            return;
        }
        let Ok((form_runtime_ptr, form_handle)) = node_runtime_and_handle_from_object(scope, form)
        else {
            throw_type_error(scope, "FormData constructor requires an HTMLFormElement");
            return;
        };
        let submitter = match parsed.submitter {
            Some(value) if value.is_null_or_undefined() => None,
            Some(value) => match v8::Local::<v8::Object>::try_from(value) {
                Ok(submitter) => {
                    let Ok((submitter_runtime_ptr, submitter_handle)) =
                        node_runtime_and_handle_from_object(scope, submitter)
                    else {
                        throw_type_error(
                            scope,
                            "FormData constructor submitter must be a submit button",
                        );
                        return;
                    };
                    if submitter_runtime_ptr != form_runtime_ptr {
                        throw_dom_exception(
                            scope,
                            "NotFoundError",
                            8,
                            "The specified element is not owned by this form element.",
                        );
                        return;
                    }
                    let runtime = unsafe { &*form_runtime_ptr };
                    if !is_valid_submit_button(runtime, submitter_handle) {
                        throw_type_error(
                            scope,
                            "FormData constructor submitter must be a submit button",
                        );
                        return;
                    }
                    if form_associated_form_owner(runtime, submitter_handle) != Some(form_handle) {
                        throw_dom_exception(
                            scope,
                            "NotFoundError",
                            8,
                            "The specified element is not owned by this form element.",
                        );
                        return;
                    }
                    Some(submitter)
                }
                Err(_) => {
                    throw_type_error(
                        scope,
                        "FormData constructor submitter must be an HTMLElement or null",
                    );
                    return;
                }
            },
            None => None,
        };
        let Some(next_entries) = construct_form_data_entries_for_form(
            scope,
            form_runtime_ptr,
            form_handle,
            form,
            submitter,
        ) else {
            return;
        };
        entries = next_entries;
    }
    let form_data = args.this();
    initialize_form_data_entries(scope, form_data, &entries);
    rv.set(form_data.into());
}

pub(crate) fn construct_form_data_entries_for_form<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: crate::document_runtime::DomHandle,
    form: v8::Local<'s, v8::Object>,
    submitter: Option<v8::Local<'s, v8::Object>>,
) -> Option<Vec<(String, v8::Global<v8::Value>)>> {
    if !unsafe { &mut *runtime_ptr }.begin_form_data_construction(form_handle) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "Cannot construct FormData while the form data set is already being constructed.",
        );
        return None;
    }

    let entries = serialize_form_data_controls(scope, runtime_ptr, form_handle, submitter);
    let entries = dispatch_form_data_event_with_entries(scope, form, &entries).unwrap_or(entries);
    unsafe { &mut *runtime_ptr }.end_form_data_construction(form_handle);
    Some(entries)
}

fn dispatch_form_data_event_with_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    form: v8::Local<'s, v8::Object>,
    entries: &[(String, v8::Global<v8::Value>)],
) -> Option<Vec<(String, v8::Global<v8::Value>)>> {
    let form_data = new_empty_form_data_object(scope)?;
    set_form_data_entries(scope, form_data, entries);
    dispatch_form_data_event(scope, form, form_data);
    Some(form_data_entries(scope, form_data))
}

fn new_empty_form_data_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let constructor = global
        .get(scope, v8str(scope, "FormData").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    constructor.new_instance(scope, &[])
}

fn dispatch_form_data_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    form: v8::Local<'s, v8::Object>,
    form_data: v8::Local<'s, v8::Object>,
) {
    let Ok((runtime_ptr, form_handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, form)
    else {
        return;
    };
    let Some(event) = new_form_data_event(scope, form_data) else {
        return;
    };
    crate::context_bootstrap::events::mark_event_trusted(scope, event);
    let _ = crate::native_bridge::element::dispatch_public_event(
        scope,
        runtime_ptr,
        form_handle,
        event,
    );
}

fn new_form_data_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    form_data: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let constructor = global
        .get(scope, v8str(scope, "FormDataEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let init = FormDataEventInitDeclaration::new(form_data, true, false)
        .bind(scope)
        .expect("FormDataEvent init declaration should bind");
    let event_type = v8str(scope, "formdata");
    constructor.new_instance(scope, &[event_type.into(), init.into()])
}

pub(super) fn form_data_append_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_form_data_receiver(scope, &args, "append") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<FormDataMutationArgs>(scope, &args) else {
        return;
    };
    let Some(value) = form_data_value_arg(scope, parsed.value, parsed.filename, "FormData.append")
    else {
        return;
    };
    let value = v8::Global::new(scope, value);
    mutate_form_data_entries(scope, object, |entries| {
        push_form_data_entry(entries, &parsed.name, value);
    });
    rv.set_undefined();
}

pub(super) fn form_data_set_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_form_data_receiver(scope, &args, "set") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<FormDataMutationArgs>(scope, &args) else {
        return;
    };
    let Some(value) = form_data_value_arg(scope, parsed.value, parsed.filename, "FormData.set")
    else {
        return;
    };
    let value = v8::Global::new(scope, value);
    mutate_form_data_entries(scope, object, |entries| {
        let mut value = Some(value);
        let mut did_replace = false;
        entries.retain_mut(|(key, entry_value)| {
            if *key != parsed.name {
                return true;
            }
            if !did_replace {
                *entry_value = value
                    .take()
                    .expect("first matching FormData entry should receive replacement value");
                did_replace = true;
                return true;
            }
            false
        });
        if let Some(value) = value {
            push_form_data_entry(entries, &parsed.name, value);
        }
    });
    rv.set_undefined();
}

pub(super) fn form_data_get_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_form_data_receiver(scope, &args, "get") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<FormDataNameArgs>(scope, &args) else {
        return;
    };
    for (key, value) in form_data_entries(scope, object) {
        if key == parsed.name {
            rv.set(v8::Local::new(scope, &value));
            return;
        }
    }
    rv.set_null();
}

pub(super) fn form_data_get_all_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_form_data_receiver(scope, &args, "getAll") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<FormDataNameArgs>(scope, &args) else {
        return;
    };
    let entries = form_data_entries(scope, object);
    let values = entries
        .into_iter()
        .filter_map(|(key, value)| (key == parsed.name).then(|| v8::Local::new(scope, &value)))
        .collect::<Vec<_>>();
    let array = serialize_v8_iter_array(scope, values).unwrap_or_else(|| v8::Array::new(scope, 0));
    rv.set(array.into());
}

pub(super) fn form_data_has_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_form_data_receiver(scope, &args, "has") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<FormDataNameArgs>(scope, &args) else {
        return;
    };
    let entries = form_data_entries(scope, object);
    let present = entries.iter().any(|(key, _)| *key == parsed.name);
    rv.set(v8::Boolean::new(scope, present).into());
}

pub(super) fn form_data_delete_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_form_data_receiver(scope, &args, "delete") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<FormDataNameArgs>(scope, &args) else {
        return;
    };
    mutate_form_data_entries(scope, object, |entries| {
        entries.retain(|(key, _)| *key != parsed.name);
    });
    rv.set_undefined();
}

pub(super) fn form_data_keys_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_form_data_receiver(scope, &args, "keys") else {
        return;
    };
    if let Some(iter) = live_form_data_iterator(scope, object, FormDataIteratorKind::Keys) {
        rv.set(iter);
    } else {
        rv.set(v8::undefined(scope).into());
    }
}

pub(super) fn form_data_values_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_form_data_receiver(scope, &args, "values") else {
        return;
    };
    if let Some(iter) = live_form_data_iterator(scope, object, FormDataIteratorKind::Values) {
        rv.set(iter);
    } else {
        rv.set(v8::undefined(scope).into());
    }
}

pub(super) fn form_data_entries_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_form_data_receiver(scope, &args, "entries") else {
        return;
    };
    if let Some(iter) = live_form_data_iterator(scope, object, FormDataIteratorKind::Entries) {
        rv.set(iter);
    } else {
        rv.set(v8::undefined(scope).into());
    }
}

pub(super) fn form_data_for_each_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_form_data_receiver(scope, &args, "forEach") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<FormDataForEachArgs>(scope, &args) else {
        return;
    };
    let this_arg = parsed
        .this_arg
        .unwrap_or_else(|| v8::undefined(scope).into());
    let callback = parsed.callback.prepare(scope);
    let mut index = 0;
    loop {
        let entries = form_data_entries(scope, object);
        let Some((key, value)) = entries.get(index) else {
            break;
        };
        index += 1;
        let Some(key) = v8_string(scope, key) else {
            continue;
        };
        let value = v8::Local::new(scope, value);
        if invoke_synchronous_webidl_callback_function(
            scope,
            &callback,
            this_arg,
            &[value, key.into(), object.into()],
        )
        .is_none()
        {
            return;
        }
    }
    rv.set_undefined();
}
