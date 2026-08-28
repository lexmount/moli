use crate::{
    context_bootstrap::{
        current_child_browsing_context_handle_for_runtime_scope, current_worker_script_url,
    },
    context_bootstrap::{
        is_readable_stream_object, is_transform_stream_object, is_writable_stream_object,
        message_port_id_from_object,
    },
    structured_clone::{
        RuntimeMessageAgentCluster, V8StructuredClonePayload, deserialize_from_wire,
        deserialize_message_event_from_wire, remote_structured_clone_attachment_count_is_supported,
        serialize_for_wire_for_remote_runtime_message, serialize_for_wire_for_runtime,
        serialize_for_wire_for_runtime_message, serialize_for_wire_for_runtime_with_transfers,
        serialize_for_wire_for_storage,
    },
    types::MessagePortId,
    util::{context_host_ptr_from_global_bridge, get_private_value, v8_string, v8str},
    webidl,
};

use super::slots::{
    FORM_DATA_ENTRIES_SLOT, NAVIGATOR_RUNTIME_DATA_SLOT, URL_HREF_SLOT,
    URL_SEARCH_PARAMS_PAIRS_SLOT,
};

struct PostMessageTransferList<'s> {
    array_buffers: Vec<v8::Local<'s, v8::ArrayBuffer>>,
    message_ports: Vec<v8::Local<'s, v8::Object>>,
    readable_streams: Vec<v8::Local<'s, v8::Object>>,
    writable_streams: Vec<v8::Local<'s, v8::Object>>,
    transform_streams: Vec<v8::Local<'s, v8::Object>>,
}

#[derive(Clone, Copy)]
enum TransferListOperation<'a> {
    PostMessage(&'a str),
    StructuredClone,
}

impl TransferListOperation<'_> {
    fn conversion_context(self) -> webidl::Context {
        match self {
            Self::PostMessage(_) => webidl::Context::member("postMessage", "transfer"),
            Self::StructuredClone => {
                webidl::Context::member("StructuredSerializeOptions", "transfer")
            }
        }
    }

    fn throw_type_error(self, scope: &mut v8::PinScope<'_, '_>, message: &str) {
        match self {
            Self::PostMessage(interface_name) => {
                throw_post_message_type_error(scope, interface_name, message)
            }
            Self::StructuredClone => {
                let formatted = format!("Failed to execute 'structuredClone': {message}");
                let message =
                    v8_string(scope, &formatted).unwrap_or_else(|| v8::String::empty(scope));
                scope.throw_exception(v8::Exception::type_error(scope, message));
            }
        }
    }

    fn throw_data_clone_error(self, scope: &mut v8::PinScope<'_, '_>, message: &str) {
        match self {
            Self::PostMessage(interface_name) => {
                throw_post_message_data_clone_error(scope, interface_name, message)
            }
            Self::StructuredClone => throw_data_clone_error(scope, message),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeMessageSourceSecurity {
    origin: Option<String>,
    agent_cluster: RuntimeMessageAgentCluster,
}

impl RuntimeMessageSourceSecurity {
    pub(crate) fn window(origin: String) -> Self {
        Self {
            origin: Some(origin),
            agent_cluster: RuntimeMessageAgentCluster::WindowOrDedicatedWorker,
        }
    }

    fn current(scope: &mut v8::PinScope<'_, '_>) -> Self {
        Self {
            origin: current_runtime_message_origin(scope),
            agent_cluster: current_runtime_message_agent_cluster(scope),
        }
    }
}

pub(crate) fn structured_serialize_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<V8StructuredClonePayload> {
    reject_uncloneable_web_platform_object(scope, value)?;
    serialize_for_wire_for_runtime(scope, value)
}

pub(crate) fn structured_serialize_value_for_storage<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<V8StructuredClonePayload> {
    reject_uncloneable_web_platform_object(scope, value)?;
    serialize_for_wire_for_storage(scope, value)
}

pub(crate) fn structured_serialize_value_for_post_message<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    transfer_arg: Option<v8::Local<'s, v8::Value>>,
    interface_name: &str,
) -> Option<V8StructuredClonePayload> {
    structured_serialize_value_for_post_message_with_source_port(
        scope,
        value,
        transfer_arg,
        interface_name,
        None,
    )
}

pub(crate) fn structured_serialize_value_for_post_message_with_source_port<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    transfer_arg: Option<v8::Local<'s, v8::Value>>,
    interface_name: &str,
    source_port_id: Option<MessagePortId>,
) -> Option<V8StructuredClonePayload> {
    let transfers =
        parse_post_message_transfer_list(scope, transfer_arg, interface_name, source_port_id)?;
    if is_uncloneable_web_platform_object(scope, value)
        && !transfer_list_contains_stream_value(value, &transfers)
    {
        throw_post_message_data_clone_error(
            scope,
            interface_name,
            "This object is not structured-serializable.",
        );
        return None;
    }
    let mut payload = serialize_for_wire_for_runtime_message(
        scope,
        value,
        &transfers.array_buffers,
        &transfers.message_ports,
        &transfers.readable_streams,
        &transfers.writable_streams,
        &transfers.transform_streams,
    )?;
    attach_runtime_message_source(&mut payload, RuntimeMessageSourceSecurity::current(scope));
    Some(payload)
}

pub(crate) fn structured_serialize_value_for_window_post_message<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    transfer_arg: Option<v8::Local<'s, v8::Value>>,
    source_security: RuntimeMessageSourceSecurity,
) -> Option<V8StructuredClonePayload> {
    let transfers = parse_window_post_message_transfer_list(scope, transfer_arg)?;
    structured_serialize_value_for_window_post_message_transfers(
        scope,
        value,
        transfers,
        source_security,
        WindowPostMessageTransport::InProcess,
    )
}

pub(crate) fn structured_serialize_value_for_remote_window_post_message<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    transfer_arg: Option<v8::Local<'s, v8::Value>>,
    source_security: RuntimeMessageSourceSecurity,
) -> Option<V8StructuredClonePayload> {
    let transfers = parse_window_post_message_transfer_list(scope, transfer_arg)?;
    structured_serialize_value_for_window_post_message_transfers(
        scope,
        value,
        transfers,
        source_security,
        WindowPostMessageTransport::RemoteEndpoint,
    )
}

pub(crate) fn structured_serialize_value_for_window_post_message_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    options_arg: v8::Local<'s, v8::Value>,
    source_security: RuntimeMessageSourceSecurity,
) -> Option<V8StructuredClonePayload> {
    let transfers = parse_window_post_message_options_transfer_list(scope, options_arg)?;
    structured_serialize_value_for_window_post_message_transfers(
        scope,
        value,
        transfers,
        source_security,
        WindowPostMessageTransport::InProcess,
    )
}

pub(crate) fn structured_serialize_value_for_remote_window_post_message_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    options_arg: v8::Local<'s, v8::Value>,
    source_security: RuntimeMessageSourceSecurity,
) -> Option<V8StructuredClonePayload> {
    let transfers = parse_window_post_message_options_transfer_list(scope, options_arg)?;
    structured_serialize_value_for_window_post_message_transfers(
        scope,
        value,
        transfers,
        source_security,
        WindowPostMessageTransport::RemoteEndpoint,
    )
}

#[derive(Clone, Copy)]
enum WindowPostMessageTransport {
    InProcess,
    RemoteEndpoint,
}

fn structured_serialize_value_for_window_post_message_transfers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    transfers: PostMessageTransferList<'s>,
    source_security: RuntimeMessageSourceSecurity,
    transport: WindowPostMessageTransport,
) -> Option<V8StructuredClonePayload> {
    if matches!(transport, WindowPostMessageTransport::RemoteEndpoint)
        && ![
            transfers.array_buffers.len(),
            transfers.message_ports.len(),
            transfers.readable_streams.len(),
            transfers.writable_streams.len(),
            transfers.transform_streams.len(),
        ]
        .into_iter()
        .all(remote_structured_clone_attachment_count_is_supported)
    {
        throw_post_message_data_clone_error(
            scope,
            "Window",
            "transfer list exceeds the remote transport attachment limit.",
        );
        return None;
    }
    if is_uncloneable_web_platform_object(scope, value)
        && !transfer_list_contains_stream_value(value, &transfers)
    {
        throw_post_message_data_clone_error(
            scope,
            "Window",
            "This object is not structured-serializable.",
        );
        return None;
    }
    let mut payload = match transport {
        WindowPostMessageTransport::InProcess => serialize_for_wire_for_runtime_message(
            scope,
            value,
            &transfers.array_buffers,
            &transfers.message_ports,
            &transfers.readable_streams,
            &transfers.writable_streams,
            &transfers.transform_streams,
        ),
        WindowPostMessageTransport::RemoteEndpoint => {
            serialize_for_wire_for_remote_runtime_message(
                scope,
                value,
                &transfers.array_buffers,
                &transfers.message_ports,
                &transfers.readable_streams,
                &transfers.writable_streams,
                &transfers.transform_streams,
            )
        }
    }?;
    attach_runtime_message_source(&mut payload, source_security);
    Some(payload)
}

fn empty_post_message_transfer_list<'s>() -> PostMessageTransferList<'s> {
    PostMessageTransferList {
        array_buffers: Vec::new(),
        message_ports: Vec::new(),
        readable_streams: Vec::new(),
        writable_streams: Vec::new(),
        transform_streams: Vec::new(),
    }
}

fn transfer_list_contains_stream_value<'s>(
    value: v8::Local<'s, v8::Value>,
    transfers: &PostMessageTransferList<'s>,
) -> bool {
    v8::Local::<v8::Object>::try_from(value).is_ok_and(|object| {
        transfers
            .readable_streams
            .iter()
            .chain(&transfers.writable_streams)
            .chain(&transfers.transform_streams)
            .any(|stream| stream.strict_equals(object.into()))
    })
}

pub(crate) fn structured_deserialize_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: &V8StructuredClonePayload,
) -> Option<v8::Local<'s, v8::Value>> {
    deserialize_from_wire(scope, payload)
}

pub(crate) fn structured_deserialize_value_for_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: &V8StructuredClonePayload,
) -> Option<(v8::Local<'s, v8::Value>, v8::Local<'s, v8::Array>)> {
    deserialize_message_event_from_wire(scope, payload)
}

pub(crate) fn current_runtime_message_origin(scope: &mut v8::PinScope<'_, '_>) -> Option<String> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &*host_ptr };
        let top_origin = moli_url::origin_ascii_serialization(host.document_url());
        if let Some(handle) = current_child_browsing_context_handle_for_runtime_scope(scope) {
            return host.child_browsing_context_target_origin(handle);
        }
        return Some(top_origin);
    }
    current_worker_script_url(scope).map(|url| moli_url::origin_ascii_serialization(&url))
}

pub(crate) fn current_runtime_message_agent_cluster(
    scope: &mut v8::PinScope<'_, '_>,
) -> RuntimeMessageAgentCluster {
    if context_host_ptr_from_global_bridge(scope).is_some() {
        return RuntimeMessageAgentCluster::WindowOrDedicatedWorker;
    }
    if crate::worker::service_worker_runtime_identity(scope).is_some() {
        return RuntimeMessageAgentCluster::ServiceWorker;
    }
    if crate::worker::worker_uses_shared_worker_agent_cluster(scope) {
        RuntimeMessageAgentCluster::SharedWorker
    } else {
        RuntimeMessageAgentCluster::WindowOrDedicatedWorker
    }
}

pub(crate) fn wasm_module_message_allowed_for_target(
    payload: &V8StructuredClonePayload,
    target_origin: Option<&str>,
    target_agent_cluster: RuntimeMessageAgentCluster,
) -> bool {
    if !payload.metadata.contains_wasm_module || !payload.metadata.origin_check_required {
        return true;
    }
    if payload.metadata.remote_agent_cluster_mismatch {
        return false;
    }
    if payload.metadata.locked_to_sender_agent_cluster
        && payload.metadata.sender_agent_cluster != Some(target_agent_cluster)
    {
        return false;
    }
    matches!(
        (payload.metadata.sender_origin.as_deref(), target_origin),
        (Some(sender), Some(target)) if sender == target
    )
}

pub(crate) fn wasm_module_message_allowed_for_target_origin(
    payload: &V8StructuredClonePayload,
    target_origin: Option<&str>,
) -> bool {
    wasm_module_message_allowed_for_target(
        payload,
        target_origin,
        RuntimeMessageAgentCluster::WindowOrDedicatedWorker,
    )
}

pub(crate) fn runtime_message_allowed_for_current_target(
    scope: &mut v8::PinScope<'_, '_>,
    payload: &V8StructuredClonePayload,
) -> bool {
    let target_origin = current_runtime_message_origin(scope);
    let target_agent_cluster = current_runtime_message_agent_cluster(scope);
    wasm_module_message_allowed_for_target(payload, target_origin.as_deref(), target_agent_cluster)
}

fn attach_runtime_message_source(
    payload: &mut V8StructuredClonePayload,
    source_security: RuntimeMessageSourceSecurity,
) {
    if payload.metadata.contains_wasm_module && payload.metadata.origin_check_required {
        payload.metadata.sender_origin = source_security.origin;
        payload.metadata.sender_agent_cluster = Some(source_security.agent_cluster);
    }
}

pub(crate) fn structured_clone_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let bytes = structured_serialize_value(scope, value)?;
    structured_deserialize_value(scope, &bytes)
}

pub(crate) fn structured_clone_value_with_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let transfers = parse_structured_clone_options_transfer_list(scope, options)?;
    if is_uncloneable_web_platform_object(scope, value)
        && !transfer_list_contains_stream_value(value, &transfers)
    {
        TransferListOperation::StructuredClone
            .throw_data_clone_error(scope, "This object is not structured-serializable.");
        return None;
    }
    let payload = serialize_for_wire_for_runtime_with_transfers(
        scope,
        value,
        &transfers.array_buffers,
        &transfers.message_ports,
        &transfers.readable_streams,
        &transfers.writable_streams,
        &transfers.transform_streams,
    )?;
    structured_deserialize_value(scope, &payload)
}

pub(crate) fn structured_clone_value_for_storage<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let bytes = structured_serialize_value_for_storage(scope, value)?;
    structured_deserialize_value(scope, &bytes)
}

fn reject_uncloneable_web_platform_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<()> {
    if value.is_shared_array_buffer() {
        throw_data_clone_error(scope, "SharedArrayBuffer could not be cloned.");
        return None;
    }
    if !is_uncloneable_web_platform_object(scope, value) {
        return Some(());
    }
    throw_data_clone_error(scope, "This object is not structured-serializable.");
    None
}

fn is_uncloneable_web_platform_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return false;
    };
    has_present_slot(scope, object, URL_HREF_SLOT)
        || crate::native_bridge::object_is_node_wrapper_or_detached(scope, object)
        || has_present_slot(scope, object, URL_SEARCH_PARAMS_PAIRS_SLOT)
        || has_present_slot(scope, object, FORM_DATA_ENTRIES_SLOT)
        || has_present_slot(scope, object, NAVIGATOR_RUNTIME_DATA_SLOT)
        || is_readable_stream_object(scope, object)
        || is_writable_stream_object(scope, object)
        || is_transform_stream_object(scope, object)
}

fn has_present_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> bool {
    get_private_value(scope, object, slot)
        .or_else(|| {
            let key = v8str(scope, slot);
            object.get(scope, key.into())
        })
        .is_some_and(|value| !value.is_null_or_undefined())
}

fn parse_post_message_transfer_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transfer_arg: Option<v8::Local<'s, v8::Value>>,
    interface_name: &str,
    source_port_id: Option<MessagePortId>,
) -> Option<PostMessageTransferList<'s>> {
    let Some(transfer_value) =
        resolve_post_message_transfer_value(scope, transfer_arg, interface_name)?
    else {
        return Some(empty_post_message_transfer_list());
    };
    let operation = TransferListOperation::PostMessage(interface_name);
    let sequence = transfer_sequence(scope, transfer_value, operation)?;

    parse_transfer_values(scope, sequence, operation, source_port_id)
}

fn parse_window_post_message_transfer_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transfer_arg: Option<v8::Local<'s, v8::Value>>,
) -> Option<PostMessageTransferList<'s>> {
    let Some(transfer_value) = transfer_arg else {
        return Some(empty_post_message_transfer_list());
    };
    if transfer_value.is_undefined() {
        return Some(empty_post_message_transfer_list());
    }
    if transfer_value.is_null() {
        throw_post_message_type_error(scope, "Window", "transfer list is not an iterable object.");
        return None;
    }
    let operation = TransferListOperation::PostMessage("Window");
    let sequence = transfer_sequence(scope, transfer_value, operation)?;
    parse_transfer_values(scope, sequence, operation, None)
}

fn parse_window_post_message_options_transfer_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options_arg: v8::Local<'s, v8::Value>,
) -> Option<PostMessageTransferList<'s>> {
    let Ok(options) = v8::Local::<v8::Object>::try_from(options_arg) else {
        return Some(empty_post_message_transfer_list());
    };
    let transfer_key = v8str(scope, "transfer").into();
    if !options.has(scope, transfer_key).unwrap_or(false) {
        return Some(empty_post_message_transfer_list());
    }
    let transfer = options.get(scope, transfer_key)?;
    if transfer.is_undefined() {
        return Some(empty_post_message_transfer_list());
    }
    let operation = TransferListOperation::PostMessage("Window");
    let sequence = transfer_sequence(scope, transfer, operation)?;
    parse_transfer_values(scope, sequence, operation, None)
}

fn parse_structured_clone_options_transfer_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options_arg: v8::Local<'s, v8::Value>,
) -> Option<PostMessageTransferList<'s>> {
    if options_arg.is_null_or_undefined() {
        return Some(empty_post_message_transfer_list());
    }
    let Ok(options) = v8::Local::<v8::Object>::try_from(options_arg) else {
        TransferListOperation::StructuredClone.throw_type_error(scope, "options is not an object.");
        return None;
    };
    let transfer_key = v8str(scope, "transfer").into();
    let transfer = options.get(scope, transfer_key)?;
    if transfer.is_undefined() {
        return Some(empty_post_message_transfer_list());
    }
    let operation = TransferListOperation::StructuredClone;
    let sequence = transfer_sequence(scope, transfer, operation)?;
    parse_transfer_values(scope, sequence, operation, None)
}

fn parse_transfer_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sequence: Vec<v8::Local<'s, v8::Value>>,
    operation: TransferListOperation<'_>,
    source_port_id: Option<MessagePortId>,
) -> Option<PostMessageTransferList<'s>> {
    let mut array_buffers = Vec::with_capacity(sequence.len());
    let mut message_ports = Vec::new();
    let mut readable_streams = Vec::new();
    let mut writable_streams = Vec::new();
    let mut transform_streams = Vec::new();
    let mut seen: Vec<v8::Local<'s, v8::Value>> = Vec::new();
    for candidate in sequence {
        if candidate.is_null_or_undefined() {
            operation.throw_type_error(scope, "transfer list contains a null or undefined entry.");
            return None;
        }
        if seen
            .iter()
            .any(|existing| existing.strict_equals(candidate))
        {
            operation.throw_data_clone_error(
                scope,
                "transfer list contains duplicate transferable entries.",
            );
            return None;
        }
        if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(candidate) {
            if buffer.was_detached() {
                operation.throw_data_clone_error(
                    scope,
                    "transfer list contains a detached ArrayBuffer.",
                );
                return None;
            }
            if !buffer.is_detachable() {
                operation.throw_type_error(
                    scope,
                    "transfer list contains a non-transferable ArrayBuffer.",
                );
                return None;
            }
            seen.push(buffer.into());
            array_buffers.push(buffer);
            continue;
        }
        if let Ok(port) = v8::Local::<v8::Object>::try_from(candidate)
            && let Some(port_id) = message_port_id_from_object(scope, port)
        {
            if Some(port_id) == source_port_id {
                operation.throw_data_clone_error(
                    scope,
                    "transfer list contains the source MessagePort.",
                );
                return None;
            }
            seen.push(port.into());
            message_ports.push(port);
            continue;
        }
        if let Ok(stream) = v8::Local::<v8::Object>::try_from(candidate)
            && is_readable_stream_object(scope, stream)
        {
            seen.push(stream.into());
            readable_streams.push(stream);
            continue;
        }
        if let Ok(stream) = v8::Local::<v8::Object>::try_from(candidate)
            && is_writable_stream_object(scope, stream)
        {
            seen.push(stream.into());
            writable_streams.push(stream);
            continue;
        }
        if let Ok(stream) = v8::Local::<v8::Object>::try_from(candidate)
            && is_transform_stream_object(scope, stream)
        {
            seen.push(stream.into());
            transform_streams.push(stream);
            continue;
        }
        operation
            .throw_data_clone_error(scope, "transfer list contains a non-transferable object.");
        return None;
    }
    Some(PostMessageTransferList {
        array_buffers,
        message_ports,
        readable_streams,
        writable_streams,
        transform_streams,
    })
}

fn resolve_post_message_transfer_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transfer_arg: Option<v8::Local<'s, v8::Value>>,
    interface_name: &str,
) -> Option<Option<v8::Local<'s, v8::Value>>> {
    let Some(transfer_arg) = transfer_arg.filter(|value| !value.is_null_or_undefined()) else {
        return Some(None);
    };
    if transfer_arg.is_array() {
        return Some(Some(transfer_arg));
    }
    let Ok(object) = v8::Local::<v8::Object>::try_from(transfer_arg) else {
        throw_post_message_type_error(
            scope,
            interface_name,
            "parameter 2 is not an iterable object or options dictionary.",
        );
        return None;
    };
    let transfer_key = v8str(scope, "transfer").into();
    if object.has(scope, transfer_key).unwrap_or(false) {
        let transfer = object.get(scope, transfer_key)?;
        if transfer.is_null_or_undefined() {
            return Some(None);
        }
        return Some(Some(transfer));
    }
    if has_iterator(scope, object) {
        return Some(Some(transfer_arg));
    }
    Some(None)
}

fn transfer_sequence<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    operation: TransferListOperation<'_>,
) -> Option<Vec<v8::Local<'s, v8::Value>>> {
    match webidl::convert::<webidl::Sequence<v8::Local<'s, v8::Value>>>(
        scope,
        value,
        operation.conversion_context(),
    ) {
        Ok(sequence) => Some(sequence.0),
        Err(error) => {
            if !error.is_pending_exception() {
                operation.throw_type_error(scope, "transfer list is not an iterable object.");
            }
            None
        }
    }
}

fn has_iterator<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>) -> bool {
    object
        .get(scope, v8::Symbol::get_iterator(scope).into())
        .is_some_and(|value| !value.is_null_or_undefined())
}

fn throw_post_message_type_error(
    scope: &mut v8::PinScope<'_, '_>,
    interface_name: &str,
    message: &str,
) {
    let formatted = format!("Failed to execute 'postMessage' on '{interface_name}': {message}");
    let message = v8_string(scope, &formatted).unwrap_or_else(|| v8::String::empty(scope));
    scope.throw_exception(v8::Exception::type_error(scope, message));
}

fn throw_data_clone_error(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    let exception =
        crate::context_bootstrap::new_dom_exception_value(scope, message, "DataCloneError");
    scope.throw_exception(exception);
}

fn throw_post_message_data_clone_error(
    scope: &mut v8::PinScope<'_, '_>,
    interface_name: &str,
    message: &str,
) {
    let formatted = format!("Failed to execute 'postMessage' on '{interface_name}': {message}");
    let exception =
        crate::context_bootstrap::new_dom_exception_value(scope, &formatted, "DataCloneError");
    scope.throw_exception(exception);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structured_clone::{RuntimeMessageAgentCluster, StructuredCloneMetadata};

    fn wasm_message(sender_origin: &str) -> V8StructuredClonePayload {
        let mut payload = V8StructuredClonePayload::default();
        payload.metadata = StructuredCloneMetadata {
            contains_wasm_module: true,
            origin_check_required: true,
            locked_to_sender_agent_cluster: true,
            remote_agent_cluster_mismatch: false,
            sender_agent_cluster: Some(RuntimeMessageAgentCluster::WindowOrDedicatedWorker),
            sender_origin: Some(sender_origin.to_owned()),
        };
        payload
    }

    #[test]
    fn wasm_module_message_origin_check_allows_exact_origin_only_by_default() {
        let payload = wasm_message("https://example.test");

        assert!(wasm_module_message_allowed_for_target_origin(
            &payload,
            Some("https://example.test"),
        ));
        assert!(!wasm_module_message_allowed_for_target_origin(
            &payload,
            Some("https://www.example.test"),
        ));
        assert!(!wasm_module_message_allowed_for_target_origin(
            &payload,
            Some("null"),
        ));
    }

    #[test]
    fn wasm_module_message_origin_check_rejects_cross_origin_in_same_agent_cluster() {
        let payload = wasm_message("https://www.example.test");
        assert!(!wasm_module_message_allowed_for_target_origin(
            &payload,
            Some("https://child.example.test"),
        ));
        assert!(!wasm_module_message_allowed_for_target_origin(
            &payload,
            Some("https://www.example.test:8443"),
        ));
    }

    #[test]
    fn wasm_module_message_origin_check_rejects_different_agent_cluster() {
        let payload = wasm_message("https://example.test");

        for target_agent_cluster in [
            RuntimeMessageAgentCluster::SharedWorker,
            RuntimeMessageAgentCluster::ServiceWorker,
        ] {
            assert!(!wasm_module_message_allowed_for_target(
                &payload,
                Some("https://example.test"),
                target_agent_cluster,
            ));
        }
    }

    #[test]
    fn remote_wasm_message_is_rejected_before_missing_attachment_deserialization() {
        let mut payload = wasm_message("https://example.test");
        payload.metadata.remote_agent_cluster_mismatch = true;

        assert!(!wasm_module_message_allowed_for_target_origin(
            &payload,
            Some("https://example.test"),
        ));
    }
}
