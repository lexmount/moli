use anyhow::Result;
use moli_protocol_cdp::CdpMessage;
use serde_json::{Value, json};

use crate::cdp_writer::CdpSocketSink;

use self::{frontend_registry::FrontendRegistry, pending_commands::PendingCommandTable};

mod command;
mod frontend_registry;
mod output;
mod pending_commands;
mod session_output;

pub(super) struct CdpRoutedFrontend {
    frontend_id: u64,
    sink: CdpSocketSink,
}

impl CdpRoutedFrontend {
    pub(super) fn frontend_id(&self) -> u64 {
        self.frontend_id
    }

    pub(super) fn enqueue_message(self, message: impl Into<CdpMessage>) -> bool {
        self.sink.enqueue_owned_message(message)
    }

    pub(super) fn into_sink(self) -> CdpSocketSink {
        self.sink
    }
}

#[derive(Default)]
pub(super) struct CdpFrontendRoutingState {
    // The downstream protocol connection is shared, so client command ids and
    // session ownership must never be used as global frontend identities.
    pending_commands: PendingCommandTable,
    frontends: FrontendRegistry,
}

impl CdpFrontendRoutingState {
    pub(super) fn register_browser_frontend(
        &mut self,
        frontend_id: u64,
        session_id: String,
        sink: CdpSocketSink,
    ) -> Result<()> {
        self.frontends
            .register_browser_frontend(frontend_id, session_id, sink)
    }

    pub(super) fn register_target_frontend(
        &mut self,
        frontend_id: u64,
        target_id: String,
        session_id: String,
        sink: CdpSocketSink,
    ) -> Result<()> {
        self.frontends
            .register_target_frontend(frontend_id, target_id, session_id, sink)
    }

    pub(super) fn unregister_browser_frontend(&mut self, frontend_id: u64) -> Option<String> {
        let session_id = self.frontends.unregister_browser_frontend(frontend_id);
        if session_id.is_some() {
            self.pending_commands.remove_frontend(frontend_id);
        }
        session_id
    }

    pub(super) fn unregister_target_frontend(&mut self, frontend_id: u64) -> Option<String> {
        let session_id = self.frontends.unregister_target_frontend(frontend_id);
        if session_id.is_some() {
            self.pending_commands.remove_frontend(frontend_id);
        }
        session_id
    }

    pub(super) fn register_private_session(&mut self, session_id: String) -> Result<()> {
        self.frontends.register_internal_control_session(session_id)
    }

    pub(super) fn unregister_frontends_for_target(&mut self, target_id: &str) {
        for frontend_id in self.frontends.unregister_frontends_for_target(target_id) {
            self.pending_commands.remove_frontend(frontend_id);
        }
    }

    pub(super) fn frontend_by_id(&self, frontend_id: u64) -> Option<CdpRoutedFrontend> {
        self.frontends
            .frontend_sink(frontend_id)
            .map(|sink| CdpRoutedFrontend { frontend_id, sink })
    }
}

fn cdp_error_response(id: Option<u64>, code: i32, message: &str) -> Value {
    json!({
        "id": id.map(Value::from).unwrap_or(Value::Null),
        "error": {
            "code": code,
            "message": message,
        },
    })
}

#[cfg(test)]
mod tests;
