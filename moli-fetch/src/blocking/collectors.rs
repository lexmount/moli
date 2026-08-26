use std::{str, time::Duration};

use anyhow::{Error, Result};
use curl::easy::{Easy2, Handler, WriteError};
use moli_cookie_jar::{
    NetworkCookieRequestContext, SharedBrowserCookieStore, StoredCookieQueryReport,
    StoredCookieSetReport,
};
use moli_http_cache::HttpCacheBodyWriter;
use tokio::sync::{mpsc, oneshot};
use tracing::debug;
use url::Url;

use crate::{
    FetchCancelHandle, FetchConfig, NegotiatedHttpVersion, NetworkRequestExtraInfo, Request,
    client_hints::{ClientHintResponseAction, ClientHintResponsePolicy},
};

use crate::RedirectInfo;

use super::{
    StreamingHtmlResponseStart, create_streaming_cache_body_writer_for_response_parts,
    next_redirect_url_from_parts, store_response_cookies,
};

fn is_interim_response_status(status: u16) -> bool {
    (100..200).contains(&status) && status != 101
}

fn response_body_ends_at_headers(status: u16) -> bool {
    // These statuses do not expose a Fetch body. 101 switches the connection
    // out of ordinary HTTP response-body framing, while 204/205 terminate the
    // response at headers even if a test server sends bytes afterward.
    matches!(status, 101 | 204 | 205)
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RequestTransferMetrics {
    content_encoding: Option<String>,
    content_length: Option<u64>,
    transfer_size: Option<u64>,
    namelookup_ms: Option<u64>,
    connect_ms: Option<u64>,
    appconnect_ms: Option<u64>,
    pretransfer_ms: Option<u64>,
    starttransfer_ms: Option<u64>,
    total_ms: Option<u64>,
}

pub(crate) fn transfer_metrics_from_easy<H: Handler>(
    easy: &Easy2<H>,
    headers: &[(String, String)],
) -> RequestTransferMetrics {
    let content_encoding = response_header_value(headers, "content-encoding").map(str::to_owned);
    let content_length = response_header_value(headers, "content-length")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0);
    let transfer_size = easy
        .download_size()
        .ok()
        .map(|size| size.max(0.0) as u64)
        .zip(easy.header_size().ok())
        .map(|(download_size, header_size)| download_size.saturating_add(header_size));
    let namelookup_ms = easy.namelookup_time().ok().map(duration_to_ms);
    let connect_ms = easy.connect_time().ok().map(duration_to_ms);
    let appconnect_ms = easy.appconnect_time().ok().map(duration_to_ms);
    let pretransfer_ms = easy.pretransfer_time().ok().map(duration_to_ms);
    let starttransfer_ms = easy.starttransfer_time().ok().map(duration_to_ms);
    let total_ms = easy.total_time().ok().map(duration_to_ms);

    RequestTransferMetrics {
        content_encoding,
        content_length,
        transfer_size,
        namelookup_ms,
        connect_ms,
        appconnect_ms,
        pretransfer_ms,
        starttransfer_ms,
        total_ms,
    }
}

fn duration_to_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn response_header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

pub(crate) fn log_request_completion(
    method: &str,
    request_url: &Url,
    final_url: &Url,
    status: u16,
    transfer_metrics: &RequestTransferMetrics,
) {
    debug!(
        method = %method,
        url = %request_url,
        final_url = %final_url,
        status,
        content_encoding = transfer_metrics.content_encoding.as_deref().unwrap_or("identity"),
        content_length = transfer_metrics.content_length,
        transfer_size = transfer_metrics.transfer_size,
        namelookup_ms = transfer_metrics.namelookup_ms,
        connect_ms = transfer_metrics.connect_ms,
        appconnect_ms = transfer_metrics.appconnect_ms,
        pretransfer_ms = transfer_metrics.pretransfer_ms,
        starttransfer_ms = transfer_metrics.starttransfer_ms,
        total_ms = transfer_metrics.total_ms,
        "fetch request complete"
    );
}

#[derive(Debug, Default)]
pub(crate) struct ResponseCollector {
    body: Vec<u8>,
    headers: Vec<(String, String)>,
    max_response_size: Option<usize>,
    response_too_large: bool,
    cancel_handle: Option<FetchCancelHandle>,
}

impl ResponseCollector {
    pub(crate) fn new(cancel_handle: Option<FetchCancelHandle>) -> Self {
        Self {
            cancel_handle,
            ..Self::default()
        }
    }

    pub(crate) fn begin_request(&mut self, max_response_size: Option<usize>) {
        self.body.clear();
        self.headers.clear();
        self.max_response_size = max_response_size;
        self.response_too_large = false;
    }

    pub(crate) fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    pub(crate) fn body(&self) -> &[u8] {
        &self.body
    }
}

#[derive(Debug)]
pub struct StreamingResponseCollector {
    cookie_store: SharedBrowserCookieStore,
    headers: Vec<(String, String)>,
    current_url: Option<Url>,
    current_cookie_context: Option<NetworkCookieRequestContext>,
    status: u16,
    max_response_size: Option<usize>,
    response_too_large: bool,
    response_bytes_received: usize,
    cache_body_writer: Option<HttpCacheBodyWriter>,
    cache_plan: Option<StreamingCachePlan>,
    started: bool,
    header_terminated: bool,
    start_tx: Option<oneshot::Sender<Result<StreamingHtmlResponseStart>>>,
    body_tx: Option<mpsc::UnboundedSender<String>>,
    request_cookie_report: Option<StoredCookieQueryReport>,
    response_credentials_allowed: bool,
    redirect_chain: Vec<RedirectInfo>,
    cookie_set_reports: Vec<StoredCookieSetReport>,
    callback_error: Option<String>,
    client_hint_response_policy: Option<ClientHintResponsePolicy>,
    client_hint_restart_requested: bool,
    cancel_handle: FetchCancelHandle,
    utf8_pending: Vec<u8>,
    negotiated_http_version: Option<NegotiatedHttpVersion>,
    network_request_extra_info: Option<NetworkRequestExtraInfo>,
}

#[derive(Debug, Clone)]
pub struct StreamingCachePlan {
    config: FetchConfig,
    request: Request,
    request_url: Url,
    cookie_header: Option<String>,
}

impl StreamingCachePlan {
    pub(crate) fn new(
        config: FetchConfig,
        request: Request,
        request_url: Url,
        cookie_header: Option<String>,
    ) -> Self {
        Self {
            config,
            request,
            request_url,
            cookie_header,
        }
    }

    fn create_body_writer(
        &self,
        status: u16,
        headers: &[(String, String)],
    ) -> Result<Option<HttpCacheBodyWriter>> {
        create_streaming_cache_body_writer_for_response_parts(
            &self.config,
            &self.request,
            &self.request_url,
            self.cookie_header.as_deref(),
            status,
            headers,
        )
    }
}

#[derive(Debug)]
pub struct RawStreamingResponseCollector {
    cookie_store: SharedBrowserCookieStore,
    headers: Vec<(String, String)>,
    current_url: Option<Url>,
    current_cookie_context: Option<NetworkCookieRequestContext>,
    status: u16,
    max_response_size: Option<usize>,
    response_too_large: bool,
    response_bytes_received: usize,
    declared_identity_body_length: Option<usize>,
    cache_body_writer: Option<HttpCacheBodyWriter>,
    cache_plan: Option<StreamingCachePlan>,
    started: bool,
    header_terminated: bool,
    start_tx: Option<oneshot::Sender<Result<StreamingHtmlResponseStart>>>,
    body_tx: Option<mpsc::UnboundedSender<Vec<u8>>>,
    request_cookie_report: Option<StoredCookieQueryReport>,
    response_credentials_allowed: bool,
    redirect_chain: Vec<RedirectInfo>,
    cookie_set_reports: Vec<StoredCookieSetReport>,
    defer_not_modified_start: bool,
    callback_error: Option<String>,
    client_hint_response_policy: Option<ClientHintResponsePolicy>,
    client_hint_restart_requested: bool,
    cancel_handle: FetchCancelHandle,
    negotiated_http_version: Option<NegotiatedHttpVersion>,
    network_request_extra_info: Option<NetworkRequestExtraInfo>,
}

impl StreamingResponseCollector {
    pub fn new(
        cookie_store: SharedBrowserCookieStore,
        start_tx: oneshot::Sender<Result<StreamingHtmlResponseStart>>,
        body_tx: mpsc::UnboundedSender<String>,
        cancel_handle: FetchCancelHandle,
    ) -> Self {
        Self {
            cookie_store,
            headers: Vec::new(),
            current_url: None,
            current_cookie_context: None,
            status: 0,
            max_response_size: None,
            response_too_large: false,
            response_bytes_received: 0,
            cache_body_writer: None,
            cache_plan: None,
            started: false,
            header_terminated: false,
            start_tx: Some(start_tx),
            body_tx: Some(body_tx),
            request_cookie_report: None,
            response_credentials_allowed: true,
            redirect_chain: Vec::new(),
            cookie_set_reports: Vec::new(),
            callback_error: None,
            client_hint_response_policy: None,
            client_hint_restart_requested: false,
            cancel_handle,
            utf8_pending: Vec::new(),
            negotiated_http_version: None,
            network_request_extra_info: None,
        }
    }

    pub fn begin_request(
        &mut self,
        max_response_size: Option<usize>,
        current_url: Url,
        current_cookie_context: NetworkCookieRequestContext,
        request_cookie_report: Option<StoredCookieQueryReport>,
        response_credentials_allowed: bool,
        redirect_chain: Vec<RedirectInfo>,
        cache_body_writer: Option<HttpCacheBodyWriter>,
    ) {
        self.headers.clear();
        self.current_url = Some(current_url);
        self.current_cookie_context = Some(current_cookie_context);
        self.status = 0;
        self.max_response_size = max_response_size;
        self.response_too_large = false;
        self.response_bytes_received = 0;
        self.cache_body_writer = cache_body_writer;
        self.cache_plan = None;
        self.started = false;
        self.header_terminated = false;
        self.request_cookie_report = request_cookie_report;
        self.response_credentials_allowed = response_credentials_allowed;
        self.redirect_chain = redirect_chain;
        self.network_request_extra_info = None;
        self.cookie_set_reports.clear();
        self.callback_error = None;
        self.client_hint_response_policy = None;
        self.client_hint_restart_requested = false;
        self.utf8_pending.clear();
        self.negotiated_http_version = None;
    }

    pub(crate) fn begin_request_with_cache_plan(
        &mut self,
        max_response_size: Option<usize>,
        current_url: Url,
        current_cookie_context: NetworkCookieRequestContext,
        request_cookie_report: Option<StoredCookieQueryReport>,
        response_credentials_allowed: bool,
        redirect_chain: Vec<RedirectInfo>,
        network_request_extra_info: Option<NetworkRequestExtraInfo>,
        cache_plan: Option<StreamingCachePlan>,
    ) {
        self.begin_request(
            max_response_size,
            current_url,
            current_cookie_context,
            request_cookie_report,
            response_credentials_allowed,
            redirect_chain,
            None,
        );
        self.network_request_extra_info = network_request_extra_info;
        self.cache_plan = cache_plan;
    }

    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    pub fn take_cache_body_writer(&mut self) -> Option<HttpCacheBodyWriter> {
        self.cache_body_writer.take()
    }

    pub fn status(&self) -> u16 {
        self.status
    }

    pub(crate) fn negotiated_http_version(&self) -> Option<NegotiatedHttpVersion> {
        self.negotiated_http_version
    }

    pub fn response_too_large_limit(&self) -> Option<usize> {
        if self.response_too_large {
            self.max_response_size
        } else {
            None
        }
    }

    pub fn started(&self) -> bool {
        self.started
    }

    pub fn header_terminated(&self) -> bool {
        self.header_terminated
    }

    pub fn take_cookie_set_reports(&mut self) -> Vec<StoredCookieSetReport> {
        std::mem::take(&mut self.cookie_set_reports)
    }

    pub fn take_callback_error(&mut self) -> Option<String> {
        self.callback_error.take()
    }

    pub(crate) fn set_client_hint_response_policy(&mut self, policy: ClientHintResponsePolicy) {
        self.client_hint_response_policy = Some(policy);
    }

    pub(crate) fn client_hint_restart_requested(&self) -> bool {
        self.client_hint_restart_requested
    }

    fn finalize_headers(&mut self) -> bool {
        if self.response_too_large {
            return false;
        }

        if is_interim_response_status(self.status) {
            return true;
        }

        let Some(current_url) = self.current_url.clone() else {
            return true;
        };

        if self.response_credentials_allowed {
            let Some(request_context) = self.current_cookie_context.as_ref() else {
                self.callback_error =
                    Some("response cookie context was not initialized".to_owned());
                return false;
            };
            match store_response_cookies(
                &self.cookie_store,
                &current_url,
                &self.headers,
                request_context,
            ) {
                Ok(reports) => {
                    self.cookie_set_reports = reports;
                }
                Err(error) => {
                    self.callback_error = Some(error.to_string());
                    return false;
                }
            }
        } else {
            self.cookie_set_reports.clear();
        }
        if self
            .client_hint_response_policy
            .as_ref()
            .is_some_and(|policy| {
                policy.observe_response(&current_url, &self.headers)
                    == ClientHintResponseAction::RestartNavigation
            })
        {
            self.client_hint_restart_requested = true;
            return false;
        }
        // Create the cache writer only after final response headers prove the
        // response is cacheable.
        self.maybe_create_cache_body_writer();
        self.maybe_emit_start();
        if response_body_ends_at_headers(self.status) && self.started {
            self.finish_streaming_body();
            self.header_terminated = true;
            return false;
        }
        true
    }

    fn maybe_create_cache_body_writer(&mut self) {
        if self.cache_body_writer.is_some() {
            self.cache_plan.take();
            return;
        }
        let Some(cache_plan) = self.cache_plan.take() else {
            return;
        };
        match cache_plan.create_body_writer(self.status, &self.headers) {
            Ok(Some(writer)) => {
                self.cache_body_writer = Some(writer);
            }
            Ok(None) => {}
            Err(error) => {
                debug!("failed to create streaming cache body writer: {error}");
            }
        }
    }

    fn maybe_emit_start(&mut self) {
        if self.started || self.response_too_large {
            return;
        }
        let Some(current_url) = self.current_url.clone() else {
            return;
        };
        if next_redirect_url_from_parts(&current_url, self.status, &self.headers, 0)
            .ok()
            .flatten()
            .is_some()
        {
            return;
        }
        self.started = true;
        if let Some(start_tx) = self.start_tx.take() {
            let _ = start_tx.send(Ok(StreamingHtmlResponseStart {
                final_url: current_url,
                status: self.status,
                headers: self.headers.clone(),
                request_cookie_report: self.request_cookie_report.clone(),
                cookie_set_reports: self.cookie_set_reports.clone(),
                redirected: !self.redirect_chain.is_empty(),
                redirect_chain: self.redirect_chain.clone(),
                from_cache: false,
                negotiated_http_version: self.negotiated_http_version,
                network_request_extra_info: self.network_request_extra_info.clone(),
            }));
        }
    }

    fn send_lossy_chunk(&mut self, chunk: &[u8]) {
        // Ignore bytes for interim/redirect responses until the final response
        // has been announced to the streaming consumer.
        if chunk.is_empty() || !self.started {
            return;
        }
        // Chunk events are high-cardinality on large documents. Keep them
        // behind the explicit nav-timing switch instead of relying only on the
        // subscriber's INFO filter, otherwise ordinary INFO logging becomes a
        // per-chunk hot path.
        if moli_trace::cdp_nav_timing_enabled() {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %self.current_url.as_ref().map(Url::as_str).unwrap_or(""),
                chunk_bytes = chunk.len(),
                response_bytes_received = self.response_bytes_received,
                stage = "streaming_html_chunk_send",
            );
        }
        let chunk = String::from_utf8_lossy(chunk).into_owned();
        if let Some(body_tx) = &self.body_tx {
            let send_failed = body_tx.send(chunk).is_err();
            if send_failed {
                self.cancel_handle.cancel();
                self.body_tx.take();
            }
        }
    }

    fn write_cache_body_bytes(&mut self, data: &[u8]) {
        if data.is_empty() || !self.started {
            return;
        }
        if let Some(cache_body_writer) = self.cache_body_writer.as_mut()
            && let Err(error) = cache_body_writer.write_all(data)
        {
            // Cache writes are best-effort. Like Chromium, a cache write failure
            // should not break delivery of the network response to the consumer.
            debug!("failed to append streaming response body to cache: {error}");
            self.cache_body_writer.take();
        }
    }

    fn stream_body_bytes(&mut self, data: &[u8]) {
        self.utf8_pending.extend_from_slice(data);
        loop {
            match str::from_utf8(&self.utf8_pending) {
                Ok(valid) => {
                    let valid = valid.as_bytes().to_vec();
                    self.send_lossy_chunk(&valid);
                    self.utf8_pending.clear();
                    break;
                }
                Err(error) => {
                    let valid_up_to = error.valid_up_to();
                    if valid_up_to > 0 {
                        let valid = self.utf8_pending[..valid_up_to].to_vec();
                        self.send_lossy_chunk(&valid);
                    }
                    match error.error_len() {
                        Some(error_len) => {
                            let invalid_end = valid_up_to + error_len;
                            let invalid = self.utf8_pending[valid_up_to..invalid_end].to_vec();
                            self.send_lossy_chunk(&invalid);
                            self.utf8_pending.drain(..invalid_end);
                        }
                        None => {
                            self.utf8_pending.drain(..valid_up_to);
                            break;
                        }
                    }
                }
            }
        }
    }

    pub fn finish_streaming_body(&mut self) {
        if !self.utf8_pending.is_empty() {
            let tail = std::mem::take(&mut self.utf8_pending);
            self.send_lossy_chunk(&tail);
        }
        self.body_tx.take();
    }

    pub(crate) fn take_response_channels(
        &mut self,
    ) -> (
        Option<oneshot::Sender<Result<StreamingHtmlResponseStart>>>,
        Option<mpsc::UnboundedSender<String>>,
    ) {
        (self.start_tx.take(), self.body_tx.take())
    }

    pub fn abort_started_body(&mut self) {
        self.body_tx.take();
    }

    pub fn fail(&mut self, error: Error) {
        if let Some(start_tx) = self.start_tx.take() {
            let _ = start_tx.send(Err(error));
        }
        self.body_tx.take();
    }
}

impl RawStreamingResponseCollector {
    pub fn new(
        cookie_store: SharedBrowserCookieStore,
        start_tx: oneshot::Sender<Result<StreamingHtmlResponseStart>>,
        body_tx: mpsc::UnboundedSender<Vec<u8>>,
        cancel_handle: FetchCancelHandle,
    ) -> Self {
        Self {
            cookie_store,
            headers: Vec::new(),
            current_url: None,
            current_cookie_context: None,
            status: 0,
            max_response_size: None,
            response_too_large: false,
            response_bytes_received: 0,
            declared_identity_body_length: None,
            cache_body_writer: None,
            cache_plan: None,
            started: false,
            header_terminated: false,
            start_tx: Some(start_tx),
            body_tx: Some(body_tx),
            request_cookie_report: None,
            response_credentials_allowed: true,
            redirect_chain: Vec::new(),
            cookie_set_reports: Vec::new(),
            defer_not_modified_start: false,
            callback_error: None,
            client_hint_response_policy: None,
            client_hint_restart_requested: false,
            cancel_handle,
            negotiated_http_version: None,
            network_request_extra_info: None,
        }
    }

    pub fn begin_request(
        &mut self,
        max_response_size: Option<usize>,
        current_url: Url,
        current_cookie_context: NetworkCookieRequestContext,
        request_cookie_report: Option<StoredCookieQueryReport>,
        response_credentials_allowed: bool,
        redirect_chain: Vec<RedirectInfo>,
    ) {
        self.headers.clear();
        self.current_url = Some(current_url);
        self.current_cookie_context = Some(current_cookie_context);
        self.status = 0;
        self.max_response_size = max_response_size;
        self.response_too_large = false;
        self.response_bytes_received = 0;
        self.declared_identity_body_length = None;
        self.cancel_handle.reset_response_progress();
        self.cache_body_writer = None;
        self.cache_plan = None;
        self.started = false;
        self.header_terminated = false;
        self.request_cookie_report = request_cookie_report;
        self.response_credentials_allowed = response_credentials_allowed;
        self.redirect_chain = redirect_chain;
        self.network_request_extra_info = None;
        self.cookie_set_reports.clear();
        self.defer_not_modified_start = false;
        self.callback_error = None;
        self.client_hint_response_policy = None;
        self.client_hint_restart_requested = false;
        self.negotiated_http_version = None;
    }

    pub(crate) fn begin_request_with_cache_plan(
        &mut self,
        max_response_size: Option<usize>,
        current_url: Url,
        current_cookie_context: NetworkCookieRequestContext,
        request_cookie_report: Option<StoredCookieQueryReport>,
        response_credentials_allowed: bool,
        redirect_chain: Vec<RedirectInfo>,
        network_request_extra_info: Option<NetworkRequestExtraInfo>,
        cache_plan: Option<StreamingCachePlan>,
        defer_not_modified_start: bool,
    ) {
        self.begin_request(
            max_response_size,
            current_url,
            current_cookie_context,
            request_cookie_report,
            response_credentials_allowed,
            redirect_chain,
        );
        self.network_request_extra_info = network_request_extra_info;
        self.cache_plan = cache_plan;
        self.defer_not_modified_start = defer_not_modified_start;
    }

    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    pub fn status(&self) -> u16 {
        self.status
    }

    pub(crate) fn negotiated_http_version(&self) -> Option<NegotiatedHttpVersion> {
        self.negotiated_http_version
    }

    pub fn response_too_large_limit(&self) -> Option<usize> {
        if self.response_too_large {
            self.max_response_size
        } else {
            None
        }
    }

    pub fn started(&self) -> bool {
        self.started
    }

    pub fn header_terminated(&self) -> bool {
        self.header_terminated
    }

    pub fn take_cookie_set_reports(&mut self) -> Vec<StoredCookieSetReport> {
        std::mem::take(&mut self.cookie_set_reports)
    }

    pub fn take_cache_body_writer(&mut self) -> Option<HttpCacheBodyWriter> {
        self.cache_body_writer.take()
    }

    pub fn take_callback_error(&mut self) -> Option<String> {
        self.callback_error.take()
    }

    pub(crate) fn set_client_hint_response_policy(&mut self, policy: ClientHintResponsePolicy) {
        self.client_hint_response_policy = Some(policy);
    }

    pub(crate) fn client_hint_restart_requested(&self) -> bool {
        self.client_hint_restart_requested
    }

    fn finalize_headers(&mut self) -> bool {
        if self.response_too_large {
            return false;
        }

        if is_interim_response_status(self.status) {
            return true;
        }

        let Some(current_url) = self.current_url.clone() else {
            return true;
        };

        if self.response_credentials_allowed {
            let Some(request_context) = self.current_cookie_context.as_ref() else {
                self.callback_error =
                    Some("response cookie context was not initialized".to_owned());
                return false;
            };
            match store_response_cookies(
                &self.cookie_store,
                &current_url,
                &self.headers,
                request_context,
            ) {
                Ok(reports) => {
                    self.cookie_set_reports = reports;
                }
                Err(error) => {
                    self.callback_error = Some(error.to_string());
                    return false;
                }
            }
        } else {
            self.cookie_set_reports.clear();
        }
        if self
            .client_hint_response_policy
            .as_ref()
            .is_some_and(|policy| {
                policy.observe_response(&current_url, &self.headers)
                    == ClientHintResponseAction::RestartNavigation
            })
        {
            self.client_hint_restart_requested = true;
            return false;
        }
        self.declared_identity_body_length = identity_encoded_content_length(&self.headers);
        if self.declared_identity_body_length == Some(0) {
            self.cancel_handle.mark_declared_response_body_complete();
        }
        self.maybe_create_cache_body_writer();
        self.maybe_emit_start();
        if response_body_ends_at_headers(self.status) && self.started {
            self.finish_streaming_body();
            self.header_terminated = true;
            return false;
        }
        true
    }

    fn maybe_create_cache_body_writer(&mut self) {
        if self.cache_body_writer.is_some() {
            self.cache_plan.take();
            return;
        }
        let Some(cache_plan) = self.cache_plan.take() else {
            return;
        };
        match cache_plan.create_body_writer(self.status, &self.headers) {
            Ok(Some(writer)) => {
                self.cache_body_writer = Some(writer);
            }
            Ok(None) => {}
            Err(error) => {
                debug!("failed to create raw streaming cache body writer: {error}");
            }
        }
    }

    fn maybe_emit_start(&mut self) {
        if self.started || self.response_too_large {
            return;
        }
        let Some(current_url) = self.current_url.clone() else {
            return;
        };
        if next_redirect_url_from_parts(&current_url, self.status, &self.headers, 0)
            .ok()
            .flatten()
            .is_some()
        {
            return;
        }
        if self.defer_not_modified_start && self.status == 304 {
            return;
        }
        self.started = true;
        if let Some(start_tx) = self.start_tx.take() {
            let _ = start_tx.send(Ok(StreamingHtmlResponseStart {
                final_url: current_url,
                status: self.status,
                headers: self.headers.clone(),
                request_cookie_report: self.request_cookie_report.clone(),
                cookie_set_reports: self.cookie_set_reports.clone(),
                redirected: !self.redirect_chain.is_empty(),
                redirect_chain: self.redirect_chain.clone(),
                from_cache: false,
                negotiated_http_version: self.negotiated_http_version,
                network_request_extra_info: self.network_request_extra_info.clone(),
            }));
        }
    }

    fn send_chunk(&mut self, chunk: &[u8]) {
        // Ignore bytes for interim/redirect responses until the final response
        // has been announced to the streaming consumer.
        if chunk.is_empty() || !self.started {
            return;
        }
        // Raw streaming uses the same high-cardinality path as decoded HTML
        // streaming, so keep this aligned with the HTML collector gate above.
        if moli_trace::cdp_nav_timing_enabled() {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %self.current_url.as_ref().map(Url::as_str).unwrap_or(""),
                chunk_bytes = chunk.len(),
                response_bytes_received = self.response_bytes_received,
                stage = "streaming_raw_chunk_send",
            );
        }
        if let Some(body_tx) = &self.body_tx {
            let send_failed = body_tx.send(chunk.to_vec()).is_err();
            if send_failed {
                self.cancel_handle.cancel();
                self.body_tx.take();
            }
        }
    }

    fn write_cache_body_bytes(&mut self, data: &[u8]) {
        if data.is_empty() || !self.started {
            return;
        }
        if let Some(cache_body_writer) = self.cache_body_writer.as_mut()
            && let Err(error) = cache_body_writer.write_all(data)
        {
            debug!("failed to append raw streaming response body to cache: {error}");
            self.cache_body_writer.take();
        }
    }

    pub fn finish_streaming_body(&mut self) {
        self.body_tx.take();
    }

    pub fn take_response_channels(
        &mut self,
    ) -> (
        Option<oneshot::Sender<Result<StreamingHtmlResponseStart>>>,
        Option<mpsc::UnboundedSender<Vec<u8>>>,
    ) {
        (self.start_tx.take(), self.body_tx.take())
    }

    pub fn abort_started_body(&mut self) {
        self.body_tx.take();
    }

    pub fn fail(&mut self, error: Error) {
        if let Some(start_tx) = self.start_tx.take() {
            let _ = start_tx.send(Err(error));
        }
        self.body_tx.take();
    }
}

impl Handler for ResponseCollector {
    fn write(&mut self, data: &[u8]) -> std::result::Result<usize, WriteError> {
        if let Some(limit) = self.max_response_size
            && self.body.len() + data.len() > limit
        {
            self.response_too_large = true;
            return Ok(0);
        }

        self.body.extend_from_slice(data);
        Ok(data.len())
    }

    fn progress(&mut self, _: f64, _: f64, _: f64, _: f64) -> bool {
        !self
            .cancel_handle
            .as_ref()
            .is_some_and(FetchCancelHandle::is_cancelled)
    }

    fn header(&mut self, data: &[u8]) -> bool {
        let line = String::from_utf8_lossy(data);
        let line = line.trim_end_matches(['\r', '\n']);

        if line.is_empty() {
            return !self.response_too_large;
        }

        if line.starts_with("HTTP/") {
            self.headers.clear();
            self.body.clear();
            return true;
        }

        let Some((name, value)) = line.split_once(':') else {
            return true;
        };

        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_owned();

        if name == "content-length"
            && self
                .max_response_size
                .zip(value.parse::<usize>().ok())
                .is_some_and(|(limit, content_length)| content_length > limit)
        {
            self.response_too_large = true;
            return false;
        }

        self.headers.push((name, value));
        true
    }
}

impl Handler for StreamingResponseCollector {
    fn write(&mut self, data: &[u8]) -> std::result::Result<usize, WriteError> {
        if self.cancel_handle.is_cancelled() {
            return Ok(0);
        }

        if let Some(limit) = self.max_response_size
            && self.response_bytes_received + data.len() > limit
        {
            self.response_too_large = true;
            return Ok(0);
        }

        self.response_bytes_received += data.len();
        self.write_cache_body_bytes(data);
        self.stream_body_bytes(data);
        if self.cancel_handle.is_cancelled() {
            return Ok(0);
        }
        Ok(data.len())
    }

    fn header(&mut self, data: &[u8]) -> bool {
        let line = String::from_utf8_lossy(data);
        let line = line.trim_end_matches(['\r', '\n']);

        if line.is_empty() {
            return self.finalize_headers();
        }

        if let Some(status) = line
            .strip_prefix("HTTP/")
            .and_then(|rest| rest.split_whitespace().nth(1))
            .and_then(|status| status.parse::<u16>().ok())
        {
            self.headers.clear();
            self.status = status;
            self.negotiated_http_version = NegotiatedHttpVersion::from_status_line(line);
            self.response_bytes_received = 0;
            self.response_too_large = false;
            self.cookie_set_reports.clear();
            return true;
        }

        let Some((name, value)) = line.split_once(':') else {
            return true;
        };

        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_owned();

        if name == "content-length"
            && self
                .max_response_size
                .zip(value.parse::<usize>().ok())
                .is_some_and(|(limit, content_length)| content_length > limit)
        {
            self.response_too_large = true;
            return false;
        }

        self.headers.push((name, value));
        true
    }

    fn progress(&mut self, _: f64, _: f64, _: f64, _: f64) -> bool {
        !self.cancel_handle.is_cancelled()
    }
}

impl Handler for RawStreamingResponseCollector {
    fn write(&mut self, data: &[u8]) -> std::result::Result<usize, WriteError> {
        if self.cancel_handle.is_cancelled() {
            return Ok(0);
        }

        if let Some(limit) = self.max_response_size
            && self.response_bytes_received + data.len() > limit
        {
            self.response_too_large = true;
            return Ok(0);
        }

        self.response_bytes_received += data.len();
        if self.declared_identity_body_length == Some(self.response_bytes_received) {
            self.cancel_handle.mark_declared_response_body_complete();
        }
        self.write_cache_body_bytes(data);
        self.send_chunk(data);
        if self.cancel_handle.is_cancelled() {
            return Ok(0);
        }
        Ok(data.len())
    }

    fn header(&mut self, data: &[u8]) -> bool {
        let line = String::from_utf8_lossy(data);
        let line = line.trim_end_matches(['\r', '\n']);

        if line.is_empty() {
            return self.finalize_headers();
        }

        if let Some(status) = line
            .strip_prefix("HTTP/")
            .and_then(|rest| rest.split_whitespace().nth(1))
            .and_then(|status| status.parse::<u16>().ok())
        {
            self.headers.clear();
            self.status = status;
            self.negotiated_http_version = NegotiatedHttpVersion::from_status_line(line);
            self.response_bytes_received = 0;
            self.response_too_large = false;
            self.cookie_set_reports.clear();
            return true;
        }

        let Some((name, value)) = line.split_once(':') else {
            return true;
        };

        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_owned();

        if name == "content-length"
            && self
                .max_response_size
                .zip(value.parse::<usize>().ok())
                .is_some_and(|(limit, content_length)| content_length > limit)
        {
            self.response_too_large = true;
            return false;
        }

        self.headers.push((name, value));
        true
    }

    fn progress(&mut self, _: f64, _: f64, _: f64, _: f64) -> bool {
        !self.cancel_handle.is_cancelled()
    }
}

fn identity_encoded_content_length(headers: &[(String, String)]) -> Option<usize> {
    let cannot_compare_delivered_body_length = headers.iter().any(|(name, value)| {
        (name.eq_ignore_ascii_case("content-encoding")
            && !value.trim().eq_ignore_ascii_case("identity"))
            || name.eq_ignore_ascii_case("transfer-encoding")
    });
    if cannot_compare_delivered_body_length {
        return None;
    }

    let mut values = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .flat_map(|(_, value)| value.split(','))
        .map(|value| value.trim().parse::<usize>());
    let first = values.next()?.ok()?;
    values
        .all(|value| value.ok() == Some(first))
        .then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_cookie_jar::new_shared_browser_cookie_store;

    fn raw_streaming_collector_with_headers(
        cancel_handle: FetchCancelHandle,
        headers: &[&str],
    ) -> (
        RawStreamingResponseCollector,
        mpsc::UnboundedReceiver<Vec<u8>>,
    ) {
        let (start_tx, _start_rx) = oneshot::channel();
        let (body_tx, body_rx) = mpsc::unbounded_channel();
        let mut collector = RawStreamingResponseCollector::new(
            new_shared_browser_cookie_store(),
            start_tx,
            body_tx,
            cancel_handle,
        );
        collector.begin_request(
            None,
            Url::parse("https://example.test/events").unwrap(),
            NetworkCookieRequestContext::top_level_navigation("GET"),
            None,
            true,
            Vec::new(),
        );
        assert!(collector.header(b"HTTP/1.1 200 OK\r\n"));
        for header in headers {
            assert!(collector.header(format!("{header}\r\n").as_bytes()));
        }
        assert!(collector.header(b"\r\n"));
        (collector, body_rx)
    }

    #[test]
    fn response_easy_with_cancel_observes_shutdown_token() {
        let cancel_handle = FetchCancelHandle::new();
        let mut easy = Easy2::new(ResponseCollector::new(Some(cancel_handle.clone())));

        assert!(easy.get_mut().progress(0.0, 0.0, 0.0, 0.0));

        cancel_handle.cancel();

        assert!(!easy.get_mut().progress(0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn raw_streaming_exact_identity_content_length_commits_response_completion() {
        let cancel_handle = FetchCancelHandle::new();
        let (mut collector, _body_rx) =
            raw_streaming_collector_with_headers(cancel_handle.clone(), &["Content-Length: 4"]);

        assert_eq!(collector.write(b"abc").unwrap(), 3);
        assert!(!cancel_handle.response_completion_is_committed());
        assert_eq!(collector.write(b"d").unwrap(), 1);
        assert!(cancel_handle.response_completion_is_committed());
    }

    #[test]
    fn raw_streaming_framed_or_encoded_body_waits_for_transport_terminal() {
        for headers in [
            ["Content-Length: 4", "Transfer-Encoding: chunked"],
            ["Content-Length: 4", "Content-Encoding: gzip"],
        ] {
            let cancel_handle = FetchCancelHandle::new();
            let (mut collector, _body_rx) =
                raw_streaming_collector_with_headers(cancel_handle.clone(), &headers);

            assert_eq!(collector.write(b"body").unwrap(), 4);
            assert!(!cancel_handle.response_completion_is_committed());
        }
    }
}
