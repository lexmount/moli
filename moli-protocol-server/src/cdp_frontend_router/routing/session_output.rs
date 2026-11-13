use crate::cdp_writer::CheckedCdpMessage;
use moli_inspector_json::InspectorJson;
use moli_protocol_cdp::CdpEventContractError;

/// A session's complete path back to its WebSocket, also captured by pending
/// replies so detachment cannot lose an already admitted command's terminal.
#[derive(Clone, Default)]
pub(super) struct SessionOutputRoute {
    payload_session_id: Option<String>,
    envelopes: Vec<TargetMessageEnvelope>,
}

#[derive(Clone)]
struct TargetMessageEnvelope {
    target_session_id: String,
    carrier_session_id: Option<String>,
}

impl SessionOutputRoute {
    pub(super) fn flat(session_id: Option<&str>) -> Self {
        Self {
            payload_session_id: session_id.map(str::to_owned),
            ..Self::default()
        }
    }

    pub(super) fn nest(&mut self, session_id: &str) {
        self.envelopes.push(TargetMessageEnvelope {
            target_session_id: session_id.to_owned(),
            carrier_session_id: self.payload_session_id.take(),
        });
    }

    pub(super) fn set_payload_session_id(&mut self, session_id: Option<&str>) {
        self.payload_session_id = session_id.map(str::to_owned);
    }

    pub(super) fn encode(
        &self,
        message: impl Into<InspectorJson>,
    ) -> Result<CheckedCdpMessage, CdpEventContractError> {
        let mut message = message.into();
        message.set_session_id(self.payload_session_id.as_deref());
        let mut message = CheckedCdpMessage::try_new(message, "session routing")?;
        for envelope in self.envelopes.iter().rev() {
            message = message.wrap_target(
                &envelope.target_session_id,
                envelope.carrier_session_id.as_deref(),
            );
        }
        Ok(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn checks_unknown_notifications_before_adding_any_target_shell() {
        let mut route = SessionOutputRoute::flat(Some("SID-root"));
        route.nest("SID-child");
        let error = route
            .encode(json!({"method":"Runtime.someUnknownEvent"}))
            .unwrap_err();
        assert_eq!(error.method(), "Runtime.someUnknownEvent");
    }

    #[test]
    fn checks_preexisting_nested_messages_before_adding_ancestors() {
        let mut route = SessionOutputRoute::flat(None);
        route.nest("SID-outer");
        let error = route
            .encode(json!({
                "method":"Target.receivedMessageFromTarget", "params":{
                    "sessionId":"SID-inner",
                    "message":r#"{"method":"Runtime.someUnknownEvent"}"#,
                },
            }))
            .unwrap_err();
        assert_eq!(error.method(), "Runtime.someUnknownEvent");
        assert_eq!(error.session_id(), Some("SID-inner"));
    }

    #[test]
    fn nested_target_routes_preserve_raw_utf16_and_session_envelopes() {
        let mut route = SessionOutputRoute::flat(Some("SID-root"));
        route.nest("SID-middle");
        route.nest("SID-inner");
        let raw = InspectorJson::from_slice(
            br#"{"method":"Runtime.consoleAPICalled","params":{"args":[{"value":"\udfff"}]}}"#,
        )
        .unwrap();
        let original = raw.to_string();
        let wire = route.encode(raw.clone()).unwrap();
        assert_eq!(wire.session_id(), Some("SID-root"));
        let middle: Value =
            serde_json::from_str(wire.value()["params"]["message"].as_str().unwrap()).unwrap();
        assert_eq!(wire.value()["params"]["sessionId"], "SID-middle");
        assert_eq!(middle["params"]["sessionId"], "SID-inner");
        assert_eq!(middle["params"]["message"], original);
        assert!(!raw.is_materialized());
    }
}
