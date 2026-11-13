use std::{fmt, ops::Deref};

use moli_inspector_json::InspectorJson;
use moli_protocol_cdp::{CdpEventContractError, validate_event_names};
use moli_protocol_cdp_defs::LocalCdpEvent;
use serde_json::{Value, json};

/// Only checked messages can reach the socket queue or acquire a Target shell.
/// The envelope has no mutable access to its method or parameter fields.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CheckedCdpMessage(InspectorJson);

impl CheckedCdpMessage {
    pub(crate) fn try_new(
        message: impl Into<InspectorJson>,
        source: &'static str,
    ) -> Result<Self, CdpEventContractError> {
        let message = message.into();
        validate_event_names(&message, source)?;
        Ok(Self(message))
    }

    pub(crate) fn wrap_target(
        self,
        target_session_id: &str,
        carrier_session_id: Option<&str>,
    ) -> Self {
        let mut outer: InspectorJson = json!({
            "method": LocalCdpEvent::TargetReceivedMessageFromTarget.method(),
            "params": {"sessionId": target_session_id, "message": self.0.to_string()},
        })
        .into();
        outer.set_session_id(carrier_session_id);
        Self(outer)
    }

    pub(crate) fn into_message(self) -> InspectorJson {
        self.0
    }

    pub(crate) fn into_value(self) -> Value {
        self.0.into_value()
    }
}

impl Deref for CheckedCdpMessage {
    type Target = InspectorJson;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for CheckedCdpMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
