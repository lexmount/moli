use std::collections::BTreeSet;

use serde_json::Value;

use crate::RuntimeBindingRegistration;

/// Opaque V8 Inspector session state.
///
/// The bytes are V8-owned CBOR and must be passed back without parsing or
/// rewriting. `V8InspectorSessionAttach` represents first attach separately,
/// so even an empty value remains a valid, distinct reattach state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V8InspectorSessionState(Vec<u8>);

impl V8InspectorSessionState {
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

/// How a renderer should connect one V8 Inspector session.
///
/// `FirstAttach` has no prior V8 agent and may need protocol-owned bootstrap
/// commands. `Reattach` must restore the supplied opaque state without
/// replaying protocol listener flags into the V8 agent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum V8InspectorSessionAttach {
    #[default]
    FirstAttach,
    Reattach(V8InspectorSessionState),
}

impl V8InspectorSessionAttach {
    pub fn from_optional_state(state: Option<V8InspectorSessionState>) -> Self {
        match state {
            Some(state) => Self::Reattach(state),
            None => Self::FirstAttach,
        }
    }

    pub fn reattach_state(&self) -> Option<&V8InspectorSessionState> {
        match self {
            Self::FirstAttach => None,
            Self::Reattach(state) => Some(state),
        }
    }

    pub fn is_reattach(&self) -> bool {
        matches!(self, Self::Reattach(_))
    }
}

/// Renderer-neutral, serializable control state needed to rebuild one
/// Inspector session after a document backend is replaced.
///
/// This deliberately excludes isolate-owned data such as remote objects and
/// pending callbacks. CPU-profile samples also cannot be replayed into a new
/// V8 isolate; renderer target/session state archives them as completed JSON
/// segments instead.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RendererInspectorProtocolConfiguration {
    pub runtime_bindings: Vec<RuntimeBindingRegistration>,
    pub runtime_frontend_enabled: bool,
    pub console_frontend_enabled: bool,
    pub dom_debugger_event_listener_breakpoints:
        BTreeSet<RendererDomDebuggerEventListenerBreakpoint>,
    pub dom_debugger_xhr_breakpoints: BTreeSet<RendererDomDebuggerXhrBreakpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RendererDomDebuggerEventListenerBreakpoint {
    pub event_name: String,
    pub target_name: String,
}

impl RendererDomDebuggerEventListenerBreakpoint {
    pub fn new(event_name: String, target_name: Option<String>) -> Self {
        let target_name = target_name
            .filter(|target_name| !target_name.is_empty() && target_name != "*")
            .map(|target_name| target_name.to_ascii_lowercase())
            .unwrap_or_else(|| "*".to_owned());
        Self {
            event_name,
            target_name,
        }
    }

    pub fn matches(&self, event_name: &str, target_name: &str) -> bool {
        self.event_name == event_name
            && (self.target_name == "*" || self.target_name.eq_ignore_ascii_case(target_name))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RendererDomDebuggerXhrBreakpoint {
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RendererDomDebuggerDomBreakpointType {
    SubtreeModified,
    AttributeModified,
    NodeRemoved,
}

impl RendererDomDebuggerDomBreakpointType {
    pub fn from_cdp_name(name: &str) -> Option<Self> {
        match name {
            "subtree-modified" => Some(Self::SubtreeModified),
            "attribute-modified" => Some(Self::AttributeModified),
            "node-removed" => Some(Self::NodeRemoved),
            _ => None,
        }
    }

    pub const fn cdp_name(self) -> &'static str {
        match self {
            Self::SubtreeModified => "subtree-modified",
            Self::AttributeModified => "attribute-modified",
            Self::NodeRemoved => "node-removed",
        }
    }
}

impl RendererDomDebuggerXhrBreakpoint {
    pub fn new(url: String) -> Self {
        Self { url }
    }

    pub fn matches(&self, request_url: &str) -> bool {
        request_url.contains(&self.url)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RendererInspectorSessionRestoreSnapshot {
    pub inspector_session_id: Option<String>,
    pub v8_attach: V8InspectorSessionAttach,
    pub protocol_configuration: RendererInspectorProtocolConfiguration,
}

impl RendererInspectorProtocolConfiguration {
    pub fn apply_successful_command(
        &mut self,
        command: RendererInspectorProtocolConfigurationCommand,
    ) {
        match command {
            RendererInspectorProtocolConfigurationCommand::RuntimeEnable => {
                self.runtime_frontend_enabled = true;
            }
            RendererInspectorProtocolConfigurationCommand::RuntimeDisable => {
                self.runtime_frontend_enabled = false;
                self.runtime_bindings.clear();
            }
            RendererInspectorProtocolConfigurationCommand::ConsoleEnable => {
                self.console_frontend_enabled = true;
            }
            RendererInspectorProtocolConfigurationCommand::ConsoleDisable => {
                self.console_frontend_enabled = false;
            }
        }
    }

    pub fn requires_restore(&self) -> bool {
        self.runtime_frontend_enabled
            || !self.runtime_bindings.is_empty()
            || self.console_frontend_enabled
            || !self.dom_debugger_event_listener_breakpoints.is_empty()
            || !self.dom_debugger_xhr_breakpoints.is_empty()
    }

    pub fn set_dom_debugger_event_listener_breakpoint(
        &mut self,
        breakpoint: RendererDomDebuggerEventListenerBreakpoint,
    ) {
        self.dom_debugger_event_listener_breakpoints
            .insert(breakpoint);
    }

    pub fn remove_dom_debugger_event_listener_breakpoint(
        &mut self,
        breakpoint: &RendererDomDebuggerEventListenerBreakpoint,
    ) {
        self.dom_debugger_event_listener_breakpoints
            .remove(breakpoint);
    }

    pub fn set_dom_debugger_xhr_breakpoint(
        &mut self,
        breakpoint: RendererDomDebuggerXhrBreakpoint,
    ) {
        self.dom_debugger_xhr_breakpoints.insert(breakpoint);
    }

    pub fn remove_dom_debugger_xhr_breakpoint(
        &mut self,
        breakpoint: &RendererDomDebuggerXhrBreakpoint,
    ) {
        self.dom_debugger_xhr_breakpoints.remove(breakpoint);
    }
}

/// Renderer-local transition for protocol configuration that V8 does not own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RendererInspectorProtocolConfigurationCommand {
    RuntimeEnable,
    RuntimeDisable,
    ConsoleEnable,
    ConsoleDisable,
}

impl RendererInspectorSessionRestoreSnapshot {
    pub fn requires_backend_restore(&self) -> bool {
        self.v8_attach.is_reattach() || self.protocol_configuration.requires_restore()
    }
}

pub fn renderer_inspector_protocol_configuration_command_from_method(
    method: &str,
    _params: &Value,
) -> Option<RendererInspectorProtocolConfigurationCommand> {
    let command = match method {
        "Runtime.enable" => RendererInspectorProtocolConfigurationCommand::RuntimeEnable,
        "Runtime.disable" => RendererInspectorProtocolConfigurationCommand::RuntimeDisable,
        "Console.enable" => RendererInspectorProtocolConfigurationCommand::ConsoleEnable,
        "Console.disable" => RendererInspectorProtocolConfigurationCommand::ConsoleDisable,
        _ => return None,
    };
    Some(command)
}

pub fn renderer_inspector_protocol_configuration_command_from_message(
    message: &Value,
) -> Option<(u64, RendererInspectorProtocolConfigurationCommand)> {
    let call_id = message.get("id")?.as_u64()?;
    let method = message.get("method")?.as_str()?;
    let params = message.get("params").unwrap_or(&Value::Null);
    renderer_inspector_protocol_configuration_command_from_method(method, params)
        .map(|command| (call_id, command))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn first_attach_and_empty_reattach_are_distinct() {
        let empty_state = V8InspectorSessionState::from_bytes(Vec::new());
        let first_attach = V8InspectorSessionAttach::FirstAttach;
        let empty_reattach = V8InspectorSessionAttach::Reattach(empty_state.clone());

        assert_ne!(first_attach, empty_reattach);
        assert!(first_attach.reattach_state().is_none());
        assert_eq!(empty_reattach.reattach_state(), Some(&empty_state));
        assert!(empty_state.is_empty());
    }

    #[test]
    fn canonical_runtime_disable_clears_protocol_state() {
        let mut restore = RendererInspectorSessionRestoreSnapshot {
            protocol_configuration: RendererInspectorProtocolConfiguration {
                runtime_frontend_enabled: true,
                runtime_bindings: vec![RuntimeBindingRegistration {
                    devtools_session: None,
                    name: "binding".to_owned(),
                    execution_context_name: None,
                }],
                ..Default::default()
            },
            ..RendererInspectorSessionRestoreSnapshot::default()
        };

        restore.protocol_configuration.apply_successful_command(
            RendererInspectorProtocolConfigurationCommand::RuntimeDisable,
        );

        assert!(!restore.protocol_configuration.runtime_frontend_enabled);
        assert!(restore.protocol_configuration.runtime_bindings.is_empty());
    }

    #[test]
    fn protocol_configuration_requires_restore_without_v8_backend_state() {
        let restore = RendererInspectorSessionRestoreSnapshot {
            protocol_configuration: RendererInspectorProtocolConfiguration {
                runtime_bindings: vec![RuntimeBindingRegistration {
                    devtools_session: None,
                    name: "binding".to_owned(),
                    execution_context_name: Some("utility".to_owned()),
                }],
                console_frontend_enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };

        assert_eq!(restore.v8_attach, V8InspectorSessionAttach::FirstAttach);
        assert!(restore.requires_backend_restore());
    }

    #[test]
    fn event_listener_breakpoints_are_canonical_and_require_restore() {
        let wildcard = RendererDomDebuggerEventListenerBreakpoint::new(
            "click".to_owned(),
            Some(String::new()),
        );
        let button = RendererDomDebuggerEventListenerBreakpoint::new(
            "click".to_owned(),
            Some("BUTTON".to_owned()),
        );
        assert_eq!(wildcard.target_name, "*");
        assert_eq!(button.target_name, "button");
        assert!(wildcard.matches("click", "DIV"));
        assert!(button.matches("click", "BUTTON"));
        assert!(!button.matches("click", "DIV"));

        let mut configuration = RendererInspectorProtocolConfiguration::default();
        configuration.set_dom_debugger_event_listener_breakpoint(button.clone());
        configuration.set_dom_debugger_event_listener_breakpoint(button.clone());
        assert_eq!(
            configuration.dom_debugger_event_listener_breakpoints.len(),
            1
        );
        assert!(configuration.requires_restore());
        configuration.remove_dom_debugger_event_listener_breakpoint(&button);
        assert!(!configuration.requires_restore());
    }

    #[test]
    fn xhr_breakpoints_are_canonical_and_require_restore() {
        let match_all = RendererDomDebuggerXhrBreakpoint::new(String::new());
        let needle = RendererDomDebuggerXhrBreakpoint::new("api/items".to_owned());
        assert!(match_all.matches("https://example.test/anything"));
        assert!(needle.matches("https://example.test/api/items?offset=1"));
        assert!(!needle.matches("https://example.test/api/users"));

        let mut configuration = RendererInspectorProtocolConfiguration::default();
        configuration.set_dom_debugger_xhr_breakpoint(needle.clone());
        configuration.set_dom_debugger_xhr_breakpoint(needle.clone());
        assert_eq!(configuration.dom_debugger_xhr_breakpoints.len(), 1);
        assert!(configuration.requires_restore());
        configuration.remove_dom_debugger_xhr_breakpoint(&needle);
        assert!(!configuration.requires_restore());
    }

    #[test]
    fn opaque_empty_reattach_state_still_requires_backend_restore() {
        let restore = RendererInspectorSessionRestoreSnapshot {
            v8_attach: V8InspectorSessionAttach::Reattach(V8InspectorSessionState::from_bytes(
                Vec::new(),
            )),
            ..Default::default()
        };

        assert!(restore.requires_backend_restore());
    }

    #[test]
    fn canonical_parser_covers_every_restorable_inspector_method() {
        let cases = [
            ("Runtime.enable", json!({})),
            ("Runtime.disable", json!({})),
            ("Console.enable", json!({})),
            ("Console.disable", json!({})),
        ];

        for (method, params) in cases {
            assert!(
                renderer_inspector_protocol_configuration_command_from_method(method, &params)
                    .is_some(),
                "{method} must have a canonical restore transition"
            );
        }
        assert!(
            renderer_inspector_protocol_configuration_command_from_method(
                "Runtime.setCustomObjectFormatterEnabled",
                &json!({"enabled": true}),
            )
            .is_none(),
            "opaque V8-owned Runtime configuration must not create typed restore transitions"
        );
        assert!(
            renderer_inspector_protocol_configuration_command_from_method(
                "Profiler.startPreciseCoverage",
                &json!({"callCount": true, "detailed": true}),
            )
            .is_none(),
            "opaque V8-owned Profiler configuration must not create typed restore transitions"
        );
        assert!(
            renderer_inspector_protocol_configuration_command_from_method(
                "HeapProfiler.startSampling",
                &json!({"samplingInterval": 1024, "stackDepth": 32}),
            )
            .is_none(),
            "opaque V8-owned HeapProfiler configuration must not create typed restore transitions"
        );
    }
}
