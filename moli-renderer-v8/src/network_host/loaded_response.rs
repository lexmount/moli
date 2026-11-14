use moli_fetch::{Response, ResponseHead, StreamingRawResponse};

use crate::{network::ResourceInterceptionBody, types::SubresourceResponseBody};

/// A single transport hop is available at headers. Its consumer chooses
/// whether to capture the body or retire it on redirect/replacement.
pub(super) struct LoadedResponse {
    pub(super) head: ResponseHead,
    pub(super) body: ResourceInterceptionBody,
}

impl LoadedResponse {
    pub(super) fn from_response(response: Response) -> Self {
        Self {
            head: response.head(),
            body: ResourceInterceptionBody::from_complete(
                SubresourceResponseBody::from_fetch_response(&response),
            ),
        }
    }

    pub(super) fn from_stream(
        response: StreamingRawResponse,
        disk_pool: Option<moli_disk_pool::DiskPool>,
    ) -> Self {
        Self {
            head: response.head(),
            body: ResourceInterceptionBody::new(response, disk_pool),
        }
    }

    pub(super) async fn materialize(&self) -> Result<(Response, SubresourceResponseBody), String> {
        let body = self
            .body
            .capture(usize::MAX)
            .await
            .map_err(|error| format!("{error:#}"))?;
        let bytes = body
            .materialize_bytes()
            .map_err(|error| error.to_string())?;
        let response = Response::from_head_and_lossy_body_bytes(self.head.clone(), bytes);
        Ok((response, body))
    }
}

impl Drop for LoadedResponse {
    fn drop(&mut self) {
        // Also retire body capabilities retained by a protocol reader.
        self.body.cancel();
    }
}
