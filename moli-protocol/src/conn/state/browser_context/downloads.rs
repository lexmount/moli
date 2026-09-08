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
        if !self.browser_context.contains_web_contents(web_contents) {
            return None;
        }
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
        browser_globals: &crate::conn::BrowserGlobalOverrides,
    ) -> Result<Option<DownloadObservation>, String> {
        let client = self.ensure_web_contents_resource_request_client(
            web_contents,
            fetch_defaults,
            &browser_globals.extra_headers,
            browser_globals.network_conditions,
        )?;
        self.browser_context.start_download_request(
            web_contents,
            policy,
            client,
            request,
            suggested_filename,
        )
    }

    pub(in crate::conn) fn start_download_response(
        &mut self,
        web_contents: WebContentsHandle,
        policy: &DownloadPolicy,
        url: Url,
        headers: Vec<(String, String)>,
        body: DownloadBody,
    ) -> Result<Option<DownloadObservation>, String> {
        self.browser_context
            .start_download_response(web_contents, policy, url, headers, body)
    }

    pub(in crate::conn) fn cancel_download(
        &self,
        guid: &str,
    ) -> Option<Result<(), DownloadAccessError>> {
        self.browser_context.cancel_download(guid)
    }

    pub(in crate::conn) fn read_download_artifact(
        &self,
        guid: &str,
    ) -> Option<Result<tokio::task::JoinHandle<Result<Vec<u8>, String>>, DownloadAccessError>> {
        self.browser_context.read_download_artifact(guid)
    }
}
