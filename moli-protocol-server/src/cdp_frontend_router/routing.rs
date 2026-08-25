use anyhow::Result;
use serde_json::{Value, json};

use crate::cdp_writer::CdpSocketSink;

use self::{frontend_registry::FrontendRegistry, pending_commands::PendingCommandTable};

mod command;
mod frontend_registry;
mod output;
mod pending_commands;

pub(crate) use frontend_registry::ForegroundTabFrontendRoute;

pub(super) struct CdpRoutedFrontend {
    frontend_id: u64,
    sink: CdpSocketSink,
}

impl CdpRoutedFrontend {
    pub(super) fn frontend_id(&self) -> u64 {
        self.frontend_id
    }

    pub(super) fn enqueue_message(self, message: Value) -> bool {
        self.sink.enqueue_owned_message(message)
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

    pub(super) fn register_page_frontend(
        &mut self,
        frontend_id: u64,
        target_id: String,
        session_id: String,
        sink: CdpSocketSink,
    ) -> Result<()> {
        self.frontends
            .register_page_frontend(frontend_id, target_id, session_id, sink)
    }

    pub(super) fn register_foreground_tab_frontend(
        &mut self,
        frontend_id: u64,
        browser_context_id: Option<String>,
        page_target_id: String,
        session_id: String,
        sink: CdpSocketSink,
    ) -> Result<()> {
        self.frontends.register_foreground_tab_frontend(
            frontend_id,
            browser_context_id,
            page_target_id,
            session_id,
            sink,
        )
    }

    pub(super) fn unregister_browser_frontend(&mut self, frontend_id: u64) -> Option<String> {
        let session_id = self.frontends.unregister_browser_frontend(frontend_id);
        if session_id.is_some() {
            self.pending_commands.remove_frontend(frontend_id);
        }
        session_id
    }

    pub(super) fn unregister_page_frontend(&mut self, frontend_id: u64) -> Option<String> {
        let session_id = self.frontends.unregister_page_frontend(frontend_id);
        if session_id.is_some() {
            self.pending_commands.remove_frontend(frontend_id);
        }
        session_id
    }

    pub(super) fn unregister_foreground_tab_frontend(
        &mut self,
        frontend_id: u64,
    ) -> Option<String> {
        let session_id = self
            .frontends
            .unregister_foreground_tab_frontend(frontend_id);
        if session_id.is_some() {
            self.pending_commands.remove_frontend(frontend_id);
        }
        session_id
    }

    pub(super) fn foreground_tab_frontends_for_browser_context(
        &self,
        browser_context_id: Option<&str>,
    ) -> Vec<ForegroundTabFrontendRoute> {
        self.frontends
            .foreground_tab_frontends_for_browser_context(browser_context_id)
    }

    pub(super) fn replace_foreground_tab_frontend_base(
        &mut self,
        frontend_id: u64,
        browser_context_id: Option<String>,
        page_target_id: String,
        session_id: String,
    ) -> Result<String> {
        self.frontends.replace_foreground_tab_frontend_base(
            frontend_id,
            browser_context_id,
            page_target_id,
            session_id,
        )
    }

    pub(super) fn register_private_session(&mut self, session_id: String) -> Result<()> {
        self.frontends.register_internal_control_session(session_id)
    }

    pub(super) fn unregister_page_frontends_for_target(&mut self, target_id: &str) {
        for frontend_id in self
            .frontends
            .unregister_page_frontends_for_target(target_id)
        {
            self.pending_commands.remove_frontend(frontend_id);
        }
    }

    pub(super) fn frontend_by_id(&self, frontend_id: u64) -> Option<CdpRoutedFrontend> {
        self.frontends
            .frontend_sink(frontend_id)
            .map(|sink| CdpRoutedFrontend { frontend_id, sink })
    }
}

fn remove_top_level_session_id(message: &mut Value) {
    if let Some(object) = message.as_object_mut() {
        object.remove("sessionId");
    }
}

fn set_top_level_session_id(message: &mut Value, session_id: Option<&str>) {
    if let Some(session_id) = session_id {
        message["sessionId"] = json!(session_id);
    } else {
        remove_top_level_session_id(message);
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
