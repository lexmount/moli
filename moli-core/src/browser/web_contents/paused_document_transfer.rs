use url::Url;

use crate::browser::{
    CapturedBody, CapturedBodyWriter, NavigationRequestLoadPolicy, ensure_materialize_limit,
};
use moli_fetch::{NetworkObservationJournal, RawResponse, ResponseHead, StreamingRawResponse};

/// Browser-owned response body retained while a navigation decision is pending.
#[derive(Debug)]
pub enum DocumentBodySource {
    BufferedRaw {
        requested_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        response: RawResponse,
        network_observation_journal: NetworkObservationJournal,
    },
    StreamingRaw {
        requested_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        response: StreamingRawResponse,
        network_observation_journal: NetworkObservationJournal,
    },
    CapturedRaw {
        requested_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        head: ResponseHead,
        body: CapturedBody,
        network_observation_journal: NetworkObservationJournal,
    },
}

#[derive(Debug)]
pub struct PausedDocumentTransfer {
    request_load_policy: NavigationRequestLoadPolicy,
    state: PausedDocumentTransferState,
    decision_claim: Option<crate::browser::navigation_decision::NavigationDecisionClaim>,
}

#[derive(Debug)]
enum PausedDocumentTransferState {
    Pending {
        body: DocumentBodySource,
    },
    ActiveBodyStream {
        stream: ActiveDocumentBodyStreamState,
    },
}

#[derive(Debug)]
struct ActiveDocumentBodyStreamState {
    requested_url: Url,
    request_method: String,
    request_headers: Vec<(String, String)>,
    response: StreamingRawResponse,
    network_observation_journal: NetworkObservationJournal,
    captured_body: CapturedBodyWriter,
    unread_body: Vec<u8>,
    offset: usize,
    finished: bool,
}

#[derive(Debug)]
pub struct PendingFetchResponseOpenedBodyStream {
    pub handle: String,
    pub buffered_bytes: Option<Vec<u8>>,
    pub transfer: PausedDocumentTransfer,
}

#[derive(Debug)]
pub enum OpenBodyStreamError {
    NotOpenable(Box<PausedDocumentTransfer>),
    Failed {
        transfer: Box<PausedDocumentTransfer>,
        message: String,
    },
}

impl PausedDocumentTransfer {
    pub(in crate::browser) fn response_snapshot(
        &self,
    ) -> (ResponseHead, NetworkObservationJournal) {
        match &self.state {
            PausedDocumentTransferState::Pending { body } => match body {
                DocumentBodySource::BufferedRaw {
                    response,
                    network_observation_journal,
                    ..
                } => (response.head(), network_observation_journal.clone()),
                DocumentBodySource::StreamingRaw {
                    response,
                    network_observation_journal,
                    ..
                } => (response.head(), network_observation_journal.clone()),
                DocumentBodySource::CapturedRaw {
                    head,
                    network_observation_journal,
                    ..
                } => (head.clone(), network_observation_journal.clone()),
            },
            PausedDocumentTransferState::ActiveBodyStream { stream } => (
                stream.response.head(),
                stream.network_observation_journal.clone(),
            ),
        }
    }

    pub fn has_pending_decision(&self) -> bool {
        self.decision_claim.is_some()
    }

    pub fn pending(
        request_load_policy: NavigationRequestLoadPolicy,
        body: DocumentBodySource,
    ) -> Self {
        Self {
            request_load_policy,
            state: PausedDocumentTransferState::Pending { body },
            decision_claim: None,
        }
    }

    pub(in crate::browser) fn claim_decision(
        &mut self,
        claim: crate::browser::navigation_decision::NavigationDecisionClaim,
    ) {
        self.decision_claim = Some(claim);
    }

    pub(in crate::browser) fn release_decision_claim(&mut self) {
        if let Some(claim) = self.decision_claim.take() {
            claim.disarm();
        }
    }

    pub fn into_pending(
        self,
    ) -> Result<(NavigationRequestLoadPolicy, DocumentBodySource), Box<Self>> {
        let Self {
            request_load_policy,
            state,
            decision_claim,
        } = self;
        match state {
            PausedDocumentTransferState::Pending { body } => Ok((request_load_policy, body)),
            state => Err(Box::new(Self {
                request_load_policy,
                state,
                decision_claim,
            })),
        }
    }

    pub fn open_body_stream(
        mut self,
        handle: String,
    ) -> Result<PendingFetchResponseOpenedBodyStream, OpenBodyStreamError> {
        let decision_claim = self.decision_claim.take();
        let mut result = self.open_body_stream_inner(handle);
        let transfer = match &mut result {
            Ok(opened) => &mut opened.transfer,
            Err(OpenBodyStreamError::NotOpenable(transfer))
            | Err(OpenBodyStreamError::Failed { transfer, .. }) => transfer.as_mut(),
        };
        transfer.decision_claim = decision_claim;
        result
    }

    fn open_body_stream_inner(
        self,
        handle: String,
    ) -> Result<PendingFetchResponseOpenedBodyStream, OpenBodyStreamError> {
        let (request_load_policy, body) = self
            .into_pending()
            .map_err(OpenBodyStreamError::NotOpenable)?;
        match body {
            DocumentBodySource::StreamingRaw {
                requested_url,
                request_method,
                request_headers,
                response,
                network_observation_journal,
            } => Ok(PendingFetchResponseOpenedBodyStream {
                handle,
                buffered_bytes: None,
                transfer: Self {
                    request_load_policy,
                    decision_claim: None,
                    state: PausedDocumentTransferState::ActiveBodyStream {
                        stream: ActiveDocumentBodyStreamState::new(
                            requested_url,
                            request_method,
                            request_headers,
                            response,
                            network_observation_journal,
                        ),
                    },
                },
            }),
            DocumentBodySource::BufferedRaw {
                requested_url,
                request_method,
                request_headers,
                response,
                network_observation_journal,
            } => {
                let bytes = response.clone_body_bytes();
                Ok(PendingFetchResponseOpenedBodyStream {
                    handle,
                    buffered_bytes: Some(bytes),
                    transfer: Self::pending(
                        request_load_policy,
                        DocumentBodySource::BufferedRaw {
                            requested_url,
                            request_method,
                            request_headers,
                            response,
                            network_observation_journal,
                        },
                    ),
                })
            }
            DocumentBodySource::CapturedRaw {
                requested_url,
                request_method,
                request_headers,
                head,
                body,
                network_observation_journal,
            } => {
                let bytes = match body.materialize_bytes() {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        return Err(OpenBodyStreamError::Failed {
                            transfer: Box::new(Self::pending(
                                request_load_policy,
                                DocumentBodySource::CapturedRaw {
                                    requested_url,
                                    request_method,
                                    request_headers,
                                    head,
                                    body,
                                    network_observation_journal,
                                },
                            )),
                            message: format!(
                                "failed to materialize captured response body: {error}"
                            ),
                        });
                    }
                };
                Ok(PendingFetchResponseOpenedBodyStream {
                    handle,
                    buffered_bytes: Some(bytes),
                    transfer: Self::pending(
                        request_load_policy,
                        DocumentBodySource::CapturedRaw {
                            requested_url,
                            request_method,
                            request_headers,
                            head,
                            body,
                            network_observation_journal,
                        },
                    ),
                })
            }
        }
    }

    pub fn body_stream_offset(&self) -> Option<usize> {
        match &self.state {
            PausedDocumentTransferState::ActiveBodyStream { stream } => Some(stream.offset),
            PausedDocumentTransferState::Pending { .. } => None,
        }
    }

    pub async fn read_body_stream_async(
        self,
        size: Option<usize>,
    ) -> Result<(Vec<u8>, bool, Self), (Self, String)> {
        let Self {
            request_load_policy,
            state,
            decision_claim,
        } = self;
        let PausedDocumentTransferState::ActiveBodyStream { mut stream } = state else {
            return Err((
                Self {
                    request_load_policy,
                    state,
                    decision_claim,
                },
                "StreamHandleNotFound".to_owned(),
            ));
        };
        match stream.read_async(size).await {
            Ok((bytes, eof)) => {
                let state = if eof {
                    match stream.finish_pending_body_source() {
                        Ok(body) => PausedDocumentTransferState::Pending { body },
                        Err(message) => {
                            return Err((
                                Self {
                                    request_load_policy,
                                    decision_claim,
                                    state: PausedDocumentTransferState::ActiveBodyStream { stream },
                                },
                                message,
                            ));
                        }
                    }
                } else {
                    PausedDocumentTransferState::ActiveBodyStream { stream }
                };
                Ok((
                    bytes,
                    eof,
                    Self {
                        request_load_policy,
                        state,
                        decision_claim,
                    },
                ))
            }
            Err(message) => Err((
                Self {
                    request_load_policy,
                    decision_claim,
                    state: PausedDocumentTransferState::ActiveBodyStream { stream },
                },
                message,
            )),
        }
    }

    pub async fn finish_body_stream_async(
        self,
    ) -> Result<(NavigationRequestLoadPolicy, DocumentBodySource), (Self, String)> {
        let Self {
            request_load_policy,
            state,
            decision_claim,
        } = self;
        match state {
            PausedDocumentTransferState::Pending { body } => Ok((request_load_policy, body)),
            PausedDocumentTransferState::ActiveBodyStream { mut stream } => {
                if let Err(message) = stream.read_async(None).await {
                    return Err((
                        Self {
                            request_load_policy,
                            decision_claim,
                            state: PausedDocumentTransferState::ActiveBodyStream { stream },
                        },
                        message,
                    ));
                }
                match stream.finish_pending_body_source() {
                    Ok(body) => Ok((request_load_policy, body)),
                    Err(message) => Err((
                        Self {
                            request_load_policy,
                            decision_claim,
                            state: PausedDocumentTransferState::ActiveBodyStream { stream },
                        },
                        message,
                    )),
                }
            }
        }
    }

    pub async fn materialize_body_limited_async(
        mut self,
        limit: usize,
    ) -> Result<(Option<Vec<u8>>, Self), (String, Self)> {
        let decision_claim = self.decision_claim.take();
        let mut result = self.materialize_body_limited_inner_async(limit).await;
        let transfer = match &mut result {
            Ok((_, transfer)) | Err((_, transfer)) => transfer,
        };
        transfer.decision_claim = decision_claim;
        result
    }

    async fn materialize_body_limited_inner_async(
        self,
        limit: usize,
    ) -> Result<(Option<Vec<u8>>, Self), (String, Self)> {
        let (request_load_policy, body) = match self.into_pending() {
            Ok(parts) => parts,
            Err(transfer) => return Ok((None, *transfer)),
        };
        match body.materialize_body_limited_async(limit).await {
            Ok((bytes, body)) => Ok((Some(bytes), Self::pending(request_load_policy, body))),
            Err((message, body)) => Err((message, Self::pending(request_load_policy, body))),
        }
    }
}

impl DocumentBodySource {
    async fn materialize_body_limited_async(
        self,
        limit: usize,
    ) -> Result<(Vec<u8>, Self), (String, Self)> {
        match self {
            Self::BufferedRaw {
                requested_url,
                request_method,
                request_headers,
                response,
                network_observation_journal,
            } => {
                if let Err(error) = ensure_materialize_limit(response.body_bytes().len(), limit) {
                    return Err((
                        error.to_string(),
                        Self::BufferedRaw {
                            requested_url,
                            request_method,
                            request_headers,
                            response,
                            network_observation_journal,
                        },
                    ));
                }
                let bytes = response.clone_body_bytes();
                Ok((
                    bytes,
                    Self::BufferedRaw {
                        requested_url,
                        request_method,
                        request_headers,
                        response,
                        network_observation_journal,
                    },
                ))
            }
            Self::StreamingRaw {
                requested_url,
                request_method,
                request_headers,
                response,
                network_observation_journal,
            } => {
                let preserved_head = response.head();
                let (head, body) = match capture_streaming_raw_response(response).await {
                    Ok(captured) => captured,
                    Err(message) => {
                        return Err((
                            message,
                            Self::CapturedRaw {
                                requested_url,
                                request_method,
                                request_headers,
                                head: preserved_head,
                                body: CapturedBody::from_bytes(Vec::new()),
                                network_observation_journal,
                            },
                        ));
                    }
                };
                let result = body.materialize_bytes_limited(limit).map_err(|error| {
                    format!("failed to materialize captured response body: {error}")
                });
                let source = Self::CapturedRaw {
                    requested_url,
                    request_method,
                    request_headers,
                    head,
                    body,
                    network_observation_journal,
                };
                match result {
                    Ok(bytes) => Ok((bytes, source)),
                    Err(message) => Err((message, source)),
                }
            }
            Self::CapturedRaw {
                requested_url,
                request_method,
                request_headers,
                head,
                body,
                network_observation_journal,
            } => {
                let result = body.materialize_bytes_limited(limit).map_err(|error| {
                    format!("failed to materialize captured response body: {error}")
                });
                let source = Self::CapturedRaw {
                    requested_url,
                    request_method,
                    request_headers,
                    head,
                    body,
                    network_observation_journal,
                };
                match result {
                    Ok(bytes) => Ok((bytes, source)),
                    Err(message) => Err((message, source)),
                }
            }
        }
    }
}

impl ActiveDocumentBodyStreamState {
    fn new(
        requested_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        response: StreamingRawResponse,
        network_observation_journal: NetworkObservationJournal,
    ) -> Self {
        Self {
            requested_url,
            request_method,
            request_headers,
            response,
            network_observation_journal,
            captured_body: CapturedBodyWriter::default(),
            unread_body: Vec::new(),
            offset: 0,
            finished: false,
        }
    }

    async fn read_async(&mut self, size: Option<usize>) -> Result<(Vec<u8>, bool), String> {
        let mut bytes = Vec::new();
        match size {
            Some(limit) => {
                while bytes.len() < limit {
                    if self.unread_body.is_empty() && !self.finished {
                        self.read_next_chunk_async().await?;
                    }
                    if self.unread_body.is_empty() {
                        break;
                    }
                    let remaining = limit.saturating_sub(bytes.len());
                    self.drain_unread(&mut bytes, remaining);
                }
            }
            None => {
                while !self.unread_body.is_empty() || !self.finished {
                    if self.unread_body.is_empty() {
                        self.read_next_chunk_async().await?;
                    }
                    let remaining = self.unread_body.len();
                    self.drain_unread(&mut bytes, remaining);
                }
            }
        }
        self.offset = self.offset.saturating_add(bytes.len());
        Ok((bytes, self.finished && self.unread_body.is_empty()))
    }

    fn drain_unread(&mut self, bytes: &mut Vec<u8>, limit: usize) {
        let take = limit.min(self.unread_body.len());
        bytes.extend(self.unread_body.drain(..take));
    }

    async fn read_next_chunk_async(&mut self) -> Result<(), String> {
        if self.finished {
            return Ok(());
        }
        if let Some(chunk) = self.response.next_chunk().await {
            self.captured_body
                .append(&chunk)
                .map_err(|error| format!("failed to capture response body stream: {error}"))?;
            self.unread_body.extend(chunk);
            return Ok(());
        }
        self.response
            .finish()
            .await
            .map_err(|error| format!("failed to read page body from stream: {error}"))?;
        self.finished = true;
        Ok(())
    }

    fn finish_pending_body_source(&mut self) -> Result<DocumentBodySource, String> {
        let head = self.response.head();
        let body = self
            .captured_body
            .finish_in_place()
            .map_err(|error| format!("failed to finish captured response body: {error}"))?;
        Ok(DocumentBodySource::CapturedRaw {
            requested_url: self.requested_url.clone(),
            request_method: self.request_method.clone(),
            request_headers: self.request_headers.clone(),
            head,
            body,
            network_observation_journal: self.network_observation_journal.clone(),
        })
    }
}

async fn capture_streaming_raw_response(
    mut response: StreamingRawResponse,
) -> Result<(ResponseHead, CapturedBody), String> {
    let head = response.head();
    let mut body = CapturedBodyWriter::default();
    while let Some(chunk) = response.next_chunk().await {
        body.append(&chunk)
            .map_err(|error| format!("failed to capture response body stream: {error}"))?;
    }
    response
        .finish()
        .await
        .map_err(|error| format!("failed to read page body from stream: {error}"))?;
    let body = body
        .finish()
        .map_err(|error| format!("failed to finish captured response body: {error}"))?;
    Ok((head, body))
}
