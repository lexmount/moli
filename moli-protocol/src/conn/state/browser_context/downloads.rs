use moli_core::browser::{
    DownloadAccessError, DownloadBody, DownloadObservation, DownloadPolicy, WebContentsHandle,
};
use moli_fetch::{FetchConfig, Request};
use url::Url;

use super::BrowserContext;

impl BrowserContext {
    pub(in crate::conn) fn download_frame_id_for_web_contents(
        &self,
        web_contents: WebContentsHandle,
    ) -> Option<&str> {
        self.physical.web_contents(web_contents).ok()?;
        self.page_targets
            .get_for_web_contents(web_contents.id())
            .map(crate::conn::PageAgentHost::target_id)
    }

    pub(in crate::conn) fn start_download_request(
        &mut self,
        web_contents: WebContentsHandle,
        fetch_defaults: FetchConfig,
        policy: &DownloadPolicy,
        request: Request,
        suggested_filename: Option<String>,
    ) -> Result<Option<DownloadObservation>, String> {
        let client =
            self.ensure_web_contents_resource_request_client(web_contents, fetch_defaults)?;
        self.physical
            .downloads
            .start_request(policy, client, request, suggested_filename)
    }

    pub(in crate::conn) fn start_download_response(
        &mut self,
        web_contents: WebContentsHandle,
        policy: &DownloadPolicy,
        url: Url,
        headers: Vec<(String, String)>,
        body: DownloadBody,
    ) -> Result<Option<DownloadObservation>, String> {
        self.physical.web_contents(web_contents)?;
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
