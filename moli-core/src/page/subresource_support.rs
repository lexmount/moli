use anyhow::Result;
use url::Url;

use super::protocol_support::{PendingSubresourceContinueOutcome, SubresourceAuthCredentials};
use super::{CompletedPageCommand, Page, PendingPageCommand};
use super::{RendererPendingJavaScriptDialog, RendererSyntheticResponseBody};
use crate::RendererOutputFence;
use crate::renderer::{RendererPageCommand, RendererPageReply};

impl Page {
    pub fn has_open_modal_javascript_dialog(&self) -> bool {
        self.handle.has_open_modal_javascript_dialog()
    }

    pub fn take_pending_modal_javascript_dialogs(&self) -> Vec<RendererPendingJavaScriptDialog> {
        self.handle.take_pending_modal_javascript_dialogs()
    }

    pub async fn stop_document_lifecycle_async(&mut self) -> Result<()> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::StopDocumentLifecycle)
            .await?;
        expect_page_reply!(
            reply,
            "stop document lifecycle page command",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub async fn continue_pending_subresource_fetch_async(
        &mut self,
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<String>>,
        headers: Option<Vec<(String, String)>>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<PendingSubresourceContinueOutcome> {
        let pending = self.start_continue_pending_subresource_fetch(
            internal_id,
            url,
            method,
            body,
            headers,
            intercept_response,
            handle_auth_requests,
        )?;
        self.finish_continue_pending_subresource_fetch(pending.wait().await?)
    }

    pub fn start_continue_pending_subresource_fetch(
        &self,
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<String>>,
        headers: Option<Vec<(String, String)>>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::ContinuePendingSubresourceFetch {
            internal_id,
            url,
            method,
            body,
            headers,
            intercept_response,
            handle_auth_requests,
        })
    }

    pub fn finish_continue_pending_subresource_fetch(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<PendingSubresourceContinueOutcome> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "continue subresource fetch page command",
            "a continue outcome reply",
            RendererPageReply::PendingSubresourceContinueOutcome(outcome) => Ok(outcome),
        )
    }

    pub async fn continue_pending_subresource_auth_async(
        &mut self,
        internal_id: u64,
        auth: SubresourceAuthCredentials,
    ) -> Result<PendingSubresourceContinueOutcome> {
        let pending = self.start_continue_pending_subresource_auth(internal_id, auth)?;
        self.finish_continue_pending_subresource_auth(pending.wait().await?)
    }

    pub fn start_continue_pending_subresource_auth(
        &self,
        internal_id: u64,
        auth: SubresourceAuthCredentials,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::ContinuePendingSubresourceAuth {
            internal_id,
            auth,
        })
    }

    pub fn finish_continue_pending_subresource_auth(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<PendingSubresourceContinueOutcome> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "continue subresource auth page command",
            "a continue outcome reply",
            RendererPageReply::PendingSubresourceContinueOutcome(outcome) => Ok(outcome),
        )
    }

    pub async fn fail_pending_subresource_auth_async(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<Option<RendererOutputFence>> {
        let pending = self.start_fail_pending_subresource_auth(internal_id, error_text)?;
        self.finish_fail_pending_subresource_auth(pending.wait().await?)
    }

    pub fn start_cancel_pending_subresource_auth(
        &self,
        internal_id: u64,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::CancelPendingSubresourceAuth { internal_id })
    }

    pub fn finish_cancel_pending_subresource_auth(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<RendererOutputFence>> {
        let output = self.finish_page_command_turn(completion);
        let (completion, predecessor) = output.into_completion_and_predecessor();
        let (reply, _, _) = completion.into_parts();
        expect_page_reply!(
            reply,
            "cancel subresource auth",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )?;
        Ok(predecessor)
    }

    pub fn start_fail_pending_subresource_auth(
        &self,
        internal_id: u64,
        error_text: String,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::FailPendingSubresourceAuth {
            internal_id,
            error_text,
        })
    }

    pub fn finish_fail_pending_subresource_auth(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<RendererOutputFence>> {
        let output = self.finish_page_command_turn(completion);
        let (completion, predecessor) = output.into_completion_and_predecessor();
        let (reply, _, _) = completion.into_parts();
        expect_page_reply!(
            reply,
            "fail subresource auth",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )?;
        Ok(predecessor)
    }

    pub async fn fail_pending_subresource_fetch_async(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<Option<RendererOutputFence>> {
        let pending = self.start_fail_pending_subresource_fetch(internal_id, error_text)?;
        self.finish_fail_pending_subresource_fetch(pending.wait().await?)
    }

    pub fn start_fail_pending_subresource_fetch(
        &self,
        internal_id: u64,
        error_text: String,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::FailPendingSubresourceFetch {
            internal_id,
            error_text,
        })
    }

    pub fn finish_fail_pending_subresource_fetch(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<RendererOutputFence>> {
        let output = self.finish_page_command_turn(completion);
        let (completion, predecessor) = output.into_completion_and_predecessor();
        let (reply, _, _) = completion.into_parts();
        expect_page_reply!(
            reply,
            "fail subresource fetch",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )?;
        Ok(predecessor)
    }

    pub async fn fulfill_pending_subresource_fetch_async(
        &mut self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> Result<()> {
        let pending = self.start_fulfill_pending_subresource_fetch(
            internal_id,
            response_code,
            response_headers,
            response_body,
        )?;
        self.finish_fulfill_pending_subresource_fetch(pending.wait().await?)
    }

    pub fn start_fulfill_pending_subresource_fetch(
        &self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::FulfillPendingSubresourceFetch {
            internal_id,
            response_code,
            response_headers,
            response_body,
        })
    }

    pub fn finish_fulfill_pending_subresource_fetch(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "fulfill subresource fetch",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub async fn continue_pending_subresource_response_async(
        &mut self,
        internal_id: u64,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    ) -> Result<()> {
        let pending = self.start_continue_pending_subresource_response(
            internal_id,
            response_code,
            response_headers,
        )?;
        self.finish_continue_pending_subresource_response(pending.wait().await?)
    }

    pub fn start_continue_pending_subresource_response(
        &self,
        internal_id: u64,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::ContinuePendingSubresourceResponse {
            internal_id,
            response_code,
            response_headers,
        })
    }

    pub fn finish_continue_pending_subresource_response(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "continue subresource response",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub async fn fail_pending_subresource_response_async(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<Option<RendererOutputFence>> {
        let pending = self.start_fail_pending_subresource_response(internal_id, error_text)?;
        self.finish_fail_pending_subresource_response(pending.wait().await?)
    }

    pub fn start_fail_pending_subresource_response(
        &self,
        internal_id: u64,
        error_text: String,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::FailPendingSubresourceResponse {
            internal_id,
            error_text,
        })
    }

    pub fn finish_fail_pending_subresource_response(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<RendererOutputFence>> {
        let output = self.finish_page_command_turn(completion);
        let (completion, predecessor) = output.into_completion_and_predecessor();
        let (reply, _, _) = completion.into_parts();
        expect_page_reply!(
            reply,
            "fail subresource response",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )?;
        Ok(predecessor)
    }

    pub async fn fulfill_pending_subresource_response_async(
        &mut self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> Result<()> {
        let pending = self.start_fulfill_pending_subresource_response(
            internal_id,
            response_code,
            response_headers,
            response_body,
        )?;
        self.finish_fulfill_pending_subresource_response(pending.wait().await?)
    }

    pub fn start_fulfill_pending_subresource_response(
        &self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::FulfillPendingSubresourceResponse {
            internal_id,
            response_code,
            response_headers,
            response_body,
        })
    }

    pub fn finish_fulfill_pending_subresource_response(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "fulfill subresource response",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub async fn receive_synthetic_websocket_text_async(
        &mut self,
        socket_id: u64,
        data: String,
    ) -> Result<()> {
        let pending = self.start_receive_synthetic_websocket_text(socket_id, data)?;
        self.finish_receive_synthetic_websocket_text(pending.wait().await?)
    }

    pub fn start_receive_synthetic_websocket_text(
        &self,
        socket_id: u64,
        data: String,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::ReceiveSyntheticWebSocketText {
            socket_id,
            data,
        })
    }

    pub fn finish_receive_synthetic_websocket_text(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "receive synthetic websocket text",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub async fn receive_synthetic_websocket_binary_async(
        &mut self,
        socket_id: u64,
        data: Vec<u8>,
    ) -> Result<()> {
        let pending = self.start_receive_synthetic_websocket_binary(socket_id, data)?;
        self.finish_receive_synthetic_websocket_binary(pending.wait().await?)
    }

    pub fn start_receive_synthetic_websocket_binary(
        &self,
        socket_id: u64,
        data: Vec<u8>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::ReceiveSyntheticWebSocketBinary {
            socket_id,
            data,
        })
    }

    pub fn finish_receive_synthetic_websocket_binary(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "receive synthetic websocket binary",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub async fn close_synthetic_websocket_from_server_async(
        &mut self,
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    ) -> Result<()> {
        let pending = self.start_close_synthetic_websocket_from_server(socket_id, code, reason)?;
        self.finish_close_synthetic_websocket_from_server(pending.wait().await?)
    }

    pub fn start_close_synthetic_websocket_from_server(
        &self,
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::CloseSyntheticWebSocketFromServer {
            socket_id,
            code,
            reason,
        })
    }

    pub fn finish_close_synthetic_websocket_from_server(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "close synthetic websocket from server",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }
}
