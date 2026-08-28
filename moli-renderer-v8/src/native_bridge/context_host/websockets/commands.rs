use super::super::JsContextHost;
use moli_websocket::Command as WebSocketCommand;

impl JsContextHost {
    pub(crate) fn signal_websocket_stream_pull(&self, socket_id: u64) {
        self.page_websocket_sender().signal_readable_pull(socket_id);
    }

    pub(crate) fn send_websocket_text(&self, socket_id: u64, text: String) -> bool {
        self.websockets
            .get(&socket_id)
            .and_then(|state| state.command_tx.as_ref())
            .map(|command_tx| command_tx.send(WebSocketCommand::SendText(text)).is_ok())
            .unwrap_or(false)
    }

    pub(crate) fn send_websocket_binary(&self, socket_id: u64, bytes: Vec<u8>) -> bool {
        self.websockets
            .get(&socket_id)
            .and_then(|state| state.command_tx.as_ref())
            .map(|command_tx| command_tx.send(WebSocketCommand::SendBinary(bytes)).is_ok())
            .unwrap_or(false)
    }

    pub(crate) fn close_websocket(
        &self,
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    ) -> bool {
        self.websockets
            .get(&socket_id)
            .and_then(|state| state.command_tx.as_ref())
            .map(|command_tx| {
                command_tx
                    .send(WebSocketCommand::Close { code, reason })
                    .is_ok()
            })
            .unwrap_or(false)
    }

    pub(crate) fn receive_synthetic_websocket_text(&self, socket_id: u64, data: String) -> bool {
        self.websockets
            .get(&socket_id)
            .filter(|state| state.synthetic)
            .and_then(|state| state.command_tx.as_ref())
            .map(|command_tx| command_tx.send(WebSocketCommand::ReceiveText(data)).is_ok())
            .unwrap_or(false)
    }

    pub(crate) fn receive_synthetic_websocket_binary(&self, socket_id: u64, data: Vec<u8>) -> bool {
        self.websockets
            .get(&socket_id)
            .filter(|state| state.synthetic)
            .and_then(|state| state.command_tx.as_ref())
            .map(|command_tx| {
                command_tx
                    .send(WebSocketCommand::ReceiveBinary(data))
                    .is_ok()
            })
            .unwrap_or(false)
    }

    pub(crate) fn close_synthetic_websocket_from_server(
        &self,
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    ) -> bool {
        self.websockets
            .get(&socket_id)
            .filter(|state| state.synthetic)
            .and_then(|state| state.command_tx.as_ref())
            .map(|command_tx| {
                command_tx
                    .send(WebSocketCommand::ServerClose { code, reason })
                    .is_ok()
            })
            .unwrap_or(false)
    }

    pub(crate) fn websocket_dispatch_target<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        socket_id: u64,
    ) -> Option<(
        super::super::OwnerDispatchScope,
        super::super::RuntimeObservableContextToken,
        v8::Local<'s, v8::Context>,
        v8::Local<'s, v8::Object>,
    )> {
        let stale_owner = self
            .websockets
            .get(&socket_id)
            .map(|state| (state.owner.owner(), state.owner.dispatch_scope()))
            .filter(|(owner, dispatch_scope)| {
                !self.window_execution_context_owner_is_current(*owner, *dispatch_scope)
            });
        if let Some((owner, dispatch_scope)) = stale_owner {
            self.retire_websocket(socket_id);
            tracing::debug!(
                socket_id,
                ?owner,
                ?dispatch_scope,
                "dropped WebSocket event for retired execution context"
            );
            return None;
        }
        let state = self.websockets.get(&socket_id)?;
        Some((
            state.owner.dispatch_scope(),
            state.owner.realm_token(),
            state.owner.context(scope),
            v8::Local::new(scope, &state.wrapper),
        ))
    }

    pub(crate) fn forget_websocket(&mut self, socket_id: u64) {
        self.websockets.remove(&socket_id);
    }

    pub(crate) fn retire_websockets_for_execution_context_owner(
        &mut self,
        owner: super::super::WindowExecutionContextOwner,
    ) -> usize {
        let socket_ids = self
            .websockets
            .iter()
            .filter_map(|(socket_id, state)| (state.owner.owner() == owner).then_some(*socket_id))
            .collect::<Vec<_>>();
        let retired = socket_ids.len();
        for socket_id in socket_ids {
            self.retire_websocket(socket_id);
        }
        retired
    }

    pub(crate) fn retire_websockets_for_context_token(
        &mut self,
        context_token: super::super::RuntimeObservableContextToken,
    ) -> usize {
        let socket_ids = self
            .websockets
            .iter()
            .filter_map(|(socket_id, state)| {
                (state.owner.realm_token() == context_token).then_some(*socket_id)
            })
            .collect::<Vec<_>>();
        let retired = socket_ids.len();
        for socket_id in socket_ids {
            self.retire_websocket(socket_id);
        }
        retired
    }

    pub(crate) fn retire_websocket(&mut self, socket_id: u64) -> bool {
        let Some(state) = self.websockets.remove(&socket_id) else {
            return false;
        };
        if let Some(command_tx) = state.command_tx {
            let _ = command_tx.send(WebSocketCommand::Close {
                code: None,
                reason: String::new(),
            });
        }
        let internal_ids = [state.fetch_internal_id, state.response_interception_pending]
            .into_iter()
            .flatten()
            .collect();
        let retired_subresource_count =
            self.retire_websocket_subresource_fetches(socket_id, internal_ids);
        tracing::debug!(
            socket_id,
            owner = ?state.owner.owner(),
            retired_subresource_count,
            "retired WebSocket with execution context"
        );
        true
    }
}
