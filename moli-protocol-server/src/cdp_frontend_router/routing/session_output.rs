use moli_inspector_json::InspectorJson;
use moli_protocol_cdp_defs::LocalCdpEvent;
use serde_json::json;

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

    pub(super) fn encode(&self, message: impl Into<InspectorJson>) -> InspectorJson {
        let mut message = message.into();
        message.set_session_id(self.payload_session_id.as_deref());
        for envelope in self.envelopes.iter().rev() {
            message = json!({"method": LocalCdpEvent::TargetReceivedMessageFromTarget.method(), "params":{
                "sessionId": envelope.target_session_id, "message":message.to_string(),
            }})
            .into();
            message.set_session_id(envelope.carrier_session_id.as_deref());
        }
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn nested_target_routes_preserve_opaque_notifications_and_sessions() {
        let mut route = SessionOutputRoute::flat(Some("SID-root"));
        route.nest("SID-middle");
        route.nest("SID-inner");
        let raw = InspectorJson::from_slice(
            br#"{"method":"Runtime.someUnknownEvent","params":{"args":[{"value":"\udfff"}]}}"#,
        )
        .unwrap();
        let original = raw.to_string();
        let wire = route.encode(raw.clone());
        assert_eq!(wire.session_id(), Some("SID-root"));
        let middle: Value =
            serde_json::from_str(wire.value()["params"]["message"].as_str().unwrap()).unwrap();
        assert_eq!(wire.value()["params"]["sessionId"], "SID-middle");
        assert_eq!(middle["params"]["sessionId"], "SID-inner");
        assert_eq!(middle["params"]["message"], original);
        assert!(!raw.is_materialized());
    }
}
