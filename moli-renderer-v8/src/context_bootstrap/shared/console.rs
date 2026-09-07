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
    let mut arg_snapshot_values = Vec::with_capacity(parts.capacity());
    for index in 0..args.length() {
        let snapshot = console_arg_remote_object_json(scope, args.get(index));
        parts.push(console_arg_text(&snapshot));
        arg_snapshot_values.push(snapshot);
    }
    let text = parts.join(" ");
    let message = format!("{level}: {text}");
    let stack = current_console_stack(scope);

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
        let value = v8::Local::<v8::String>::try_from(value)
            .expect("string console argument")
            .to_rust_string_lossy(scope);
        return serde_json::json!({
            "type": "string",
            "value": value,
        });
    }
    if value.is_function() {
        return serde_json::json!({
            "type": "function",
            "description": console_value_description(scope, value),
        });
    }
    if value.is_symbol() {
        let symbol = v8::Local::<v8::Symbol>::try_from(value).expect("symbol console argument");
        let description = v8::Local::<v8::String>::try_from(symbol.description(scope))
            .map(|description| description.to_rust_string_lossy(scope))
            .unwrap_or_default();
        return serde_json::json!({
            "type": "symbol",
            "description": format!("Symbol({description})"),
        });
    }
    if value.is_big_int() {
        // ToString of a primitive BigInt cannot invoke author conversion hooks.
        let mut description = value
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_default();
        description.push('n');
        return serde_json::json!({
            "type": "bigint",
            "unserializableValue": description,
        });
    }

    // This is the renderer-owned reporting snapshot, not the Inspector's
    // RemoteObject. The original V8 console still supplies inspectable objectIds
    // to CDP. Never serialize/coerce objects here: getters, toJSON, conversion
    // hooks and Proxy traps belong to the page, not to log bookkeeping.
    let subtype = if value.is_proxy() {
        Some("proxy")
    } else if value.is_array() {
        Some("array")
    } else if value.is_native_error() {
        Some("error")
    } else if value.is_reg_exp() {
        Some("regexp")
    } else if value.is_date() {
        Some("date")
    } else if value.is_promise() {
        Some("promise")
    } else if value.is_map() {
        Some("map")
    } else if value.is_set() {
        Some("set")
    } else if value.is_typed_array() {
        Some("typedarray")
    } else if value.is_array_buffer() {
        Some("arraybuffer")
    } else {
        None
    };
    let mut object = serde_json::json!({
        "type": "object", "description": console_value_description(scope, value)
    });
    if let Some(subtype) = subtype {
        object["subtype"] = serde_json::json!(subtype);
    }
    object
}

fn console_value_description(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> String {
    // V8 implements ToDetailString with NoSideEffectsToString under a
    // no-script scope. Unlike ToString, this cannot call author conversion
    // hooks, but still retains useful native Error/function descriptions.
    value
        .to_detail_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default()
}

fn console_arg_text(snapshot: &serde_json::Value) -> String {
    if let Some(value) = snapshot.get("value") {
        return value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string());
    }
    snapshot
        .get("description")
        .or_else(|| snapshot.get("unserializableValue"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("undefined")
        .to_owned()
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
