//! Trusted tool events and one toolchange task per registry notification.

use super::bindings::{check_target, document_owner, js_string};
use super::tasks::queue_task;
use crate::document_runtime::DomHandle;
use crate::native_bridge::JsContextHost;
use crate::util::{get_private_value, set_private_value};
use crate::web_api_interfaces;

pub(super) const LISTENERS_SLOT: &str = "__moliModelContextListeners";
const HANDLERS: &[(&str, &str)] = &[
    ("ontoolchange", "toolchange"),
    ("ontoolactivated", "toolactivated"),
    ("ontoolcancel", "toolcancel"),
];

pub(super) fn dispatch_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &str,
    name: Option<&str>,
) {
    let event = if let Some(name) = name {
        let constructor_name = if event_type == "toolcancel" {
            "ToolCancelEvent"
        } else {
            "ToolActivatedEvent"
        };
        let Ok(constructor) = crate::context_bootstrap::ensure_intrinsic_interface_constructor(
            scope,
            constructor_name,
        ) else {
            return;
        };
        let init = v8::Object::new(scope);
        let _ = init.create_data_property(
            scope,
            js_string(scope, "toolName").into(),
            js_string(scope, name).into(),
        );
        let Some(event) =
            constructor.new_instance(scope, &[js_string(scope, event_type).into(), init.into()])
        else {
            return;
        };
        event
    } else {
        let event = crate::context_bootstrap::events::new_event_state(scope);
        crate::context_bootstrap::events::initialize_event_object(
            scope, event, event_type, false, false,
        );
        event
    };
    crate::context_bootstrap::events::mark_event_trusted(scope, event);
    let owner = crate::context_bootstrap::shared_event_targets::shared_target_owner(scope, target);
    crate::context_bootstrap::media_queries::dispatch_simple_event_target_event(
        scope,
        owner,
        LISTENERS_SLOT,
        event_type,
        event,
    );
}

pub(super) fn handler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !web_api_interfaces::ModelContext::is_instance(scope, args.this()) {
        let error = v8::Exception::type_error(scope, js_string(scope, "Illegal invocation"));
        scope.throw_exception(error);
        return;
    }
    let index = args.data().int32_value(scope).unwrap_or(-1) as usize;
    let Some((property, _)) = HANDLERS.get(index) else {
        return;
    };
    let target =
        crate::context_bootstrap::shared_event_targets::shared_target_owner(scope, args.this());
    rv.set(get_private_value(scope, target, property).unwrap_or_else(|| v8::null(scope).into()));
}

pub(super) fn handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !web_api_interfaces::ModelContext::is_instance(scope, args.this()) {
        let error = v8::Exception::type_error(scope, js_string(scope, "Illegal invocation"));
        scope.throw_exception(error);
        return;
    }
    let index = args.data().int32_value(scope).unwrap_or(-1) as usize;
    let Some((property, event_type)) = HANDLERS.get(index) else {
        return;
    };
    let target =
        crate::context_bootstrap::shared_event_targets::shared_target_owner(scope, args.this());
    let active = args.get(0).is_object();
    let value = if active {
        args.get(0)
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, target, property, value);
    crate::context_bootstrap::media_queries::simple_object_event_set_ordered_handler(
        scope,
        target,
        LISTENERS_SLOT,
        *event_type,
        property,
        active,
    );
}

pub(super) fn queue_tool_change<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    document: DomHandle,
    origin: &url::Origin,
    exposed_to: &[url::Origin],
) {
    queue_tool_change_tasks(scope, host_ptr, document, origin, exposed_to, false);
}

pub(super) fn queue_form_removal_change<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    document: DomHandle,
    origin: &url::Origin,
) {
    // Synchronous invalidation occurs inside the DOM mutation. Its registration
    // synchronization is queued at the end of that turn. Relay each removal
    // notification so synchronization precedes author event listeners, as it
    // does before the browser's notification returns to Blink.
    queue_tool_change_tasks(scope, host_ptr, document, origin, &[], true);
}

fn queue_tool_change_tasks<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    document: DomHandle,
    origin: &url::Origin,
    exposed_to: &[url::Origin],
    relay: bool,
) {
    let targets = {
        let host = unsafe { &*host_ptr };
        let tree = host.native_bridge().web_mcp.documents[&document].frame_tree;
        host.native_bridge()
            .web_mcp
            .documents
            .iter()
            .filter(|(document, entry)| {
                entry.frame_tree == tree
                    && document_owner(host, **document) == Some(entry.owner)
                    && host
                        .document_permissions_policy_for_document_handle(**document)
                        .is_some_and(|policy| policy.tools_enabled())
                    && (&entry.origin == origin || exposed_to.contains(&entry.origin))
            })
            .map(|(document, entry)| (*document, v8::Local::new(scope, &entry.target)))
            .collect::<Vec<_>>()
    };
    for (_, target) in targets {
        let Some(context) = target.get_creation_context(scope) else {
            continue;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let callback = if relay {
            v8::Function::builder(tool_change_relay_callback)
                .data(target.into())
                .build(scope)
        } else {
            v8::Function::builder(tool_change_callback)
                .data(target.into())
                .build(scope)
        }
        .expect("WebMCP toolchange task");
        queue_task(scope, host_ptr, callback);
    }
}

fn tool_change_relay_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    if let Ok((host_ptr, _)) = check_target(scope, target) {
        let callback = v8::Function::builder(tool_change_callback)
            .data(target.into())
            .build(scope)
            .expect("WebMCP toolchange dispatch task");
        queue_task(scope, host_ptr, callback);
    }
}

fn tool_change_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    if check_target(scope, target).is_ok() {
        dispatch_event(scope, target, "toolchange", None);
    }
}
