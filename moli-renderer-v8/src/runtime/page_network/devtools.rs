use super::*;

impl PageVm {
    pub(crate) fn continue_pending_subresource_fetch(
        &mut self,
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<bytes::Bytes>>,
        headers: Option<moli_fetch::RequestHeaderOverride>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<crate::PendingSubresourceContinueOutcome> {
        let execution = self.vm_mut().continue_pending_subresource_fetch_body(
            internal_id,
            url,
            method,
            body,
            headers,
            intercept_response,
            handle_auth_requests,
        )?;
        self.finish_async_subresource_command(execution)
    }

    pub(crate) fn continue_pending_subresource_auth(
        &mut self,
        internal_id: u64,
        auth: crate::SubresourceAuthCredentials,
    ) -> Result<crate::PendingSubresourceContinueOutcome> {
        let execution = self
            .vm_mut()
            .continue_pending_subresource_auth_body(internal_id, auth)?;
        self.finish_async_subresource_command(execution)
    }

    pub(crate) fn fail_pending_subresource_auth(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<()> {
        let execution = self
            .vm_mut()
            .fail_pending_subresource_auth_body(internal_id, error_text)?;
        self.finish_async_subresource_command(execution)
    }

    pub(crate) fn cancel_pending_subresource_auth(&mut self, internal_id: u64) -> Result<()> {
        let execution = self
            .vm_mut()
            .cancel_pending_subresource_auth_body(internal_id)?;
        self.finish_async_subresource_command(execution)
    }

    pub(crate) fn fail_pending_subresource_fetch(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<()> {
        let execution = self
            .vm_mut()
            .fail_pending_subresource_fetch_body(internal_id, error_text)?;
        self.finish_async_subresource_command(execution)
    }

    pub(crate) fn fulfill_pending_subresource_fetch(
        &mut self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
        request_overrides: Option<moli_fetch::RequestOverrides>,
    ) -> Result<()> {
        let execution = self.vm_mut().fulfill_pending_subresource_fetch_body(
            internal_id,
            response_code,
            response_headers,
            response_body,
            request_overrides,
        )?;
        self.finish_async_subresource_command(execution)
    }

    pub(crate) fn continue_pending_subresource_response(
        &mut self,
        internal_id: u64,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, Vec<u8>)>>,
    ) -> Result<()> {
        let execution = self.vm_mut().continue_pending_subresource_response_body(
            internal_id,
            response_code,
            response_headers,
        )?;
        self.finish_async_subresource_command(execution)
    }

    pub(crate) fn fail_pending_subresource_response(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<()> {
        let execution = self
            .vm_mut()
            .fail_pending_subresource_response_body(internal_id, error_text)?;
        self.finish_async_subresource_command(execution)
    }

    pub(crate) fn fulfill_pending_subresource_response(
        &mut self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> Result<()> {
        let execution = self.vm_mut().fulfill_pending_subresource_response_body(
            internal_id,
            response_code,
            response_headers,
            response_body,
        )?;
        self.finish_async_subresource_command(execution)
    }

    /// Finish one Fetch-interception command after its body has released all
    /// ScriptVm borrows and V8 scopes.
    ///
    /// A command that synchronously entered a Window realm owns one command-end
    /// checkpoint. Worker, WebSocket, and network-only branches do not borrow
    /// that authority merely because they share the same protocol command.
    fn finish_async_subresource_command<T>(
        &mut self,
        execution: crate::script_vm::AsyncSubresourceCommandExecution<T>,
    ) -> Result<T> {
        let (output, activity, post_checkpoint_event) = execution.into_parts();
        if matches!(
            activity,
            crate::script_vm::AsyncSubresourceFetchBodyActivity::WindowRealmEntered
        ) {
            self.vm_mut()
                .finish_async_subresource_command_checkpoint()?;
        }
        if let Some(event) = post_checkpoint_event {
            self.vm_mut().publish_async_subresource_command_event(event);
        }
        Ok(output)
    }

    pub(crate) fn receive_synthetic_websocket_text(&self, socket_id: u64, data: String) -> bool {
        self.vm().receive_synthetic_websocket_text(socket_id, data)
    }

    pub(crate) fn receive_synthetic_websocket_binary(&self, socket_id: u64, data: Vec<u8>) -> bool {
        self.vm()
            .receive_synthetic_websocket_binary(socket_id, data)
    }

    pub(crate) fn close_synthetic_websocket_from_server(
        &self,
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    ) -> bool {
        self.vm()
            .close_synthetic_websocket_from_server(socket_id, code, reason)
    }
}
