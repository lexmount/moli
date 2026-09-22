use super::{
    RendererOutputItem, RendererOutputRecord, RendererOwnerAction, RendererProtocolObservation,
};

impl RendererOutputRecord {
    /// Conservative retained-memory charge for one frozen semantic record.
    ///
    /// The transport also has a message-count ceiling. This charge adds the
    /// variable payloads that dominate real Console/Inspector/Network floods
    /// and applies a per-record floor for enum and allocator overhead. It
    /// measures retained semantic data rather than future CDP JSON, which does
    /// not exist until protocol projection.
    pub(crate) fn transport_charge_bytes(&self) -> usize {
        const RECORD_FLOOR_BYTES: usize = 512;
        let payload = match self.item() {
            RendererOutputItem::Observation(observation) => {
                observation_transport_charge_bytes(observation)
            }
            RendererOutputItem::OwnerAction(action) => owner_action_transport_charge_bytes(action),
        };
        std::mem::size_of::<Self>()
            .saturating_add(payload)
            .max(RECORD_FLOOR_BYTES)
    }
}

fn string_charge(value: &str) -> usize {
    // String capacity is not exposed through every borrowed payload. Charging
    // twice the visible UTF-8 length covers ordinary geometric growth while
    // keeping accounting allocation-free on a V8 callback path.
    value.len().saturating_mul(2)
}

fn json_charge(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            std::mem::size_of::<serde_json::Value>()
        }
        serde_json::Value::String(value) => {
            std::mem::size_of::<serde_json::Value>().saturating_add(string_charge(value))
        }
        serde_json::Value::Array(values) => values.iter().fold(
            values
                .capacity()
                .saturating_mul(std::mem::size_of::<serde_json::Value>()),
            |total, value| total.saturating_add(json_charge(value)),
        ),
        serde_json::Value::Object(values) => {
            values
                .iter()
                .fold(values.len().saturating_mul(64), |total, (key, value)| {
                    total
                        .saturating_add(string_charge(key))
                        .saturating_add(json_charge(value))
                })
        }
    }
}

fn observation_transport_charge_bytes(observation: &RendererProtocolObservation) -> usize {
    match observation {
        RendererProtocolObservation::MainDocumentCommit(commit) => [
            commit.frame_id.as_str(),
            commit.loader_id.as_str(),
            commit.url.as_str(),
            commit.security_origin.as_str(),
            commit.secure_context_type.as_str(),
        ]
        .into_iter()
        .map(string_charge)
        .sum(),
        RendererProtocolObservation::DocumentTitleChanged(change) => string_charge(&change.title),
        RendererProtocolObservation::DocumentLifecycle(_) => 0,
        RendererProtocolObservation::Network { item, .. } => item.renderer_transport_charge_bytes(),
        RendererProtocolObservation::RuntimeBinding(call) => {
            string_charge(&call.name).saturating_add(string_charge(&call.payload))
        }
        RendererProtocolObservation::DomMutations(batch) => batch.events.iter().fold(
            batch.events.capacity().saturating_mul(128),
            |total, event| total.saturating_add(dom_mutation_transport_charge_bytes(event)),
        ),
        RendererProtocolObservation::RuntimeInspector(batch) => batch.messages.iter().fold(
            batch.messages.capacity().saturating_mul(128),
            |total, message| {
                total.saturating_add(runtime_inspector_message_transport_charge_bytes(message))
            },
        ),
        RendererProtocolObservation::RuntimeConsole(console) => {
            let args = console.args.iter().fold(0usize, |total, value| {
                total.saturating_add(json_charge(value))
            });
            string_charge(&console.message)
                .saturating_add(console.stack.as_deref().map(string_charge).unwrap_or(0))
                .saturating_add(args)
        }
        RendererProtocolObservation::InspectorIssue { issue, .. } => {
            issue.renderer_transport_charge_bytes()
        }
        RendererProtocolObservation::WindowOpen(event) => event.window_features.iter().fold(
            string_charge(&event.url).saturating_add(string_charge(&event.window_name)),
            |total, feature| total.saturating_add(string_charge(feature)),
        ),
        RendererProtocolObservation::RuntimeLifecycleError { text, .. } => string_charge(text),
    }
}

fn owner_action_transport_charge_bytes(action: &RendererOwnerAction) -> usize {
    match action {
        RendererOwnerAction::FileChooser(event) => {
            event.source_frame_id().map(string_charge).unwrap_or(0)
        }
        RendererOwnerAction::Download(event) => {
            let mut total = string_charge(&event.url).saturating_add(
                event
                    .suggested_filename
                    .as_deref()
                    .map(string_charge)
                    .unwrap_or(0),
            );
            if let Some(response) = &event.response {
                total = total
                    .saturating_add(string_charge(&response.final_url))
                    .saturating_add(headers_charge(&response.headers))
                    .saturating_add(response.body.capacity());
            }
            total
        }
        RendererOwnerAction::JavaScriptDialog(event) => [
            event.source_url(),
            event.dialog_type(),
            event.message(),
            event.default_prompt(),
        ]
        .into_iter()
        .map(string_charge)
        .sum(),
        RendererOwnerAction::Popup(event) => {
            string_charge(event.url()).saturating_add(string_charge(event.target_name()))
        }
        RendererOwnerAction::ChildFrameTree { event, .. } => match event {
            crate::protocol_types::ChildFrameTreeEventSnapshot::Attached(event) => {
                string_charge(&event.frame_id).saturating_add(
                    event
                        .parent_frame_id
                        .as_deref()
                        .map(string_charge)
                        .unwrap_or(0),
                )
            }
            crate::protocol_types::ChildFrameTreeEventSnapshot::Detached(event) => {
                string_charge(&event.frame_id)
            }
        },
        RendererOwnerAction::ChildFrameDocumentOpened { event, .. } => [
            Some(event.frame_id.as_str()),
            event.parent_frame_id.as_deref(),
            event.loader_id.as_deref(),
            event.name.as_deref(),
            Some(event.url.as_str()),
        ]
        .into_iter()
        .flatten()
        .map(string_charge)
        .sum(),
        RendererOwnerAction::ChildFrameDocumentNetwork { event, .. } => {
            string_charge(&event.frame_id)
                .saturating_add(
                    event
                        .parent_frame_id
                        .as_deref()
                        .map(string_charge)
                        .unwrap_or(0),
                )
                .saturating_add(string_charge(&event.loader_id))
                .saturating_add(child_frame_document_network_charge(&event.snapshot))
        }
        RendererOwnerAction::ChildFrameLoad { event, .. } => [
            Some(event.frame_id.as_str()),
            event.parent_frame_id.as_deref(),
            event.loader_id.as_deref(),
            event.name.as_deref(),
            Some(event.url.as_str()),
        ]
        .into_iter()
        .flatten()
        .map(string_charge)
        .sum::<usize>()
        .saturating_add(
            event
                .document_network
                .as_ref()
                .map(child_frame_document_network_charge)
                .unwrap_or(0),
        ),
        RendererOwnerAction::SameDocumentNavigation(event) => {
            let navigation = event.navigation();
            string_charge(&navigation.url)
                .saturating_add(string_charge(&navigation.navigation_type))
        }
        RendererOwnerAction::TopLevelLocationNavigation(event) => string_charge(event.url())
            .saturating_add(string_charge(event.request_method()))
            .saturating_add(
                event
                    .request_body()
                    .map(|body| body.len())
                    .unwrap_or_default(),
            )
            .saturating_add(headers_charge(event.request_headers())),
        RendererOwnerAction::TopLevelHistoryTraversal(_) => 0,
        RendererOwnerAction::SubresourceFetchPause { info, .. } => {
            info.renderer_transport_charge_bytes()
        }
        RendererOwnerAction::SubresourceContinue { event, .. } => {
            event.renderer_transport_charge_bytes()
        }
        RendererOwnerAction::DetachedParserScriptFetchPause { info, .. } => {
            info.renderer_transport_charge_bytes()
        }
        RendererOwnerAction::SharedWorkerTargetLifecycle(event) => {
            shared_worker_event_transport_charge_bytes(event)
        }
        RendererOwnerAction::ServiceWorkerTargetLifecycle(event) => {
            service_worker_event_transport_charge_bytes(event)
        }
        RendererOwnerAction::DedicatedWorkerTargetLifecycle(event) => {
            dedicated_worker_event_transport_charge_bytes(event)
        }
    }
}

fn headers_charge(headers: &[(String, String)]) -> usize {
    headers.iter().fold(
        headers
            .len()
            .saturating_mul(std::mem::size_of::<(String, String)>()),
        |total, (name, value)| {
            total
                .saturating_add(string_charge(name))
                .saturating_add(string_charge(value))
        },
    )
}

fn child_frame_document_network_charge(
    snapshot: &crate::protocol_types::ChildFrameDocumentNetworkSnapshot,
) -> usize {
    [
        snapshot.request_url.as_str(),
        snapshot.request_method.as_str(),
        snapshot.final_url.as_str(),
    ]
    .into_iter()
    .map(string_charge)
    .sum::<usize>()
    .saturating_add(headers_charge(&snapshot.request_headers))
    .saturating_add(headers_charge(&snapshot.response_headers))
    .saturating_add(
        snapshot
            .response_body
            .as_ref()
            .map(|body| body.renderer_transport_retained_memory_bytes())
            .unwrap_or_default(),
    )
}

fn runtime_inspector_message_transport_charge_bytes(
    message: &crate::runtime::RendererRuntimeInspectorMessage,
) -> usize {
    message.renderer_transport_charge_bytes_with(json_charge, string_charge)
}

fn dom_mutation_transport_charge_bytes(event: &crate::runtime::RendererDomMutationEvent) -> usize {
    match event {
        crate::runtime::RendererDomMutationEvent::AttributeModified { name, value, .. } => {
            string_charge(name).saturating_add(string_charge(value))
        }
        crate::runtime::RendererDomMutationEvent::AttributeRemoved { name, .. } => {
            string_charge(name)
        }
        crate::runtime::RendererDomMutationEvent::CharacterDataModified {
            character_data, ..
        } => string_charge(character_data),
        crate::runtime::RendererDomMutationEvent::SetChildNodes { nodes, .. } => nodes
            .iter()
            .fold(nodes.capacity().saturating_mul(256), |total, node| {
                total.saturating_add(node.renderer_transport_charge_bytes())
            }),
        crate::runtime::RendererDomMutationEvent::ChildNodeInserted { node, .. } => {
            node.renderer_transport_charge_bytes()
        }
        crate::runtime::RendererDomMutationEvent::ChildNodeCountUpdated { .. }
        | crate::runtime::RendererDomMutationEvent::ChildNodeRemoved { .. } => 0,
    }
}

fn shared_worker_event_transport_charge_bytes(
    event: &crate::runtime::RendererSharedWorkerTargetEvent,
) -> usize {
    match event {
        crate::runtime::RendererSharedWorkerTargetEvent::Created(info) => {
            string_charge(&info.url).saturating_add(string_charge(&info.name))
        }
        crate::runtime::RendererSharedWorkerTargetEvent::Console { message, .. } => {
            message.args.iter().fold(
                string_charge(&message.message)
                    .saturating_add(message.stack.as_deref().map(string_charge).unwrap_or(0)),
                |total, value| total.saturating_add(json_charge(value)),
            )
        }
        crate::runtime::RendererSharedWorkerTargetEvent::RuntimeInspectorMessages {
            inspector_session_id,
            messages,
            ..
        } => messages.iter().fold(
            inspector_session_id
                .as_deref()
                .map(string_charge)
                .unwrap_or(0),
            |total, message| {
                total.saturating_add(runtime_inspector_message_transport_charge_bytes(message))
            },
        ),
        crate::runtime::RendererSharedWorkerTargetEvent::Destroyed { .. } => 0,
    }
}

fn service_worker_event_transport_charge_bytes(
    event: &crate::runtime::RendererServiceWorkerTargetEvent,
) -> usize {
    match event {
        crate::runtime::RendererServiceWorkerTargetEvent::Created { info, .. } => {
            string_charge(&info.script_url).saturating_add(string_charge(&info.scope_url))
        }
        crate::runtime::RendererServiceWorkerTargetEvent::Stopped { reason, .. } => {
            string_charge(reason)
        }
        crate::runtime::RendererServiceWorkerTargetEvent::Console { message, .. } => {
            message.args.iter().fold(
                string_charge(&message.message)
                    .saturating_add(message.stack.as_deref().map(string_charge).unwrap_or(0)),
                |total, value| total.saturating_add(json_charge(value)),
            )
        }
        crate::runtime::RendererServiceWorkerTargetEvent::Exception { message, .. } => [
            message.message.as_str(),
            message.filename.as_str(),
            message.event_kind.as_str(),
            message.phase.as_str(),
            message.source.as_str(),
        ]
        .into_iter()
        .map(string_charge)
        .sum(),
        crate::runtime::RendererServiceWorkerTargetEvent::FetchDiagnostic {
            diagnostic, ..
        } => {
            let mut total = [
                diagnostic.document_url.as_str(),
                diagnostic.request_url.as_str(),
                diagnostic.method.as_str(),
                diagnostic.destination.as_str(),
            ]
            .into_iter()
            .map(string_charge)
            .sum::<usize>();
            total = diagnostic
                .request_headers
                .iter()
                .fold(total, |total, (name, value)| {
                    total
                        .saturating_add(string_charge(name))
                        .saturating_add(string_charge(value))
                });
            total = total.saturating_add(
                diagnostic
                    .request_body
                    .as_deref()
                    .map(string_charge)
                    .unwrap_or(0),
            );
            match &diagnostic.result {
                crate::runtime::RendererServiceWorkerFetchDiagnosticResult::Fallback => total,
                crate::runtime::RendererServiceWorkerFetchDiagnosticResult::Response {
                    final_url,
                    status_text,
                    response_headers,
                    ..
                } => response_headers.iter().fold(
                    total
                        .saturating_add(string_charge(final_url))
                        .saturating_add(string_charge(status_text)),
                    |total, (name, value)| {
                        total
                            .saturating_add(string_charge(name))
                            .saturating_add(string_charge(value))
                    },
                ),
                crate::runtime::RendererServiceWorkerFetchDiagnosticResult::Failure { message } => {
                    total.saturating_add(string_charge(message))
                }
            }
        }
        crate::runtime::RendererServiceWorkerTargetEvent::RuntimeInspectorMessages {
            inspector_session_id,
            messages,
            ..
        } => messages.iter().fold(
            inspector_session_id
                .as_deref()
                .map(string_charge)
                .unwrap_or(0),
            |total, message| {
                total.saturating_add(runtime_inspector_message_transport_charge_bytes(message))
            },
        ),
        crate::runtime::RendererServiceWorkerTargetEvent::Started { .. }
        | crate::runtime::RendererServiceWorkerTargetEvent::Destroyed { .. }
        | crate::runtime::RendererServiceWorkerTargetEvent::VersionUpdated { .. } => 0,
    }
}

fn dedicated_worker_event_transport_charge_bytes(
    event: &crate::runtime::RendererDedicatedWorkerTargetEvent,
) -> usize {
    match event {
        crate::runtime::RendererDedicatedWorkerTargetEvent::Created(info) => [
            info.request_url.as_str(),
            info.document_url.as_str(),
            info.name.as_str(),
        ]
        .into_iter()
        .map(string_charge)
        .sum(),
        crate::runtime::RendererDedicatedWorkerTargetEvent::ScriptLoaded {
            script_url,
            response,
            ..
        } => string_charge(script_url)
            .saturating_add(navigation_response_transport_charge_bytes(response)),
        crate::runtime::RendererDedicatedWorkerTargetEvent::ScriptLoadFailed {
            script_url,
            error_message,
            response,
            ..
        } => string_charge(script_url)
            .saturating_add(string_charge(error_message))
            .saturating_add(
                response
                    .as_deref()
                    .map(navigation_response_transport_charge_bytes)
                    .unwrap_or(0),
            ),
        crate::runtime::RendererDedicatedWorkerTargetEvent::Console { message, .. } => {
            message.args.iter().fold(
                string_charge(&message.message)
                    .saturating_add(message.stack.as_deref().map(string_charge).unwrap_or(0)),
                |total, value| total.saturating_add(json_charge(value)),
            )
        }
        crate::runtime::RendererDedicatedWorkerTargetEvent::RuntimeInspectorMessages {
            inspector_session_id,
            messages,
            ..
        } => messages.iter().fold(
            inspector_session_id
                .as_deref()
                .map(string_charge)
                .unwrap_or(0),
            |total, message| {
                total.saturating_add(runtime_inspector_message_transport_charge_bytes(message))
            },
        ),
        crate::runtime::RendererDedicatedWorkerTargetEvent::Destroyed { .. } => 0,
    }
}

fn navigation_response_transport_charge_bytes(
    response: &crate::protocol_types::NavigationResponse,
) -> usize {
    string_charge(response.final_url.as_str())
        .saturating_add(headers_charge(&response.headers))
        .saturating_add(response.body_bytes().len())
        .saturating_add(
            response
                .network_request_headers()
                .map(headers_charge)
                .unwrap_or(0),
        )
}
