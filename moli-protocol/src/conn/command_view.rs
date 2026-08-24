use std::str::FromStr;

use moli_page_types::RendererInspectorResponseDelivery;
use moli_protocol_cdp::{CdpRendererCommandPolicy, ParsedCdpCommand};
use serde::Deserialize;
use serde_json::{Map, Value};
use tracing::debug;

use crate::{
    DevToolsBrowserContextId, DevToolsCommandContext, DevToolsProtocol, DevToolsSessionId,
    DevToolsTargetId,
};

/// Ephemeral view of the current in-flight protocol command.
/// Passed to every domain handler.
pub struct Cmd<'a> {
    pub id: Option<u64>,
    pub method: &'a str,
    /// The part of the method after the first dot, e.g. "getVersion".
    pub action: &'a str,
    /// The validated params object. Missing and explicit `null` both map to
    /// `None`, matching Chromium's empty-params envelope semantics.
    pub params: Option<&'a Map<String, Value>>,
    pub session_id: Option<&'a str>,
    /// Raw JSON of the entire message, useful for inspector passthrough.
    pub json: &'a str,
    /// Renderer scheduling facts derived from the same validated method as
    /// every other field in this view.
    ///
    /// This is private so callers cannot pair one command with another
    /// command's policy. Domain dispatchers consume it through
    /// [`Self::renderer_policy`].
    renderer_policy: CdpRendererCommandPolicy,
    /// Explicit terminal-response destination for a command synthesized by an
    /// internal protocol adapter. Ordinary frontend commands use the domain's
    /// Chromium-compatible default.
    terminal_response_delivery_override: Option<RendererInspectorResponseDelivery>,
}

impl<'a> Cmd<'a> {
    /// Build the only production command view from one validated ingress
    /// command.
    pub(crate) fn from_parsed(command: &'a ParsedCdpCommand) -> Option<Self> {
        let request = command.request();
        let (_, action) = request.method().split_once('.')?;
        Some(Self {
            id: Some(request.id()),
            method: request.method(),
            action,
            params: request.params(),
            session_id: request.session_id(),
            json: command.json(),
            renderer_policy: command.renderer_policy(),
            terminal_response_delivery_override: None,
        })
    }

    pub(crate) fn with_terminal_response_delivery_override(
        mut self,
        response_delivery: Option<RendererInspectorResponseDelivery>,
    ) -> Self {
        self.terminal_response_delivery_override = response_delivery;
        self
    }

    /// Build a low-level domain-test view while keeping method and renderer
    /// policy inseparable.
    ///
    /// Tests may intentionally supply params that are not serialized in
    /// `json`, but `json` must still be a valid command for the same method.
    #[cfg(test)]
    pub(crate) fn for_test(
        id: Option<u64>,
        method: &'a str,
        params: &'a Value,
        session_id: Option<&'a str>,
        json: &'a str,
    ) -> Self {
        let parsed = ParsedCdpCommand::parse_str(json)
            .expect("test command JSON must be a valid CDP command");
        assert_eq!(
            parsed.method(),
            method,
            "test command method must match its serialized command"
        );
        let (_, action) = method
            .split_once('.')
            .expect("test command method must contain a domain separator");
        Self {
            id,
            method,
            action,
            params: match params {
                Value::Null => None,
                Value::Object(params) => Some(params),
                _ => panic!("test command params must be an object or null"),
            },
            session_id,
            json,
            renderer_policy: parsed.renderer_policy(),
            terminal_response_delivery_override: None,
        }
    }

    pub const fn renderer_policy(&self) -> CdpRendererCommandPolicy {
        self.renderer_policy
    }

    pub(crate) fn terminal_response_delivery(
        &self,
        default: RendererInspectorResponseDelivery,
    ) -> RendererInspectorResponseDelivery {
        self.terminal_response_delivery_override.unwrap_or(default)
    }

    /// Build the protocol-neutral command context for shared DevTools command
    /// dispatch. Existing domain handlers still own target/browser-context
    /// resolution; those IDs can be filled by the caller once resolved.
    pub fn devtools_command_context(
        &self,
        target_id: Option<impl Into<DevToolsTargetId>>,
        browser_context_id: Option<impl Into<DevToolsBrowserContextId>>,
    ) -> DevToolsCommandContext {
        DevToolsCommandContext {
            protocol: DevToolsProtocol::Cdp,
            session_id: self.session_id.map(DevToolsSessionId::from),
            target_id: target_id.map(Into::into),
            browser_context_id: browser_context_id.map(Into::into),
        }
    }

    /// Deserialize params into a typed struct. Returns `None` when params is
    /// missing/null; returns an error string on malformed JSON.
    pub fn get_params<T: for<'de> Deserialize<'de>>(&self) -> Result<Option<T>, &'static str> {
        let Some(params) = self.params else {
            return Ok(None);
        };
        serde_path_to_error::deserialize(params)
            .map(Some)
            .map_err(|error| {
                debug!(
                    method = self.method,
                    params_path = %error.path(),
                    error = %error.inner(),
                    "invalid CDP params"
                );
                "InvalidParams"
            })
    }

    /// Parse the action part into a domain-owned enum.
    ///
    /// CDP action names are only meaningful inside their domain: many domains
    /// have their own `enable`, `disable`, or `get*` methods. Keep the raw
    /// action on `Cmd` for protocol passthrough, but let each domain use a
    /// strum-backed enum instead of matching raw strings at every dispatch
    /// branch.
    pub fn parse_action<T: FromStr>(&self) -> Option<T> {
        self.action.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn cdp_cmd_builds_protocol_neutral_command_context() {
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(7),
            "Runtime.evaluate",
            &params,
            Some("session-1"),
            r#"{"id":7,"method":"Runtime.evaluate"}"#,
        );

        let context = cmd.devtools_command_context(Some("target-1"), Some("context-1"));

        assert_eq!(context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            context.session_id.as_ref().map(DevToolsSessionId::as_str),
            Some("session-1")
        );
        assert_eq!(
            context.target_id.as_ref().map(DevToolsTargetId::as_str),
            Some("target-1")
        );
        assert_eq!(
            context
                .browser_context_id
                .as_ref()
                .map(DevToolsBrowserContextId::as_str),
            Some("context-1")
        );
    }

    #[test]
    fn parsed_command_view_keeps_method_action_and_renderer_policy_together() {
        let parsed = ParsedCdpCommand::parse_str(
            r#"{"id":9,"method":"Runtime.evaluate","params":{"expression":"1"}}"#,
        )
        .expect("test Runtime command should parse");

        let cmd = Cmd::from_parsed(&parsed).expect("qualified method should produce a view");

        assert_eq!(cmd.method, "Runtime.evaluate");
        assert_eq!(cmd.action, "evaluate");
        assert_eq!(cmd.renderer_policy(), parsed.renderer_policy());
    }

    #[test]
    #[should_panic(expected = "test command method must match its serialized command")]
    fn test_command_view_rejects_a_policy_from_another_method() {
        let params = Value::Null;
        let _ = Cmd::for_test(
            Some(10),
            "Runtime.enable",
            &params,
            None,
            r#"{"id":10,"method":"Runtime.evaluate"}"#,
        );
    }
}
