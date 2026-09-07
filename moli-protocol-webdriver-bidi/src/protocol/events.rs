use std::collections::{BTreeMap, BTreeSet};

use moli_protocol::devtools_runtime::{
    AutomationEvent, NavigationFrameEventKind, webdriver_bidi_navigation_id_from_loader_id,
};
use serde_json::{Value, json};

use crate::events::{
    bidi_log_level, bidi_log_method, bidi_log_text_from_remote_values,
    bidi_remote_value_from_cdp_remote_object, bidi_stack_trace_from_cdp, bidi_timestamp_millis,
    browsing_context_history_updated_event, browsing_context_navigation_event,
    log_entry_added_event, log_entry_added_generic_event_from_protocol_message,
    non_empty_json_string, runtime_realm_id_from_protocol_context,
};

use super::event_manager::BidiBufferedEventStore;
use super::types::BidiSubscription;

const MAX_BUFFERED_BIDI_LOG_EVENTS: usize = 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct BidiBrowsingContextEventState {
    next_navigation_serial: u64,
    contexts: BTreeMap<String, BidiBrowsingContextNavigation>,
    last_lifecycle_context: Option<String>,
    emitted_lifecycle_events: BTreeSet<BidiLifecycleEmissionKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BidiBrowsingContextNavigation {
    context: String,
    url: String,
    loader_id: Option<String>,
    navigation_id: Option<String>,
    serial: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct BidiLifecycleEmissionKey {
    method: String,
    context: String,
    serial: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BidiLogEventState {
    realms_by_execution_context: BTreeMap<i64, BidiRuntimeRealmInfo>,
    buffered_events: BidiBufferedEventStore,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct BidiDownloadEventState {
    downloads: BTreeMap<String, BidiDownloadInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BidiDownloadInfo {
    context: String,
    url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BidiRuntimeRealmInfo {
    realm: String,
    context: Option<String>,
}

impl Default for BidiLogEventState {
    fn default() -> Self {
        Self {
            realms_by_execution_context: BTreeMap::new(),
            buffered_events: BidiBufferedEventStore::with_capacity(MAX_BUFFERED_BIDI_LOG_EVENTS),
        }
    }
}

impl BidiBrowsingContextEventState {
    pub(super) fn event_from_automation_event(&mut self, event: &AutomationEvent) -> Option<Value> {
        match event {
            AutomationEvent::NavigationStarted(event) => {
                let navigation = self.record_navigation_started(
                    event.frame_id.as_str().to_owned(),
                    event.url.clone(),
                    event
                        .loader_id
                        .as_ref()
                        .map(|loader| loader.as_str().to_owned()),
                    event
                        .navigation_id
                        .as_ref()
                        .map(|navigation| navigation.as_str().to_owned()),
                );
                Some(browsing_context_navigation_event(
                    "browsingContext.navigationStarted",
                    &navigation.context,
                    &navigation.url,
                    navigation.navigation_id.as_deref(),
                ))
            }
            AutomationEvent::NavigationFrame(event) => match event.kind {
                NavigationFrameEventKind::StartedNavigating => {
                    let navigation = self.record_navigation_started(
                        event.frame_id.as_str().to_owned(),
                        event.url.clone(),
                        event
                            .loader_id
                            .as_ref()
                            .map(|loader| loader.as_str().to_owned()),
                        None,
                    );
                    Some(browsing_context_navigation_event(
                        "browsingContext.navigationStarted",
                        &navigation.context,
                        &navigation.url,
                        navigation.navigation_id.as_deref(),
                    ))
                }
                NavigationFrameEventKind::Navigated => {
                    self.record_navigation_committed(
                        event.frame_id.as_str().to_owned(),
                        event.url.clone(),
                        event
                            .loader_id
                            .as_ref()
                            .map(|loader| loader.as_str().to_owned()),
                    );
                    None
                }
                _ => None,
            },
            AutomationEvent::DomContentLoaded(event) => self.lifecycle_event(
                "browsingContext.domContentLoaded",
                Some(event.frame_id.as_str()),
                event.loader_id.as_ref().map(|loader| loader.as_str()),
            ),
            AutomationEvent::Load(event) => self.lifecycle_event(
                "browsingContext.load",
                Some(event.frame_id.as_str()),
                event.loader_id.as_ref().map(|loader| loader.as_str()),
            ),
            AutomationEvent::SameDocumentNavigation(event) => {
                match event.navigation_type.as_str() {
                    "fragment" => Some(browsing_context_navigation_event(
                        "browsingContext.fragmentNavigated",
                        event.frame_id.as_str(),
                        &event.url,
                        None,
                    )),
                    "historyApi" => Some(browsing_context_history_updated_event(
                        event.frame_id.as_str(),
                        &event.url,
                    )),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub(super) fn forget_context(&mut self, context: &str) {
        self.contexts.remove(context);
        self.emitted_lifecycle_events
            .retain(|key| key.context != context);
        if self.last_lifecycle_context.as_deref() == Some(context) {
            self.last_lifecycle_context = None;
        }
    }

    pub(super) fn event_from_protocol_message(&mut self, message: &Value) -> Option<Value> {
        match message.get("method").and_then(Value::as_str) {
            Some("Page.frameStartedNavigating") => {
                let params = &message["params"];
                let context = non_empty_json_string(&params["frameId"])?;
                let url = non_empty_json_string(&params["url"]).unwrap_or_default();
                let loader_id = non_empty_json_string(&params["loaderId"]);
                let navigation = self.record_navigation_started(context, url, loader_id, None);
                Some(browsing_context_navigation_event(
                    "browsingContext.navigationStarted",
                    &navigation.context,
                    &navigation.url,
                    navigation.navigation_id.as_deref(),
                ))
            }
            Some("Page.frameNavigated") => {
                let frame = &message["params"]["frame"];
                let context = non_empty_json_string(&frame["id"])?;
                let url = non_empty_json_string(&frame["url"]).unwrap_or_default();
                let loader_id = non_empty_json_string(&frame["loaderId"]);
                self.record_navigation_committed(context, url, loader_id);
                None
            }
            Some("Page.domContentEventFired") => {
                self.lifecycle_event("browsingContext.domContentLoaded", None, None)
            }
            Some("Page.loadEventFired") => self.lifecycle_event("browsingContext.load", None, None),
            Some("Page.lifecycleEvent") => {
                let params = &message["params"];
                let method = match params.get("name").and_then(Value::as_str) {
                    Some("DOMContentLoaded") => "browsingContext.domContentLoaded",
                    Some("load") => "browsingContext.load",
                    _ => return None,
                };
                self.lifecycle_event(
                    method,
                    non_empty_json_string(&params["frameId"]).as_deref(),
                    non_empty_json_string(&params["loaderId"]).as_deref(),
                )
            }
            Some("Page.navigatedWithinDocument") => {
                let params = &message["params"];
                let context = non_empty_json_string(&params["frameId"])?;
                let url = non_empty_json_string(&params["url"]).unwrap_or_default();
                match params.get("navigationType").and_then(Value::as_str) {
                    Some("fragment") => Some(browsing_context_navigation_event(
                        "browsingContext.fragmentNavigated",
                        &context,
                        &url,
                        None,
                    )),
                    Some("historyApi") => {
                        Some(browsing_context_history_updated_event(&context, &url))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn record_navigation_started(
        &mut self,
        context: String,
        url: String,
        loader_id: Option<String>,
        navigation_id: Option<String>,
    ) -> BidiBrowsingContextNavigation {
        self.next_navigation_serial = self.next_navigation_serial.saturating_add(1);
        let navigation_id = navigation_id.or_else(|| {
            loader_id
                .as_deref()
                .map(webdriver_bidi_navigation_id_from_loader_id)
                .map(|navigation| navigation.into_string())
        });
        let navigation = BidiBrowsingContextNavigation {
            context: context.clone(),
            url,
            loader_id,
            navigation_id,
            serial: self.next_navigation_serial,
        };
        self.contexts.insert(context.clone(), navigation.clone());
        self.last_lifecycle_context = Some(context);
        navigation
    }

    fn record_navigation_committed(
        &mut self,
        context: String,
        url: String,
        loader_id: Option<String>,
    ) -> BidiBrowsingContextNavigation {
        let navigation_id = loader_id
            .as_deref()
            .map(webdriver_bidi_navigation_id_from_loader_id)
            .map(|navigation| navigation.into_string());
        let mut navigation = match self.contexts.remove(&context) {
            Some(mut navigation) => {
                navigation.url = url;
                navigation.loader_id = loader_id;
                if navigation.navigation_id.is_none() {
                    navigation.navigation_id = navigation_id;
                }
                navigation
            }
            None => self.record_navigation_started(context.clone(), url, loader_id, navigation_id),
        };
        if navigation.serial == 0 {
            self.next_navigation_serial = self.next_navigation_serial.saturating_add(1);
            navigation.serial = self.next_navigation_serial;
        }
        self.contexts.insert(context.clone(), navigation.clone());
        self.last_lifecycle_context = Some(context);
        navigation
    }

    fn lifecycle_event(
        &mut self,
        method: &str,
        context: Option<&str>,
        loader_id: Option<&str>,
    ) -> Option<Value> {
        let navigation = self.navigation_for_lifecycle(context, loader_id)?;
        let key = BidiLifecycleEmissionKey {
            method: method.to_owned(),
            context: navigation.context.clone(),
            serial: navigation.serial,
        };
        if !self.emitted_lifecycle_events.insert(key) {
            return None;
        }
        Some(browsing_context_navigation_event(
            method,
            &navigation.context,
            &navigation.url,
            navigation.navigation_id.as_deref(),
        ))
    }

    fn navigation_for_lifecycle(
        &self,
        context: Option<&str>,
        loader_id: Option<&str>,
    ) -> Option<BidiBrowsingContextNavigation> {
        let navigation = match context {
            Some(context) => self.contexts.get(context),
            None => self
                .last_lifecycle_context
                .as_deref()
                .and_then(|context| self.contexts.get(context))
                .or_else(|| {
                    (self.contexts.len() == 1)
                        .then(|| self.contexts.values().next())
                        .flatten()
                }),
        }?;
        if let Some(loader_id) = loader_id
            && navigation
                .loader_id
                .as_deref()
                .is_some_and(|known| known != loader_id)
        {
            return None;
        }
        Some(navigation.clone())
    }
}

impl BidiDownloadEventState {
    pub(super) fn event_from_automation_event(&mut self, event: &AutomationEvent) -> Option<Value> {
        match event {
            AutomationEvent::BrowserDownloadWillBegin(event) => self
                .download_will_begin_event_from_parts(
                    event.guid.clone(),
                    event.frame_id.as_str().to_owned(),
                    event.url.clone(),
                    event.suggested_filename.clone(),
                ),
            AutomationEvent::BrowserDownloadProgress(event) => self.download_end_event_from_parts(
                &event.guid,
                &event.state,
                event.file_path.as_deref(),
            ),
            _ => None,
        }
    }

    pub(super) fn event_from_protocol_message(&mut self, message: &Value) -> Option<Value> {
        match message.get("method").and_then(Value::as_str) {
            Some("Browser.downloadWillBegin") => self.download_will_begin_event(message),
            Some("Browser.downloadProgress") => self.download_end_event(message),
            _ => None,
        }
    }

    pub(super) fn forget_context(&mut self, context: &str) -> Vec<Value> {
        let guids = self
            .downloads
            .iter()
            .filter_map(|(guid, download)| (download.context == context).then_some(guid.clone()))
            .collect::<Vec<_>>();
        guids
            .into_iter()
            .filter_map(|guid| self.downloads.remove(&guid))
            .map(|download| Self::download_end_event_for_download(download, "canceled", None))
            .collect()
    }

    fn download_will_begin_event(&mut self, message: &Value) -> Option<Value> {
        let params = &message["params"];
        let guid = non_empty_json_string(&params["guid"])?;
        let context = non_empty_json_string(&params["frameId"])?;
        let url = non_empty_json_string(&params["url"]).unwrap_or_default();
        let suggested_filename =
            non_empty_json_string(&params["suggestedFilename"]).unwrap_or_default();
        self.download_will_begin_event_from_parts(guid, context, url, suggested_filename)
    }

    fn download_will_begin_event_from_parts(
        &mut self,
        guid: String,
        context: String,
        url: String,
        suggested_filename: String,
    ) -> Option<Value> {
        let duplicate_guid = self.downloads.contains_key(&guid);
        debug_assert!(!duplicate_guid, "duplicate download guid: {guid}");
        if duplicate_guid {
            return None;
        }
        self.downloads.insert(
            guid,
            BidiDownloadInfo {
                context: context.clone(),
                url: url.clone(),
            },
        );
        Some(json!({
            "type": "event",
            "method": "browsingContext.downloadWillBegin",
            "params": {
                "context": context,
                "navigation": Value::Null,
                "suggestedFilename": suggested_filename,
                "timestamp": bidi_timestamp_millis(),
                "url": url,
            },
        }))
    }

    fn download_end_event(&mut self, message: &Value) -> Option<Value> {
        let params = &message["params"];
        let state = params.get("state").and_then(Value::as_str)?;
        let guid = non_empty_json_string(&params["guid"])?;
        let file_path = (state == "completed")
            .then(|| non_empty_json_string(&params["filePath"]))
            .flatten();
        self.download_end_event_from_parts(&guid, state, file_path.as_deref())
    }

    fn download_end_event_from_parts(
        &mut self,
        guid: &str,
        state: &str,
        file_path: Option<&str>,
    ) -> Option<Value> {
        let status = match state {
            "completed" => "complete",
            "canceled" => "canceled",
            _ => return None,
        };
        let download = self.downloads.remove(guid)?;
        Some(Self::download_end_event_for_download(
            download,
            status,
            file_path.map(str::to_owned),
        ))
    }

    fn download_end_event_for_download(
        download: BidiDownloadInfo,
        status: &str,
        file_path: Option<String>,
    ) -> Value {
        let mut event = json!({
            "type": "event",
            "method": "browsingContext.downloadEnd",
            "params": {
                "context": download.context,
                "navigation": Value::Null,
                "status": status,
                "timestamp": bidi_timestamp_millis(),
                "url": download.url,
            },
        });
        if status == "complete"
            && let Some(file_path) = file_path
            && let Some(object) = event["params"].as_object_mut()
        {
            object.insert("filepath".to_owned(), json!(file_path));
        }
        event
    }
}

impl BidiLogEventState {
    pub(super) fn forget_context(&mut self, context: &str) {
        self.realms_by_execution_context
            .retain(|_, realm| realm.context.as_deref() != Some(context));
        self.buffered_events.forget_context(context);
    }

    pub(super) fn record_runtime_realm_from_protocol_message(
        &mut self,
        message: &Value,
        owner_context: Option<&str>,
    ) {
        match message.get("method").and_then(Value::as_str) {
            Some("Runtime.executionContextCreated") => {
                let context = &message["params"]["context"];
                let Some(execution_context_id) = context.get("id").and_then(Value::as_i64) else {
                    return;
                };
                let aux_data = &context["auxData"];
                let realm =
                    runtime_realm_id_from_protocol_context(context, aux_data, owner_context)
                        .unwrap_or_else(|| execution_context_id.to_string());
                let browsing_context = owner_context
                    .map(str::to_owned)
                    .or_else(|| context["auxData"]["frameId"].as_str().map(str::to_owned));
                self.realms_by_execution_context.insert(
                    execution_context_id,
                    BidiRuntimeRealmInfo {
                        realm,
                        context: browsing_context,
                    },
                );
            }
            Some("Runtime.executionContextDestroyed") => {
                if let Some(execution_context_id) = message["params"]["executionContextId"].as_i64()
                {
                    self.realms_by_execution_context
                        .remove(&execution_context_id);
                }
            }
            Some("Runtime.executionContextsCleared") => {
                self.realms_by_execution_context.clear();
            }
            _ => {}
        }
    }

    pub(super) fn realm_for_execution_context_destroyed_message(
        &self,
        message: &Value,
    ) -> Option<String> {
        if message.get("method").and_then(Value::as_str)
            != Some("Runtime.executionContextDestroyed")
        {
            return None;
        }
        let execution_context_id = message["params"]["executionContextId"].as_i64()?;
        self.realms_by_execution_context
            .get(&execution_context_id)
            .map(|realm| realm.realm.clone())
    }

    pub(super) fn event_from_protocol_message(
        &self,
        message: &Value,
        owner_context: Option<&str>,
    ) -> Option<Value> {
        match message.get("method").and_then(Value::as_str) {
            Some("Runtime.consoleAPICalled") => {
                self.console_event_from_protocol_message(message, owner_context)
            }
            Some("Runtime.exceptionThrown") => {
                self.javascript_event_from_protocol_message(message, owner_context)
            }
            Some("Log.entryAdded") => {
                log_entry_added_generic_event_from_protocol_message(message, owner_context)
            }
            _ => None,
        }
    }

    pub(super) fn runtime_source_from_automation_event(
        &self,
        event: &AutomationEvent,
        owner_context: Option<&str>,
    ) -> Option<Value> {
        let execution_context_id = match event {
            AutomationEvent::RuntimeConsoleApiCalled(event) => event.execution_context_id,
            AutomationEvent::ScriptException(event) => event.execution_context_id,
            _ => return None,
        };
        Some(self.source_for_execution_context(execution_context_id, owner_context))
    }

    fn console_event_from_protocol_message(
        &self,
        message: &Value,
        owner_context: Option<&str>,
    ) -> Option<Value> {
        let params = &message["params"];
        let console_type = params.get("type").and_then(Value::as_str).unwrap_or("log");
        let args = params
            .get("args")
            .and_then(Value::as_array)
            .map(|args| {
                args.iter()
                    .map(bidi_remote_value_from_cdp_remote_object)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let source = self.source_for_execution_context(
            params.get("executionContextId").and_then(Value::as_i64),
            owner_context,
        );
        let text = bidi_log_text_from_remote_values(console_type, &args);
        let mut entry = json!({
            "type": "console",
            "method": bidi_log_method(console_type),
            "level": bidi_log_level(console_type),
            "source": source,
            "text": text,
            "timestamp": bidi_timestamp_millis(),
            "args": args,
        });
        if let Some(stack_trace) = bidi_stack_trace_from_cdp(params.get("stackTrace"))
            && let Some(entry) = entry.as_object_mut()
        {
            entry.insert("stackTrace".to_owned(), stack_trace);
        }
        Some(log_entry_added_event(entry))
    }

    fn javascript_event_from_protocol_message(
        &self,
        message: &Value,
        owner_context: Option<&str>,
    ) -> Option<Value> {
        let params = &message["params"];
        let details = &params["exceptionDetails"];
        let source = self.source_for_execution_context(
            details.get("executionContextId").and_then(Value::as_i64),
            owner_context,
        );
        let text = details["exception"]["description"]
            .as_str()
            .or_else(|| details.get("text").and_then(Value::as_str))
            .unwrap_or_default();
        let mut entry = json!({
            "type": "javascript",
            "level": "error",
            "source": source,
            "text": text,
            "timestamp": bidi_timestamp_millis(),
        });
        if let Some(stack_trace) = bidi_stack_trace_from_cdp(details.get("stackTrace"))
            && let Some(entry) = entry.as_object_mut()
        {
            entry.insert("stackTrace".to_owned(), stack_trace);
        }
        Some(log_entry_added_event(entry))
    }

    fn source_for_execution_context(
        &self,
        execution_context_id: Option<i64>,
        owner_context: Option<&str>,
    ) -> Value {
        let realm = execution_context_id
            .and_then(|id| self.realms_by_execution_context.get(&id))
            .cloned();
        let realm_id = realm.as_ref().map(|realm| realm.realm.clone()).or_else(|| {
            execution_context_id
                .filter(|execution_context_id| *execution_context_id > 0)
                .map(|id| id.to_string())
        });
        let context = owner_context
            .map(str::to_owned)
            .or_else(|| realm.as_ref().and_then(|realm| realm.context.clone()));
        let mut source = json!({});
        if let Some(realm_id) = realm_id {
            source["realm"] = json!(realm_id);
        } else if context.is_none() {
            source["realm"] = json!("unknown");
        }
        if let Some(context) = context
            && let Some(source) = source.as_object_mut()
        {
            source.insert("context".to_owned(), json!(context));
        }
        source
    }

    pub(super) fn buffer_event(&mut self, event: Value) -> u64 {
        self.buffered_events.buffer_event(event)
    }

    pub(super) fn mark_buffered_event_sent(&mut self, id: u64, channel: Option<&str>) {
        self.buffered_events.mark_event_sent(id, channel);
    }

    pub(super) fn replay_matching_buffered_events(
        &mut self,
        subscriptions: &[BidiSubscription],
        context_user_contexts: &BTreeMap<String, String>,
        context_top_level_contexts: &BTreeMap<String, String>,
    ) -> Vec<Value> {
        self.buffered_events.replay_matching_events(
            subscriptions,
            context_user_contexts,
            context_top_level_contexts,
        )
    }
}
