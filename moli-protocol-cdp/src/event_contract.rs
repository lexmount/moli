use std::fmt;

use moli_inspector_json::{InspectorEnvelope, InspectorJson};

use crate::is_advertised_event;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CdpEventContractError {
    method: String,
    session_id: Option<String>,
    source: &'static str,
    reason: &'static str,
}

impl CdpEventContractError {
    pub fn method(&self) -> &str {
        &self.method
    }
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
    pub fn source(&self) -> &'static str {
        self.source
    }

    fn new(
        method: &str,
        session_id: Option<&str>,
        source: &'static str,
        reason: &'static str,
    ) -> Self {
        Self {
            method: method.to_owned(),
            session_id: session_id.map(str::to_owned),
            source,
            reason,
        }
    }
}

impl fmt::Display for CdpEventContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} ({})", self.method, self.reason, self.source)
    }
}
impl std::error::Error for CdpEventContractError {}

/// Read only the method envelope. Inspector parameter bytes remain opaque.
pub fn validate_event_name(
    message: &InspectorJson,
    source: &'static str,
) -> Result<(), CdpEventContractError> {
    validate_name(message, message.session_id(), source)
}

fn validate_name(
    message: &InspectorJson,
    session_id: Option<&str>,
    source: &'static str,
) -> Result<(), CdpEventContractError> {
    if !message.has_field("method") {
        return Ok(());
    }
    let method = message.method().ok_or_else(|| {
        CdpEventContractError::new(
            "<invalid event method>",
            session_id,
            source,
            "event method must be a string",
        )
    })?;
    if !is_advertised_event(method) {
        return Err(CdpEventContractError::new(
            method,
            session_id,
            source,
            "event is not advertised by this build",
        ));
    }
    Ok(())
}

/// Validate nested non-flat Target messages before they can be wrapped again.
/// No parameter object is materialized, and the original JSON is not rewritten.
pub fn validate_event_names(
    message: &InspectorJson,
    source: &'static str,
) -> Result<(), CdpEventContractError> {
    let mut next = inspect_message(message, message.session_id(), source)?;
    while let Some((text, session_id)) = next {
        let nested = InspectorEnvelope::from_slice(text.as_bytes()).map_err(|_| {
            CdpEventContractError::new(
                "Target.receivedMessageFromTarget",
                Some(&session_id),
                source,
                "invalid nested JSON message",
            )
        })?;
        let session = nested.session_id().or(Some(session_id.as_str()));
        if let Some(method) = nested.method()
            && !is_advertised_event(method)
        {
            return Err(CdpEventContractError::new(
                method,
                session,
                source,
                "event is not advertised by this build",
            ));
        }
        next = nested_message(nested.method(), session, source, |key| {
            nested.string_param(key)
        })?;
    }
    Ok(())
}

fn inspect_message(
    message: &InspectorJson,
    inherited_session: Option<&str>,
    source: &'static str,
) -> Result<Option<(String, String)>, CdpEventContractError> {
    let session = message.session_id().or(inherited_session);
    validate_name(message, session, source)?;
    nested_message(message.method(), session, source, |key| {
        message.string_param(key)
    })
}

fn nested_message(
    method: Option<&str>,
    session: Option<&str>,
    source: &'static str,
    read_param: impl Fn(&str) -> serde_json::Result<Option<String>>,
) -> Result<Option<(String, String)>, CdpEventContractError> {
    if method != Some("Target.receivedMessageFromTarget") {
        return Ok(None);
    }
    let invalid = || {
        CdpEventContractError::new(
            "Target.receivedMessageFromTarget",
            session,
            source,
            "invalid Target message envelope",
        )
    };
    let session_id = read_param("sessionId")
        .map_err(|_| invalid())?
        .ok_or_else(invalid)?;
    let text = read_param("message")
        .map_err(|_| invalid())?
        .ok_or_else(invalid)?;
    Ok(Some((text, session_id)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_unknown_event_even_in_a_forwarded_domain() {
        let message = json!({"method":"Runtime.someUnknownEvent","sessionId":"SID-1"}).into();
        let error = validate_event_names(&message, "test Inspector").unwrap_err();
        assert_eq!(error.method(), "Runtime.someUnknownEvent");
        assert_eq!(error.session_id(), Some("SID-1"));
    }

    #[test]
    fn validation_preserves_opaque_inspector_parameters() {
        let message = InspectorJson::from_slice(
            br#"{"method":"Runtime.consoleAPICalled","params":{"args":[{"value":"\ud800"}]}}"#,
        )
        .unwrap();
        let before = message.to_string();
        validate_event_names(&message, "V8").unwrap();
        assert!(!message.is_materialized());
        assert_eq!(message.to_string(), before);
    }

    #[test]
    fn nested_target_cannot_hide_an_unadvertised_event() {
        let inner = json!({"method":"Runtime.someUnknownEvent"}).to_string();
        let middle = json!({"method":"Target.receivedMessageFromTarget","params":{"sessionId":"SID-inner","message":inner}}).to_string();
        let message = InspectorJson::from_slice(json!({"method":"Target.receivedMessageFromTarget","params":{"sessionId":"SID-middle","message":middle}}).to_string().as_bytes()).unwrap();
        let error = validate_event_names(&message, "nested test").unwrap_err();
        assert_eq!(error.method(), "Runtime.someUnknownEvent");
        assert_eq!(error.session_id(), Some("SID-inner"));
        assert!(!message.is_materialized());
    }

    #[test]
    fn valid_nested_messages_and_command_replies_are_accepted() {
        for inner in [
            r#"{"id":1,"result":{"value":"\udfff"}}"#,
            r#"{"method":"Page.loadEventFired","params":{"timestamp":1}}"#,
        ] {
            let message = InspectorJson::from_slice(json!({"method":"Target.receivedMessageFromTarget","params":{"sessionId":"SID-1","message":inner}}).to_string().as_bytes()).unwrap();
            let before = message.to_string();
            validate_event_names(&message, "test").unwrap();
            assert!(!message.is_materialized());
            assert_eq!(message.to_string(), before);
        }
    }

    #[test]
    fn nested_host_metadata_does_not_relax_inspector_input_validation() {
        let inner = r#"{"method":"Fetch.requestPaused","params":{"__moliBlockedInterceptors":["intercept-1"]}}"#;
        assert!(InspectorJson::from_slice(inner.as_bytes()).is_err());
        let outer = InspectorJson::from_slice(
            json!({
                "method":"Target.receivedMessageFromTarget",
                "params":{"sessionId":"SID-1","message":inner}
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        validate_event_names(&outer, "host routing").unwrap();
        assert!(!outer.is_materialized());
    }
}
