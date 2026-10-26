use anyhow::Context;
use moli_protocol::ParsedCdpCommand;
use moli_protocol_cdp::CdpCommandParseError;
use serde_json::Value;

use super::{
    CdpFrontendRoutingState, cdp_error_response,
    frontend_registry::DirectChildLookupError,
    pending_commands::{CdpCommandFrontend, PendingCommandEffect, PendingCommandRoute},
};
use crate::cdp_frontend_router::CdpPreparedFrontendCommand;

mod target_message;

fn immediate_error_response(
    frontend_id: u64,
    command_id: Option<u64>,
    client_session_id: Option<&str>,
    code: i32,
    error_message: &str,
) -> CdpPreparedFrontendCommand {
    let mut message = cdp_error_response(command_id, code, error_message);
    if let Some(session_id) = client_session_id {
        message["sessionId"] = Value::from(session_id);
    }
    CdpPreparedFrontendCommand::ImmediateResponse {
        frontend_id,
        message,
    }
}

impl CdpFrontendRoutingState {
    #[cfg(test)]
    pub(in crate::cdp_frontend_router) fn prepare_command_str(
        &mut self,
        frontend_id: u64,
        raw: String,
    ) -> Option<CdpPreparedFrontendCommand> {
        self.prepare_parsed_command(frontend_id, ParsedCdpCommand::parse_str(raw))
    }

    pub(in crate::cdp_frontend_router) fn prepare_parsed_command(
        &mut self,
        frontend_id: u64,
        command: Result<ParsedCdpCommand, CdpCommandParseError>,
    ) -> Option<CdpPreparedFrontendCommand> {
        match command {
            Ok(command) => self.prepare_command(frontend_id, command),
            Err(error) => Some(immediate_error_response(
                frontend_id,
                error.command_id(),
                None,
                error.response_code(),
                error.response_message(),
            )),
        }
    }

    pub(super) fn prepare_command(
        &mut self,
        frontend_id: u64,
        command: ParsedCdpCommand,
    ) -> Option<CdpPreparedFrontendCommand> {
        let output = command
            .request()
            .session_id()
            .and_then(|session| self.frontends.session(session))
            .filter(|session| session.frontend_id == frontend_id)
            .map(|session| session.output.clone());
        let mut prepared = if command.method() == "Target.sendMessageToTarget" {
            self.prepare_target_message(frontend_id, command)?
        } else {
            self.prepare_command_core(frontend_id, command)?
        };
        if let Some(output) = output
            && let CdpPreparedFrontendCommand::ImmediateResponse { message, .. } = &mut prepared
            && message.get("sessionId").is_some()
        {
            *message = output.encode(std::mem::take(message));
        }
        Some(prepared)
    }

    fn prepare_command_core(
        &mut self,
        frontend_id: u64,
        command: ParsedCdpCommand,
    ) -> Option<CdpPreparedFrontendCommand> {
        let request = command.request();
        let client_command_id = request.id();
        let method = request.method().to_owned();
        let client_session_id = request.session_id().map(str::to_owned);
        let effect = if method == "Target.attachToTarget" {
            PendingCommandEffect::AttachToTarget {
                target_id: request
                    .params()
                    .and_then(|params| params.get("targetId"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                flattened_protocol: request
                    .params()
                    .and_then(|params| params.get("flatten"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }
        } else {
            PendingCommandEffect::None
        };
        let base_session_id = self.frontends.base_session_id(frontend_id)?.to_owned();
        let dispatch_session_id = if let Some(session_id) = client_session_id.as_deref() {
            if !self.frontends.owns_client_session(frontend_id, session_id) {
                return Some(immediate_error_response(
                    frontend_id,
                    Some(client_command_id),
                    None,
                    -32001,
                    "Unknown sessionId",
                ));
            }
            Some(session_id.to_owned())
        } else {
            Some(base_session_id)
        };
        let target_session_reference = match self.resolve_target_session_reference(
            frontend_id,
            dispatch_session_id.as_deref(),
            &method,
            request.params(),
        ) {
            Ok(session_id) => session_id,
            Err(error) => {
                let code = match error {
                    DirectChildLookupError::MissingSession
                    | DirectChildLookupError::MissingTarget => -32602,
                    DirectChildLookupError::AmbiguousTarget => -32000,
                };
                return Some(immediate_error_response(
                    frontend_id,
                    Some(client_command_id),
                    client_session_id.as_deref(),
                    code,
                    &error.to_string(),
                ));
            }
        };
        let command = if let Some(session_id) = target_session_reference.as_deref() {
            match command.rewrite_target_session_reference(session_id) {
                Ok(command) => command,
                Err(error) => {
                    tracing::error!(
                        ?error,
                        "frontend routing could not serialize a Target session reference"
                    );
                    return Some(immediate_error_response(
                        frontend_id,
                        Some(client_command_id),
                        client_session_id.as_deref(),
                        -32603,
                        "Internal error",
                    ));
                }
            }
        } else {
            command
        };
        let internal_command_id = match self
            .pending_commands
            .allocate_internal_command_id()
            .context("frontend routing could not allocate an internal CDP command id")
        {
            Ok(command_id) => command_id,
            Err(error) => {
                tracing::error!(?error, "frontend routing command id allocation failed");
                return Some(immediate_error_response(
                    frontend_id,
                    Some(client_command_id),
                    client_session_id.as_deref(),
                    -32603,
                    "Internal error",
                ));
            }
        };
        let command = match command
            .rewrite_frontend_route(internal_command_id, dispatch_session_id.as_deref())
        {
            Ok(command) => command,
            Err(error) => {
                tracing::error!(
                    ?error,
                    "frontend routing could not serialize a rewritten typed CDP command"
                );
                return Some(immediate_error_response(
                    frontend_id,
                    Some(client_command_id),
                    client_session_id.as_deref(),
                    -32603,
                    "Internal error",
                ));
            }
        };
        let output = dispatch_session_id
            .as_deref()
            .and_then(|session| self.frontends.session(session))
            .expect("an admitted command has a frontend session")
            .output
            .clone();
        self.pending_commands.insert(
            internal_command_id,
            PendingCommandRoute {
                frontend: CdpCommandFrontend {
                    frontend_id,
                    dispatch_session_id,
                },
                client_command_id,
                effect,
                output,
            },
        );
        Some(CdpPreparedFrontendCommand::Command(command))
    }

    fn resolve_target_session_reference(
        &self,
        frontend_id: u64,
        dispatch_session_id: Option<&str>,
        method: &str,
        params: Option<&serde_json::Map<String, Value>>,
    ) -> std::result::Result<Option<String>, DirectChildLookupError> {
        if !matches!(
            method,
            "Target.detachFromTarget" | "Target.sendMessageToTarget"
        ) {
            return Ok(None);
        }
        let Some(params) = params else {
            return Ok(None);
        };
        if let Some(session_id) = params.get("sessionId") {
            if let Some(session_id) = session_id.as_str() {
                return if self.frontends.owns_direct_child(
                    frontend_id,
                    dispatch_session_id,
                    session_id,
                ) {
                    Ok(None)
                } else {
                    Err(DirectChildLookupError::MissingSession)
                };
            }
            if !session_id.is_null() {
                // Preserve domain validation for a malformed optional value;
                // do not turn it into a valid command via targetId fallback.
                return Ok(None);
            }
        }
        let Some(target_id) = params.get("targetId").and_then(Value::as_str) else {
            return Ok(None);
        };
        self.frontends
            .direct_child_for_target(frontend_id, dispatch_session_id, target_id)
            .map(Some)
    }
}
