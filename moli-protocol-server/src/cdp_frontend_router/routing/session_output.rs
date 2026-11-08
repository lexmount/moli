use serde_json::{Value, json};

use super::set_top_level_session_id;

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

    pub(super) fn encode(&self, mut message: Value) -> Value {
        set_top_level_session_id(&mut message, self.payload_session_id.as_deref());
        for envelope in self.envelopes.iter().rev() {
            message = json!({"method":"Target.receivedMessageFromTarget", "params":{
                "sessionId": envelope.target_session_id, "message":message.to_string(),
            }});
            set_top_level_session_id(&mut message, envelope.carrier_session_id.as_deref());
        }
        message
    }
}
