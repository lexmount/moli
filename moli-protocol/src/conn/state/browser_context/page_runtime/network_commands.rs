use super::BrowserContext;
use moli_core::{
    RendererOutputFence,
    browser::{DocumentHandle, WebContentsHandle},
    page::{
        CompletedPageCommand, PendingPageCommand, PendingSubresourceContinueOutcome,
        RendererSyntheticResponseBody, SubresourceAuthCredentials, SubresourceResourceType,
    },
};
use url::Url;

/// Closed Browser-native command family for a concrete renderer Document.
/// Frontend routing identities never enter this API.
pub(crate) enum DocumentFetchCommand {
    ContinueRequest {
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<String>>,
        headers: Option<Vec<(String, String)>>,
        intercept_response: bool,
        handle_auth_requests: bool,
    },
    ContinueAuth {
        internal_id: u64,
        auth: SubresourceAuthCredentials,
    },
    CancelAuth {
        internal_id: u64,
    },
    FailAuth {
        internal_id: u64,
        error_text: String,
    },
    FailRequest {
        internal_id: u64,
        error_text: String,
    },
    FailResponse {
        internal_id: u64,
        error_text: String,
    },
    FulfillRequest {
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    },
    FulfillResponse {
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    },
    ContinueResponse {
        internal_id: u64,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    },
    DispatchWebSocketText {
        socket_id: u64,
        data: String,
    },
    DispatchWebSocketBinary {
        socket_id: u64,
        data: Vec<u8>,
    },
    CloseWebSocket {
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    },
}

#[derive(Clone, Copy)]
enum DocumentFetchCommandKind {
    InterceptionUpdate { accept_stale: bool },
    ContinueRequest,
    ContinueAuth,
    CancelAuth,
    FailAuth,
    FailRequest,
    FailResponse,
    FulfillRequest,
    FulfillResponse,
    ContinueResponse,
    DispatchWebSocketText,
    DispatchWebSocketBinary,
    CloseWebSocket,
}

pub(crate) struct PendingDocumentFetchCommand {
    document: DocumentHandle,
    kind: DocumentFetchCommandKind,
    pending: PendingPageCommand,
}

pub(crate) struct CompletedDocumentFetchCommand {
    document: DocumentHandle,
    kind: DocumentFetchCommandKind,
    completed: Result<CompletedPageCommand, String>,
}

pub(crate) enum DocumentFetchCommandOutcome {
    Continued(PendingSubresourceContinueOutcome),
    Complete,
}

impl DocumentFetchCommandOutcome {
    pub(crate) fn into_continue_outcome(self) -> Result<PendingSubresourceContinueOutcome, String> {
        match self {
            Self::Continued(outcome) => Ok(outcome),
            Self::Complete => Err("Fetch command did not produce a continue outcome".to_owned()),
        }
    }
}

impl PendingDocumentFetchCommand {
    fn new(
        document: DocumentHandle,
        kind: DocumentFetchCommandKind,
        pending: PendingPageCommand,
    ) -> Self {
        Self {
            document,
            kind,
            pending,
        }
    }

    pub(crate) async fn wait(self) -> CompletedDocumentFetchCommand {
        CompletedDocumentFetchCommand {
            document: self.document,
            kind: self.kind,
            completed: self.pending.wait().await.map_err(|error| error.to_string()),
        }
    }
}

impl CompletedDocumentFetchCommand {
    pub(crate) fn document(&self) -> DocumentHandle {
        self.document
    }

    pub(crate) fn renderer_output_predecessor(&self) -> Option<RendererOutputFence> {
        self.completed
            .as_ref()
            .ok()
            .and_then(CompletedPageCommand::renderer_output_predecessor)
    }

    pub(crate) fn is_interception_update(&self) -> bool {
        matches!(
            self.kind,
            DocumentFetchCommandKind::InterceptionUpdate { .. }
        )
    }

    fn into_parts(
        self,
    ) -> (
        DocumentHandle,
        DocumentFetchCommandKind,
        Result<CompletedPageCommand, String>,
    ) {
        (self.document, self.kind, self.completed)
    }
}

impl BrowserContext {
    pub(crate) fn start_document_fetch_command(
        &self,
        document: DocumentHandle,
        command: DocumentFetchCommand,
    ) -> Result<PendingDocumentFetchCommand, String> {
        let page = &self.physical.document(document)?.page;
        let (kind, pending) = match command {
            DocumentFetchCommand::ContinueRequest {
                internal_id,
                url,
                method,
                body,
                headers,
                intercept_response,
                handle_auth_requests,
            } => (
                DocumentFetchCommandKind::ContinueRequest,
                page.start_continue_pending_subresource_fetch(
                    internal_id,
                    url,
                    method,
                    body,
                    headers,
                    intercept_response,
                    handle_auth_requests,
                ),
            ),
            DocumentFetchCommand::ContinueAuth { internal_id, auth } => (
                DocumentFetchCommandKind::ContinueAuth,
                page.start_continue_pending_subresource_auth(internal_id, auth),
            ),
            DocumentFetchCommand::CancelAuth { internal_id } => (
                DocumentFetchCommandKind::CancelAuth,
                page.start_cancel_pending_subresource_auth(internal_id),
            ),
            DocumentFetchCommand::FailAuth {
                internal_id,
                error_text,
            } => (
                DocumentFetchCommandKind::FailAuth,
                page.start_fail_pending_subresource_auth(internal_id, error_text),
            ),
            DocumentFetchCommand::FailRequest {
                internal_id,
                error_text,
            } => (
                DocumentFetchCommandKind::FailRequest,
                page.start_fail_pending_subresource_fetch(internal_id, error_text),
            ),
            DocumentFetchCommand::FailResponse {
                internal_id,
                error_text,
            } => (
                DocumentFetchCommandKind::FailResponse,
                page.start_fail_pending_subresource_response(internal_id, error_text),
            ),
            DocumentFetchCommand::FulfillRequest {
                internal_id,
                response_code,
                response_headers,
                response_body,
            } => (
                DocumentFetchCommandKind::FulfillRequest,
                page.start_fulfill_pending_subresource_fetch(
                    internal_id,
                    response_code,
                    response_headers,
                    response_body,
                ),
            ),
            DocumentFetchCommand::FulfillResponse {
                internal_id,
                response_code,
                response_headers,
                response_body,
            } => (
                DocumentFetchCommandKind::FulfillResponse,
                page.start_fulfill_pending_subresource_response(
                    internal_id,
                    response_code,
                    response_headers,
                    response_body,
                ),
            ),
            DocumentFetchCommand::ContinueResponse {
                internal_id,
                response_code,
                response_headers,
            } => (
                DocumentFetchCommandKind::ContinueResponse,
                page.start_continue_pending_subresource_response(
                    internal_id,
                    response_code,
                    response_headers,
                ),
            ),
            DocumentFetchCommand::DispatchWebSocketText { socket_id, data } => (
                DocumentFetchCommandKind::DispatchWebSocketText,
                page.start_receive_synthetic_websocket_text(socket_id, data),
            ),
            DocumentFetchCommand::DispatchWebSocketBinary { socket_id, data } => (
                DocumentFetchCommandKind::DispatchWebSocketBinary,
                page.start_receive_synthetic_websocket_binary(socket_id, data),
            ),
            DocumentFetchCommand::CloseWebSocket {
                socket_id,
                code,
                reason,
            } => (
                DocumentFetchCommandKind::CloseWebSocket,
                page.start_close_synthetic_websocket_from_server(socket_id, code, reason),
            ),
        };
        Ok(PendingDocumentFetchCommand::new(
            document,
            kind,
            pending.map_err(|error| error.to_string())?,
        ))
    }

    pub(crate) fn start_web_contents_fetch_interception_update(
        &mut self,
        web_contents: WebContentsHandle,
        enabled: bool,
        resource_type: Option<SubresourceResourceType>,
        accept_stale_completion: bool,
    ) -> Result<Option<PendingDocumentFetchCommand>, String> {
        let contents = self.physical.web_contents_mut(web_contents)?;
        let document = contents
            .main_frame
            .current_document
            .as_ref()
            .map(|document| document.id);
        let pending = contents.start_fetch_interception_update(enabled, resource_type)?;
        Ok(match (document, pending) {
            (Some(document), Some(pending)) => Some(PendingDocumentFetchCommand::new(
                DocumentHandle::new(web_contents, document),
                DocumentFetchCommandKind::InterceptionUpdate {
                    accept_stale: accept_stale_completion,
                },
                pending,
            )),
            (None, None) => None,
            _ => unreachable!("fetch interception update must follow current Document presence"),
        })
    }

    pub(crate) fn install_web_contents_fetch_interception_policy(
        &mut self,
        web_contents: WebContentsHandle,
        enabled: bool,
        resource_type: Option<SubresourceResourceType>,
    ) -> Result<(), String> {
        self.physical
            .web_contents_mut(web_contents)?
            .install_fetch_interception_policy(enabled, resource_type);
        Ok(())
    }

    pub(crate) fn finish_document_fetch_command(
        &mut self,
        completed: CompletedDocumentFetchCommand,
    ) -> Result<DocumentFetchCommandOutcome, String> {
        let (document, kind, completed) = completed.into_parts();
        let completion = completed?;
        if let DocumentFetchCommandKind::InterceptionUpdate { accept_stale } = kind {
            return self.finish_document_fetch_interception_update(
                document,
                completion,
                accept_stale,
            );
        }
        let page = &mut self.physical.document_mut(document)?.page;
        match kind {
            DocumentFetchCommandKind::ContinueRequest => page
                .finish_continue_pending_subresource_fetch(completion)
                .map(DocumentFetchCommandOutcome::Continued),
            DocumentFetchCommandKind::ContinueAuth => page
                .finish_continue_pending_subresource_auth(completion)
                .map(DocumentFetchCommandOutcome::Continued),
            DocumentFetchCommandKind::CancelAuth => page
                .finish_cancel_pending_subresource_auth(completion)
                .map(|_| DocumentFetchCommandOutcome::Complete),
            DocumentFetchCommandKind::FailAuth => page
                .finish_fail_pending_subresource_auth(completion)
                .map(|_| DocumentFetchCommandOutcome::Complete),
            DocumentFetchCommandKind::FailRequest => page
                .finish_fail_pending_subresource_fetch(completion)
                .map(|_| DocumentFetchCommandOutcome::Complete),
            DocumentFetchCommandKind::FailResponse => page
                .finish_fail_pending_subresource_response(completion)
                .map(|_| DocumentFetchCommandOutcome::Complete),
            DocumentFetchCommandKind::FulfillRequest => page
                .finish_fulfill_pending_subresource_fetch(completion)
                .map(|()| DocumentFetchCommandOutcome::Complete),
            DocumentFetchCommandKind::FulfillResponse => page
                .finish_fulfill_pending_subresource_response(completion)
                .map(|()| DocumentFetchCommandOutcome::Complete),
            DocumentFetchCommandKind::ContinueResponse => page
                .finish_continue_pending_subresource_response(completion)
                .map(|()| DocumentFetchCommandOutcome::Complete),
            DocumentFetchCommandKind::DispatchWebSocketText => page
                .finish_receive_synthetic_websocket_text(completion)
                .map(|()| DocumentFetchCommandOutcome::Complete),
            DocumentFetchCommandKind::DispatchWebSocketBinary => page
                .finish_receive_synthetic_websocket_binary(completion)
                .map(|()| DocumentFetchCommandOutcome::Complete),
            DocumentFetchCommandKind::CloseWebSocket => page
                .finish_close_synthetic_websocket_from_server(completion)
                .map(|()| DocumentFetchCommandOutcome::Complete),
            DocumentFetchCommandKind::InterceptionUpdate { .. } => unreachable!(),
        }
        .map_err(|error| error.to_string())
    }

    fn finish_document_fetch_interception_update(
        &mut self,
        document: DocumentHandle,
        completion: CompletedPageCommand,
        accept_stale: bool,
    ) -> Result<DocumentFetchCommandOutcome, String> {
        match self.physical.document_mut(document) {
            Ok(document) => document
                .page
                .finish_set_fetch_subresource_interception(completion)
                .map(|()| DocumentFetchCommandOutcome::Complete)
                .map_err(|error| error.to_string()),
            Err(error) if error == "NoDocumentLoaded" || accept_stale => {
                completion
                    .into_unit_page_command_turn()
                    .map(drop)
                    .map_err(|error| error.to_string())?;
                Ok(DocumentFetchCommandOutcome::Complete)
            }
            Err(error) if error == "Document changed" => Err("Renderer Page changed".to_owned()),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn finish_unobserved_document_fetch_interception_update(
        completed: CompletedDocumentFetchCommand,
    ) -> Result<DocumentFetchCommandOutcome, String> {
        let (_, kind, completed) = completed.into_parts();
        if !matches!(kind, DocumentFetchCommandKind::InterceptionUpdate { .. }) {
            return Err("NoDocumentLoaded".to_owned());
        }
        completed?
            .into_unit_page_command_turn()
            .map(drop)
            .map_err(|error| error.to_string())?;
        Ok(DocumentFetchCommandOutcome::Complete)
    }
}
