use moli_protocol::{ParsedCdpCommand, domains::target::target_message_uses_implicit_session};
use serde_json::{Value, json};

use super::super::{CdpFrontendRoutingState, cdp_error_response};
use crate::cdp_frontend_router::CdpPreparedFrontendCommand;

impl CdpFrontendRoutingState {
    /// Legacy Target messages are a transport envelope. Admit their actual
    /// command through the ordinary scheduler, with its own global request id,
    /// renderer policy, navigation gate, and session-local call registration.
    pub(super) fn prepare_target_message(
        &mut self,
        frontend_id: u64,
        mut command: ParsedCdpCommand,
    ) -> Option<CdpPreparedFrontendCommand> {
        let carrier_session_id = command.request().session_id().map(str::to_owned);
        if carrier_session_id
            .as_deref()
            .is_some_and(|id| !self.frontends.owns_client_session(frontend_id, id))
        {
            return self.prepare_command_core(frontend_id, command);
        }
        let base_session_id = self.frontends.base_session_id(frontend_id)?.to_owned();
        let mut route = self
            .frontends
            .session(carrier_session_id.as_deref().unwrap_or(&base_session_id))?
            .output
            .clone();
        let mut nested = false;
        let mut acknowledgements = Vec::new();
        loop {
            if command.method() != "Target.sendMessageToTarget" {
                break;
            }
            let request = command.request();
            let Some(raw) = request
                .params()
                .and_then(|params| params.get("message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
            else {
                break;
            };
            let dispatch_session_id = request.session_id().unwrap_or(&base_session_id);
            let reference = match self.resolve_target_session_reference(
                frontend_id,
                Some(dispatch_session_id),
                request.method(),
                request.params(),
            ) {
                Ok(reference) => reference,
                // The ordinary command boundary produces its precise error.
                Err(_) => break,
            };
            let Some(target_session_id) = reference.or_else(|| {
                request
                    .params()?
                    .get("sessionId")?
                    .as_str()
                    .map(str::to_owned)
            }) else {
                break;
            };
            let ack = json!({"id": request.id(), "result": {}});
            acknowledgements.push(route.encode(ack));
            route.nest(&target_session_id);
            nested = true;
            let parsed = match ParsedCdpCommand::parse_str(raw) {
                Ok(parsed) => parsed,
                Err(error) => {
                    acknowledgements.push(route.encode(cdp_error_response(
                        error.command_id(),
                        error.response_code(),
                        error.response_message(),
                    )));
                    return Some(CdpPreparedFrontendCommand::TargetMessage {
                        frontend_id,
                        acknowledgements,
                        command: None,
                    });
                }
            };
            let payload_session_id = parsed.request().session_id().map(str::to_owned);
            route.set_payload_session_id(payload_session_id.as_deref());
            let id = parsed.request().id();
            let session = payload_session_id.as_deref().or_else(|| {
                target_message_uses_implicit_session(&parsed).then_some(target_session_id.as_str())
            });
            command = match parsed.rewrite_frontend_route(id, session) {
                Ok(command) => command,
                Err(error) => {
                    tracing::error!(
                        ?error,
                        "legacy Target message could not preserve its validated routing envelope"
                    );
                    acknowledgements.push(route.encode(cdp_error_response(
                        Some(id),
                        -32603,
                        "Internal error",
                    )));
                    return Some(CdpPreparedFrontendCommand::TargetMessage {
                        frontend_id,
                        acknowledgements,
                        command: None,
                    });
                }
            };
        }
        let prepared = self.prepare_command_core(frontend_id, command)?;
        if !nested {
            return Some(prepared);
        }
        match prepared {
            CdpPreparedFrontendCommand::Command(command) => {
                self.pending_commands
                    .bind_output_route(command.request().id(), route);
                Some(CdpPreparedFrontendCommand::TargetMessage {
                    frontend_id,
                    acknowledgements,
                    command: Some(command),
                })
            }
            CdpPreparedFrontendCommand::ImmediateResponse { message, .. } => {
                if message.get("sessionId").is_none() {
                    // Unknown payload sessions have no valid reply session.
                    route.set_payload_session_id(None);
                }
                acknowledgements.push(route.encode(message));
                Some(CdpPreparedFrontendCommand::TargetMessage {
                    frontend_id,
                    acknowledgements,
                    command: None,
                })
            }
            CdpPreparedFrontendCommand::TargetMessage { .. } => {
                unreachable!("the command core does not unwrap a transport envelope")
            }
        }
    }
}
