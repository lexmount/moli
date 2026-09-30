use std::collections::{BTreeMap, BTreeSet};

use moli_protocol::automation::{
    AutomationCommand, AutomationContext, DevToolsBrowserContextId, DevToolsSessionId,
    DevToolsTargetId, FrontendProtocol,
};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BidiCommand {
    pub id: u64,
    pub method: String,
    pub params: Value,
    pub channel: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BidiErrorCode {
    InvalidArgument,
    InvalidSelector,
    InvalidSessionId,
    NoSuchAlert,
    NoSuchHandle,
    NoSuchHistoryEntry,
    NoSuchNode,
    NoSuchNetworkCollector,
    NoSuchNetworkData,
    NoSuchRequest,
    NoSuchScript,
    NoSuchFrame,
    NoSuchUserContext,
    SessionNotCreated,
    UnableToCaptureScreen,
    UnableToSetFileInput,
    UnableToSetCookie,
    UnsupportedOperation,
    UnknownCommand,
    UnknownError,
}

impl BidiErrorCode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgument => "invalid argument",
            Self::InvalidSelector => "invalid selector",
            Self::InvalidSessionId => "invalid session id",
            Self::NoSuchAlert => "no such alert",
            Self::NoSuchHandle => "no such handle",
            Self::NoSuchHistoryEntry => "no such history entry",
            Self::NoSuchNode => "no such node",
            Self::NoSuchNetworkCollector => "no such network collector",
            Self::NoSuchNetworkData => "no such network data",
            Self::NoSuchRequest => "no such request",
            Self::NoSuchScript => "no such script",
            Self::NoSuchFrame => "no such frame",
            Self::NoSuchUserContext => "no such user context",
            Self::SessionNotCreated => "session not created",
            Self::UnableToCaptureScreen => "unable to capture screen",
            Self::UnableToSetFileInput => "unable to set file input",
            Self::UnableToSetCookie => "unable to set cookie",
            Self::UnsupportedOperation => "unsupported operation",
            Self::UnknownCommand => "unknown command",
            Self::UnknownError => "unknown error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BidiError {
    pub code: BidiErrorCode,
    pub message: String,
}

impl BidiError {
    pub fn new(code: BidiErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BidiDevToolsCommandContext {
    pub session_id: String,
    pub browser_context_id: Option<String>,
}

impl BidiDevToolsCommandContext {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            browser_context_id: None,
        }
    }

    pub fn with_browser_context_id(
        session_id: impl Into<String>,
        browser_context_id: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            browser_context_id: Some(browser_context_id.into()),
        }
    }

    pub(crate) fn command_context(&self, target_id: Option<DevToolsTargetId>) -> AutomationContext {
        self.command_context_with_browser_context_id(
            target_id,
            self.browser_context_id
                .as_deref()
                .map(DevToolsBrowserContextId::from),
        )
    }

    pub(crate) fn command_context_with_browser_context_id(
        &self,
        target_id: Option<DevToolsTargetId>,
        browser_context_id: Option<DevToolsBrowserContextId>,
    ) -> AutomationContext {
        AutomationContext {
            protocol: FrontendProtocol::WebDriverBidi,
            session_id: Some(DevToolsSessionId::from(self.session_id.as_str())),
            target_id,
            browser_context_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BidiDevToolsCommandDispatch {
    pub id: u64,
    pub session_id: String,
    pub command: AutomationCommand,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BidiInputCommandDispatch {
    pub id: u64,
    pub session_id: String,
    pub context: String,
    pub command: BidiInputCommand,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BidiInputCommand {
    PerformActions { params: Value },
    ReleaseActions,
    SetFiles { params: Value },
}

#[derive(Debug, Clone, PartialEq)]
pub struct BidiCommandOutcome {
    pub response: Value,
    pub session_id: Option<String>,
    pub channel: Option<String>,
    pub close_connection: bool,
    pub automation_command: Option<BidiDevToolsCommandDispatch>,
    pub input_command: Option<BidiInputCommandDispatch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BidiSubscription {
    pub(super) id: String,
    pub(super) events: BTreeSet<String>,
    pub(super) contexts: BTreeSet<String>,
    pub(super) user_contexts: BTreeSet<String>,
    pub(super) channel: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BidiSessionRegistry {
    next_session_number: u64,
    active_sessions: BTreeSet<String>,
}

impl Default for BidiSessionRegistry {
    fn default() -> Self {
        Self {
            next_session_number: 1,
            active_sessions: BTreeSet::new(),
        }
    }
}

impl BidiSessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active_session_count(&self) -> usize {
        self.active_sessions.len()
    }

    pub fn contains_session(&self, session_id: &str) -> bool {
        self.active_sessions.contains(session_id)
    }

    pub fn register_session(&mut self, session_id: impl Into<String>) -> bool {
        self.active_sessions.insert(session_id.into())
    }

    pub(super) fn create_session(&mut self) -> String {
        let session_id = format!("bidi-session-{}", self.next_session_number);
        self.next_session_number = self.next_session_number.saturating_add(1);
        self.active_sessions.insert(session_id.clone());
        session_id
    }

    pub fn release_session(&mut self, session_id: &str) {
        self.active_sessions.remove(session_id);
    }
}

impl BidiCommandOutcome {
    pub(super) fn respond(response: Value, session_id: Option<String>) -> Self {
        Self {
            response,
            session_id,
            channel: None,
            close_connection: false,
            automation_command: None,
            input_command: None,
        }
    }
}

pub(super) fn is_event_subscribed_by(
    subscriptions: &[BidiSubscription],
    event: &Value,
    context_user_contexts: &BTreeMap<String, String>,
    context_top_level_contexts: &BTreeMap<String, String>,
) -> bool {
    !bidi_event_subscribed_channels(
        subscriptions,
        event,
        context_user_contexts,
        context_top_level_contexts,
    )
    .is_empty()
}

pub(super) fn bidi_event_subscribed_channels(
    subscriptions: &[BidiSubscription],
    event: &Value,
    context_user_contexts: &BTreeMap<String, String>,
    context_top_level_contexts: &BTreeMap<String, String>,
) -> BTreeSet<Option<String>> {
    let Some(method) = event.get("method").and_then(Value::as_str) else {
        return BTreeSet::new();
    };
    let params = event.get("params");
    let context = params
        .and_then(|params| params.get("context"))
        .and_then(Value::as_str)
        .or_else(|| {
            params
                .and_then(|params| params.get("source"))
                .and_then(|source| source.get("context"))
                .and_then(Value::as_str)
        });
    let user_context = params
        .and_then(|params| params.get("userContext"))
        .and_then(Value::as_str)
        .or_else(|| {
            context.and_then(|context| {
                context_user_contexts
                    .get(context)
                    .map(std::string::String::as_str)
            })
        });
    subscriptions
        .iter()
        .filter(|subscription| {
            subscription.events.contains(method)
                && if !subscription.contexts.is_empty() {
                    context.is_some_and(|context| {
                        bidi_context_subscription_matches(
                            context,
                            &subscription.contexts,
                            context_top_level_contexts,
                        )
                    })
                } else if !subscription.user_contexts.is_empty() {
                    user_context.is_some_and(|user_context| {
                        subscription.user_contexts.contains(user_context)
                    })
                } else {
                    true
                }
        })
        .map(|subscription| subscription.channel.clone())
        .collect()
}

fn bidi_context_subscription_matches(
    event_context: &str,
    subscribed_contexts: &BTreeSet<String>,
    context_top_level_contexts: &BTreeMap<String, String>,
) -> bool {
    if subscribed_contexts.contains(event_context) {
        return true;
    }
    let event_top_level = bidi_top_level_context(event_context, context_top_level_contexts);
    subscribed_contexts.iter().any(|context| {
        bidi_top_level_context(context, context_top_level_contexts) == event_top_level
    })
}

fn bidi_top_level_context<'a>(
    context: &'a str,
    context_top_level_contexts: &'a BTreeMap<String, String>,
) -> &'a str {
    context_top_level_contexts
        .get(context)
        .map(std::string::String::as_str)
        .unwrap_or(context)
}

pub fn parse_bidi_command(message: Value) -> Result<BidiCommand, BidiError> {
    let Some(command) = message.as_object() else {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "BiDi command must be an object",
        ));
    };
    let id = command
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| BidiError::new(BidiErrorCode::InvalidArgument, "id must be a uint"))?;
    let method = command
        .get("method")
        .and_then(Value::as_str)
        .filter(|method| !method.is_empty())
        .ok_or_else(|| {
            BidiError::new(
                BidiErrorCode::InvalidArgument,
                "method must be a non-empty string",
            )
        })?
        .to_owned();
    let params = command.get("params").cloned().unwrap_or_else(|| json!({}));
    if !params.is_object() {
        return Err(BidiError::new(
            BidiErrorCode::InvalidArgument,
            "params must be an object",
        ));
    }
    let channel = match command.get("goog:channel") {
        Some(Value::String(channel)) if channel.is_empty() => None,
        Some(Value::String(channel)) => Some(channel.clone()),
        Some(_) => {
            return Err(BidiError::new(
                BidiErrorCode::InvalidArgument,
                "goog:channel must be a string",
            ));
        }
        None => None,
    };
    Ok(BidiCommand {
        id,
        method,
        params,
        channel,
    })
}

pub(super) fn bidi_message_with_channel(mut message: Value, channel: Option<&str>) -> Value {
    if let Some(channel) = channel
        && let Some(message) = message.as_object_mut()
    {
        message.insert("goog:channel".to_owned(), json!(channel));
    }
    message
}

pub(super) fn is_known_session_command(method: &str) -> bool {
    matches!(
        method,
        "session.end"
            | "browser.close"
            | "session.subscribe"
            | "session.unsubscribe"
            | "browser.createUserContext"
            | "browser.getClientWindows"
            | "browser.getUserContexts"
            | "browser.removeUserContext"
            | "browser.setDownloadBehavior"
            | "browser.setClientWindowState"
            | "browsingContext.activate"
            | "browsingContext.captureScreenshot"
            | "browsingContext.close"
            | "browsingContext.create"
            | "browsingContext.getTree"
            | "browsingContext.handleUserPrompt"
            | "browsingContext.locateNodes"
            | "browsingContext.navigate"
            | "browsingContext.print"
            | "browsingContext.reload"
            | "browsingContext.setViewport"
            | "browsingContext.traverseHistory"
            | "emulation.setLocaleOverride"
            | "emulation.setGeolocationOverride"
            | "emulation.setNetworkConditions"
            | "emulation.setTimezoneOverride"
            | "emulation.setUserAgentOverride"
            | "permissions.setPermission"
            | "input.performActions"
            | "input.releaseActions"
            | "input.setFiles"
            | "script.callFunction"
            | "script.disown"
            | "script.addPreloadScript"
            | "script.evaluate"
            | "script.getRealms"
            | "script.removePreloadScript"
            | "storage.deleteCookies"
            | "storage.getCookies"
            | "storage.setCookie"
            | "network.addIntercept"
            | "network.addDataCollector"
            | "network.continueRequest"
            | "network.continueResponse"
            | "network.continueWithAuth"
            | "network.disownData"
            | "network.failRequest"
            | "network.getData"
            | "network.provideResponse"
            | "network.removeDataCollector"
            | "network.removeIntercept"
            | "network.setCacheBehavior"
            | "network.setExtraHeaders"
    )
}

pub(super) fn is_automation_command(method: &str) -> bool {
    matches!(
        method,
        "browser.createUserContext"
            | "browser.getClientWindows"
            | "browser.getUserContexts"
            | "browser.removeUserContext"
            | "browser.setDownloadBehavior"
            | "browser.setClientWindowState"
            | "browsingContext.close"
            | "browsingContext.activate"
            | "browsingContext.captureScreenshot"
            | "browsingContext.create"
            | "browsingContext.getTree"
            | "browsingContext.handleUserPrompt"
            | "browsingContext.locateNodes"
            | "browsingContext.navigate"
            | "browsingContext.print"
            | "browsingContext.reload"
            | "browsingContext.setViewport"
            | "browsingContext.traverseHistory"
            | "emulation.setLocaleOverride"
            | "emulation.setGeolocationOverride"
            | "emulation.setNetworkConditions"
            | "emulation.setTimezoneOverride"
            | "emulation.setUserAgentOverride"
            | "permissions.setPermission"
            | "network.continueRequest"
            | "network.continueResponse"
            | "network.continueWithAuth"
            | "network.failRequest"
            | "network.getData"
            | "network.provideResponse"
            | "network.addIntercept"
            | "network.addDataCollector"
            | "network.removeIntercept"
            | "network.removeDataCollector"
            | "network.disownData"
            | "network.setCacheBehavior"
            | "network.setExtraHeaders"
            | "script.addPreloadScript"
            | "script.callFunction"
            | "script.disown"
            | "script.evaluate"
            | "script.getRealms"
            | "script.removePreloadScript"
            | "storage.deleteCookies"
            | "storage.getCookies"
            | "storage.setCookie"
    )
}
