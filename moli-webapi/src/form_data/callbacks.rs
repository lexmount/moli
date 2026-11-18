use super::iterators::{FormDataIteratorKind, live_form_data_iterator};
use super::storage::{
    form_data_entries, form_data_is_object, initialize_form_data_entries, mutate_form_data_entries,
    normalize_form_data_value, push_form_data_entry,
};
use super::*;
use crate::callbacks::invoke_synchronous_webidl_callback_function;
use crate::interfaces as web_api_interfaces;
use moli_webapi_declare::serialize_v8_iter_array;
use moli_webidl as webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FormData")]
struct FormDataConstructorArgs<'s> {
    #[webidl(interface = web_api_interfaces::HTMLFormElement)]
    form: Option<v8::Local<'s, v8::Object>>,
    #[webidl(index = 1, nullable, interface = web_api_interfaces::HTMLElement)]
    submitter: Option<v8::Local<'s, v8::Object>>,
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

    let Some(parsed) = webidl::parse_args::<FormDataConstructorArgs<'s>>(scope, &args) else {
        return;
    };
    let entries = if let Some(form) = parsed.form {
        let Some(entries) = runtime(scope)
            .and_then(|runtime| (runtime.entries_for_form)(scope, form, parsed.submitter))
        else {
            return;
        };
        entries
    } else {
        Vec::new()
    };
    let form_data = args.this();
    initialize_form_data_entries(scope, form_data, &entries);
    rv.set(form_data.into());
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
