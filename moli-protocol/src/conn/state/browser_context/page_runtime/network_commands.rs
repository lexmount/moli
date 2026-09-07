use super::BrowserContext;
use moli_core::page::{
    CompletedPageCommand, PendingPageCommand, PendingSubresourceContinueOutcome,
    RendererSyntheticResponseBody, SubresourceAuthCredentials,
};
use url::Url;

impl BrowserContext {
    pub(crate) fn start_continue_pending_subresource_fetch_for_target(
        &self,
        target_id: &str,
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<String>>,
        headers: Option<Vec<(String, String)>>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_continue_pending_subresource_fetch(
                internal_id,
                url,
                method,
                body,
                headers,
                intercept_response,
                handle_auth_requests,
            )
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_continue_pending_subresource_auth_for_target(
        &self,
        target_id: &str,
        internal_id: u64,
        auth: SubresourceAuthCredentials,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_continue_pending_subresource_auth(internal_id, auth)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_cancel_pending_subresource_auth_for_target(
        &self,
        target_id: &str,
        internal_id: u64,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_cancel_pending_subresource_auth(internal_id)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_fail_pending_subresource_auth_for_target(
        &self,
        target_id: &str,
        internal_id: u64,
        error_text: String,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_fail_pending_subresource_auth(internal_id, error_text)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_fail_pending_subresource_fetch_for_target(
        &self,
        target_id: &str,
        internal_id: u64,
        error_text: String,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_fail_pending_subresource_fetch(internal_id, error_text)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_fail_pending_subresource_response_for_target(
        &self,
        target_id: &str,
        internal_id: u64,
        error_text: String,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_fail_pending_subresource_response(internal_id, error_text)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_fulfill_pending_subresource_fetch_for_target(
        &self,
        target_id: &str,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_fulfill_pending_subresource_fetch(
                internal_id,
                response_code,
                response_headers,
                response_body,
            )
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_fulfill_pending_subresource_response_for_target(
        &self,
        target_id: &str,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_fulfill_pending_subresource_response(
                internal_id,
                response_code,
                response_headers,
                response_body,
            )
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_continue_pending_subresource_response_for_target(
        &self,
        target_id: &str,
        internal_id: u64,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_continue_pending_subresource_response(
                internal_id,
                response_code,
                response_headers,
            )
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_receive_synthetic_websocket_text_for_target(
        &self,
        target_id: &str,
        socket_id: u64,
        data: String,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_receive_synthetic_websocket_text(socket_id, data)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_receive_synthetic_websocket_binary_for_target(
        &self,
        target_id: &str,
        socket_id: u64,
        data: Vec<u8>,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_receive_synthetic_websocket_binary(socket_id, data)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_close_synthetic_websocket_from_server_for_target(
        &self,
        target_id: &str,
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_close_synthetic_websocket_from_server(socket_id, code, reason)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_continue_pending_subresource_fetch_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<PendingSubresourceContinueOutcome, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_continue_pending_subresource_fetch(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_continue_pending_subresource_auth_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<PendingSubresourceContinueOutcome, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_continue_pending_subresource_auth(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_receive_synthetic_websocket_text_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_receive_synthetic_websocket_text(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_receive_synthetic_websocket_binary_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_receive_synthetic_websocket_binary(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_close_synthetic_websocket_from_server_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_close_synthetic_websocket_from_server(completion)
            .map_err(|error| error.to_string())
    }
    pub(crate) fn finish_target_subresource_auth_terminal(
        &mut self,
        target_id: &str,
        completion: moli_core::page::CompletedPageCommand,
        expose_challenged_response: bool,
    ) -> Result<(), String> {
        let page = self
            .loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?;
        let result = if expose_challenged_response {
            page.finish_cancel_pending_subresource_auth(completion)
        } else {
            page.finish_fail_pending_subresource_auth(completion)
        };
        result.map(|_| ()).map_err(|error| error.to_string())
    }
    pub(crate) fn finish_fail_pending_subresource_fetch_for_target(
        &mut self,
        target_id: &str,
        completion: moli_core::page::CompletedPageCommand,
    ) -> Result<(), String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_fail_pending_subresource_fetch(completion)
            .map(|_| ())
            .map_err(|error| format!("subresource fetch fail failed: {error}"))
    }

    pub(crate) fn finish_fail_pending_subresource_response_for_target(
        &mut self,
        target_id: &str,
        completion: moli_core::page::CompletedPageCommand,
    ) -> Result<(), String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_fail_pending_subresource_response(completion)
            .map(|_| ())
            .map_err(|error| format!("subresource response fail failed: {error}"))
    }

    pub(crate) fn finish_fulfill_pending_subresource_fetch_for_target(
        &mut self,
        target_id: &str,
        completion: moli_core::page::CompletedPageCommand,
    ) -> Result<(), String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_fulfill_pending_subresource_fetch(completion)
            .map(|_| ())
            .map_err(|error| format!("subresource fetch fulfill failed: {error}"))
    }

    pub(crate) fn finish_fulfill_pending_subresource_response_for_target(
        &mut self,
        target_id: &str,
        completion: moli_core::page::CompletedPageCommand,
    ) -> Result<(), String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_fulfill_pending_subresource_response(completion)
            .map(|_| ())
            .map_err(|error| format!("subresource response fulfill failed: {error}"))
    }

    pub(crate) fn finish_continue_pending_subresource_response_for_target(
        &mut self,
        target_id: &str,
        completion: moli_core::page::CompletedPageCommand,
    ) -> Result<(), String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_continue_pending_subresource_response(completion)
            .map(|_| ())
            .map_err(|error| format!("subresource response continue failed: {error}"))
    }
    pub(crate) async fn continue_pending_subresource_fetch_for_target_async(
        &mut self,
        target_id: &str,
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<String>>,
        headers: Option<Vec<(String, String)>>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<PendingSubresourceContinueOutcome, String> {
        let page = self
            .loaded_page_for_target_mut(target_id)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.continue_pending_subresource_fetch_async(
            internal_id,
            url,
            method,
            body,
            headers,
            intercept_response,
            handle_auth_requests,
        )
        .await
        .map_err(|error| format!("subresource fetch continue failed: {error}"))
    }

    pub(crate) async fn continue_pending_subresource_auth_for_target_async(
        &mut self,
        target_id: &str,
        internal_id: u64,
        auth: SubresourceAuthCredentials,
    ) -> Result<PendingSubresourceContinueOutcome, String> {
        let page = self
            .loaded_page_for_target_mut(target_id)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.continue_pending_subresource_auth_async(internal_id, auth)
            .await
            .map_err(|error| format!("subresource auth continue failed: {error}"))
    }

    pub(crate) async fn fail_pending_subresource_auth_for_target_async(
        &mut self,
        target_id: &str,
        internal_id: u64,
        error_text: String,
    ) -> Result<Option<moli_core::RendererOutputFence>, String> {
        let page = self
            .loaded_page_for_target_mut(target_id)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.fail_pending_subresource_auth_async(internal_id, error_text)
            .await
            .map_err(|error| format!("subresource auth fail failed: {error}"))
    }

    pub(crate) async fn fail_pending_subresource_fetch_for_target_async(
        &mut self,
        target_id: &str,
        internal_id: u64,
        error_text: String,
    ) -> Result<Option<moli_core::RendererOutputFence>, String> {
        let page = self
            .loaded_page_for_target_mut(target_id)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.fail_pending_subresource_fetch_async(internal_id, error_text)
            .await
            .map_err(|error| format!("subresource fetch fail failed: {error}"))
    }

    pub(crate) async fn fulfill_pending_subresource_fetch_for_target_async(
        &mut self,
        target_id: &str,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: moli_core::page::RendererSyntheticResponseBody,
    ) -> Result<(), String> {
        let page = self
            .loaded_page_for_target_mut(target_id)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.fulfill_pending_subresource_fetch_async(
            internal_id,
            response_code,
            response_headers,
            response_body,
        )
        .await
        .map_err(|error| format!("subresource fetch fulfill failed: {error}"))
    }

    pub(crate) async fn continue_pending_subresource_response_for_target_async(
        &mut self,
        target_id: &str,
        internal_id: u64,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    ) -> Result<(), String> {
        let page = self
            .loaded_page_for_target_mut(target_id)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.continue_pending_subresource_response_async(
            internal_id,
            response_code,
            response_headers,
        )
        .await
        .map_err(|error| format!("subresource response continue failed: {error}"))
    }

    pub(crate) async fn fail_pending_subresource_response_for_target_async(
        &mut self,
        target_id: &str,
        internal_id: u64,
        error_text: String,
    ) -> Result<Option<moli_core::RendererOutputFence>, String> {
        let page = self
            .loaded_page_for_target_mut(target_id)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.fail_pending_subresource_response_async(internal_id, error_text)
            .await
            .map_err(|error| format!("subresource response fail failed: {error}"))
    }

    pub(crate) async fn fulfill_pending_subresource_response_for_target_async(
        &mut self,
        target_id: &str,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: moli_core::page::RendererSyntheticResponseBody,
    ) -> Result<(), String> {
        let page = self
            .loaded_page_for_target_mut(target_id)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.fulfill_pending_subresource_response_async(
            internal_id,
            response_code,
            response_headers,
            response_body,
        )
        .await
        .map_err(|error| format!("subresource response fulfill failed: {error}"))
    }

    pub(crate) async fn receive_synthetic_websocket_text_for_target_async(
        &mut self,
        target_id: &str,
        socket_id: u64,
        data: String,
    ) -> Result<(), String> {
        let page = self
            .loaded_page_for_target_mut(target_id)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.receive_synthetic_websocket_text_async(socket_id, data)
            .await
            .map_err(|error| format!("synthetic websocket text dispatch failed: {error}"))
    }

    pub(crate) async fn receive_synthetic_websocket_binary_for_target_async(
        &mut self,
        target_id: &str,
        socket_id: u64,
        data: Vec<u8>,
    ) -> Result<(), String> {
        let page = self
            .loaded_page_for_target_mut(target_id)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.receive_synthetic_websocket_binary_async(socket_id, data)
            .await
            .map_err(|error| format!("synthetic websocket binary dispatch failed: {error}"))
    }

    pub(crate) async fn close_synthetic_websocket_from_server_for_target_async(
        &mut self,
        target_id: &str,
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    ) -> Result<(), String> {
        let page = self
            .loaded_page_for_target_mut(target_id)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.close_synthetic_websocket_from_server_async(socket_id, code, reason)
            .await
            .map_err(|error| format!("synthetic websocket close dispatch failed: {error}"))
    }
}
