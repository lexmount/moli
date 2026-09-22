use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use moli_protocol::devtools_runtime::{
    DevToolsCommandContext, DevToolsNavigationWait, DevToolsProtocol, DevToolsSessionId,
    DevToolsTargetId,
};

use crate::actions::ClassicActionState;

pub const CLASSIC_ELEMENT_REFERENCE_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";
pub const CLASSIC_SHADOW_ROOT_REFERENCE_KEY: &str = "shadow-6066-11e4-a52e-4f735466cecf";
pub const CLASSIC_FRAME_REFERENCE_KEY: &str = "frame-075b-4da1-b6ba-e579c2d3230a";
pub const CLASSIC_WINDOW_REFERENCE_KEY: &str = "window-fcc6-11e5-b4f8-330a88ab9d7f";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassicErrorCode {
    InvalidArgument,
    InvalidCookieDomain,
    InvalidElementState,
    InvalidSelector,
    ElementNotInteractable,
    InvalidSessionId,
    JavascriptError,
    NoSuchAlert,
    NoSuchCookie,
    NoSuchElement,
    NoSuchFrame,
    NoSuchShadowRoot,
    NoSuchWindow,
    MoveTargetOutOfBounds,
    ScriptTimeout,
    SessionNotCreated,
    DetachedShadowRoot,
    StaleElementReference,
    Timeout,
    UnknownCommand,
    UnknownError,
    UnexpectedAlertOpen,
    UnsupportedOperation,
}

impl ClassicErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgument => "invalid argument",
            Self::InvalidCookieDomain => "invalid cookie domain",
            Self::InvalidElementState => "invalid element state",
            Self::InvalidSelector => "invalid selector",
            Self::ElementNotInteractable => "element not interactable",
            Self::InvalidSessionId => "invalid session id",
            Self::JavascriptError => "javascript error",
            Self::NoSuchAlert => "no such alert",
            Self::NoSuchCookie => "no such cookie",
            Self::NoSuchElement => "no such element",
            Self::NoSuchFrame => "no such frame",
            Self::NoSuchShadowRoot => "no such shadow root",
            Self::NoSuchWindow => "no such window",
            Self::MoveTargetOutOfBounds => "move target out of bounds",
            Self::ScriptTimeout => "script timeout",
            Self::SessionNotCreated => "session not created",
            Self::DetachedShadowRoot => "detached shadow root",
            Self::StaleElementReference => "stale element reference",
            Self::Timeout => "timeout",
            Self::UnknownCommand => "unknown command",
            Self::UnknownError => "unknown error",
            Self::UnexpectedAlertOpen => "unexpected alert open",
            Self::UnsupportedOperation => "unsupported operation",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicError {
    pub code: ClassicErrorCode,
    pub message: String,
    pub data: Option<Value>,
}

impl ClassicError {
    pub fn new(code: ClassicErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(code: ClassicErrorCode, message: impl Into<String>, data: Value) -> Self {
        Self {
            code,
            message: message.into(),
            data: Some(data),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicUnhandledPromptBehavior {
    returned_capability: Value,
    default_handler: ClassicPromptHandler,
    alert: Option<ClassicPromptHandler>,
    before_unload: Option<ClassicPromptHandler>,
    confirm: Option<ClassicPromptHandler>,
    file: Option<ClassicPromptHandler>,
    file_uses_default_handler: bool,
    prompt: Option<ClassicPromptHandler>,
}

impl Default for ClassicUnhandledPromptBehavior {
    fn default() -> Self {
        Self {
            returned_capability: json!("dismiss and notify"),
            default_handler: ClassicPromptHandler::Dismiss { notify: true },
            alert: None,
            before_unload: None,
            confirm: None,
            file: None,
            file_uses_default_handler: false,
            prompt: None,
        }
    }
}

impl ClassicUnhandledPromptBehavior {
    pub fn from_capability(value: Option<&Value>) -> Result<Self, ClassicError> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        if let Some(handler) = value.as_str() {
            return Ok(Self {
                returned_capability: json!(handler),
                default_handler: ClassicPromptHandler::from_capability(handler)?,
                file_uses_default_handler: true,
                ..Self::default()
            });
        }
        let Some(object) = value.as_object() else {
            return Err(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "unhandledPromptBehavior must be a string or object",
            ));
        };
        let mut behavior = Self {
            returned_capability: Value::Object(Map::new()),
            ..Self::default()
        };
        let mut returned = Map::new();
        let mut has_default = false;
        let mut has_file = false;
        for (key, value) in object {
            let Some(handler) = value.as_str() else {
                return Err(ClassicError::new(
                    ClassicErrorCode::InvalidArgument,
                    "unhandledPromptBehavior handler must be a string",
                ));
            };
            let handler = ClassicPromptHandler::from_capability(handler)?;
            match key.as_str() {
                "default" => {
                    behavior.default_handler = handler;
                    has_default = true;
                }
                "alert" => behavior.alert = Some(handler),
                "beforeUnload" => behavior.before_unload = Some(handler),
                "confirm" => behavior.confirm = Some(handler),
                "file" => {
                    behavior.file = Some(handler);
                    has_file = true;
                }
                "prompt" => behavior.prompt = Some(handler),
                _ => {
                    return Err(ClassicError::new(
                        ClassicErrorCode::InvalidArgument,
                        "unhandledPromptBehavior prompt type is not supported",
                    ));
                }
            }
            returned.insert(key.clone(), value.clone());
        }
        behavior.file_uses_default_handler = has_default && !has_file;
        behavior.returned_capability = Value::Object(returned);
        Ok(behavior)
    }

    pub fn returned_capability(&self) -> Value {
        self.returned_capability.clone()
    }

    pub fn handler_for_prompt_type(&self, prompt_type: &str) -> ClassicPromptHandler {
        let specific = match prompt_type {
            "alert" => self.alert,
            "beforeunload" | "beforeUnload" => self.before_unload,
            "confirm" => self.confirm,
            "file" => self.file,
            "prompt" => self.prompt,
            _ => None,
        };
        specific.unwrap_or(self.default_handler)
    }

    pub fn file_prompt_handler_for_bidi_script_commands(&self) -> Option<&'static str> {
        let handler = self.file.or_else(|| {
            self.file_uses_default_handler
                .then_some(self.default_handler)
        })?;
        match handler {
            ClassicPromptHandler::Accept { .. } => Some("accept"),
            ClassicPromptHandler::Dismiss { .. } => Some("dismiss"),
            ClassicPromptHandler::Ignore => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassicPromptHandler {
    Accept { notify: bool },
    Dismiss { notify: bool },
    Ignore,
}

impl ClassicPromptHandler {
    fn from_capability(value: &str) -> Result<Self, ClassicError> {
        match value {
            "accept" => Ok(Self::Accept { notify: false }),
            "accept and notify" => Ok(Self::Accept { notify: true }),
            "dismiss" => Ok(Self::Dismiss { notify: false }),
            "dismiss and notify" => Ok(Self::Dismiss { notify: true }),
            "ignore" => Ok(Self::Ignore),
            _ => Err(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "unhandledPromptBehavior handler value is not supported",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicDevToolsCommandContext {
    pub session_id: String,
    pub target_id: Option<String>,
    protocol: DevToolsProtocol,
}

impl ClassicDevToolsCommandContext {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            target_id: None,
            protocol: DevToolsProtocol::WebDriverClassic,
        }
    }

    pub fn with_target_id(session_id: impl Into<String>, target_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            target_id: Some(target_id.into()),
            protocol: DevToolsProtocol::WebDriverClassic,
        }
    }

    pub fn with_protocol_and_target_id(
        protocol: DevToolsProtocol,
        session_id: impl Into<String>,
        target_id: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            target_id: Some(target_id.into()),
            protocol,
        }
    }

    pub(crate) fn command_context(&self) -> DevToolsCommandContext {
        DevToolsCommandContext {
            protocol: self.protocol,
            session_id: Some(DevToolsSessionId::from(self.session_id.as_str())),
            target_id: self.target_id.as_deref().map(DevToolsTargetId::from),
            browser_context_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassicSessionState {
    pub session_id: String,
    pub current_target_id: Option<String>,
    pub current_frame_id: Option<String>,
    pub timeouts: ClassicTimeouts,
    pub page_load_strategy: ClassicPageLoadStrategy,
    pub unhandled_prompt_behavior: ClassicUnhandledPromptBehavior,
    pub action_state: ClassicActionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicTimeouts {
    pub script: Option<u64>,
    pub page_load: Option<u64>,
    pub implicit: Option<u64>,
}

impl Default for ClassicTimeouts {
    fn default() -> Self {
        Self {
            script: Some(30_000),
            page_load: Some(300_000),
            implicit: Some(0),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ClassicPageLoadStrategy {
    None,
    Eager,
    #[default]
    Normal,
}

impl ClassicPageLoadStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Eager => "eager",
            Self::Normal => "normal",
        }
    }

    pub fn navigation_wait(self) -> DevToolsNavigationWait {
        match self {
            Self::None => DevToolsNavigationWait::None,
            Self::Eager => DevToolsNavigationWait::DomContentLoaded,
            Self::Normal => DevToolsNavigationWait::Load,
        }
    }
}

#[derive(Debug, Default)]
pub struct ClassicSessionRegistry {
    next_session_id: u64,
    sessions: BTreeMap<String, ClassicSessionState>,
}

impl ClassicSessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_session(&mut self) -> ClassicSessionState {
        self.create_session_with_page_load_strategy(ClassicPageLoadStrategy::default())
    }

    pub fn create_session_with_page_load_strategy(
        &mut self,
        page_load_strategy: ClassicPageLoadStrategy,
    ) -> ClassicSessionState {
        self.create_session_with_capabilities(
            page_load_strategy,
            ClassicUnhandledPromptBehavior::default(),
        )
    }

    pub fn create_session_with_capabilities(
        &mut self,
        page_load_strategy: ClassicPageLoadStrategy,
        unhandled_prompt_behavior: ClassicUnhandledPromptBehavior,
    ) -> ClassicSessionState {
        self.next_session_id = self.next_session_id.wrapping_add(1);
        let session_id = format!("classic-session-{}", self.next_session_id);
        let session = ClassicSessionState {
            session_id: session_id.clone(),
            current_target_id: None,
            current_frame_id: None,
            timeouts: ClassicTimeouts::default(),
            page_load_strategy,
            unhandled_prompt_behavior,
            action_state: ClassicActionState::default(),
        };
        self.sessions.insert(session_id, session.clone());
        session
    }

    pub fn has_session(&self, session_id: &str) -> bool {
        self.sessions.contains_key(session_id)
    }

    pub fn release_session(&mut self, session_id: &str) -> bool {
        self.sessions.remove(session_id).is_some()
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn set_current_target_id(
        &mut self,
        session_id: &str,
        target_id: impl Into<String>,
    ) -> bool {
        let Some(session) = self.sessions.get_mut(session_id) else {
            return false;
        };
        session.current_target_id = Some(target_id.into());
        session.current_frame_id = None;
        true
    }

    pub fn current_target_id(&self, session_id: &str) -> Option<&str> {
        self.sessions
            .get(session_id)
            .and_then(|session| session.current_target_id.as_deref())
    }

    pub fn set_current_frame_id(&mut self, session_id: &str, frame_id: Option<String>) -> bool {
        let Some(session) = self.sessions.get_mut(session_id) else {
            return false;
        };
        session.current_frame_id = frame_id;
        true
    }

    pub fn current_frame_id(&self, session_id: &str) -> Option<Option<&str>> {
        self.sessions
            .get(session_id)
            .map(|session| session.current_frame_id.as_deref())
    }

    pub fn timeouts(&self, session_id: &str) -> Option<ClassicTimeouts> {
        self.sessions
            .get(session_id)
            .map(|session| session.timeouts)
    }

    pub fn set_timeouts(&mut self, session_id: &str, timeouts: ClassicTimeouts) -> bool {
        let Some(session) = self.sessions.get_mut(session_id) else {
            return false;
        };
        session.timeouts = timeouts;
        true
    }

    pub fn page_load_strategy(&self, session_id: &str) -> Option<ClassicPageLoadStrategy> {
        self.sessions
            .get(session_id)
            .map(|session| session.page_load_strategy)
    }

    pub fn unhandled_prompt_behavior(
        &self,
        session_id: &str,
    ) -> Option<ClassicUnhandledPromptBehavior> {
        self.sessions
            .get(session_id)
            .map(|session| session.unhandled_prompt_behavior.clone())
    }

    pub fn action_state_mut(&mut self, session_id: &str) -> Option<&mut ClassicActionState> {
        self.sessions
            .get_mut(session_id)
            .map(|session| &mut session.action_state)
    }
}
