use super::*;
use std::{cell::RefCell, rc::Rc};

#[derive(Debug, Default)]
pub(crate) struct ConsoleMessageBuffers {
    messages: Vec<String>,
    details: Vec<serde_json::Value>,
}

pub(crate) fn install_console_message_buffers_for_context(context: v8::Local<'_, v8::Context>) {
    let _previous = context.set_slot(Rc::new(RefCell::new(ConsoleMessageBuffers::default())));
}

fn current_console_message_buffers(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<Rc<RefCell<ConsoleMessageBuffers>>> {
    scope
        .get_current_context()
        .get_slot::<RefCell<ConsoleMessageBuffers>>()
}

pub(crate) fn snapshot_console_messages_for_current_context(
    scope: &mut v8::PinScope<'_, '_>,
) -> Vec<String> {
    current_console_message_buffers(scope)
        .map(|buffers| buffers.borrow().messages.clone())
        .unwrap_or_default()
}

pub(crate) fn snapshot_console_message_details_for_current_context(
    scope: &mut v8::PinScope<'_, '_>,
) -> Vec<serde_json::Value> {
    current_console_message_buffers(scope)
        .map(|buffers| buffers.borrow().details.clone())
        .unwrap_or_default()
}

pub(in crate::context_bootstrap) fn append_console_message<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    level: &str,
) {
    let mut parts = Vec::with_capacity(args.length().max(0) as usize);
    for index in 0..args.length() {
        let value = args.get(index);
        let text = value
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_else(|| String::from("undefined"));
        parts.push(text);
    }
    let text = parts.join(" ");
    let message = format!("{level}: {text}");
    let stack = current_console_stack(scope);

    let mut arg_snapshot_values = Vec::with_capacity(args.length().max(0) as usize);
    for index in 0..args.length() {
        let value = args.get(index);
        arg_snapshot_values.push(console_arg_remote_object_json(scope, value));
    }

    if let Some(buffers) = current_console_message_buffers(scope) {
        let mut buffers = buffers.borrow_mut();
        buffers.messages.push(message.clone());
        let mut entry = serde_json::json!({
            "level": level,
            "text": text,
            "message": message,
            "args": arg_snapshot_values.clone(),
        });
        if let Some(stack) = stack.as_deref()
            && let Some(object) = entry.as_object_mut()
        {
            object.insert(
                "stack".to_owned(),
                serde_json::Value::String(stack.to_owned()),
            );
        }
        buffers.details.push(entry);
    }

    record_runtime_observable_console_source_event(scope, message, arg_snapshot_values, stack);
}

pub(crate) fn current_console_stack(scope: &mut v8::PinScope<'_, '_>) -> Option<String> {
    let stack = v8::StackTrace::current_stack_trace(scope, 32)?;
    let mut frames = Vec::with_capacity(stack.get_frame_count());
    for index in 0..stack.get_frame_count() {
        let Some(frame) = stack.get_frame(scope, index) else {
            continue;
        };
        let function_name = frame
            .get_function_name(scope)
            .map(|name| name.to_rust_string_lossy(scope))
            .unwrap_or_default();
        let url = frame
            .get_script_name_or_source_url(scope)
            .map(|name| name.to_rust_string_lossy(scope))
            .filter(|url| !url.is_empty())
            .unwrap_or_else(|| "<anonymous>".to_owned());
        let location = format!("{url}:{}:{}", frame.get_line_number(), frame.get_column());
        if function_name.is_empty() {
            frames.push(format!("at {location}"));
        } else {
            frames.push(format!("at {function_name} ({location})"));
        }
    }
    (!frames.is_empty()).then(|| frames.join("\n"))
}

pub(crate) fn console_arg_remote_object_json(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> serde_json::Value {
    if value.is_undefined() {
        return serde_json::json!({ "type": "undefined" });
    }
    if value.is_null() {
        return serde_json::json!({ "type": "object", "subtype": "null", "value": null });
    }
    if value.is_boolean() {
        return serde_json::json!({
            "type": "boolean",
            "value": value.boolean_value(scope),
        });
    }
    if value.is_number() {
        if let Some(number) = value.number_value(scope) {
            if let Some(unserializable) = number_unserializable_value(number) {
                return serde_json::json!({
                    "type": "number",
                    "unserializableValue": unserializable,
                });
            }
            if number.fract() == 0.0 && number >= i64::MIN as f64 && number <= i64::MAX as f64 {
                return serde_json::json!({
                    "type": "number",
                    "value": number as i64,
                });
            }
            return serde_json::json!({
                "type": "number",
                "value": number,
            });
        }
        return serde_json::json!({ "type": "number" });
    }
    if value.is_string() {
        let value = value
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_default();
        return serde_json::json!({
            "type": "string",
            "value": value,
        });
    }
    if value.is_function() {
        return serde_json::json!({
            "type": "function",
            "description": value_description(scope, value),
        });
    }
    if value.is_symbol() {
        return serde_json::json!({
            "type": "symbol",
            "description": value_description(scope, value),
        });
    }
    if value.is_big_int() {
        let mut description = value_description(scope, value);
        description.push('n');
        return serde_json::json!({
            "type": "bigint",
            "unserializableValue": description,
        });
    }

    let mut object = serde_json::json!({
        "type": "object",
        "description": value_description(scope, value),
    });
    if let Some(serialized) = json_serializable_console_value(scope, value)
        && let Some(object) = object.as_object_mut()
    {
        object.insert("value".to_owned(), serialized);
    }
    if value.is_array()
        && let Some(object) = object.as_object_mut()
    {
        object.insert(
            "subtype".to_owned(),
            serde_json::Value::String("array".to_owned()),
        );
    }
    object
}

fn json_serializable_console_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<serde_json::Value> {
    let json = {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let scope = try_catch.init();
        let body = v8::json::stringify(&scope, value)?;
        let body = body.to_rust_string_lossy(&scope);
        if body == "undefined" {
            return None;
        }
        body
    };
    serde_json::from_str(&json).ok()
}

fn value_description(scope: &mut v8::PinScope<'_, '_>, value: v8::Local<'_, v8::Value>) -> String {
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default()
}

fn record_runtime_observable_console_source_event(
    scope: &mut v8::PinScope<'_, '_>,
    message: String,
    args: Vec<serde_json::Value>,
    stack: Option<String>,
) {
    let Some(token) = crate::native_bridge::current_runtime_observable_context_token(scope) else {
        return;
    };
    let execution_context_id = i64::from(v8::inspector::V8Inspector::execution_context_id(
        scope.get_current_context(),
    ));
    if execution_context_id == 0 {
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    unsafe {
        (*host_ptr).record_runtime_observable_console_source_event(
            token,
            execution_context_id,
            message,
            args,
            stack,
        );
    }
}

fn number_unserializable_value(value: f64) -> Option<&'static str> {
    if value.is_nan() {
        Some("NaN")
    } else if value == f64::INFINITY {
        Some("Infinity")
    } else if value == f64::NEG_INFINITY {
        Some("-Infinity")
    } else if value == 0.0 && value.is_sign_negative() {
        Some("-0")
    } else {
        None
    }
}

pub(in crate::context_bootstrap) fn throw_error(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    let Some(message) = v8_string(scope, message) else {
        return;
    };
    let exception = v8::Exception::error(scope, message);
    scope.throw_exception(exception);
}

pub(in crate::context_bootstrap) fn throw_error_exception(
    scope: &mut v8::PinScope<'_, '_>,
    message: &str,
) {
    throw_error(scope, message);
}
