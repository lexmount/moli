use std::{cell::Cell, pin::pin};

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};

use super::{ScriptVm, inspector::PageInspectorSessionTarget};
use crate::{
    context_bootstrap::{
        SimpleObjectEventListenerInspectorSnapshot,
        simple_event_target_inspector_listener_snapshots,
    },
    document_runtime::{DomHandle, EventTargetHandle},
    host::EventListenerInspectorSnapshot,
    native_bridge::{JsContextHost, OwnerDispatchScope, node_runtime_and_handle_from_object},
    runtime::{RendererDomDebuggerEventListener, RendererDomDebuggerEventListenersResolution},
    util::v8str,
};

struct RootedEventListenerSnapshot {
    event_type: String,
    original: v8::Global<v8::Value>,
    callback: v8::Global<v8::Object>,
    is_callable: bool,
    use_capture: bool,
    passive: bool,
    once: bool,
    backend_node_id: Option<u32>,
}

impl ScriptVm {
    pub(crate) fn dom_debugger_event_listeners(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
        depth: i32,
        pierce: bool,
    ) -> Result<RendererDomDebuggerEventListenersResolution> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let context_host = self._context_host.clone();
        let page_inspector = &self.page_inspector;
        let renderer_document_isolate = self.renderer_document_isolate.clone();
        let next_call_id = Cell::new(self.next_internal_frontend_inspector_call_id);
        let result = renderer_document_isolate
            .with_entered_renderer_document_isolate_and_inspector_mut(|isolate, inspector| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let default_context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, default_context);
                let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
                page_inspector.with_session_and_outbound(
                    inspector,
                    PageInspectorSessionTarget::Frontend(inspector_session_id),
                    |session, outbound, _| {
                        let unwrapped = match session.unwrap_object(
                            scope,
                            v8::inspector::StringView::from(object_id.as_bytes()),
                        ) {
                            Ok(unwrapped) => unwrapped,
                            Err(error) => {
                                return Ok(RendererDomDebuggerEventListenersResolution::InvalidRemoteObjectId(
                                    inspector_error_message(error),
                                ));
                            }
                        };
                        let object_group = unwrapped
                            .object_group
                            .as_ref()
                            .map(|group| format!("{}", group.string()))
                            .unwrap_or_default();
                        let source_context = unwrapped.context;
                        let scope = &mut v8::ContextScope::new(scope, source_context);
                        let Ok(source_object) =
                            v8::Local::<v8::Object>::try_from(unwrapped.value)
                        else {
                            return Ok(RendererDomDebuggerEventListenersResolution::Found(
                                Vec::new(),
                            ));
                        };

                        let mut snapshots = rooted_event_listener_snapshots(
                            scope,
                            host_ptr,
                            source_object,
                            source_context,
                            depth,
                            pierce,
                        );
                        // Chromium performs a stable two-pass partition: all capture
                        // listeners first, while preserving node traversal and target-local
                        // listener order inside each partition.
                        snapshots.sort_by_key(|snapshot| !snapshot.use_capture);

                        let mut listeners = Vec::with_capacity(snapshots.len());
                        for snapshot in snapshots {
                            let Some(function) = effective_listener_function(scope, &snapshot)
                            else {
                                continue;
                            };
                            let handler = if object_group.is_empty() {
                                None
                            } else {
                                Some(wrap_inspector_value(
                                    scope,
                                    host_ptr,
                                    session,
                                    &outbound,
                                    &next_call_id,
                                    object_id,
                                    &object_group,
                                    function.into(),
                                )?)
                            };
                            let original_handler = if object_group.is_empty() {
                                None
                            } else {
                                let original = v8::Local::new(scope, &snapshot.original);
                                Some(wrap_inspector_value(
                                    scope,
                                    host_ptr,
                                    session,
                                    &outbound,
                                    &next_call_id,
                                    object_id,
                                    &object_group,
                                    original,
                                )?)
                            };
                            listeners.push(RendererDomDebuggerEventListener {
                                event_type: snapshot.event_type,
                                use_capture: snapshot.use_capture,
                                passive: snapshot.passive,
                                once: snapshot.once,
                                script_id: function.script_id().to_string(),
                                line_number: function
                                    .get_script_line_number()
                                    .and_then(|line| i32::try_from(line).ok())
                                    .unwrap_or(-1),
                                column_number: function
                                    .get_script_column_number()
                                    .and_then(|column| i32::try_from(column).ok())
                                    .unwrap_or(-1),
                                handler,
                                original_handler,
                                backend_node_id: snapshot.backend_node_id,
                            });
                        }
                        Ok(RendererDomDebuggerEventListenersResolution::Found(
                            listeners,
                        ))
                    },
                )
            });
        self.next_internal_frontend_inspector_call_id = next_call_id.get();
        result
    }
}

fn rooted_event_listener_snapshots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    source_object: v8::Local<'s, v8::Object>,
    source_context: v8::Local<'s, v8::Context>,
    depth: i32,
    pierce: bool,
) -> Vec<RootedEventListenerSnapshot> {
    let host = unsafe { &mut *host_ptr };
    if let Ok((runtime_ptr, root)) = node_runtime_and_handle_from_object(scope, source_object)
        && runtime_ptr == host_ptr
        && host.dom_host().node(root).is_some()
    {
        return rooted_node_event_listener_snapshots(
            scope,
            host,
            root,
            source_context,
            depth,
            pierce,
        );
    }

    if source_object.strict_equals(source_context.global(scope).into()) {
        let target = match host
            .window_execution_context_identity_for_v8_context(scope, source_context)
            .map(|identity| identity.dispatch_scope())
        {
            Some(OwnerDispatchScope::Child(child_handle)) => host
                .current_child_window_event_target(child_handle)
                .map(EventTargetHandle::ChildWindow),
            Some(OwnerDispatchScope::Top) => Some(EventTargetHandle::Window),
            None => None,
        };
        return target
            .map(|target| {
                rooted_registered_event_listener_snapshots(
                    scope,
                    host,
                    host_listener_snapshots(host, target),
                    source_context,
                    false,
                    None,
                )
            })
            .unwrap_or_default();
    }

    let snapshots = simple_event_target_inspector_listener_snapshots(scope, source_object);
    rooted_simple_event_listener_snapshots(scope, snapshots, source_context)
}

fn rooted_node_event_listener_snapshots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &mut JsContextHost,
    root: DomHandle,
    source_context: v8::Local<'s, v8::Context>,
    depth: i32,
    pierce: bool,
) -> Vec<RootedEventListenerSnapshot> {
    let depth = if depth < 0 { i32::MAX } else { depth };
    let mut pending = vec![(root, depth)];
    let mut snapshots = Vec::new();
    while let Some((node, remaining_depth)) = pending.pop() {
        if host.dom_host().node(node).is_none() {
            continue;
        }
        let backend_node_id = host.renderer_backend_node_id_for_live_handle(node);
        snapshots.extend(rooted_registered_event_listener_snapshots(
            scope,
            host,
            host.inspector_event_listener_snapshots(EventTargetHandle::Node(node)),
            source_context,
            pierce,
            backend_node_id,
        ));
        if remaining_depth <= 1 {
            continue;
        }
        let child_depth = remaining_depth - 1;
        let children = host.dom_host().child_handles(node).collect::<Vec<_>>();
        for child in children.into_iter().rev() {
            pending.push((child, child_depth));
        }
        if pierce {
            if let Some(shadow_root) = host.dom_host().shadow_root_handle(node) {
                pending.push((shadow_root, child_depth));
            }
            if let Some(child_document) = host.child_browsing_context_document_handle(node) {
                pending.push((child_document, child_depth));
            }
        }
    }
    snapshots
}

fn host_listener_snapshots(
    host: &JsContextHost,
    target: EventTargetHandle,
) -> Vec<EventListenerInspectorSnapshot> {
    match target {
        EventTargetHandle::ChildWindow(target) => {
            host.child_window_inspector_listener_snapshots(target)
        }
        EventTargetHandle::Window | EventTargetHandle::Node(_) => {
            host.inspector_event_listener_snapshots(target)
        }
    }
}

fn rooted_registered_event_listener_snapshots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &JsContextHost,
    snapshots: Vec<EventListenerInspectorSnapshot>,
    source_context: v8::Local<'s, v8::Context>,
    report_for_all_contexts: bool,
    backend_node_id: Option<u32>,
) -> Vec<RootedEventListenerSnapshot> {
    snapshots
        .into_iter()
        .filter_map(|snapshot| {
            let relevant_context =
                host.event_callback_relevant_context(scope, snapshot.callback_id)?;
            if !report_for_all_contexts && relevant_context != source_context {
                return None;
            }
            let original = host.event_callback_value(scope, snapshot.callback_id)?;
            let callback = v8::Local::<v8::Object>::try_from(original).ok()?;
            Some(RootedEventListenerSnapshot {
                event_type: snapshot.event_type,
                original: v8::Global::new(scope, original),
                callback: v8::Global::new(scope, callback),
                is_callable: callback.is_callable(),
                use_capture: snapshot.capture,
                passive: snapshot.passive,
                once: snapshot.once,
                backend_node_id,
            })
        })
        .collect()
}

fn rooted_simple_event_listener_snapshots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    snapshots: Vec<SimpleObjectEventListenerInspectorSnapshot<'s>>,
    source_context: v8::Local<'s, v8::Context>,
) -> Vec<RootedEventListenerSnapshot> {
    snapshots
        .into_iter()
        .filter_map(|snapshot| {
            if snapshot.relevant_context != source_context {
                return None;
            }
            Some(RootedEventListenerSnapshot {
                event_type: snapshot.event_type,
                original: v8::Global::new(scope, snapshot.original),
                callback: v8::Global::new(scope, snapshot.callback),
                is_callable: snapshot.is_callable,
                use_capture: snapshot.capture,
                passive: snapshot.passive,
                once: snapshot.once,
                backend_node_id: None,
            })
        })
        .collect()
}

fn effective_listener_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    snapshot: &RootedEventListenerSnapshot,
) -> Option<v8::Local<'s, v8::Function>> {
    let callback = v8::Local::new(scope, &snapshot.callback);
    if snapshot.is_callable {
        return v8::Local::<v8::Function>::try_from(callback).ok();
    }

    let try_catch = pin!(v8::TryCatch::new(scope));
    let scope = &mut try_catch.init();
    callback
        .get(scope, v8str(scope, "handleEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
}

#[allow(clippy::too_many_arguments)]
fn wrap_inspector_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    session: &v8::inspector::V8InspectorSession,
    outbound: &super::inspector::InspectorOutbound,
    next_call_id: &Cell<i32>,
    source_object_id: &str,
    object_group: &str,
    value: v8::Local<'s, v8::Value>,
) -> Result<Value> {
    let call_id = next_internal_frontend_inspector_call_id(next_call_id, outbound)?;
    let token = unsafe { &mut *host_ptr }
        .register_internal_inspector_value_reference(scope, value)
        .ok_or_else(|| anyhow!("failed to allocate internal Inspector value reference"))?;
    let request = serde_json::to_string(&json!({
        "id": call_id,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": source_object_id,
            "functionDeclaration": format!(
                "function() {{ return __moliHostResolveInternalInspectorValueReference({token}); }}"
            ),
            "objectGroup": object_group,
            "silent": true,
            "returnByValue": false,
            "generatePreview": false,
        }
    }))
    .context("failed to encode internal Runtime.callFunctionOn request");
    let request = match request {
        Ok(request) => request,
        Err(error) => {
            unsafe { &mut *host_ptr }.discard_internal_inspector_value_reference(token);
            return Err(error);
        }
    };

    let snapshot_len = outbound.len();
    {
        let _internal_response_capture = outbound.capture_internal_dispatch_response(call_id);
        let _dispatch_response_capture = outbound.capture_dispatch_responses();
        crate::inspector_microtasks::with_scoped_inspector_microtasks(scope, || {
            session.dispatch_protocol_message(v8::inspector::StringView::from(request.as_bytes()));
        });
    }
    unsafe { &mut *host_ptr }.discard_internal_inspector_value_reference(token);
    let response = outbound
        .take_response_for_call_id_after(snapshot_len, i64::from(call_id))
        .ok_or_else(|| {
            anyhow!("internal Runtime.callFunctionOn `{call_id}` returned no response")
        })?;
    if let Some(error) = response.get("error") {
        return Err(anyhow!(
            "internal Runtime.callFunctionOn `{call_id}` failed: {error}"
        ));
    }
    if let Some(exception) = response.pointer("/result/exceptionDetails") {
        return Err(anyhow!(
            "internal Runtime.callFunctionOn `{call_id}` threw: {exception}"
        ));
    }
    response
        .pointer("/result/result")
        .cloned()
        .ok_or_else(|| anyhow!("internal Runtime.callFunctionOn `{call_id}` returned no object"))
}

fn next_internal_frontend_inspector_call_id(
    next_call_id: &Cell<i32>,
    outbound: &super::inspector::InspectorOutbound,
) -> Result<i32> {
    for _ in 0..1024 {
        let call_id = next_call_id.get();
        next_call_id.set(
            call_id
                .checked_sub(1)
                .filter(|next| *next < 0)
                .unwrap_or(-1),
        );
        if outbound.internal_dispatch_call_id_is_available(call_id) {
            return Ok(call_id);
        }
    }
    Err(anyhow!(
        "internal frontend Inspector call id space is temporarily exhausted"
    ))
}

fn inspector_error_message(error: v8::UniquePtr<v8::inspector::StringBuffer>) -> String {
    error
        .as_ref()
        .map(|error| format!("{}", error.string()))
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| "Invalid remote object id".to_owned())
}
