use std::{cell::Cell, ops::Deref};

use moli_page_types::RendererDevToolsAgentToken;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    devtools::pause::RendererInspectorPauseBridge, inspector_session::InspectorSessionOutput,
};

/// A document's V8 session, with a temporary replacement pause policy.
/// All methods run on the isolate owner; the shared bridge only requests work.
/// V8 15.2 stores this setting in restore state, so retain the frontend value
/// while the outgoing document temporarily skips pauses.
pub(in crate::script_vm) struct RendererInspectorSession {
    v8: v8::inspector::V8InspectorSession,
    document: Option<(RendererInspectorPauseBridge, RendererDevToolsAgentToken)>,
    frontend_skip_all_pauses: Cell<Option<bool>>,
}

impl Deref for RendererInspectorSession {
    type Target = v8::inspector::V8InspectorSession;

    fn deref(&self) -> &Self::Target {
        &self.v8
    }
}

impl RendererInspectorSession {
    pub(super) fn new(
        v8: v8::inspector::V8InspectorSession,
        document: Option<(RendererInspectorPauseBridge, RendererDevToolsAgentToken)>,
    ) -> Self {
        let session = Self {
            v8,
            document,
            frontend_skip_all_pauses: Cell::new(None),
        };
        session.sync_document_replacement();
        session
    }

    pub(super) fn sync_document_replacement(&self) {
        let replacing = self
            .document
            .as_ref()
            .is_some_and(|(bridge, agent)| bridge.document_replacement_active(*agent));
        match (replacing, self.frontend_skip_all_pauses.get()) {
            (true, None) => {
                self.frontend_skip_all_pauses
                    .set(Some(self.skip_all_pauses()));
                self.v8.set_skip_all_pauses(true);
            }
            (false, Some(previous)) => {
                self.v8.set_skip_all_pauses(previous);
                self.frontend_skip_all_pauses.set(None);
            }
            _ => {}
        }
    }

    /// Export V8's own bytes with the frontend policy, never the temporary
    /// replacement policy. The setter and serialization cannot execute JS.
    pub(in crate::script_vm) fn state(&self) -> Vec<u8> {
        self.sync_document_replacement();
        if let Some(previous) = self.frontend_skip_all_pauses.get() {
            self.v8.set_skip_all_pauses(previous);
            let state = self.v8.state();
            self.v8.set_skip_all_pauses(true);
            state
        } else {
            self.v8.state()
        }
    }

    pub(in crate::script_vm) fn dispatch_protocol_message(
        &self,
        message: v8::inspector::StringView,
    ) {
        self.sync_document_replacement();
        if let Some(previous) = self.frontend_skip_all_pauses.get()
            && moli_v8_util::decode_inspector_protocol_message(message)
                .is_ok_and(|message| changes_pause_policy(&message))
        {
            // These synchronous settings do not run JS. Let V8 validate and
            // apply the command, then retain its resulting frontend policy.
            self.frontend_skip_all_pauses.set(None);
            self.v8.set_skip_all_pauses(previous);
        }
        self.v8.dispatch_protocol_message(message);
        self.sync_document_replacement();
    }

    pub(in crate::script_vm) fn dispatch_with_runtime_defaults(
        &self,
        raw_json: &str,
        output: &impl InspectorSessionOutput,
    ) -> Result<(), String> {
        crate::inspector_session::dispatch_with_runtime_defaults(
            &self.v8,
            raw_json,
            output,
            |message| self.dispatch_protocol_message(message),
        )
    }

    fn skip_all_pauses(&self) -> bool {
        #[derive(Deserialize)]
        struct SessionState {
            #[serde(rename = "Debugger")]
            debugger: DebuggerState,
        }
        #[derive(Deserialize)]
        struct DebuggerState {
            #[serde(default, rename = "skipAllPauses")]
            skip_all_pauses: bool,
        }

        // The public V8 API has a setter but no getter. Read only this setting
        // when entering a replacement scope, as for our Runtime.enable defaults.
        // Restore snapshots remain unmodified bytes serialized by V8 itself.
        let json = v8::crdtp::cbor_to_json(&self.v8.state())
            .expect("V8 Inspector session state must be valid CBOR");
        serde_json::from_slice::<SessionState>(&json)
            .expect("V8 Inspector session state must contain Debugger policy")
            .debugger
            .skip_all_pauses
    }
}

fn changes_pause_policy(message: &Value) -> bool {
    matches!(
        message.get("method").and_then(Value::as_str),
        Some("Debugger.setSkipAllPauses" | "Debugger.disable")
    )
}

#[cfg(test)]
mod tests;
