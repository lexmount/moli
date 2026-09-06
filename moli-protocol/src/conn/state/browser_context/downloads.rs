use moli_core::browser::{DownloadAccessError, DownloadBody, DownloadObservation, DownloadPolicy};
use moli_fetch::{FetchConfig, Request};
use url::Url;

use super::BrowserContext;

impl BrowserContext {
    // Value-only migration bridge; admission and execution belong to the physical
    // Context. Removed with the wrapper at the typed Browser API cutover (24b/30).
    pub(in crate::conn) fn start_download_request(
        &mut self,
        target: &str,
        fetch_defaults: FetchConfig,
        default_policy: &DownloadPolicy,
        request: Request,
        suggested_filename: Option<String>,
    ) -> Result<Option<DownloadObservation>, String> {
        let client = self.ensure_target_resource_request_client(target, fetch_defaults)?;
        let policy = self
            .physical
            .download_policy
            .as_ref()
            .unwrap_or(default_policy);
        self.physical
            .downloads
            .start_request(policy, client, request, suggested_filename)
    }

    pub(in crate::conn) fn start_download_response(
        &mut self,
        default_policy: &DownloadPolicy,
        url: Url,
        headers: Vec<(String, String)>,
        body: DownloadBody,
    ) -> Result<Option<DownloadObservation>, String> {
        let policy = self
            .physical
            .download_policy
            .as_ref()
            .unwrap_or(default_policy);
        self.physical
            .downloads
            .start_response(policy, url, headers, body)
    }

    pub(in crate::conn) fn cancel_download(
        &self,
        guid: &str,
    ) -> Option<Result<(), DownloadAccessError>> {
        self.physical.downloads.cancel(guid)
    }

    pub(in crate::conn) fn read_download_artifact(
        &self,
        guid: &str,
    ) -> Option<Result<tokio::task::JoinHandle<Result<Vec<u8>, String>>, DownloadAccessError>> {
        self.physical.downloads.read_artifact(guid)
    }
}
