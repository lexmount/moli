use moli_core::network::WebStorageMutationSubscription;
use moli_core::network::{WebStorageAreaKind, WebStorageMutation, WebStorageMutationRecord};
use serde_json::{Value, json};

use crate::conn::{BackgroundProtocolEvent, CdpConnection, CommandOwnerScope};
use crate::domains::command_output::protocol_message_background_event;

pub(crate) fn take_pending_dom_storage_outputs_for_owner(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
) -> Vec<BackgroundProtocolEvent> {
    let mut events = Vec::new();
    for (event_session_id, subscription) in
        dom_storage_subscriptions_for_browser_context_owner(conn, owner)
    {
        events.extend(
            subscription.drain().into_iter().filter_map(|record| {
                event_from_mutation_record(record, event_session_id.as_deref())
            }),
        );
    }
    events
}

fn dom_storage_subscriptions_for_browser_context_owner(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
) -> Vec<(Option<String>, WebStorageMutationSubscription)> {
    let Some((browser_context_id, _)) = conn.target_owner_identity_for_owner(owner) else {
        return dom_storage_subscriptions_for_owner(conn, owner);
    };
    let Some(browser_context) = conn.browser_context_by_id(&browser_context_id) else {
        return dom_storage_subscriptions_for_owner(conn, owner);
    };

    let mut subscriptions = Vec::new();
    for target in browser_context.page_targets.iter() {
        if let Some(subscription) = target
            .devtools_sessions
            .primary()
            .dom_storage_session_state
            .mutation_subscription()
        {
            let event_session_id = target.session_id().map(str::to_owned);
            if browser_context.is_active_target(target.target_id()) || event_session_id.is_some() {
                subscriptions.push((event_session_id, subscription.clone()));
            }
        }
        subscriptions.extend(target.devtools_sessions.attached_entries().filter_map(
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

fn dom_storage_subscriptions_for_owner(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
) -> Vec<(Option<String>, WebStorageMutationSubscription)> {
    conn.page_event_session_ids_for_owner(owner)
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
