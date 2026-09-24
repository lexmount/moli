use moli_fetch::{FetchCancelHandle, Request};

use super::ResourceRequestClient;

pub enum RendererNetworkResourceLoadPreparation {
    Ready(Box<RendererPreparedNetworkResourceLoad>),
    FrameNotFound,
    CspViolation,
    UnsupportedUrlScheme,
}

pub struct RendererPreparedNetworkResourceLoad {
    request_client: ResourceRequestClient,
    request: Request,
}

impl RendererPreparedNetworkResourceLoad {
    pub(crate) fn new(request_client: ResourceRequestClient, request: Request) -> Self {
        Self {
            request_client,
            request,
        }
    }

    pub async fn execute(self) -> RendererNetworkResourceLoadOutcome {
        let cancel_handle = FetchCancelHandle::new();
        let fetch_result = match self
            .request_client
            .fetch_raw_stream_with_cancel_and_network_metadata(self.request, cancel_handle)
            .await
        {
            Ok(response) => response,
            Err(error) => {
                return RendererNetworkResourceLoadOutcome::FailedBeforeResponse(format!(
                    "{error:#}"
                ));
            }
        };
        let mut response = fetch_result.into_response();
        let status = response.status;
        let headers = response.headers.clone();
        let mut body = Vec::new();
        while let Some(chunk) = response.next_chunk().await {
            body.extend_from_slice(&chunk);
        }
        let completion_error = response
            .finish()
            .await
            .err()
            .map(|error| format!("{error:#}"));
        RendererNetworkResourceLoadOutcome::Response(Box::new(
            RendererNetworkResourceLoadResponse {
                status,
                headers,
                body,
                completion_error,
            },
        ))
    }
}

pub enum RendererNetworkResourceLoadOutcome {
    Response(Box<RendererNetworkResourceLoadResponse>),
    FailedBeforeResponse(String),
}

pub struct RendererNetworkResourceLoadResponse {
    pub status: u16,
    pub headers: Vec<(String, Vec<u8>)>,
    pub body: Vec<u8>,
    pub completion_error: Option<String>,
}
