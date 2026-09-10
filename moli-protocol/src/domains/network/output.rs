use moli_cookie_jar::StoredCookieQueryReport;
use moli_core::page::{
    PendingSubresourceFetchInfo, SubresourceNetworkRequestHandle, SubresourceResourceType,
};
use serde_json::Value;
use url::Url;

use crate::conn::PendingSubresourceFetchRequest;
use crate::devtools_runtime::DevToolsNetworkInterceptId;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TargetSubresourceFetchPauseOutput {
    network_output: TargetSubresourceFetchPauseNetworkOutput,
    network_worker: Option<moli_core::page::RendererWorkerIdentity>,
    fetch_event_session_id: Option<String>,
    fetch_request_id: String,
    pending_fetch_request: PendingSubresourceFetchRequest,
    fetch_payload: Value,
}

impl TargetSubresourceFetchPauseOutput {
    pub(crate) fn new(
        network_output: TargetSubresourceFetchPauseNetworkOutput,
        network_worker: Option<moli_core::page::RendererWorkerIdentity>,
        fetch_event_session_id: Option<String>,
        fetch_request_id: String,
        pending_fetch_request: PendingSubresourceFetchRequest,
        fetch_payload: Value,
    ) -> Self {
        Self {
            network_output,
            network_worker,
            fetch_event_session_id,
            fetch_request_id,
            pending_fetch_request,
            fetch_payload,
        }
    }

    pub(crate) fn network_output(&self) -> &TargetSubresourceFetchPauseNetworkOutput {
        &self.network_output
    }

    pub(crate) fn network_worker(&self) -> Option<&moli_core::page::RendererWorkerIdentity> {
        self.network_worker.as_ref()
    }

    pub(crate) fn into_fetch_event_parts(
        self,
    ) -> (
        Option<String>,
        String,
        PendingSubresourceFetchRequest,
        Value,
    ) {
        (
            self.fetch_event_session_id,
            self.fetch_request_id,
            self.pending_fetch_request,
            self.fetch_payload,
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TargetSubresourceFetchPauseNetworkOutput {
    network_request_id: String,
    request_handle: Option<SubresourceNetworkRequestHandle>,
    frame_id: String,
    loader_id: String,
    timestamp: f64,
    document_url: Url,
    request_url: Url,
    method: String,
    request_headers: Vec<(String, String)>,
    request_body: Option<String>,
    resource_type: SubresourceResourceType,
    request_cookie_report: Option<StoredCookieQueryReport>,
    blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
    fetch_request_id: Option<String>,
}

impl TargetSubresourceFetchPauseNetworkOutput {
    pub(crate) fn from_pending_fetch_info(
        network_request_id: String,
        frame_id: String,
        loader_id: String,
        timestamp: f64,
        document_url: Url,
        info: &PendingSubresourceFetchInfo,
    ) -> Self {
        Self {
            network_request_id,
            request_handle: info.network_request_handle,
            frame_id,
            loader_id,
            timestamp,
            document_url,
            request_url: info.url.clone(),
            method: info.method.clone(),
            request_headers: info.request_headers.clone(),
            request_body: info.request_body.clone(),
            resource_type: info.resource_type,
            request_cookie_report: info.request_cookie_report.clone(),
            blocked_intercepts: Vec::new(),
            fetch_request_id: None,
        }
    }

    pub(crate) fn with_blocked_intercepts(
        mut self,
        blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
    ) -> Self {
        self.blocked_intercepts = blocked_intercepts;
        self
    }

    pub(crate) fn with_fetch_request_id(mut self, fetch_request_id: String) -> Self {
        self.fetch_request_id = Some(fetch_request_id);
        self
    }

    pub(crate) fn network_request_id(&self) -> &str {
        &self.network_request_id
    }

    pub(crate) fn frame_id(&self) -> &str {
        &self.frame_id
    }

    pub(crate) fn loader_id(&self) -> &str {
        &self.loader_id
    }

    pub(crate) fn timestamp(&self) -> f64 {
        self.timestamp
    }

    pub(crate) fn document_url(&self) -> &Url {
        &self.document_url
    }

    pub(crate) fn request_url(&self) -> &Url {
        &self.request_url
    }

    pub(crate) fn method(&self) -> &str {
        &self.method
    }

    pub(crate) fn request_headers(&self) -> &[(String, String)] {
        &self.request_headers
    }

    pub(crate) fn request_body(&self) -> Option<&str> {
        self.request_body.as_deref()
    }

    pub(crate) fn resource_type(&self) -> SubresourceResourceType {
        self.resource_type
    }

    pub(crate) fn request_cookie_report(&self) -> Option<&StoredCookieQueryReport> {
        self.request_cookie_report.as_ref()
    }

    pub(crate) fn blocked_intercepts(&self) -> &[DevToolsNetworkInterceptId] {
        &self.blocked_intercepts
    }

    pub(crate) fn fetch_request_id(&self) -> Option<&str> {
        self.fetch_request_id.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use moli_core::page::{
        PendingSubresourceFetchInfo, SubresourceNetworkRequestHandle, SubresourceResourceType,
    };
    use url::Url;

    use super::TargetSubresourceFetchPauseNetworkOutput;

    fn pending_fetch_info(url: &str) -> PendingSubresourceFetchInfo {
        PendingSubresourceFetchInfo {
            internal_id: 7,
            network_request_handle: Some(SubresourceNetworkRequestHandle::new(17)),
            frame_id: Some("FRAME-1".to_owned()),
            document_url: Url::parse("https://example.com/page")
                .expect("document URL should parse"),
            url: Url::parse(url).expect("request URL should parse"),
            websocket_socket_id: None,
            method: "POST".to_owned(),
            request_headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
            request_body: Some("payload".to_owned()),
            request_body_bytes: Some(b"payload".to_vec()),
            resource_type: SubresourceResourceType::Fetch,
            request_cookie_report: None,
        }
    }

    #[test]
    fn fetch_pause_network_output_owns_initial_request_snapshot() {
        let info = pending_fetch_info("https://example.com/api");
        let output = TargetSubresourceFetchPauseNetworkOutput::from_pending_fetch_info(
            "REQ-7".to_owned(),
            "FRAME-OVERRIDE".to_owned(),
            "LOADER-1".to_owned(),
            123.5,
            Url::parse("https://example.com/override").expect("document URL should parse"),
            &info,
        );

        assert_eq!(output.network_request_id(), "REQ-7");
        assert_eq!(
            output.request_handle,
            Some(SubresourceNetworkRequestHandle::new(17))
        );
        assert_eq!(output.frame_id(), "FRAME-OVERRIDE");
        assert_eq!(output.loader_id(), "LOADER-1");
        assert_eq!(output.timestamp(), 123.5);
        assert_eq!(
            output.document_url().as_str(),
            "https://example.com/override"
        );
        assert_eq!(output.request_url().as_str(), "https://example.com/api");
        assert_eq!(output.method(), "POST");
        assert_eq!(
            output.request_headers(),
            &[("content-type".to_owned(), "text/plain".to_owned())]
        );
        assert_eq!(output.request_body(), Some("payload"));
        assert_eq!(output.resource_type(), SubresourceResourceType::Fetch);
        assert!(output.request_cookie_report().is_none());
    }
}
