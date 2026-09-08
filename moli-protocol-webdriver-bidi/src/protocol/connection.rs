use std::collections::{BTreeMap, BTreeSet};

use moli_protocol::devtools_runtime::AutomationEvent;
use serde_json::{Value, json};

use crate::commands::{
    devtools_command_from_bidi_command, optional_non_empty_string_array,
    required_non_empty_string_array, required_supported_event_array, unroll_bidi_events,
};
use crate::events::{
    bidi_event_from_automation_event, bidi_event_from_protocol_message_with_prompt_handler,
    input_file_dialog_opened_event, user_prompt_opened_event,
};
use crate::network::BidiNetworkEventState;
use crate::responses::{error_response, success_response};
use crate::user_context::DEFAULT_BIDI_USER_CONTEXT;

use super::event_manager::{
    BidiEventSourceHookPlan, BidiEventSourceHookScope, BidiEventSourceOwnership,
    is_bidi_download_event_name, is_bidi_network_event_name, is_bidi_runtime_source_event_name,
};
use super::events::{BidiBrowsingContextEventState, BidiDownloadEventState, BidiLogEventState};
use super::types::{
    BidiCommand, BidiCommandOutcome, BidiDevToolsCommandContext, BidiDevToolsCommandDispatch,
    BidiError, BidiErrorCode, BidiInputCommand, BidiInputCommandDispatch, BidiSessionRegistry,
    BidiSubscription, bidi_event_subscribed_channels, bidi_message_with_channel,
    is_devtools_command, is_event_subscribed_by, is_known_session_command, parse_bidi_command,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BidiConnectionState {
    session_id: Option<String>,
    web_socket_url: Option<String>,
    next_subscription_number: u64,
    subscriptions: Vec<BidiSubscription>,
    browsing_context_events: BidiBrowsingContextEventState,
    download_events: BidiDownloadEventState,
    log_events: BidiLogEventState,
    network_events: BidiNetworkEventState,
    event_source_ownership: BidiEventSourceOwnership,
    pending_release_event_source_hook_plan: Option<BidiEventSourceHookPlan>,
    pending_unsubscribe_event_source_hook_plan: Option<BidiEventSourceHookPlan>,
    context_user_contexts: BTreeMap<String, String>,
    context_top_level_contexts: BTreeMap<String, String>,
    known_user_contexts: BTreeSet<String>,
    unhandled_prompt_behavior: BidiUnhandledPromptBehavior,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BidiUnhandledPromptBehavior {
    default_handler: String,
    alert: Option<String>,
    before_unload: Option<String>,
    confirm: Option<String>,
    file: Option<String>,
    prompt: Option<String>,
}

impl Default for BidiUnhandledPromptBehavior {
    fn default() -> Self {
        Self {
            default_handler: "dismiss".to_owned(),
            alert: None,
            before_unload: None,
            confirm: None,
            file: Some("ignore".to_owned()),
            prompt: None,
        }
    }
}

impl BidiUnhandledPromptBehavior {
    fn from_capability(value: Option<&Value>) -> Result<Self, BidiError> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        if let Some(handler) = value.as_str() {
            return Ok(Self {
                default_handler: normalized_prompt_handler(handler)?.to_owned(),
                file: None,
                ..Self::default()
            });
        }
        let Some(behavior) = value.as_object() else {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "unhandledPromptBehavior must be a string or object",
            ));
        };
        let mut result = Self::default();
        let mut has_default = false;
        let mut has_file = false;
        for (key, value) in behavior {
            let Some(value) = value.as_str() else {
                return Err(BidiError::new(
                    BidiErrorCode::InvalidArgument,
                    "unhandledPromptBehavior handler must be a string",
                ));
            };
            let handler = normalized_prompt_handler(value)?.to_owned();
            match key.as_str() {
                "default" => {
                    result.default_handler = handler;
                    has_default = true;
                }
                "alert" => result.alert = Some(handler),
                "beforeUnload" => result.before_unload = Some(handler),
                "confirm" => result.confirm = Some(handler),
                "file" => {
                    result.file = Some(handler);
                    has_file = true;
                }
                "prompt" => result.prompt = Some(handler),
                _ => {}
            }
        }
        if has_default && !has_file {
            result.file = None;
        }
        Ok(result)
    }

    fn handler_for_prompt_type(&self, prompt_type: &str) -> &str {
        let specific = match prompt_type {
            "alert" => self.alert.as_deref(),
            "beforeunload" | "beforeUnload" => self.before_unload.as_deref(),
            "confirm" => self.confirm.as_deref(),
            "file" => self.file.as_deref(),
            "prompt" => self.prompt.as_deref(),
            _ => None,
        };
        specific.unwrap_or(self.default_handler.as_str())
    }
}

fn normalized_prompt_handler(value: &str) -> Result<&'static str, BidiError> {
    match value {
        "accept" | "accept and notify" => Ok("accept"),
        "dismiss" | "dismiss and notify" => Ok("dismiss"),
        "ignore" => Ok("ignore"),
        _ => Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "unhandledPromptBehavior handler value is not supported",
        )),
    }
}

impl Default for BidiConnectionState {
    fn default() -> Self {
        Self {
            session_id: None,
            web_socket_url: None,
            next_subscription_number: 0,
            subscriptions: Vec::new(),
            browsing_context_events: BidiBrowsingContextEventState::default(),
            download_events: BidiDownloadEventState::default(),
            log_events: BidiLogEventState::default(),
            network_events: BidiNetworkEventState::default(),
            event_source_ownership: BidiEventSourceOwnership::default(),
            pending_release_event_source_hook_plan: None,
            pending_unsubscribe_event_source_hook_plan: None,
            context_user_contexts: BTreeMap::new(),
            context_top_level_contexts: BTreeMap::new(),
            known_user_contexts: BTreeSet::from([DEFAULT_BIDI_USER_CONTEXT.to_owned()]),
            unhandled_prompt_behavior: BidiUnhandledPromptBehavior::default(),
        }
    }
}

impl BidiConnectionState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_web_socket_url(web_socket_url: impl Into<String>) -> Self {
        Self {
            web_socket_url: Some(web_socket_url.into()),
            ..Self::default()
        }
    }

    pub fn file_prompt_handler_for_script_commands(&self) -> Option<&str> {
        let handler = self
            .unhandled_prompt_behavior
            .handler_for_prompt_type("file");
        match handler {
            "accept" | "dismiss" => Some(handler),
            _ => None,
        }
    }

    pub fn set_file_prompt_handler_for_script_commands(&mut self, handler: Option<&str>) {
        self.unhandled_prompt_behavior.file = Some(handler.unwrap_or("ignore").to_owned());
    }

    pub fn attach_existing_session(
        &mut self,
        session_id: impl Into<String>,
        registry: &mut BidiSessionRegistry,
    ) -> bool {
        if self.session_id.is_some() {
            return false;
        }
        let session_id = session_id.into();
        if !registry.register_session(session_id.clone()) {
            return false;
        }
        self.session_id = Some(session_id);
        true
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn record_known_bidi_subscription_context(
        &mut self,
        context: &str,
        top_level_context: &str,
        user_context: Option<&str>,
    ) {
        self.record_bidi_context_mapping(context, Some(top_level_context), user_context);
    }

    pub fn record_known_bidi_user_context(&mut self, user_context: &str) {
        self.known_user_contexts.insert(user_context.to_owned());
    }

    pub fn handle_message(&mut self, message: Value) -> BidiCommandOutcome {
        let mut registry = BidiSessionRegistry::new();
        self.handle_message_with_session_registry(message, &mut registry)
    }

    pub fn handle_message_with_session_registry(
        &mut self,
        message: Value,
        registry: &mut BidiSessionRegistry,
    ) -> BidiCommandOutcome {
        let id = message.get("id").and_then(Value::as_u64);
        let mut outcome = match parse_bidi_command(message) {
            Ok(command) => self.dispatch_command_outcome_with_session_registry(command, registry),
            Err(error) => BidiCommandOutcome::respond(
                error_response(id, error.code, &error.message),
                self.session_id.clone(),
            ),
        };
        outcome.session_id = self.session_id.clone();
        outcome
    }

    pub fn dispatch_command(&mut self, command: BidiCommand) -> Value {
        let mut registry = BidiSessionRegistry::new();
        self.dispatch_command_with_session_registry(command, &mut registry)
    }

    pub fn dispatch_command_with_session_registry(
        &mut self,
        command: BidiCommand,
        registry: &mut BidiSessionRegistry,
    ) -> Value {
        self.dispatch_command_outcome_with_session_registry(command, registry)
            .response
    }

    pub fn dispatch_command_outcome_with_session_registry(
        &mut self,
        command: BidiCommand,
        registry: &mut BidiSessionRegistry,
    ) -> BidiCommandOutcome {
        let channel = command.channel.clone();
        let mut outcome = match command.method.as_str() {
            "session.status" => BidiCommandOutcome::respond(
                self.session_status(command.id),
                self.session_id.clone(),
            ),
            "session.new" => BidiCommandOutcome::respond(
                self.session_new(command.id, command.params, registry),
                self.session_id.clone(),
            ),
            "session.end" => self.session_end_outcome(command.id, registry),
            "browser.close" => self.browser_close_outcome(command.id, command.params, registry),
            method if self.session_id.is_none() && is_known_session_command(method) => {
                BidiCommandOutcome::respond(
                    error_response(
                        Some(command.id),
                        BidiErrorCode::InvalidSessionId,
                        "session not found",
                    ),
                    self.session_id.clone(),
                )
            }
            "session.subscribe" => BidiCommandOutcome::respond(
                self.session_subscribe(command.id, command.params, channel.clone()),
                self.session_id.clone(),
            ),
            "session.unsubscribe" => BidiCommandOutcome::respond(
                self.session_unsubscribe(command.id, command.params, channel.clone()),
                self.session_id.clone(),
            ),
            "input.performActions" | "input.releaseActions" | "input.setFiles" => {
                self.dispatch_input_command(command)
            }
            method if is_devtools_command(method) => self.dispatch_devtools_command(command),
            method if is_known_session_command(method) => BidiCommandOutcome::respond(
                error_response(
                    Some(command.id),
                    BidiErrorCode::UnsupportedOperation,
                    "BiDi command is known but not implemented yet",
                ),
                self.session_id.clone(),
            ),
            method => BidiCommandOutcome::respond(
                error_response(Some(command.id), BidiErrorCode::UnknownCommand, method),
                self.session_id.clone(),
            ),
        };
        outcome.channel = channel;
        outcome.response = bidi_message_with_channel(outcome.response, outcome.channel.as_deref());
        outcome
    }

    fn dispatch_devtools_command(&self, command: BidiCommand) -> BidiCommandOutcome {
        let Some(session_id) = self.session_id.as_deref() else {
            return BidiCommandOutcome::respond(
                error_response(
                    Some(command.id),
                    BidiErrorCode::InvalidSessionId,
                    "session not found",
                ),
                self.session_id.clone(),
            );
        };
        let context = BidiDevToolsCommandContext::new(session_id);
        match devtools_command_from_bidi_command(&command, &context) {
            Ok(devtools_command) => BidiCommandOutcome {
                response: error_response(
                    Some(command.id),
                    BidiErrorCode::UnsupportedOperation,
                    "BiDi DevTools command execution is not wired yet",
                ),
                session_id: self.session_id.clone(),
                channel: None,
                close_connection: false,
                devtools_command: Some(BidiDevToolsCommandDispatch {
                    id: command.id,
                    session_id: session_id.to_owned(),
                    command: devtools_command,
                }),
                input_command: None,
            },
            Err(error) => BidiCommandOutcome::respond(
                error_response(Some(command.id), error.code, &error.message),
                self.session_id.clone(),
            ),
        }
    }

    fn dispatch_input_command(&self, command: BidiCommand) -> BidiCommandOutcome {
        let Some(session_id) = self.session_id.as_deref() else {
            return BidiCommandOutcome::respond(
                error_response(
                    Some(command.id),
                    BidiErrorCode::InvalidSessionId,
                    "session not found",
                ),
                self.session_id.clone(),
            );
        };
        let context = match command.params.get("context").and_then(Value::as_str) {
            Some(context) if !context.is_empty() => context.to_owned(),
            _ => {
                return BidiCommandOutcome::respond(
                    error_response(
                        Some(command.id),
                        BidiErrorCode::InvalidArgument,
                        "context must be a string",
                    ),
                    self.session_id.clone(),
                );
            }
        };
        let input_command = match command.method.as_str() {
            "input.performActions" => BidiInputCommand::PerformActions {
                params: command.params,
            },
            "input.releaseActions" => BidiInputCommand::ReleaseActions,
            "input.setFiles" => BidiInputCommand::SetFiles {
                params: command.params,
            },
            method => {
                return BidiCommandOutcome::respond(
                    error_response(Some(command.id), BidiErrorCode::UnknownCommand, method),
                    self.session_id.clone(),
                );
            }
        };
        BidiCommandOutcome {
            response: error_response(
                Some(command.id),
                BidiErrorCode::UnsupportedOperation,
                "BiDi input command requires server execution",
            ),
            session_id: self.session_id.clone(),
            channel: None,
            close_connection: false,
            devtools_command: None,
            input_command: Some(BidiInputCommandDispatch {
                id: command.id,
                session_id: session_id.to_owned(),
                context,
                command: input_command,
            }),
        }
    }

    pub fn release_session(&mut self, registry: &mut BidiSessionRegistry) {
        self.pending_release_event_source_hook_plan = Some(self.release_event_source_hook_plan());
        self.event_source_ownership = BidiEventSourceOwnership::default();
        if let Some(session_id) = self.session_id.take() {
            registry.release_session(&session_id);
        }
        self.subscriptions.clear();
        self.browsing_context_events = BidiBrowsingContextEventState::default();
        self.log_events = BidiLogEventState::default();
        self.network_events = BidiNetworkEventState::default();
        self.context_user_contexts.clear();
        self.context_top_level_contexts.clear();
        self.known_user_contexts.clear();
        self.known_user_contexts
            .insert(DEFAULT_BIDI_USER_CONTEXT.to_owned());
        self.unhandled_prompt_behavior = BidiUnhandledPromptBehavior::default();
    }

    fn session_status(&self, id: u64) -> Value {
        let ready = self.session_id.is_none();
        success_response(
            id,
            json!({
                "ready": ready,
                "message": if ready {
                    "Moli ready for new sessions."
                } else {
                    "already connected"
                },
            }),
        )
    }

    fn session_new(&mut self, id: u64, params: Value, registry: &mut BidiSessionRegistry) -> Value {
        if self.session_id.is_some() {
            return error_response(
                Some(id),
                BidiErrorCode::SessionNotCreated,
                "session already exists",
            );
        }
        let Some(params) = params.as_object() else {
            return error_response(
                Some(id),
                BidiErrorCode::InvalidArgument,
                "session.new params must be an object",
            );
        };
        let capabilities = params
            .get("capabilities")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !capabilities.is_object() {
            return error_response(
                Some(id),
                BidiErrorCode::InvalidArgument,
                "session.new capabilities must be an object",
            );
        }
        let unhandled_prompt_behavior = match BidiUnhandledPromptBehavior::from_capability(
            capabilities.get("unhandledPromptBehavior"),
        ) {
            Ok(behavior) => behavior,
            Err(error) => return error_response(Some(id), error.code, &error.message),
        };

        let session_id = registry.create_session();
        self.session_id = Some(session_id.clone());
        self.unhandled_prompt_behavior = unhandled_prompt_behavior;
        let mut returned_capabilities = capabilities;
        if let Some(web_socket_url) = self.web_socket_url.as_deref()
            && let Some(capabilities) = returned_capabilities.as_object_mut()
        {
            capabilities.insert("webSocketUrl".to_owned(), json!(web_socket_url));
        }
        success_response(
            id,
            json!({
                "sessionId": session_id,
                "capabilities": returned_capabilities,
            }),
        )
    }

    fn session_end(&mut self, id: u64, registry: &mut BidiSessionRegistry) -> Value {
        if self.session_id.is_none() {
            return error_response(
                Some(id),
                BidiErrorCode::InvalidSessionId,
                "session not found",
            );
        }
        self.release_session(registry);
        success_response(id, json!({}))
    }

    fn session_end_outcome(
        &mut self,
        id: u64,
        registry: &mut BidiSessionRegistry,
    ) -> BidiCommandOutcome {
        let response = self.session_end(id, registry);
        let close_connection = response["type"] == json!("success");
        let mut outcome = BidiCommandOutcome::respond(response, self.session_id.clone());
        outcome.close_connection = close_connection;
        outcome
    }

    fn browser_close_outcome(
        &mut self,
        id: u64,
        params: Value,
        registry: &mut BidiSessionRegistry,
    ) -> BidiCommandOutcome {
        if !params.as_object().is_some_and(|params| params.is_empty()) {
            return BidiCommandOutcome::respond(
                error_response(
                    Some(id),
                    BidiErrorCode::InvalidArgument,
                    "browser.close params must be empty",
                ),
                self.session_id.clone(),
            );
        }
        self.session_end_outcome(id, registry)
    }

    fn session_subscribe(&mut self, id: u64, params: Value, channel: Option<String>) -> Value {
        let events = match required_supported_event_array(&params, "events") {
            Ok(events) => events,
            Err(error) => return error_response(Some(id), error.code, &error.message),
        };
        let contexts = match optional_non_empty_string_array(&params, "contexts") {
            Ok(contexts) => contexts.unwrap_or_default(),
            Err(error) => return error_response(Some(id), error.code, &error.message),
        };
        let user_contexts = match optional_non_empty_string_array(&params, "userContexts") {
            Ok(user_contexts) => user_contexts.unwrap_or_default(),
            Err(error) => return error_response(Some(id), error.code, &error.message),
        };
        if !contexts.is_empty() && !user_contexts.is_empty() {
            return error_response(
                Some(id),
                BidiErrorCode::InvalidArgument,
                "contexts and userContexts cannot both be specified",
            );
        }
        if let Err(error) = self.validate_known_bidi_contexts(&contexts) {
            return error_response(Some(id), error.code, &error.message);
        }
        if let Err(error) = self.validate_known_bidi_user_contexts(&user_contexts) {
            return error_response(Some(id), error.code, &error.message);
        }
        let subscription_id = self.next_subscription_id();
        self.subscriptions.push(BidiSubscription {
            id: subscription_id.clone(),
            events: unroll_bidi_events(&events).collect(),
            contexts: contexts.into_iter().collect(),
            user_contexts: user_contexts.into_iter().collect(),
            channel,
        });
        success_response(id, json!({ "subscription": subscription_id }))
    }

    fn session_unsubscribe(&mut self, id: u64, params: Value, channel: Option<String>) -> Value {
        let has_events = params.get("events").is_some();
        let has_subscriptions = params.get("subscriptions").is_some();
        if !has_events && !has_subscriptions {
            return error_response(
                Some(id),
                BidiErrorCode::InvalidArgument,
                "either events or subscriptions must be specified",
            );
        }
        let result = if has_subscriptions {
            self.unsubscribe_by_subscription_ids(&params)
        } else {
            self.unsubscribe_by_events(&params, channel)
        };
        match result {
            Ok(removed_log_subscriptions) => {
                self.pending_unsubscribe_event_source_hook_plan =
                    Some(self.unsubscribe_hook_plan_preserving_log_buffer_sources(
                        &removed_log_subscriptions,
                    ));
                success_response(id, json!({}))
            }
            Err(error) => error_response(Some(id), error.code, &error.message),
        }
    }

    fn next_subscription_id(&mut self) -> String {
        self.next_subscription_number = self.next_subscription_number.saturating_add(1);
        format!(
            "00000000-0000-4000-8000-{:012x}",
            self.next_subscription_number
        )
    }

    fn unsubscribe_by_subscription_ids(
        &mut self,
        params: &Value,
    ) -> Result<Vec<BidiSubscription>, BidiError> {
        let subscription_ids = required_non_empty_string_array(params, "subscriptions")?;
        let requested: BTreeSet<String> = subscription_ids.into_iter().collect();
        if requested.len()
            != requested
                .iter()
                .filter(|id| {
                    self.subscriptions
                        .iter()
                        .any(|subscription| &subscription.id == *id)
                })
                .count()
        {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "No subscription found",
            ));
        }
        let removed_log_subscriptions = self
            .subscriptions
            .iter()
            .filter(|subscription| {
                requested.contains(&subscription.id)
                    && subscription.events.contains("log.entryAdded")
            })
            .cloned()
            .collect();
        self.subscriptions
            .retain(|subscription| !requested.contains(&subscription.id));
        Ok(removed_log_subscriptions)
    }

    fn unsubscribe_by_events(
        &mut self,
        params: &Value,
        channel: Option<String>,
    ) -> Result<Vec<BidiSubscription>, BidiError> {
        let events: BTreeSet<String> =
            unroll_bidi_events(&required_supported_event_array(params, "events")?).collect();
        let mut subscriptions = self.subscriptions.clone();
        let mut matched = BTreeSet::new();
        let mut removed_log_subscriptions = Vec::new();
        for subscription in &mut subscriptions {
            if subscription.channel != channel {
                continue;
            }
            if !subscription.contexts.is_empty() || !subscription.user_contexts.is_empty() {
                continue;
            }
            let mut removed_log_entry_added = false;
            subscription.events.retain(|event| {
                if events.contains(event) {
                    matched.insert(event.clone());
                    removed_log_entry_added |= event == "log.entryAdded";
                    false
                } else {
                    true
                }
            });
            if removed_log_entry_added {
                let mut removed = subscription.clone();
                removed.events = BTreeSet::from(["log.entryAdded".to_owned()]);
                removed_log_subscriptions.push(removed);
            }
        }
        if matched != events {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "No subscription found",
            ));
        }
        subscriptions.retain(|subscription| !subscription.events.is_empty());
        self.subscriptions = subscriptions;
        Ok(removed_log_subscriptions)
    }

    pub fn subscribed_bidi_events_from_protocol_messages<'a>(
        &mut self,
        messages: impl IntoIterator<Item = &'a Value>,
    ) -> Vec<Value> {
        self.subscribed_bidi_events_from_protocol_messages_with_context(messages, None)
    }

    pub fn subscribed_bidi_events_from_protocol_messages_with_context<'a>(
        &mut self,
        messages: impl IntoIterator<Item = &'a Value>,
        owner_context: Option<&str>,
    ) -> Vec<Value> {
        let mut events = Vec::new();
        for message in messages {
            let destroyed_realm = self
                .log_events
                .realm_for_execution_context_destroyed_message(message);
            self.record_protocol_message_state(message, owner_context);
            let prompt_handler = self.prompt_handler_for_protocol_message(message);
            if let Some(event) = bidi_event_from_protocol_message_with_prompt_handler(
                message,
                prompt_handler,
                owner_context,
                destroyed_realm.as_deref(),
            ) {
                self.record_bidi_context_user_context(&event);
                events.extend(
                    self.subscribed_bidi_event_messages_with_context(&event, owner_context),
                );
                events.extend(self.forget_destroyed_bidi_context(&event));
            }
            if let Some(event) = self
                .browsing_context_events
                .event_from_protocol_message(message)
            {
                events.extend(self.subscribed_bidi_event_messages(&event));
            }
            if let Some(event) = self.download_events.event_from_protocol_message(message) {
                events.extend(self.subscribed_bidi_event_messages(&event));
            }
            for event in self.network_events.events_from_protocol_message(message) {
                events.extend(self.subscribed_bidi_event_messages(&event));
            }
            if let Some(event) = self
                .log_events
                .event_from_protocol_message(message, owner_context)
            {
                let buffered_event_id = self.log_events.buffer_event(event.clone());
                for channel in self.subscribed_bidi_event_channels(&event) {
                    self.log_events
                        .mark_buffered_event_sent(buffered_event_id, channel.as_deref());
                    events.push(bidi_message_with_channel(event.clone(), channel.as_deref()));
                }
            }
        }
        events
    }

    pub fn subscribed_bidi_events_from_automation_events<'a>(
        &mut self,
        events: impl IntoIterator<Item = &'a AutomationEvent>,
    ) -> Vec<Value> {
        self.subscribed_bidi_events_from_automation_events_with_context(events, None)
    }

    pub fn subscribed_bidi_events_from_automation_events_with_context<'a>(
        &mut self,
        events: impl IntoIterator<Item = &'a AutomationEvent>,
        fallback_owner_context: Option<&str>,
    ) -> Vec<Value> {
        let mut bidi_events = Vec::new();
        for automation_event in events {
            for event in self
                .network_events
                .events_from_automation_event(automation_event)
            {
                bidi_events.extend(self.subscribed_bidi_event_messages(&event));
            }
            if let Some(event) = self
                .browsing_context_events
                .event_from_automation_event(automation_event)
            {
                bidi_events.extend(self.subscribed_bidi_event_messages(&event));
            }
            if let Some(event) = self
                .download_events
                .event_from_automation_event(automation_event)
            {
                bidi_events.extend(self.subscribed_bidi_event_messages(&event));
            }
            let generic_event = match automation_event {
                AutomationEvent::NavigationFrame(_)
                | AutomationEvent::NavigationStarted(_)
                | AutomationEvent::DomContentLoaded(_)
                | AutomationEvent::Load(_) => None,
                AutomationEvent::PageFileChooserOpened(event) => input_file_dialog_opened_event(
                    event,
                    Some(self.top_level_context_for(event.frame_id.as_str())),
                ),
                AutomationEvent::PageJavaScriptDialogOpening(event) => user_prompt_opened_event(
                    event,
                    self.unhandled_prompt_behavior
                        .handler_for_prompt_type(&event.dialog_type),
                ),
                _ => bidi_event_from_automation_event(automation_event),
            };
            if let Some(event) = generic_event {
                let owner_context = bidi_runtime_automation_event_owner_context(automation_event)
                    .or(fallback_owner_context);
                let mut event = bidi_automation_event_with_owner_context(event, owner_context);
                if let Some(source) = self
                    .log_events
                    .runtime_source_from_automation_event(automation_event, owner_context)
                {
                    event["params"]["source"] = source;
                }
                self.record_bidi_context_user_context(&event);
                let channels =
                    self.subscribed_bidi_event_channels_with_context(&event, owner_context);
                if !channels.is_empty() {
                    if event.get("method").and_then(Value::as_str) == Some("log.entryAdded") {
                        let buffered_event_id = self.log_events.buffer_event(event.clone());
                        for channel in &channels {
                            self.log_events
                                .mark_buffered_event_sent(buffered_event_id, channel.as_deref());
                        }
                    }
                    for channel in channels {
                        bidi_events
                            .push(bidi_message_with_channel(event.clone(), channel.as_deref()));
                    }
                } else if event.get("method").and_then(Value::as_str) == Some("log.entryAdded") {
                    self.log_events.buffer_event(event.clone());
                }
                bidi_events.extend(self.forget_destroyed_bidi_context(&event));
            }
        }
        bidi_events
    }

    pub fn record_protocol_message_state(&mut self, message: &Value, owner_context: Option<&str>) {
        self.log_events
            .record_runtime_realm_from_protocol_message(message, owner_context);
        self.record_child_frame_context_mapping_from_protocol_message(message);
    }

    pub fn replay_buffered_bidi_log_entry_events_for_subscriptions(&mut self) -> Vec<Value> {
        self.log_events.replay_matching_buffered_events(
            &self.subscriptions,
            &self.context_user_contexts,
            &self.context_top_level_contexts,
        )
    }

    pub fn record_bidi_runtime_event_source_opened(&mut self, context: &str) {
        self.event_source_ownership
            .record_runtime_context_opened(context);
    }

    pub fn record_bidi_runtime_event_source_closed(&mut self, context: &str) {
        self.event_source_ownership
            .record_runtime_context_closed(context);
    }

    pub fn record_bidi_network_event_source_opened(&mut self, context: &str) {
        self.event_source_ownership
            .record_network_context_opened(context);
    }

    pub fn record_bidi_network_event_source_closed(&mut self, context: &str) {
        self.event_source_ownership
            .record_network_context_closed(context);
    }

    pub fn record_bidi_file_dialog_opened_source_opened(&mut self, context: &str) {
        self.event_source_ownership
            .record_file_dialog_context_opened(context);
    }

    pub fn record_bidi_file_dialog_opened_source_closed(&mut self, context: &str) {
        self.event_source_ownership
            .record_file_dialog_context_closed(context);
    }

    pub fn record_bidi_download_event_source_opened(&mut self) {
        self.event_source_ownership.record_download_events_opened();
    }

    pub fn record_bidi_download_event_source_closed(&mut self) {
        self.event_source_ownership.record_download_events_closed();
    }

    pub fn subscribe_hook_plan_for_params(
        &self,
        params: &Value,
    ) -> Result<BidiEventSourceHookPlan, BidiError> {
        let events: BTreeSet<String> =
            unroll_bidi_events(&required_supported_event_array(params, "events")?).collect();
        let contexts = optional_non_empty_string_array(params, "contexts")?.unwrap_or_default();
        let user_contexts =
            optional_non_empty_string_array(params, "userContexts")?.unwrap_or_default();
        if !contexts.is_empty() && !user_contexts.is_empty() {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "contexts and userContexts cannot both be specified",
            ));
        }
        self.validate_known_bidi_contexts(&contexts)?;
        self.validate_known_bidi_user_contexts(&user_contexts)?;

        let requested_scope = self.requested_event_source_hook_scope(&contexts, &user_contexts);
        let mut plan = BidiEventSourceHookPlan::default();
        if events
            .iter()
            .any(|event| is_bidi_runtime_source_event_name(event))
            && let Some(scope) = self.new_runtime_hook_scope_for_subscription(&requested_scope)
        {
            plan.set_runtime_scope(scope);
            plan.record_runtime_context_ownership();
        }
        if events.iter().any(|event| is_bidi_network_event_name(event))
            && let Some(scope) = self.new_network_hook_scope_for_subscription(&requested_scope)
        {
            plan.set_network_scope(scope);
        }
        if events.contains("input.fileDialogOpened")
            && let Some(scope) =
                self.new_file_dialog_opened_hook_scope_for_subscription(&requested_scope)
        {
            plan.set_file_dialog_opened_scope(scope);
        }
        if events
            .iter()
            .any(|event| is_bidi_download_event_name(event))
            && !self.event_source_ownership.download_events_opened()
        {
            plan.enable_download_events();
        }
        Ok(plan)
    }

    fn unsubscribe_hook_plan(&self) -> BidiEventSourceHookPlan {
        self.unsubscribe_hook_plan_preserving_log_buffer_sources(&[])
    }

    fn unsubscribe_hook_plan_preserving_log_buffer_sources(
        &self,
        removed_log_subscriptions: &[BidiSubscription],
    ) -> BidiEventSourceHookPlan {
        let mut plan = BidiEventSourceHookPlan::default();
        let runtime_contexts = self
            .event_source_ownership
            .opened_runtime_contexts()
            .into_iter()
            .filter(|context| {
                !self.context_is_subscribed_to_any_runtime_source_event(context)
                    && !self.removed_log_subscription_matches_context(
                        removed_log_subscriptions,
                        context,
                    )
            })
            .collect::<BTreeSet<_>>();
        if !runtime_contexts.is_empty() {
            plan.set_runtime_disable_scope(BidiEventSourceHookScope::Contexts(runtime_contexts));
        }
        let network_contexts = self
            .event_source_ownership
            .opened_network_contexts()
            .into_iter()
            .filter(|context| !self.context_is_subscribed_to_any_network_event(context))
            .collect::<BTreeSet<_>>();
        if !network_contexts.is_empty() {
            plan.set_network_disable_scope(BidiEventSourceHookScope::Contexts(network_contexts));
        }
        let file_dialog_contexts = self
            .event_source_ownership
            .opened_file_dialog_contexts()
            .into_iter()
            .filter(|context| {
                !self.context_is_subscribed_to_event("input.fileDialogOpened", context)
            })
            .collect::<BTreeSet<_>>();
        if !file_dialog_contexts.is_empty() {
            plan.set_file_dialog_opened_disable_scope(BidiEventSourceHookScope::Contexts(
                file_dialog_contexts,
            ));
        }
        if self.event_source_ownership.download_events_opened()
            && !self.has_any_download_event_subscription()
        {
            plan.disable_download_events();
        }
        plan
    }

    fn removed_log_subscription_matches_context(
        &self,
        removed_log_subscriptions: &[BidiSubscription],
        context: &str,
    ) -> bool {
        if removed_log_subscriptions.is_empty() {
            return false;
        }
        let event = json!({
            "method": "log.entryAdded",
            "params": {
                "source": {
                    "context": context
                }
            }
        });
        is_event_subscribed_by(
            removed_log_subscriptions,
            &event,
            &self.context_user_contexts,
            &self.context_top_level_contexts,
        )
    }

    pub fn release_event_source_hook_plan(&self) -> BidiEventSourceHookPlan {
        let mut plan = BidiEventSourceHookPlan::default();
        let runtime_contexts = self.event_source_ownership.opened_runtime_contexts();
        if !runtime_contexts.is_empty() {
            plan.set_runtime_disable_scope(BidiEventSourceHookScope::Contexts(runtime_contexts));
        }
        let network_contexts = self.event_source_ownership.opened_network_contexts();
        if !network_contexts.is_empty() {
            plan.set_network_disable_scope(BidiEventSourceHookScope::Contexts(network_contexts));
        }
        let file_dialog_contexts = self.event_source_ownership.opened_file_dialog_contexts();
        if !file_dialog_contexts.is_empty() {
            plan.set_file_dialog_opened_disable_scope(BidiEventSourceHookScope::Contexts(
                file_dialog_contexts,
            ));
        }
        if self.event_source_ownership.download_events_opened() {
            plan.disable_download_events();
        }
        plan
    }

    fn take_release_event_source_hook_plan(&mut self) -> BidiEventSourceHookPlan {
        self.pending_release_event_source_hook_plan
            .take()
            .unwrap_or_else(|| self.release_event_source_hook_plan())
    }

    fn take_unsubscribe_event_source_hook_plan(&mut self) -> BidiEventSourceHookPlan {
        self.pending_unsubscribe_event_source_hook_plan
            .take()
            .unwrap_or_else(|| self.unsubscribe_hook_plan())
    }

    pub fn subscribed_bidi_events_from_bidi_events<'a>(
        &self,
        events: impl IntoIterator<Item = &'a Value>,
    ) -> Vec<Value> {
        self.subscribed_bidi_events_from_bidi_events_with_context(events, None)
    }

    pub fn subscribed_bidi_events_from_bidi_events_with_context<'a>(
        &self,
        events: impl IntoIterator<Item = &'a Value>,
        owner_context: Option<&str>,
    ) -> Vec<Value> {
        events
            .into_iter()
            .flat_map(|event| {
                self.subscribed_bidi_event_messages_with_context(event, owner_context)
            })
            .collect()
    }

    fn subscribed_bidi_event_messages(&self, event: &Value) -> Vec<Value> {
        self.subscribed_bidi_event_messages_with_context(event, None)
    }

    fn subscribed_bidi_event_messages_with_context(
        &self,
        event: &Value,
        owner_context: Option<&str>,
    ) -> Vec<Value> {
        self.subscribed_bidi_event_channels_with_context(event, owner_context)
            .into_iter()
            .map(|channel| bidi_message_with_channel(event.clone(), channel.as_deref()))
            .collect()
    }

    fn subscribed_bidi_event_channels(&self, event: &Value) -> BTreeSet<Option<String>> {
        self.subscribed_bidi_event_channels_with_context(event, None)
    }

    fn subscribed_bidi_event_channels_with_context(
        &self,
        event: &Value,
        owner_context: Option<&str>,
    ) -> BTreeSet<Option<String>> {
        if let Some(owner_context) = owner_context
            && bidi_event_context(event).is_none()
            && is_bidi_runtime_event_with_owner_context(event)
        {
            let event = bidi_event_with_matching_context(event, owner_context);
            return bidi_event_subscribed_channels(
                &self.subscriptions,
                &event,
                &self.context_user_contexts,
                &self.context_top_level_contexts,
            );
        }
        bidi_event_subscribed_channels(
            &self.subscriptions,
            event,
            &self.context_user_contexts,
            &self.context_top_level_contexts,
        )
    }

    pub fn subscribed_contexts_for_bidi_event(&self, method: &str) -> Option<Vec<String>> {
        self.contexts_for_bidi_event(method)
    }

    pub fn source_contexts_for_bidi_event(&self, method: &str) -> Option<Vec<String>> {
        self.contexts_for_bidi_event(method)
    }

    pub fn replay_contexts_for_bidi_event(&self, method: &str) -> Option<Vec<String>> {
        self.contexts_for_bidi_event(method)
    }

    fn contexts_for_bidi_event(&self, method: &str) -> Option<Vec<String>> {
        let mut global = false;
        let mut contexts = BTreeSet::new();
        for subscription in &self.subscriptions {
            if !subscription.events.contains(method) {
                continue;
            }
            if !subscription.contexts.is_empty() {
                contexts.extend(
                    subscription
                        .contexts
                        .iter()
                        .map(|context| self.top_level_context_for(context).to_owned()),
                );
            } else if !subscription.user_contexts.is_empty() {
                contexts.extend(
                    self.context_user_contexts
                        .iter()
                        .filter(|(_, user_context)| {
                            subscription.user_contexts.contains(*user_context)
                        })
                        .map(|(context, _)| self.top_level_context_for(context).to_owned()),
                );
            } else {
                global = true;
            }
        }
        if global {
            Some(Vec::new())
        } else if contexts.is_empty() {
            None
        } else {
            Some(contexts.into_iter().collect())
        }
    }

    fn requested_event_source_hook_scope(
        &self,
        contexts: &[String],
        user_contexts: &[String],
    ) -> BidiEventSourceHookScope {
        if !contexts.is_empty() {
            let contexts = contexts
                .iter()
                .map(|context| self.top_level_context_for(context).to_owned())
                .collect();
            return BidiEventSourceHookScope::Contexts(contexts);
        }
        if !user_contexts.is_empty() {
            let requested_user_contexts: BTreeSet<&str> =
                user_contexts.iter().map(String::as_str).collect();
            let contexts = self
                .context_user_contexts
                .iter()
                .filter(|(_, user_context)| requested_user_contexts.contains(user_context.as_str()))
                .map(|(context, _)| self.top_level_context_for(context).to_owned())
                .collect();
            return BidiEventSourceHookScope::Contexts(contexts);
        }
        BidiEventSourceHookScope::Global
    }

    fn new_runtime_hook_scope_for_subscription(
        &self,
        requested_scope: &BidiEventSourceHookScope,
    ) -> Option<BidiEventSourceHookScope> {
        match requested_scope {
            BidiEventSourceHookScope::Global => self.new_global_runtime_hook_scope(),
            BidiEventSourceHookScope::Contexts(contexts) => {
                let contexts = contexts
                    .iter()
                    .filter(|context| {
                        !self.event_source_ownership.runtime_context_opened(context)
                            && !self.context_is_subscribed_to_any_runtime_source_event(context)
                    })
                    .cloned()
                    .collect::<BTreeSet<_>>();
                (!contexts.is_empty()).then_some(BidiEventSourceHookScope::Contexts(contexts))
            }
        }
    }

    fn new_network_hook_scope_for_subscription(
        &self,
        requested_scope: &BidiEventSourceHookScope,
    ) -> Option<BidiEventSourceHookScope> {
        match requested_scope {
            BidiEventSourceHookScope::Global => self.new_global_network_hook_scope(),
            BidiEventSourceHookScope::Contexts(contexts) => {
                let contexts = contexts
                    .iter()
                    .filter(|context| !self.context_is_subscribed_to_any_network_event(context))
                    .cloned()
                    .collect::<BTreeSet<_>>();
                (!contexts.is_empty()).then_some(BidiEventSourceHookScope::Contexts(contexts))
            }
        }
    }

    fn new_file_dialog_opened_hook_scope_for_subscription(
        &self,
        requested_scope: &BidiEventSourceHookScope,
    ) -> Option<BidiEventSourceHookScope> {
        match requested_scope {
            BidiEventSourceHookScope::Global => self.new_global_file_dialog_opened_hook_scope(),
            BidiEventSourceHookScope::Contexts(contexts) => {
                let contexts = contexts
                    .iter()
                    .filter(|context| {
                        !self.context_is_subscribed_to_event("input.fileDialogOpened", context)
                    })
                    .cloned()
                    .collect::<BTreeSet<_>>();
                (!contexts.is_empty()).then_some(BidiEventSourceHookScope::Contexts(contexts))
            }
        }
    }

    fn new_global_runtime_hook_scope(&self) -> Option<BidiEventSourceHookScope> {
        if self.has_global_runtime_source_subscription() {
            return None;
        }
        let known_contexts = self.known_top_level_contexts();
        if known_contexts.is_empty() {
            return Some(BidiEventSourceHookScope::Global);
        }
        let contexts = known_contexts
            .into_iter()
            .filter(|context| {
                !self.event_source_ownership.runtime_context_opened(context)
                    && !self.context_is_subscribed_to_any_runtime_source_event(context)
            })
            .collect::<BTreeSet<_>>();
        (!contexts.is_empty()).then_some(BidiEventSourceHookScope::Contexts(contexts))
    }

    fn new_global_network_hook_scope(&self) -> Option<BidiEventSourceHookScope> {
        if self.has_global_network_subscription() {
            return None;
        }
        let known_contexts = self.known_top_level_contexts();
        if known_contexts.is_empty() {
            return Some(BidiEventSourceHookScope::Global);
        }
        let contexts = known_contexts
            .into_iter()
            .filter(|context| !self.context_is_subscribed_to_any_network_event(context))
            .collect::<BTreeSet<_>>();
        (!contexts.is_empty()).then_some(BidiEventSourceHookScope::Contexts(contexts))
    }

    fn new_global_file_dialog_opened_hook_scope(&self) -> Option<BidiEventSourceHookScope> {
        if self.has_global_subscription_for_event("input.fileDialogOpened") {
            return None;
        }
        let known_contexts = self.known_top_level_contexts();
        if known_contexts.is_empty() {
            return Some(BidiEventSourceHookScope::Global);
        }
        let contexts = known_contexts
            .into_iter()
            .filter(|context| {
                !self.context_is_subscribed_to_event("input.fileDialogOpened", context)
            })
            .collect::<BTreeSet<_>>();
        (!contexts.is_empty()).then_some(BidiEventSourceHookScope::Contexts(contexts))
    }

    fn known_top_level_contexts(&self) -> BTreeSet<String> {
        self.context_top_level_contexts.values().cloned().collect()
    }

    fn has_global_subscription_for_event(&self, event: &str) -> bool {
        self.subscriptions.iter().any(|subscription| {
            subscription.events.contains(event)
                && subscription.contexts.is_empty()
                && subscription.user_contexts.is_empty()
        })
    }

    fn has_global_runtime_source_subscription(&self) -> bool {
        self.subscriptions.iter().any(|subscription| {
            subscription.contexts.is_empty()
                && subscription.user_contexts.is_empty()
                && subscription
                    .events
                    .iter()
                    .any(|event| is_bidi_runtime_source_event_name(event))
        })
    }

    fn has_global_network_subscription(&self) -> bool {
        self.subscriptions.iter().any(|subscription| {
            subscription.contexts.is_empty()
                && subscription.user_contexts.is_empty()
                && subscription
                    .events
                    .iter()
                    .any(|event| is_bidi_network_event_name(event))
        })
    }

    fn has_any_download_event_subscription(&self) -> bool {
        self.subscriptions.iter().any(|subscription| {
            subscription
                .events
                .iter()
                .any(|event| is_bidi_download_event_name(event))
        })
    }

    fn context_is_subscribed_to_any_network_event(&self, context: &str) -> bool {
        [
            "network.beforeRequestSent",
            "network.responseStarted",
            "network.authRequired",
            "network.responseCompleted",
            "network.fetchError",
        ]
        .iter()
        .any(|event| self.context_is_subscribed_to_event(event, context))
    }

    fn context_is_subscribed_to_any_runtime_source_event(&self, context: &str) -> bool {
        [
            "log.entryAdded",
            "script.realmCreated",
            "script.realmDestroyed",
        ]
        .iter()
        .any(|event| self.context_is_subscribed_to_event(event, context))
    }

    fn context_is_subscribed_to_event(&self, method: &str, context: &str) -> bool {
        let event = if method == "log.entryAdded" {
            json!({
                "method": method,
                "params": {
                    "source": {
                        "context": context
                    }
                }
            })
        } else {
            json!({
                "method": method,
                "params": {
                    "context": context
                }
            })
        };
        self.is_subscribed_to_bidi_event(&event)
    }

    fn is_subscribed_to_bidi_event(&self, event: &Value) -> bool {
        is_event_subscribed_by(
            &self.subscriptions,
            event,
            &self.context_user_contexts,
            &self.context_top_level_contexts,
        )
    }

    pub fn record_bidi_command_response(
        &mut self,
        method: Option<&str>,
        params: Option<&Value>,
        response: &Value,
    ) -> BidiEventSourceHookPlan {
        if response.get("type").and_then(Value::as_str) != Some("success") {
            return BidiEventSourceHookPlan::default();
        }
        match method {
            Some("browsingContext.create") => {
                let Some(context) = response
                    .get("result")
                    .and_then(|result| result.get("context"))
                    .and_then(Value::as_str)
                else {
                    return BidiEventSourceHookPlan::default();
                };
                let user_context = params
                    .and_then(|params| params.get("userContext"))
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_BIDI_USER_CONTEXT);
                self.record_bidi_context_mapping(context, Some(context), Some(user_context));
                let mut plan = BidiEventSourceHookPlan::default();
                let mut runtime_contexts = BTreeSet::new();
                runtime_contexts.insert(context.to_owned());
                plan.set_runtime_scope(BidiEventSourceHookScope::Contexts(runtime_contexts));
                if self.context_is_subscribed_to_any_runtime_source_event(context) {
                    plan.record_runtime_context_ownership();
                }
                if self.context_is_subscribed_to_any_network_event(context) {
                    let mut network_contexts = BTreeSet::new();
                    network_contexts.insert(context.to_owned());
                    plan.set_network_scope(BidiEventSourceHookScope::Contexts(network_contexts));
                }
                if self.context_is_subscribed_to_event("input.fileDialogOpened", context) {
                    let mut file_dialog_contexts = BTreeSet::new();
                    file_dialog_contexts.insert(context.to_owned());
                    plan.set_file_dialog_opened_scope(BidiEventSourceHookScope::Contexts(
                        file_dialog_contexts,
                    ));
                }
                plan
            }
            Some("browsingContext.getTree") => {
                if let Some(contexts) = response
                    .get("result")
                    .and_then(|result| result.get("contexts"))
                    .and_then(Value::as_array)
                {
                    for context in contexts {
                        self.record_bidi_context_info(context);
                    }
                }
                BidiEventSourceHookPlan::default()
            }
            Some("browsingContext.close") => {
                if let Some(context) = params
                    .and_then(|params| params.get("context"))
                    .and_then(Value::as_str)
                {
                    self.forget_bidi_context_mapping(context);
                }
                BidiEventSourceHookPlan::default()
            }
            Some("session.end" | "browser.close") => self.take_release_event_source_hook_plan(),
            Some("session.unsubscribe") => self.take_unsubscribe_event_source_hook_plan(),
            Some("browser.createUserContext") => {
                if let Some(user_context) = response
                    .get("result")
                    .and_then(|result| result.get("userContext"))
                    .and_then(Value::as_str)
                {
                    self.known_user_contexts.insert(user_context.to_owned());
                }
                BidiEventSourceHookPlan::default()
            }
            Some("browser.getUserContexts") => {
                if let Some(user_contexts) = response
                    .get("result")
                    .and_then(|result| result.get("userContexts"))
                    .and_then(Value::as_array)
                {
                    for user_context in user_contexts {
                        if let Some(user_context) =
                            user_context.get("userContext").and_then(Value::as_str)
                        {
                            self.known_user_contexts.insert(user_context.to_owned());
                        }
                    }
                }
                BidiEventSourceHookPlan::default()
            }
            Some("browser.removeUserContext") => {
                if let Some(user_context) = params
                    .and_then(|params| params.get("userContext"))
                    .and_then(Value::as_str)
                {
                    self.known_user_contexts.remove(user_context);
                    self.forget_bidi_user_context_mapping(user_context);
                }
                BidiEventSourceHookPlan::default()
            }
            _ => BidiEventSourceHookPlan::default(),
        }
    }

    pub fn context_created_event_source_hook_plan(
        &self,
        events: &[Value],
    ) -> BidiEventSourceHookPlan {
        let mut runtime_contexts = BTreeSet::new();
        let mut network_contexts = BTreeSet::new();
        let mut file_dialog_contexts = BTreeSet::new();
        for event in events {
            if event.get("method").and_then(Value::as_str) != Some("browsingContext.contextCreated")
            {
                continue;
            }
            let Some(context) = event
                .get("params")
                .and_then(|params| params.get("context"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            if self.context_is_subscribed_to_any_runtime_source_event(context) {
                runtime_contexts.insert(context.to_owned());
            }
            if self.context_is_subscribed_to_any_network_event(context) {
                network_contexts.insert(context.to_owned());
            }
            if self.context_is_subscribed_to_event("input.fileDialogOpened", context) {
                file_dialog_contexts.insert(context.to_owned());
            }
        }

        let mut plan = BidiEventSourceHookPlan::default();
        if !runtime_contexts.is_empty() {
            plan.set_runtime_scope(BidiEventSourceHookScope::Contexts(runtime_contexts));
            plan.record_runtime_context_ownership();
        }
        if !network_contexts.is_empty() {
            plan.set_network_scope(BidiEventSourceHookScope::Contexts(network_contexts));
        }
        if !file_dialog_contexts.is_empty() {
            plan.set_file_dialog_opened_scope(BidiEventSourceHookScope::Contexts(
                file_dialog_contexts,
            ));
        }
        plan
    }

    fn validate_known_bidi_contexts(&self, contexts: &[String]) -> Result<(), BidiError> {
        if contexts
            .iter()
            .any(|context| !self.context_top_level_contexts.contains_key(context))
        {
            return Err(BidiError::new(
                BidiErrorCode::NoSuchFrame,
                "context not found",
            ));
        }
        Ok(())
    }

    fn validate_known_bidi_user_contexts(&self, user_contexts: &[String]) -> Result<(), BidiError> {
        if user_contexts
            .iter()
            .any(|user_context| !self.known_user_contexts.contains(user_context))
        {
            return Err(BidiError::new(
                BidiErrorCode::NoSuchUserContext,
                "user context not found",
            ));
        }
        Ok(())
    }

    fn top_level_context_for<'a>(&'a self, context: &'a str) -> &'a str {
        self.context_top_level_contexts
            .get(context)
            .map(std::string::String::as_str)
            .unwrap_or(context)
    }

    fn prompt_handler_for_protocol_message(&self, message: &Value) -> &str {
        if message.get("method").and_then(Value::as_str) != Some("Page.javascriptDialogOpening") {
            return "dismiss";
        }
        let prompt_type = message["params"]["type"].as_str().unwrap_or_default();
        self.unhandled_prompt_behavior
            .handler_for_prompt_type(prompt_type)
    }

    fn record_bidi_context_user_context(&mut self, event: &Value) {
        if event.get("method").and_then(Value::as_str) != Some("browsingContext.contextCreated") {
            return;
        }
        let Some(params) = event.get("params") else {
            return;
        };
        let Some(context) = params.get("context").and_then(Value::as_str) else {
            return;
        };
        let Some(user_context) = params.get("userContext").and_then(Value::as_str) else {
            return;
        };
        let client_window = params
            .get("clientWindow")
            .and_then(Value::as_str)
            .unwrap_or(context);
        self.record_bidi_context_mapping(context, Some(client_window), Some(user_context));
        self.record_bidi_context_info(params);
    }

    fn record_child_frame_context_mapping_from_protocol_message(&mut self, message: &Value) {
        if message.get("method").and_then(Value::as_str) != Some("Page.frameAttached") {
            return;
        }
        let params = &message["params"];
        let Some(context) = params.get("frameId").and_then(Value::as_str) else {
            return;
        };
        let Some(parent_context) = params.get("parentFrameId").and_then(Value::as_str) else {
            return;
        };
        let top_level_context = self.top_level_context_for(parent_context).to_owned();
        let user_context = self
            .context_user_contexts
            .get(parent_context)
            .or_else(|| self.context_user_contexts.get(&top_level_context))
            .cloned();
        self.record_bidi_context_mapping(
            context,
            Some(&top_level_context),
            user_context.as_deref(),
        );
    }

    fn record_bidi_context_info(&mut self, info: &Value) {
        let Some(context) = info.get("context").and_then(Value::as_str) else {
            return;
        };
        let client_window = info.get("clientWindow").and_then(Value::as_str);
        let user_context = info.get("userContext").and_then(Value::as_str);
        self.record_bidi_context_mapping(context, client_window, user_context);
        if let Some(children) = info.get("children").and_then(Value::as_array) {
            for child in children {
                self.record_bidi_context_info(child);
            }
        }
    }

    fn record_bidi_context_mapping(
        &mut self,
        context: &str,
        client_window: Option<&str>,
        user_context: Option<&str>,
    ) {
        if let Some(user_context) = user_context {
            self.context_user_contexts
                .insert(context.to_owned(), user_context.to_owned());
            self.known_user_contexts.insert(user_context.to_owned());
        }
        self.context_top_level_contexts.insert(
            context.to_owned(),
            client_window.unwrap_or(context).to_owned(),
        );
    }

    fn forget_bidi_context_mapping(&mut self, context: &str) {
        self.event_source_ownership.forget_context(context);
        self.context_user_contexts.remove(context);
        self.context_top_level_contexts.remove(context);
        let destroyed_top_level = context.to_owned();
        self.context_user_contexts.retain(|child, _| {
            self.context_top_level_contexts
                .get(child)
                .is_none_or(|top_level| top_level != &destroyed_top_level)
        });
        self.context_top_level_contexts
            .retain(|_, top_level| top_level != &destroyed_top_level);
    }

    fn forget_bidi_user_context_mapping(&mut self, user_context: &str) {
        self.context_user_contexts
            .retain(|_, context_user_context| context_user_context != user_context);
    }

    fn forget_destroyed_bidi_context(&mut self, event: &Value) -> Vec<Value> {
        if event.get("method").and_then(Value::as_str) != Some("browsingContext.contextDestroyed") {
            return Vec::new();
        }
        let Some(context) = event
            .get("params")
            .and_then(|params| params.get("context"))
            .and_then(Value::as_str)
        else {
            return Vec::new();
        };
        self.forget_bidi_context_mapping(context);
        self.browsing_context_events.forget_context(context);
        let canceled_download_events = self.download_events.forget_context(context);
        self.log_events.forget_context(context);
        self.network_events.forget_context(context);
        canceled_download_events
            .into_iter()
            .flat_map(|event| self.subscribed_bidi_event_messages(&event))
            .collect()
    }
}

fn bidi_event_context(event: &Value) -> Option<&str> {
    let params = event.get("params")?;
    params.get("context").and_then(Value::as_str).or_else(|| {
        params
            .get("source")
            .and_then(|source| source.get("context"))
            .and_then(Value::as_str)
    })
}

fn is_bidi_runtime_event_with_owner_context(event: &Value) -> bool {
    matches!(
        event.get("method").and_then(Value::as_str),
        Some("log.entryAdded" | "script.realmCreated" | "script.realmDestroyed")
    )
}

fn bidi_event_with_matching_context(event: &Value, owner_context: &str) -> Value {
    let mut event = event.clone();
    if !event.get("params").is_some_and(Value::is_object) {
        event["params"] = json!({});
    }
    if let Some(params) = event.get_mut("params").and_then(Value::as_object_mut) {
        params.insert("context".to_owned(), json!(owner_context));
    }
    event
}

fn bidi_runtime_automation_event_owner_context(event: &AutomationEvent) -> Option<&str> {
    match event {
        AutomationEvent::RuntimeExecutionContextCreated(event)
        | AutomationEvent::RuntimeExecutionContextDestroyed(event)
            if event.frame_id.is_none() =>
        {
            event.target_id.as_ref().map(|target_id| target_id.as_str())
        }
        AutomationEvent::RuntimeExecutionContextsCleared(event) => {
            event.target_id.as_ref().map(|target_id| target_id.as_str())
        }
        AutomationEvent::RuntimeConsoleApiCalled(event) => {
            event.target_id.as_ref().map(|target_id| target_id.as_str())
        }
        AutomationEvent::LogEntryAdded(event) => {
            event.target_id.as_ref().map(|target_id| target_id.as_str())
        }
        AutomationEvent::ScriptMessage(event) => {
            event.target_id.as_ref().map(|target_id| target_id.as_str())
        }
        AutomationEvent::ScriptException(event) => {
            event.target_id.as_ref().map(|target_id| target_id.as_str())
        }
        _ => None,
    }
}

fn bidi_automation_event_with_owner_context(
    mut event: Value,
    owner_context: Option<&str>,
) -> Value {
    let Some(owner_context) = owner_context else {
        return event;
    };
    if event.get("method").and_then(Value::as_str) != Some("log.entryAdded") {
        return event;
    }
    let Some(source) = event
        .get_mut("params")
        .and_then(|params| params.get_mut("source"))
        .and_then(Value::as_object_mut)
    else {
        return event;
    };
    source
        .entry("context".to_owned())
        .or_insert_with(|| json!(owner_context));
    event
}
