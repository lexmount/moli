use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde_json::Value;

use super::types::{BidiSubscription, bidi_event_subscribed_channels, bidi_message_with_channel};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BidiEventSourceHookPlan {
    runtime_contexts: Option<Vec<String>>,
    runtime_disabled_contexts: Option<Vec<String>>,
    record_runtime_context_ownership: bool,
    network_contexts: Option<Vec<String>>,
    network_disabled_contexts: Option<Vec<String>>,
    file_dialog_opened_contexts: Option<Vec<String>>,
    file_dialog_opened_disabled_contexts: Option<Vec<String>>,
    download_events_enabled: bool,
    download_events_disabled: bool,
}

impl BidiEventSourceHookPlan {
    pub fn runtime_contexts(&self) -> Option<&[String]> {
        self.runtime_contexts.as_deref()
    }

    pub fn runtime_disabled_contexts(&self) -> Option<&[String]> {
        self.runtime_disabled_contexts.as_deref()
    }

    pub fn records_runtime_context_ownership(&self) -> bool {
        self.record_runtime_context_ownership
    }

    pub fn network_contexts(&self) -> Option<&[String]> {
        self.network_contexts.as_deref()
    }

    pub fn network_disabled_contexts(&self) -> Option<&[String]> {
        self.network_disabled_contexts.as_deref()
    }

    pub fn file_dialog_opened_contexts(&self) -> Option<&[String]> {
        self.file_dialog_opened_contexts.as_deref()
    }

    pub fn file_dialog_opened_disabled_contexts(&self) -> Option<&[String]> {
        self.file_dialog_opened_disabled_contexts.as_deref()
    }

    pub fn download_events_enabled(&self) -> bool {
        self.download_events_enabled
    }

    pub fn download_events_disabled(&self) -> bool {
        self.download_events_disabled
    }

    pub(super) fn set_runtime_scope(&mut self, scope: BidiEventSourceHookScope) {
        self.runtime_contexts = Some(scope.into_contexts());
    }

    pub(super) fn record_runtime_context_ownership(&mut self) {
        self.record_runtime_context_ownership = true;
    }

    pub(super) fn set_runtime_disable_scope(&mut self, scope: BidiEventSourceHookScope) {
        self.runtime_disabled_contexts = Some(scope.into_contexts());
    }

    pub(super) fn set_network_scope(&mut self, scope: BidiEventSourceHookScope) {
        self.network_contexts = Some(scope.into_contexts());
    }

    pub(super) fn set_network_disable_scope(&mut self, scope: BidiEventSourceHookScope) {
        self.network_disabled_contexts = Some(scope.into_contexts());
    }

    pub(super) fn set_file_dialog_opened_scope(&mut self, scope: BidiEventSourceHookScope) {
        self.file_dialog_opened_contexts = Some(scope.into_contexts());
    }

    pub(super) fn set_file_dialog_opened_disable_scope(&mut self, scope: BidiEventSourceHookScope) {
        self.file_dialog_opened_disabled_contexts = Some(scope.into_contexts());
    }

    pub(super) fn enable_download_events(&mut self) {
        self.download_events_enabled = true;
    }

    pub(super) fn disable_download_events(&mut self) {
        self.download_events_disabled = true;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct BidiEventSourceOwnership {
    runtime_contexts: BTreeSet<String>,
    network_contexts: BTreeSet<String>,
    file_dialog_opened_contexts: BTreeSet<String>,
    download_events: bool,
}

impl BidiEventSourceOwnership {
    pub(super) fn runtime_context_opened(&self, context: &str) -> bool {
        self.runtime_contexts.contains(context)
    }

    pub(super) fn opened_runtime_contexts(&self) -> BTreeSet<String> {
        self.runtime_contexts.clone()
    }

    pub(super) fn opened_network_contexts(&self) -> BTreeSet<String> {
        self.network_contexts.clone()
    }

    pub(super) fn opened_file_dialog_contexts(&self) -> BTreeSet<String> {
        self.file_dialog_opened_contexts.clone()
    }

    pub(super) fn download_events_opened(&self) -> bool {
        self.download_events
    }

    pub(super) fn record_runtime_context_opened(&mut self, context: &str) {
        self.runtime_contexts.insert(context.to_owned());
    }

    pub(super) fn record_runtime_context_closed(&mut self, context: &str) {
        self.runtime_contexts.remove(context);
    }

    pub(super) fn record_network_context_opened(&mut self, context: &str) {
        self.network_contexts.insert(context.to_owned());
    }

    pub(super) fn record_network_context_closed(&mut self, context: &str) {
        self.network_contexts.remove(context);
    }

    pub(super) fn record_file_dialog_context_opened(&mut self, context: &str) {
        self.file_dialog_opened_contexts.insert(context.to_owned());
    }

    pub(super) fn record_file_dialog_context_closed(&mut self, context: &str) {
        self.file_dialog_opened_contexts.remove(context);
    }

    pub(super) fn record_download_events_opened(&mut self) {
        self.download_events = true;
    }

    pub(super) fn record_download_events_closed(&mut self) {
        self.download_events = false;
    }

    pub(super) fn forget_context(&mut self, context: &str) {
        self.runtime_contexts.remove(context);
        self.network_contexts.remove(context);
        self.file_dialog_opened_contexts.remove(context);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BidiEventSourceHookScope {
    Global,
    Contexts(BTreeSet<String>),
}

impl BidiEventSourceHookScope {
    fn into_contexts(self) -> Vec<String> {
        match self {
            Self::Global => Vec::new(),
            Self::Contexts(contexts) => contexts.into_iter().collect(),
        }
    }
}

pub(super) fn is_bidi_runtime_source_event_name(event: &str) -> bool {
    matches!(
        event,
        "log.entryAdded" | "script.realmCreated" | "script.realmDestroyed"
    )
}

pub(super) fn is_bidi_network_event_name(event: &str) -> bool {
    event.starts_with("network.")
}

pub(super) fn is_bidi_download_event_name(event: &str) -> bool {
    matches!(
        event,
        "browsingContext.downloadWillBegin" | "browsingContext.downloadEnd"
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BidiBufferedEventStore {
    capacity: usize,
    next_event_id: u64,
    events: VecDeque<BidiBufferedEvent>,
    last_sent_by_key: BTreeMap<BidiBufferedEventKey, u64>,
}

impl BidiBufferedEventStore {
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            next_event_id: 0,
            events: VecDeque::new(),
            last_sent_by_key: BTreeMap::new(),
        }
    }

    pub(super) fn buffer_event(&mut self, event: Value) -> u64 {
        self.next_event_id = self.next_event_id.saturating_add(1);
        let id = self.next_event_id;
        if self.events.len() >= self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(BidiBufferedEvent { id, event });
        id
    }

    pub(super) fn mark_event_sent(&mut self, id: u64, channel: Option<&str>) {
        let Some(event) = self.events.iter().find(|event| event.id == id) else {
            return;
        };
        self.mark_event_sent_by_key(id, &event.key(channel));
    }

    pub(super) fn forget_context(&mut self, context: &str) {
        self.events
            .retain(|event| bidi_event_context(&event.event).as_deref() != Some(context));
        self.last_sent_by_key
            .retain(|key, _| key.context.as_deref() != Some(context));
    }

    pub(super) fn replay_matching_events(
        &mut self,
        subscriptions: &[BidiSubscription],
        context_user_contexts: &BTreeMap<String, String>,
        context_top_level_contexts: &BTreeMap<String, String>,
    ) -> Vec<Value> {
        let replayed = self
            .events
            .iter()
            .flat_map(|event| {
                bidi_event_subscribed_channels(
                    subscriptions,
                    &event.event,
                    context_user_contexts,
                    context_top_level_contexts,
                )
                .into_iter()
                .filter(|channel| self.event_is_new_for_channel(event, channel.as_deref()))
                .map(|channel| {
                    let key = event.key(channel.as_deref());
                    let message =
                        bidi_message_with_channel(event.event.clone(), channel.as_deref());
                    (event.id, key, message)
                })
                .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for (id, key, _) in &replayed {
            self.mark_event_sent_by_key(*id, key);
        }
        replayed
            .into_iter()
            .map(|(_, _, event)| event)
            .collect::<Vec<_>>()
    }

    fn event_is_new_for_channel(&self, event: &BidiBufferedEvent, channel: Option<&str>) -> bool {
        self.last_sent_by_key
            .get(&event.key(channel))
            .is_none_or(|last_sent_id| event.id > *last_sent_id)
    }

    fn mark_event_sent_by_key(&mut self, id: u64, key: &BidiBufferedEventKey) {
        self.last_sent_by_key
            .entry(key.clone())
            .and_modify(|last_sent_id| *last_sent_id = (*last_sent_id).max(id))
            .or_insert(id);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BidiBufferedEvent {
    id: u64,
    event: Value,
}

impl BidiBufferedEvent {
    fn key(&self, channel: Option<&str>) -> BidiBufferedEventKey {
        BidiBufferedEventKey {
            method: self
                .event
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            context: bidi_event_context(&self.event),
            channel: channel.map(str::to_owned),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct BidiBufferedEventKey {
    method: String,
    context: Option<String>,
    channel: Option<String>,
}

fn bidi_event_context(event: &Value) -> Option<String> {
    let params = event.get("params")?;
    params
        .get("context")
        .and_then(Value::as_str)
        .or_else(|| {
            params
                .get("source")
                .and_then(|source| source.get("context"))
                .and_then(Value::as_str)
        })
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use serde_json::json;

    use super::*;

    fn log_subscription_for_context(context: &str) -> BidiSubscription {
        BidiSubscription {
            id: "subscription-1".to_owned(),
            events: BTreeSet::from(["log.entryAdded".to_owned()]),
            contexts: BTreeSet::from([context.to_owned()]),
            user_contexts: BTreeSet::new(),
            channel: None,
        }
    }

    fn log_subscription_for_context_and_channel(
        context: &str,
        channel: Option<&str>,
    ) -> BidiSubscription {
        BidiSubscription {
            id: format!("subscription-{channel:?}"),
            events: BTreeSet::from(["log.entryAdded".to_owned()]),
            contexts: BTreeSet::from([context.to_owned()]),
            user_contexts: BTreeSet::new(),
            channel: channel.map(str::to_owned),
        }
    }

    fn context_maps(context: &str) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
        (
            BTreeMap::from([(context.to_owned(), "default".to_owned())]),
            BTreeMap::from([(context.to_owned(), context.to_owned())]),
        )
    }

    fn log_entry(context: &str, text: &str) -> Value {
        json!({
            "method": "log.entryAdded",
            "params": {
                "level": "info",
                "method": "log",
                "text": text,
                "source": {
                    "realm": "realm-1",
                    "context": context
                }
            }
        })
    }

    #[test]
    fn buffered_event_replay_tracks_last_sent_without_draining() {
        let mut store = BidiBufferedEventStore::with_capacity(8);
        store.buffer_event(log_entry("FRAME-1", "cached"));

        let subscriptions = vec![log_subscription_for_context("FRAME-1")];
        let (context_user_contexts, context_top_level_contexts) = context_maps("FRAME-1");

        let replayed = store.replay_matching_events(
            &subscriptions,
            &context_user_contexts,
            &context_top_level_contexts,
        );
        assert_eq!(replayed.len(), 1);
        assert_eq!(store.events.len(), 1);

        let replayed_again = store.replay_matching_events(
            &subscriptions,
            &context_user_contexts,
            &context_top_level_contexts,
        );
        assert!(replayed_again.is_empty());
        assert_eq!(store.events.len(), 1);
    }

    #[test]
    fn live_sent_buffered_event_is_not_replayed_to_same_channel() {
        let mut store = BidiBufferedEventStore::with_capacity(8);
        let event_id = store.buffer_event(log_entry("FRAME-1", "live"));
        store.mark_event_sent(event_id, None);

        let subscriptions = vec![log_subscription_for_context("FRAME-1")];
        let (context_user_contexts, context_top_level_contexts) = context_maps("FRAME-1");
        let replayed = store.replay_matching_events(
            &subscriptions,
            &context_user_contexts,
            &context_top_level_contexts,
        );

        assert!(replayed.is_empty());
        assert_eq!(store.events.len(), 1);
    }

    #[test]
    fn buffered_event_last_sent_is_tracked_per_channel() {
        let mut store = BidiBufferedEventStore::with_capacity(8);
        let event_id = store.buffer_event(log_entry("FRAME-1", "cached"));
        store.mark_event_sent(event_id, Some("alpha"));

        let subscriptions = vec![
            log_subscription_for_context_and_channel("FRAME-1", Some("alpha")),
            log_subscription_for_context_and_channel("FRAME-1", Some("beta")),
        ];
        let (context_user_contexts, context_top_level_contexts) = context_maps("FRAME-1");

        let replayed = store.replay_matching_events(
            &subscriptions,
            &context_user_contexts,
            &context_top_level_contexts,
        );

        assert_eq!(replayed.len(), 1);
        assert_eq!(replayed[0]["goog:channel"], json!("beta"));
        assert_eq!(store.events.len(), 1);
    }
}
