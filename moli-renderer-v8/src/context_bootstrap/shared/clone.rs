use crate::{
    context_bootstrap::{
        current_child_browsing_context_handle_for_runtime_scope, current_worker_script_url,
        message_port_id_from_object,
    },
    structured_clone::{
        PreparedStructuredClone, RuntimeMessageAgentCluster, StructuredCloneMetadata,
        V8StructuredClonePayload, WireTransfer, WireTransfers, deserialize_from_wire,
        deserialize_message_event_from_wire, prepare_for_window_post_message,
        serialize_for_wire_for_runtime, serialize_for_wire_for_runtime_message,
        serialize_for_wire_for_runtime_with_transfers, serialize_for_wire_for_storage,
    },
    types::MessagePortId,
    util::{context_host_ptr_from_global_bridge, v8_string, v8str},
    webidl,
};

pub(in crate::context_bootstrap) struct PostMessageTransferList<'s> {
    entries: Vec<WireTransfer<'s>>,
}

impl<'s> PostMessageTransferList<'s> {
    fn wire_transfers(&self) -> WireTransfers<'_, 's> {
        WireTransfers {
            entries: &self.entries,
        }
    }

    pub(in crate::context_bootstrap) fn serialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
    ) -> Option<V8StructuredClonePayload> {
        let mut payload =
            serialize_for_wire_for_runtime_message(scope, value, &self.wire_transfers())?;
        attach_runtime_message_source(
            &mut payload.metadata,
            RuntimeMessageSourceSecurity::current(scope),
        );
        Some(payload)
    }
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
    serialize_for_wire_for_runtime(scope, value)
}

pub(crate) fn structured_serialize_value_for_storage<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<V8StructuredClonePayload> {
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
    transfers.serialize(scope, value)
}

pub(crate) fn prepare_value_for_window_post_message<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    objects: Vec<v8::Local<'s, v8::Object>>,
    source_security: RuntimeMessageSourceSecurity,
) -> Option<PreparedStructuredClone<'s>> {
    let values = objects.into_iter().map(Into::into).collect();
    let transfers = parse_transfer_values(
        scope,
        values,
        TransferListOperation::PostMessage("Window"),
        None,
    )?;
    let mut prepared = prepare_for_window_post_message(scope, value, &transfers.wire_transfers())?;
    attach_runtime_message_source(prepared.metadata_mut(), source_security);
    Some(prepared)
}

fn empty_post_message_transfer_list<'s>() -> PostMessageTransferList<'s> {
    PostMessageTransferList {
        entries: Vec::new(),
    }
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
        let top_origin = host.main_document_origin();
        if let Some(popup_id) = crate::native_bridge::active_lightweight_popup_id(scope) {
            return host.lightweight_popup_origin(popup_id);
        }
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
    metadata: &mut StructuredCloneMetadata,
    source_security: RuntimeMessageSourceSecurity,
) {
    if metadata.contains_wasm_module && metadata.origin_check_required {
        metadata.sender_origin = source_security.origin;
        metadata.sender_agent_cluster = Some(source_security.agent_cluster);
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
    let payload = structured_serialize_value_with_options(scope, value, options)?;
    structured_deserialize_value(scope, &payload)
}

pub(crate) fn structured_serialize_value_with_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> Option<V8StructuredClonePayload> {
    let transfers = parse_structured_clone_options_transfer_list(scope, options)?;
    serialize_for_wire_for_runtime_with_transfers(scope, value, &transfers.wire_transfers())
}

pub(crate) fn structured_clone_value_for_storage<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let bytes = structured_serialize_value_for_storage(scope, value)?;
    structured_deserialize_value(scope, &bytes)
}

pub(in crate::context_bootstrap) fn parse_post_message_transfer_list<'s>(
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
    let mut entries = Vec::with_capacity(sequence.len());
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
            if !buffer.was_detached() && !buffer.is_detachable() {
                operation.throw_type_error(
                    scope,
                    "transfer list contains a non-transferable ArrayBuffer.",
                );
                return None;
            }
            seen.push(buffer.into());
            entries.push(WireTransfer::ArrayBuffer(buffer));
            continue;
        }
        if let Ok(object) = v8::Local::<v8::Object>::try_from(candidate) {
            match moli_webapi_declare::web_api_object_type(scope, object).map(|kind| kind.name()) {
                Some("VideoFrame") => {
                    if crate::context_bootstrap::video_frame::is_closed(scope, object) {
                        operation.throw_data_clone_error(
                            scope,
                            "transfer list contains a closed VideoFrame.",
                        );
                        return None;
                    }
                    seen.push(object.into());
                    entries.push(WireTransfer::VideoFrame(object));
                    continue;
                }
                Some("MessagePort") => {
                    let target = moli_webapi_declare::web_api_object_target(scope, object)?;
                    if seen
                        .iter()
                        .any(|existing| existing.strict_equals(target.into()))
                    {
                        operation.throw_data_clone_error(
                            scope,
                            "transfer list contains duplicate transferable entries.",
                        );
                        return None;
                    }
                    if source_port_id.is_some()
                        && message_port_id_from_object(scope, target) == source_port_id
                    {
                        operation.throw_data_clone_error(
                            scope,
                            "transfer list contains the source MessagePort.",
                        );
                        return None;
                    }
                    // Transfer state is checked after the message's getters.
                    // A closed connection still has transferable identity.
                    seen.push(target.into());
                    entries.push(WireTransfer::MessagePort(target));
                    continue;
                }
                Some("ReadableStream") => {
                    seen.push(object.into());
                    entries.push(WireTransfer::ReadableStream(object));
                    continue;
                }
                Some("WritableStream") => {
                    seen.push(object.into());
                    entries.push(WireTransfer::WritableStream(object));
                    continue;
                }
                Some("TransformStream") => {
                    seen.push(object.into());
                    entries.push(WireTransfer::TransformStream(object));
                    continue;
                }
                _ => {}
            }
        }
        operation
            .throw_data_clone_error(scope, "transfer list contains a non-transferable object.");
        return None;
    }
    Some(PostMessageTransferList { entries })
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
}
