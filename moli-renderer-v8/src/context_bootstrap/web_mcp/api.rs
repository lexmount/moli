use super::bindings::{check_target, document_for_target, document_owner, dom_error};
use super::events::queue_tool_change;
use super::state::{AbortRegistration, CallbackTool, RegisteredTool, ToolExecutor, ToolMetadata};
use super::tasks::{queue_task, remove_abort_registration, signal_reason, task_context, task_data};
use super::{conversion, devtools, execution, registry};
use crate::document_runtime::DomHandle;
use crate::native_bridge::JsContextHost;
use crate::util::context_host_ptr_from_global_bridge;
use crate::web_api_interfaces;
use crate::window_webidl_callback::WindowWebIdlCallbackFunction;

use crate::{abort_signal_route::ResolvedAbortSignal, webidl};

// Promise-returning Web IDL operations reject even when argument conversion
// throws. Preserve the original exception from getters, proxies and toJSON.
macro_rules! promise_callback {
    ($callback:ident, $operation:ident) => {
        pub(super) fn $callback<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            mut rv: v8::ReturnValue<'_, v8::Value>,
        ) {
            let Some(resolver) = v8::PromiseResolver::new(scope) else {
                return;
            };
            rv.set(resolver.get_promise(scope).into());
            let exception = {
                let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
                let mut scope = try_catch.init();
                if let Err(error) = $operation(&mut scope, &args, resolver) {
                    webidl::throw_error(&mut scope, &error);
                }
                let exception = scope
                    .exception()
                    .map(|value| v8::Global::new(&scope, value));
                scope.reset();
                exception
            };
            if let Some(exception) = exception {
                let _ = resolver.reject(scope, v8::Local::new(scope, &exception));
            }
        }
    };
}

promise_callback!(register_tool_callback, register_tool);
promise_callback!(get_tools_callback, get_tools);
promise_callback!(execute_tool_callback, execute_tool);

fn receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<(), webidl::WebIdlError> {
    if web_api_interfaces::ModelContext::is_instance(scope, args.this()) {
        Ok(())
    } else {
        Err(webidl::WebIdlError::custom_message("Illegal invocation"))
    }
}

fn active_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Result<(*mut JsContextHost, DomHandle), webidl::WebIdlError> {
    check_target(scope, target).map_err(|error| {
        scope.throw_exception(error);
        webidl::WebIdlError::pending_exception(webidl::Context::member("ModelContext", "document"))
    })
}

fn register_tool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> Result<(), webidl::WebIdlError> {
    receiver(scope, args)?;
    let definition = conversion::dictionary(scope, args, 0, "ModelContext.registerTool")?;
    let definition =
        webidl::parse_dictionary_object::<conversion::ToolDefinition>(scope, definition)?;
    let options = conversion::dictionary(scope, args, 1, "ModelContext.registerTool")?;
    let options = webidl::parse_dictionary_object::<conversion::RegisterOptions>(scope, options)?;
    let target = args.this();
    let (host_ptr, document) = active_target(scope, target)?;
    let error = if unsafe { &*host_ptr }
        .native_bridge()
        .web_mcp
        .documents
        .get(&document)
        .is_some_and(|entry| entry.tools.contains_key(&definition.name))
    {
        Some("Duplicate tool name")
    } else if !moli_webmcp::is_valid_tool_name(&definition.name) {
        Some("Invalid tool name")
    } else if definition.description.is_empty() {
        Some("Description is required")
    } else {
        None
    };
    if let Some(message) = error {
        let error = dom_error(scope, "InvalidStateError", message);
        let _ = resolver.reject(scope, error);
        return Ok(());
    }
    let input_schema = definition
        .input_schema
        .map(|schema| conversion::stringify_object(scope, schema))
        .transpose()?;
    // JSON.stringify invokes author toJSON/getters. They can replace the
    // document or detach its frame, retiring the registry checked above.
    let (host_ptr, document) = active_target(scope, target)?;
    if let Some(signal) = options.signal
        && ResolvedAbortSignal::resolve(scope, signal)
            .is_some_and(|signal| signal.is_aborted(scope))
    {
        let reason = signal_reason(scope, signal);
        let _ = resolver.reject(scope, reason);
        return Ok(());
    }
    let exposed_to = conversion::validate_origins(scope, options.exposed_to)?;
    // An author serializer may have registered this name after the first
    // duplicate check. Preserve that registration and its pending resolver.
    if unsafe { &*host_ptr }.native_bridge().web_mcp.documents[&document]
        .tools
        .contains_key(&definition.name)
    {
        let error = dom_error(scope, "InvalidStateError", "Duplicate tool name");
        let _ = resolver.reject(scope, error);
        return Ok(());
    }
    let registration = {
        let store = &mut unsafe { &mut *host_ptr }.native_bridge_mut().web_mcp;
        store.next_registration = store
            .next_registration
            .checked_add(1)
            .expect("WebMCP registration space exhausted");
        store.next_registration
    };
    let data = task_data(scope, target, registration, Some(&definition.name));
    let abort = options.signal.map(|signal| {
        let algorithm = v8::Function::builder(unregister_callback)
            .data(data.into())
            .build(scope)
            .expect("WebMCP abort algorithm");
        ResolvedAbortSignal::resolve(scope, signal)
            .expect("validated AbortSignal")
            .register_algorithm(scope, algorithm);
        AbortRegistration {
            signal: v8::Global::new(scope, signal),
            algorithm: v8::Global::new(scope, algorithm),
        }
    });
    let callback =
        WindowWebIdlCallbackFunction::new(scope, unsafe { &*host_ptr }, definition.execute);
    let tool = RegisteredTool {
        metadata: ToolMetadata {
            description: definition.description,
            title: definition.title.unwrap_or_default(),
            input_schema,
            annotations: definition.annotations,
        },
        stack_trace: devtools::capture_registration_stack(scope),
        exposed_to,
        executor: ToolExecutor::Callback(CallbackTool {
            callback,
            registration,
            abort,
            registration_resolver: Some(v8::Global::new(scope, resolver)),
        }),
    };
    let (origin, origins) = {
        let host = unsafe { &mut *host_ptr };
        let origin = host.native_bridge().web_mcp.documents[&document]
            .origin
            .clone();
        let origins = tool.exposed_to.clone();
        registry::insert_tool(host, document, definition.name, tool);
        (origin, origins)
    };
    queue_tool_change(scope, host_ptr, document, &origin, &origins);
    let ack = v8::Function::builder(registration_ack_callback)
        .data(data.into())
        .build(scope)
        .expect("WebMCP registration acknowledgement");
    queue_task(scope, host_ptr, ack);
    Ok(())
}

fn registration_ack_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((target, id, name)) = task_context(scope, &args) else {
        return;
    };
    let Ok((host_ptr, document)) = check_target(scope, target) else {
        return;
    };
    let name = name.to_rust_string_lossy(scope);
    let resolver = unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .web_mcp
        .documents
        .get_mut(&document)
        .and_then(|entry| entry.tools.get_mut(&name))
        .and_then(|tool| match &mut tool.executor {
            ToolExecutor::Callback(callback) if callback.registration == id => {
                callback.registration_resolver.take()
            }
            _ => None,
        });
    if let Some(resolver) = resolver {
        let resolver = v8::Local::new(scope, &resolver);
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
    }
}

fn unregister_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((target, id, name)) = task_context(scope, &args) else {
        return;
    };
    let Some(document) = document_for_target(scope, target) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let name = name.to_rust_string_lossy(scope);
    let origin = {
        let store = &unsafe { &*host_ptr }.native_bridge().web_mcp;
        let Some(entry) = store.documents.get(&document) else {
            return;
        };
        if entry
            .tools
            .get(&name)
            .is_none_or(|tool| {
                !matches!(&tool.executor, ToolExecutor::Callback(callback) if callback.registration == id)
            })
        {
            return;
        }
        entry.origin.clone()
    };
    if let Some(tool) = registry::take_tool(unsafe { &mut *host_ptr }, document, name) {
        let ToolExecutor::Callback(callback) = tool.executor else {
            unreachable!("registration identity checked before removal")
        };
        if let Some(resolver) = callback.registration_resolver {
            let signal = callback
                .abort
                .as_ref()
                .map(|abort| v8::Local::new(scope, &abort.signal));
            let reason = signal
                .map(|signal| signal_reason(scope, signal))
                .unwrap_or_else(|| crate::native_bridge::abort::abort_error_value(scope));
            let resolver = v8::Local::new(scope, &resolver);
            let _ = resolver.reject(scope, reason);
        }
        remove_abort_registration(scope, callback.abort);
        queue_tool_change(scope, host_ptr, document, &origin, &tool.exposed_to);
    }
}

fn get_tools<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> Result<(), webidl::WebIdlError> {
    receiver(scope, args)?;
    let options = conversion::dictionary(scope, args, 0, "ModelContext.getTools")?;
    let options = webidl::parse_dictionary_object::<conversion::GetOptions>(scope, options)?;
    let (host_ptr, document) = active_target(scope, args.this())?;
    let from_origins = conversion::validate_origins(scope, options.from_origins)?;
    let host = unsafe { &*host_ptr };
    let caller_tree = host.native_bridge().web_mcp.documents[&document].frame_tree;
    let caller_origin = host.native_bridge().web_mcp.documents[&document]
        .origin
        .clone();
    let mut tools = host
        .native_bridge()
        .web_mcp
        .documents
        .iter()
        .filter(|(document, entry)| {
            entry.frame_tree == caller_tree
                && document_owner(host, **document) == Some(entry.owner)
                && host
                    .document_permissions_policy_for_document_handle(**document)
                    .is_some_and(|policy| policy.tools_enabled())
                && (entry.origin == caller_origin || from_origins.contains(&entry.origin))
        })
        .flat_map(|(_, entry)| {
            entry
                .tools
                .iter()
                .filter(|(_, tool)| {
                    entry.origin == caller_origin || tool.exposed_to.contains(&caller_origin)
                })
                .map(move |(name, tool)| (entry, name, tool))
        })
        .collect::<Vec<_>>();
    tools.sort_by_key(|(_, name, _)| *name);
    let snapshots = tools
        .into_iter()
        .filter_map(|(entry, name, tool)| conversion::tool_snapshot(scope, entry, name, tool))
        .map(Into::into)
        .collect::<Vec<_>>();
    let result = v8::Array::new_with_elements(scope, &snapshots);
    let data = v8::Array::new_with_elements(scope, &[resolver.into(), result.into()]);
    let ack = v8::Function::builder(get_tools_ack_callback)
        .data(data.into())
        .build(scope)
        .expect("WebMCP discovery acknowledgement");
    queue_task(scope, host_ptr, ack);
    Ok(())
}

fn get_tools_ack_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Array>::try_from(args.data()) else {
        return;
    };
    let Some(resolver) = data
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(result) = data.get_index(scope, 1) else {
        return;
    };
    // SAFETY: This callback's private data is created above with a native
    // PromiseResolver at index zero and is never exposed to author code.
    let resolver = unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(resolver) };
    let _ = resolver.resolve(scope, result);
}

fn execute_tool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> Result<(), webidl::WebIdlError> {
    receiver(scope, args)?;
    let reference = conversion::dictionary(scope, args, 0, "ModelContext.executeTool")?;
    let reference = webidl::parse_dictionary_object::<conversion::ToolReference>(scope, reference)?;
    // IDL still converts every metadata member (including its author getters).
    // Invocation authority comes from the live registry, not caller metadata.
    let _metadata = (
        reference.annotations,
        reference.description,
        reference.input_schema,
        reference.title,
    );
    // Optional Web IDL object: undefined means missing; null and primitives fail.
    let input = if args.length() < 2 || args.get(1).is_undefined() {
        v8::Object::new(scope)
    } else {
        webidl::convert::<v8::Local<v8::Object>>(
            scope,
            args.get(1),
            webidl::Context::argument("ModelContext.executeTool", 2),
        )?
    };
    let options = conversion::dictionary(scope, args, 2, "ModelContext.executeTool")?;
    let options = webidl::parse_dictionary_object::<conversion::ExecuteOptions>(scope, options)?;
    let (host_ptr, caller_document) = active_target(scope, args.this())?;
    let expected = match url::Url::parse(&reference.origin) {
        Ok(url) if matches!(url.origin(), url::Origin::Tuple(..)) => url.origin(),
        _ => {
            let error = dom_error(
                scope,
                "NotSupportedError",
                "The provided origin is invalid or opaque.",
            );
            let _ = resolver.reject(scope, error);
            return Ok(());
        }
    };
    if matches!(
        unsafe { &*host_ptr }.native_bridge().web_mcp.documents[&caller_document].origin,
        url::Origin::Opaque(_)
    ) {
        let error = dom_error(
            scope,
            "NotSupportedError",
            "Tool invocation is not supported from an opaque origin",
        );
        let _ = resolver.reject(scope, error);
        return Ok(());
    }
    let input = conversion::stringify_object(scope, input)?;
    let (host_ptr, caller_document) = active_target(scope, args.this())?;
    if super::super::navigation_window::child_browsing_context_handle_for_runtime_owner(
        scope,
        reference.window,
    )
    .is_some_and(|handle| !unsafe { &*host_ptr }.child_browsing_context_host_is_active(handle))
    {
        let error = dom_error(scope, "InvalidStateError", "Target frame is detached.");
        let _ = resolver.reject(scope, error);
        return Ok(());
    }
    if let Some(signal) = options.signal
        && ResolvedAbortSignal::resolve(scope, signal)
            .is_some_and(|signal| signal.is_aborted(scope))
    {
        let reason = signal_reason(scope, signal);
        let _ = resolver.reject(scope, reason);
        return Ok(());
    }
    let target_document = {
        let host = unsafe { &*host_ptr };
        host.native_bridge()
            .web_mcp
            .documents
            .iter()
            .find(|(document, entry)| {
                entry.frame_tree
                    == host.native_bridge().web_mcp.documents[&caller_document].frame_tree
                    && v8::Local::new(scope, &entry.window).strict_equals(reference.window.into())
                    && document_owner(host, **document) == Some(entry.owner)
                    && host
                        .document_permissions_policy_for_document_handle(**document)
                        .is_some_and(|policy| policy.tools_enabled())
                    && entry.origin == expected
                    && entry.tools.get(&reference.name).is_some_and(|tool| {
                        entry.origin
                            == host.native_bridge().web_mcp.documents[&caller_document].origin
                            || tool.exposed_to.contains(
                                &host.native_bridge().web_mcp.documents[&caller_document].origin,
                            )
                    })
            })
            .map(|(document, _)| *document)
    };
    let Some(document) = target_document else {
        execution::reject_unavailable_tool(
            scope,
            host_ptr,
            caller_document,
            resolver,
            options.signal,
        );
        return Ok(());
    };
    execution::schedule_invocation(
        scope,
        host_ptr,
        caller_document,
        document,
        reference.name,
        input,
        Some(resolver),
        options.signal,
    );
    Ok(())
}
