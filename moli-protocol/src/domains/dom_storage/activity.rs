use moli_core::network::WebStorageMutationSubscription;
use moli_core::network::{WebStorageAreaKind, WebStorageMutation, WebStorageMutationRecord};
use serde_json::{Value, json};

use crate::{
    conn::{BackgroundProtocolEvent, CdpConnection},
    domains::{
        activity::{
            ProtocolOutputPayloads, ProtocolOutputProjectionContext, ProtocolOutputSink,
            ProtocolOutputSlot,
        },
        command_output::protocol_message_background_event,
    },
};

#[derive(Clone, Debug, Default, PartialEq)]
struct DomStoragePreparedOutputs {
    events: Vec<BackgroundProtocolEvent>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::domains) struct DomStoragePreparedOutputSlot {
    outputs: DomStoragePreparedOutputs,
}

pub(in crate::domains) const SLOT_DOM_STORAGE: ProtocolOutputSlot = ProtocolOutputSlot::DomStorage;

pub(in crate::domains) async fn project_dom_storage_async(
    context: &mut ProtocolOutputProjectionContext<'_>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    let Some(slot) = prepared_outputs.and_then(ProtocolOutputPayloads::dom_storage_mut) else {
        return;
    };
    context
        .command
        .protocol_events_mut()
        .append(&mut slot.outputs.events);
}

impl DomStoragePreparedOutputSlot {
    pub(in crate::domains) fn extend(&mut self, mut other: Self) {
        self.outputs.events.append(&mut other.outputs.events);
    }
}

impl DomStoragePreparedOutputs {
    fn append_to_output_sink(self, sink: &mut (impl ProtocolOutputSink + ?Sized)) {
        if self.events.is_empty() {
            return;
        }
        sink.push_produced_slot(SLOT_DOM_STORAGE);
        sink.push_prepared_payload(DomStoragePreparedOutputSlot { outputs: self }.into());
    }
}

pub(in crate::domains) fn append_pending_dom_storage_outputs_for_session_owner(
    conn: &CdpConnection,
    session_id: Option<&str>,
    sink: &mut (impl ProtocolOutputSink + ?Sized),
) {
    let mut outputs = DomStoragePreparedOutputs::default();
    for (event_session_id, subscription) in
        dom_storage_subscriptions_for_browser_context_owner(conn, session_id)
    {
        let records = subscription.drain();
        outputs.events.extend(
            records.into_iter().filter_map(|record| {
                event_from_mutation_record(record, event_session_id.as_deref())
            }),
        );
    }
    outputs.append_to_output_sink(sink);
}

fn dom_storage_subscriptions_for_browser_context_owner(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> Vec<(Option<String>, WebStorageMutationSubscription)> {
    let Some((browser_context_id, _)) = conn.target_owner_identity_for_session(session_id) else {
        return dom_storage_subscriptions_for_session_owner(conn, session_id);
    };
    let Some(browser_context) = conn.browser_context_by_id(&browser_context_id) else {
        return dom_storage_subscriptions_for_session_owner(conn, session_id);
    };

    let mut subscriptions = Vec::new();
    if browser_context.page_targets.active().is_some() {
        if let Some(subscription) = browser_context
            .devtools_sessions
            .primary()
            .dom_storage_session_state
            .mutation_subscription()
        {
            subscriptions.push((
                browser_context.active_session_id_owned(),
                subscription.clone(),
            ));
        }
        subscriptions.extend(
            browser_context
                .devtools_sessions
                .attached_entries()
                .filter_map(|(session_id, state)| {
                    state
                        .dom_storage_session_state
                        .mutation_subscription()
                        .map(|subscription| (Some(session_id.to_owned()), subscription.clone()))
                }),
        );
    }
    for target in browser_context.background_targets() {
        let Some(state) = browser_context.parked_page_session_state(target.target_id()) else {
            continue;
        };
        if let (Some(session_id), Some(subscription)) = (
            target.session_id(),
            state
                .devtools_sessions
                .primary()
                .dom_storage_session_state
                .mutation_subscription(),
        ) {
            subscriptions.push((Some(session_id.to_owned()), subscription.clone()));
        }
        subscriptions.extend(state.devtools_sessions.attached_entries().filter_map(
            |(session_id, state)| {
                state
                    .dom_storage_session_state
                    .mutation_subscription()
                    .map(|subscription| (Some(session_id.to_owned()), subscription.clone()))
            },
        ));
    }
    subscriptions.sort_by(|left, right| left.0.cmp(&right.0));
    subscriptions
}

fn dom_storage_subscriptions_for_session_owner(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> Vec<(Option<String>, WebStorageMutationSubscription)> {
    conn.page_event_session_ids_for_session_owner(session_id)
        .into_iter()
        .filter_map(|event_session_id| {
            let subscription = conn
                .target_devtools_session_state_for_session(event_session_id.as_deref())?
                .dom_storage_session_state
                .mutation_subscription()?
                .clone();
            Some((event_session_id, subscription))
        })
        .collect()
}

fn event_from_mutation_record(
    record: WebStorageMutationRecord,
    session_id: Option<&str>,
) -> Option<BackgroundProtocolEvent> {
    let (method, params) = match record.mutation {
        WebStorageMutation::ItemAdded {
            area_key,
            key,
            value,
        } => (
            "DOMStorage.domStorageItemAdded",
            json!({
                "storageId": storage_id_for_area_key(&area_key, record.area_kind)?,
                "key": key.to_string_lossy(),
                "newValue": value.to_string_lossy(),
            }),
        ),
        WebStorageMutation::ItemUpdated {
            area_key,
            key,
            old_value,
            new_value,
        } => (
            "DOMStorage.domStorageItemUpdated",
            json!({
                "storageId": storage_id_for_area_key(&area_key, record.area_kind)?,
                "key": key.to_string_lossy(),
                "oldValue": old_value.to_string_lossy(),
                "newValue": new_value.to_string_lossy(),
            }),
        ),
        WebStorageMutation::ItemRemoved { area_key, key, .. } => (
            "DOMStorage.domStorageItemRemoved",
            json!({
                "storageId": storage_id_for_area_key(&area_key, record.area_kind)?,
                "key": key.to_string_lossy(),
            }),
        ),
        WebStorageMutation::ItemsCleared { area_key } => (
            "DOMStorage.domStorageItemsCleared",
            json!({
                "storageId": storage_id_for_area_key(&area_key, record.area_kind)?,
            }),
        ),
    };
    let mut message = json!({
        "method": method,
        "params": params,
    });
    if let Some(session_id) = session_id {
        message["sessionId"] = Value::String(session_id.to_owned());
    }
    Some(protocol_message_background_event(message))
}

fn storage_id_for_area_key(area_key: &str, area_kind: WebStorageAreaKind) -> Option<Value> {
    let storage_key = moli_storage_key::deserialize_serialized_storage_key(area_key)?;
    if moli_storage_key::serialized_storage_key_has_opaque_origin(area_key) {
        return None;
    }
    Some(json!({
        "securityOrigin": storage_key.origin(),
        "storageKey": area_key,
        "isLocalStorage": area_kind == WebStorageAreaKind::Local,
    }))
}
