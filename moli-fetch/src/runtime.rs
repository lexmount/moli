use std::{
    any::Any,
    backtrace::Backtrace,
    cell::RefCell,
    collections::BTreeSet,
    ffi::c_long,
    fmt,
    io::Read,
    marker::PhantomData,
    num::{NonZeroU32, NonZeroUsize},
    rc::Rc,
    sync::{
        Arc, Once,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{Receiver, Sender};
use curl::easy::{Easy2, Handler, InfoType, WriteError};
use moli_cookie_jar::{
    NetworkCookieRequestContext, SharedBrowserCookieStore, StoredCookieQueryReport,
    advance_cookie_request_context,
};
use moli_curl::{
    CurlMultiCompletion, CurlMultiJob, CurlMultiRuntime, CurlMultiRuntimeConfig, CurlOriginKey,
};
use moli_url_policy::ensure_http_network_transport_url;
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};
use url::{Host, Url};

use crate::{
    FetchCancelHandle, FetchConfig, NegotiatedHttpVersion, NetworkFetchFailureContext,
    NetworkFetchFailureRequestContext, NetworkRequestExtraInfo, NetworkResponseExtraInfo,
    RawResponse, RedirectInfo, Request, Response, ResponseHead, StreamingHtmlResponse,
    StreamingRawResponse,
    blocking::{
        CachedStreamingResponseLookup, RawStreamingResponseCollector, RequestHttpVersion,
        RequestTransferMetrics, ResponseCollector, StreamingCachePlan, StreamingHtmlResponseStart,
        StreamingResponseCollector, cached_streaming_response_body_exceeds_response_limit,
        cached_streaming_response_is_stale, configure_easy, configure_openssl_tls_context,
        cookie_access_report_for_request, cookie_header_from_report,
        finish_streaming_cached_response, load_cached_streaming_response_lookup,
        log_request_completion, merge_cached_not_modified_streaming_response_lookup,
        network_request_extra_info_from_headers, next_followed_redirect_url_from_parts,
        remove_cached_response, response_headers_forbid_cache_storage, store_response_cookies,
        transfer_metrics_from_easy, validation_headers_for_cached_streaming_response_lookup,
    },
    client_hints::{
        ClientHintResponseAction, ClientHintResponsePolicy, SharedClientHintPreferences,
        SharedNavigationClientHintRestarts, prepare_client_hint_request,
    },
    dns::curl_dns_resolution,
    network_fetch_result::NetworkObservationRecorder,
    proxy_connect::{ProxyConnectResponse, ProxyConnectResponseRecorder},
};

const DEFAULT_RUNTIME_TRANSFERS: usize = 256;
const RUNTIME_POLL_INTERVAL: Duration = Duration::from_millis(50);
// curl-sys does not currently expose CURLINFO_HTTP_VERSION. This value is
// CURLINFO_LONG + 46 in curl's public curl.h ABI.
const CURLINFO_HTTP_VERSION: curl_sys::CURLINFO = curl_sys::CURLINFO_LONG + 46;
static NEXT_FETCH_RUNTIME_ID: AtomicU64 = AtomicU64::new(0);
static INSTALL_FETCH_RUNTIME_PANIC_HOOK: Once = Once::new();

thread_local! {
    /// Panic diagnostics are opt-in per semantic owner thread. The process-wide
    /// hook below observes all panics so that it can preserve the previously
    /// installed hook, but only a thread with this slot populated records fetch
    /// runtime evidence.
    static FETCH_RUNTIME_PANIC_CAPTURE: RefCell<Option<Arc<Mutex<Option<FetchRuntimePanicEvidence>>>>> =
        const { RefCell::new(None) };
}

#[derive(Clone, Debug)]
struct FetchRuntimePanicEvidence {
    location: Option<String>,
    backtrace: String,
}

/// Install one process-wide, chained panic hook.
///
/// Rust exposes panic-site location only to panic hooks, not through a joined
/// thread's payload. The hook therefore records diagnostics into a semantic
/// thread-local, per-runtime sink and then invokes the hook that was installed
/// before moli-fetch. Runtimes never share a sink, and panics on all
/// other threads are observationally unchanged. As with every process-wide
/// hook, an embedding application that replaces the hook later must chain the
/// hook it takes if it wants fetch panic diagnostics to remain available.
fn install_fetch_runtime_panic_hook() {
    INSTALL_FETCH_RUNTIME_PANIC_HOOK.call_once(|| {
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            let _ = FETCH_RUNTIME_PANIC_CAPTURE.try_with(|capture| {
                let Ok(capture) = capture.try_borrow() else {
                    return;
                };
                let Some(capture) = capture.as_ref() else {
                    return;
                };
                // Keep the most recent panic. If semantic code ever catches an
                // earlier unwind, the later uncaught JoinHandle payload must
                // not be paired with stale location/backtrace evidence.
                *capture.lock() = Some(FetchRuntimePanicEvidence {
                    location: panic_info.location().map(|location| {
                        format!(
                            "{}:{}:{}",
                            location.file(),
                            location.line(),
                            location.column()
                        )
                    }),
                    backtrace: Backtrace::force_capture().to_string(),
                });
            });
            previous_hook(panic_info);
        }));
    });
}

struct FetchRuntimePanicCaptureGuard {
    previous: Option<Arc<Mutex<Option<FetchRuntimePanicEvidence>>>>,
}

impl FetchRuntimePanicCaptureGuard {
    fn enter(capture: Arc<Mutex<Option<FetchRuntimePanicEvidence>>>) -> Self {
        let previous = FETCH_RUNTIME_PANIC_CAPTURE.with(|active| active.replace(Some(capture)));
        Self { previous }
    }
}

impl Drop for FetchRuntimePanicCaptureGuard {
    fn drop(&mut self) {
        FETCH_RUNTIME_PANIC_CAPTURE.with(|active| {
            active.replace(self.previous.take());
        });
    }
}

/// Cloneable request-side access to the fetch semantic owner.
///
/// This handle deliberately does not own the semantic thread's `JoinHandle`.
/// It is therefore safe for completion callbacks running on that thread to
/// capture and release the last request-side handle.
#[derive(Clone, Debug)]
pub(crate) struct FetchRuntimeHandle {
    inner: Arc<FetchRuntimeInner>,
}

#[derive(Debug)]
struct FetchRuntimeInner {
    request_tx: Sender<RuntimeCommand>,
    shutdown_requested: Arc<AtomicBool>,
    #[cfg(test)]
    owner_started: Arc<AtomicBool>,
}

/// Unique structured-concurrency owner of one fetch semantic thread.
///
/// Request-side code receives only [`FetchRuntimeHandle`]. The owner remains
/// at the browser/network-runtime lifetime boundary and is the only value that
/// can join the semantic thread.
#[derive(Debug)]
pub(crate) struct FetchRuntimeOwner {
    handle: FetchRuntimeHandle,
    owner_thread: Option<thread::JoinHandle<()>>,
    identity: FetchRuntimeIdentity,
    panic_evidence: Arc<Mutex<Option<FetchRuntimePanicEvidence>>>,
    join_report: Option<FetchRuntimeJoinReport>,
    panic_logged: bool,
    #[cfg(test)]
    panic_log_count: Arc<std::sync::atomic::AtomicUsize>,
    _thread_affine: PhantomData<Rc<()>>,
}

/// Stable identity of the semantic runtime whose owner was joined.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchRuntimeIdentity {
    runtime_id: u64,
    thread_name: String,
    thread_id: String,
}

impl FetchRuntimeIdentity {
    pub fn runtime_id(&self) -> u64 {
        self.runtime_id
    }

    pub fn thread_name(&self) -> &str {
        &self.thread_name
    }

    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }
}

/// Panic evidence recovered from the semantic thread's join payload.
///
/// A chained panic hook captures location and a forced backtrace on the
/// semantic owner thread before `JoinHandle` reduces the failure to a payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchRuntimePanicReport {
    payload: String,
    location: Option<String>,
    backtrace: Option<String>,
}

impl FetchRuntimePanicReport {
    pub fn payload(&self) -> &str {
        &self.payload
    }

    pub fn location(&self) -> Option<&str> {
        self.location.as_deref()
    }

    pub fn backtrace(&self) -> Option<&str> {
        self.backtrace.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FetchRuntimeJoinStatus {
    Clean,
    Panicked(FetchRuntimePanicReport),
}

/// Structured result of joining a fetch semantic runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchRuntimeJoinReport {
    identity: FetchRuntimeIdentity,
    status: FetchRuntimeJoinStatus,
}

impl FetchRuntimeJoinReport {
    pub fn identity(&self) -> &FetchRuntimeIdentity {
        &self.identity
    }

    pub fn status(&self) -> &FetchRuntimeJoinStatus {
        &self.status
    }

    pub fn is_clean(&self) -> bool {
        matches!(self.status, FetchRuntimeJoinStatus::Clean)
    }

    pub fn panic_report(&self) -> Option<&FetchRuntimePanicReport> {
        match &self.status {
            FetchRuntimeJoinStatus::Clean => None,
            FetchRuntimeJoinStatus::Panicked(report) => Some(report),
        }
    }
}

enum RuntimeCommand {
    Request(RuntimeJob),
    StreamingHtmlRequest(StreamingRuntimeJob),
    StreamingRawRequest(StreamingRawRuntimeJob),
    #[cfg(test)]
    PanicForTesting(Sender<()>),
    Shutdown,
}

type RuntimeTextResponseTx = oneshot::Sender<Result<Response>>;
pub(crate) type RuntimeTextResponseCallback = Box<dyn FnOnce(Result<Response>) + Send + 'static>;
type RuntimeRawResponseTx = oneshot::Sender<Result<RawResponse>>;
type RuntimeStreamingCompletionTx = oneshot::Sender<Result<()>>;
type RuntimeCurlCompletion = CurlMultiCompletion<FetchTransferHandler, ActiveTransferContext>;

enum RuntimeResponseTx {
    Text(RuntimeTextResponseTx),
    TextCallback(RuntimeTextResponseCallback),
    Raw(RuntimeRawResponseTx),
}

impl fmt::Debug for RuntimeResponseTx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(_) => f.write_str("RuntimeResponseTx::Text"),
            Self::TextCallback(_) => f.write_str("RuntimeResponseTx::TextCallback"),
            Self::Raw(_) => f.write_str("RuntimeResponseTx::Raw"),
        }
    }
}

impl RuntimeResponseTx {
    fn send(self, response: Result<CompletedBufferedResponse>) {
        match self {
            Self::Text(tx) => {
                let _ = tx.send(response.map(CompletedBufferedResponse::into_text_response));
            }
            Self::TextCallback(callback) => {
                callback(response.map(CompletedBufferedResponse::into_text_response));
            }
            Self::Raw(tx) => {
                let _ = tx
                    .send(response.map(CompletedBufferedResponse::into_materialized_raw_response));
            }
        }
    }
}

pub(crate) struct PendingStreamingHtmlResponse {
    started_rx: oneshot::Receiver<Result<StreamingHtmlResponseStart>>,
    body_rx: mpsc::UnboundedReceiver<String>,
    cancel_handle: FetchCancelHandle,
    completion_rx: oneshot::Receiver<Result<()>>,
}

impl PendingStreamingHtmlResponse {
    pub(crate) async fn into_response(self) -> Result<StreamingHtmlResponse> {
        let started = self
            .started_rx
            .await
            .map_err(|_| anyhow!("streaming html start channel closed"))??;
        let network_request_extra_info = started.network_request_extra_info.clone();
        Ok(StreamingHtmlResponse::new_with_head(
            started.into_head(),
            self.body_rx,
            self.cancel_handle,
            self.completion_rx,
        )
        .with_network_request_extra_info(network_request_extra_info))
    }
}

pub struct PendingStreamingRawResponse {
    started_rx: oneshot::Receiver<Result<StreamingHtmlResponseStart>>,
    body_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    cancel_handle: FetchCancelHandle,
    completion_rx: oneshot::Receiver<Result<()>>,
}

impl PendingStreamingRawResponse {
    pub async fn into_response(self) -> Result<StreamingRawResponse> {
        let started = self
            .started_rx
            .await
            .map_err(|_| anyhow!("streaming raw start channel closed"))??;
        let network_request_extra_info = started.network_request_extra_info.clone();
        Ok(StreamingRawResponse::new_with_head(
            started.into_head(),
            self.body_rx,
            self.cancel_handle,
            self.completion_rx,
        )
        .with_network_request_extra_info(network_request_extra_info))
    }
}

impl FetchRuntimeOwner {
    #[cfg(test)]
    pub(crate) fn new(config: &FetchConfig, cookie_store: SharedBrowserCookieStore) -> Self {
        Self::new_with_client_hint_preferences(
            config,
            cookie_store,
            Arc::new(Mutex::new(
                crate::client_hints::ClientHintPreferences::default(),
            )),
        )
    }

    pub(crate) fn new_with_client_hint_preferences(
        config: &FetchConfig,
        cookie_store: SharedBrowserCookieStore,
        client_hint_preferences: SharedClientHintPreferences,
    ) -> Self {
        let runtime_id = NEXT_FETCH_RUNTIME_ID
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        let (request_tx, request_rx) = crossbeam_channel::unbounded();
        let shutdown_requested = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let owner_started = Arc::new(AtomicBool::new(false));
        let (curl_runtime, curl_completion_rx) = CurlMultiRuntime::new(curl_runtime_config(config))
            .expect("failed to start fetch curl multi runtime");
        let owner = RuntimeOwner {
            config: config.clone(),
            cookie_store,
            client_hint_preferences,
            request_rx,
            curl_runtime,
            curl_completion_rx,
            shutdown_requested: Arc::clone(&shutdown_requested),
            #[cfg(test)]
            owner_started: Arc::clone(&owner_started),
        };
        install_fetch_runtime_panic_hook();
        let panic_evidence = Arc::new(Mutex::new(None));
        let thread_panic_evidence = Arc::clone(&panic_evidence);
        let owner_handle = thread::Builder::new()
            .name("lm-fetch-semantics".to_owned())
            .spawn(move || {
                let _panic_capture = FetchRuntimePanicCaptureGuard::enter(thread_panic_evidence);
                owner.run();
            })
            .expect("failed to spawn fetch runtime semantic owner thread");
        let identity = FetchRuntimeIdentity {
            runtime_id,
            thread_name: owner_handle
                .thread()
                .name()
                .unwrap_or("unnamed-fetch-runtime")
                .to_owned(),
            thread_id: format!("{:?}", owner_handle.thread().id()),
        };

        let handle = FetchRuntimeHandle {
            inner: Arc::new(FetchRuntimeInner {
                request_tx,
                shutdown_requested,
                #[cfg(test)]
                owner_started,
            }),
        };
        Self {
            handle,
            owner_thread: Some(owner_handle),
            identity,
            panic_evidence,
            join_report: None,
            panic_logged: false,
            #[cfg(test)]
            panic_log_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            _thread_affine: PhantomData,
        }
    }

    pub(crate) fn handle(&self) -> FetchRuntimeHandle {
        self.handle.clone()
    }

    #[cfg(test)]
    pub(crate) fn shutdown(mut self) -> FetchRuntimeJoinReport {
        self.request_shutdown();
        self.join()
    }

    pub(crate) fn request_shutdown(&self) {
        self.handle.request_shutdown();
    }

    #[cfg(test)]
    pub(crate) fn panic_log_count_for_testing(&self) -> Arc<std::sync::atomic::AtomicUsize> {
        Arc::clone(&self.panic_log_count)
    }

    pub(crate) fn join(&mut self) -> FetchRuntimeJoinReport {
        self.request_shutdown();
        if let Some(owner_thread) = self.owner_thread.take() {
            let status = match owner_thread.join() {
                Ok(()) => FetchRuntimeJoinStatus::Clean,
                Err(payload) => FetchRuntimeJoinStatus::Panicked(panic_report(
                    payload,
                    self.panic_evidence.lock().clone(),
                )),
            };
            self.join_report = Some(FetchRuntimeJoinReport {
                identity: self.identity.clone(),
                status,
            });
        }
        self.join_report
            .clone()
            .expect("a joined fetch runtime must retain its terminal report")
    }
}

impl std::ops::Deref for FetchRuntimeOwner {
    type Target = FetchRuntimeHandle;

    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl Drop for FetchRuntimeOwner {
    fn drop(&mut self) {
        self.request_shutdown();
        let report = self.join();
        if let Some(panic) = report.panic_report()
            && !self.panic_logged
        {
            self.panic_logged = true;
            #[cfg(test)]
            self.panic_log_count.fetch_add(1, Ordering::SeqCst);
            tracing::error!(
                runtime_id = report.identity().runtime_id(),
                thread_name = report.identity().thread_name(),
                thread_id = report.identity().thread_id(),
                panic_payload = panic.payload(),
                panic_location = panic.location().unwrap_or("unknown"),
                panic_backtrace = panic.backtrace().unwrap_or("unavailable"),
                "fetch runtime semantic owner panicked while being joined"
            );
        }
    }
}

fn panic_report(
    payload: Box<dyn Any + Send + 'static>,
    evidence: Option<FetchRuntimePanicEvidence>,
) -> FetchRuntimePanicReport {
    let payload = if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    };
    let (location, backtrace) = evidence
        .map(|evidence| (evidence.location, Some(evidence.backtrace)))
        .unwrap_or((None, None));
    FetchRuntimePanicReport {
        payload,
        location,
        backtrace,
    }
}

impl FetchRuntimeHandle {
    #[cfg(test)]
    pub(crate) fn submit(&self, request: Request) -> Result<oneshot::Receiver<Result<Response>>> {
        self.submit_with_cancel(request, FetchCancelHandle::new())
    }

    pub(crate) fn submit_auth_raw(
        &self,
        request: Request,
    ) -> Result<oneshot::Receiver<Result<RawResponse>>> {
        debug_assert!(
            request.auth_requires_buffered_transport(),
            "buffered raw fetch is reserved for auth credential replay"
        );
        let (response_tx, response_rx) = oneshot::channel();
        self.enqueue(RuntimeJob::new(
            request,
            RuntimeResponseTx::Raw(response_tx),
            FetchCancelHandle::new(),
        ))?;
        Ok(response_rx)
    }

    pub(crate) fn submit_with_cancel(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
    ) -> Result<oneshot::Receiver<Result<Response>>> {
        let (response_tx, response_rx) = oneshot::channel();
        self.enqueue(RuntimeJob::new(
            request,
            RuntimeResponseTx::Text(response_tx),
            cancel_handle,
        ))?;
        Ok(response_rx)
    }

    pub(crate) fn submit_with_cancel_callback(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
        callback: RuntimeTextResponseCallback,
    ) -> Result<()> {
        self.enqueue(RuntimeJob::new(
            request,
            RuntimeResponseTx::TextCallback(callback),
            cancel_handle,
        ))
    }

    pub(crate) fn submit_html_stream(
        &self,
        request: Request,
    ) -> Result<PendingStreamingHtmlResponse> {
        let (started_tx, started_rx) = oneshot::channel();
        let (body_tx, body_rx) = mpsc::unbounded_channel();
        let (completion_tx, completion_rx) = oneshot::channel();
        let cancel_handle = FetchCancelHandle::new();
        let job = StreamingRuntimeJob::new(
            request,
            started_tx,
            body_tx,
            completion_tx,
            cancel_handle.clone(),
        );
        self.enqueue_streaming(job)?;
        Ok(PendingStreamingHtmlResponse {
            started_rx,
            body_rx,
            cancel_handle,
            completion_rx,
        })
    }

    pub(crate) fn submit_raw_stream(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
    ) -> Result<PendingStreamingRawResponse> {
        let (started_tx, started_rx) = oneshot::channel();
        let (body_tx, body_rx) = mpsc::unbounded_channel();
        let (completion_tx, completion_rx) = oneshot::channel();
        let job = StreamingRawRuntimeJob::new(
            request,
            started_tx,
            body_tx,
            completion_tx,
            cancel_handle.clone(),
        );
        self.enqueue_raw_streaming(job)?;
        Ok(PendingStreamingRawResponse {
            started_rx,
            body_rx,
            cancel_handle,
            completion_rx,
        })
    }

    fn enqueue(&self, job: RuntimeJob) -> Result<()> {
        ensure_http_network_transport_url(&job.current_url)?;
        if self.inner.shutdown_requested.load(Ordering::SeqCst) {
            return Err(anyhow!("fetch runtime is shutting down"));
        }
        self.inner
            .request_tx
            .send(RuntimeCommand::Request(job))
            .map_err(|_| anyhow!("fetch runtime is shutting down"))?;
        Ok(())
    }

    fn enqueue_streaming(&self, job: StreamingRuntimeJob) -> Result<()> {
        ensure_http_network_transport_url(&job.current_url)?;
        if self.inner.shutdown_requested.load(Ordering::SeqCst) {
            return Err(anyhow!("fetch runtime is shutting down"));
        }
        self.inner
            .request_tx
            .send(RuntimeCommand::StreamingHtmlRequest(job))
            .map_err(|_| anyhow!("fetch runtime is shutting down"))?;
        Ok(())
    }

    fn enqueue_raw_streaming(&self, job: StreamingRawRuntimeJob) -> Result<()> {
        ensure_http_network_transport_url(&job.current_url)?;
        if self.inner.shutdown_requested.load(Ordering::SeqCst) {
            return Err(anyhow!("fetch runtime is shutting down"));
        }
        self.inner
            .request_tx
            .send(RuntimeCommand::StreamingRawRequest(job))
            .map_err(|_| anyhow!("fetch runtime is shutting down"))?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn owner_count_for_testing(&self) -> usize {
        usize::from(self.inner.owner_started.load(Ordering::SeqCst))
    }

    #[cfg(test)]
    pub(crate) fn panic_owner_for_testing(&self) {
        let (admitted_tx, admitted_rx) = crossbeam_channel::bounded(1);
        self.inner
            .request_tx
            .send(RuntimeCommand::PanicForTesting(admitted_tx))
            .expect("fetch runtime owner should accept the test panic command");
        admitted_rx
            .recv()
            .expect("fetch runtime owner should admit the test panic command");
    }

    pub(crate) fn request_shutdown(&self) {
        let first_shutdown = !self.inner.shutdown_requested.swap(true, Ordering::SeqCst);
        if first_shutdown {
            let _ = self.inner.request_tx.send(RuntimeCommand::Shutdown);
        }
    }
}

struct RuntimeOwner {
    config: FetchConfig,
    cookie_store: SharedBrowserCookieStore,
    client_hint_preferences: SharedClientHintPreferences,
    request_rx: Receiver<RuntimeCommand>,
    curl_runtime: CurlMultiRuntime<FetchTransferHandler, ActiveTransferContext>,
    curl_completion_rx: Receiver<RuntimeCurlCompletion>,
    shutdown_requested: Arc<AtomicBool>,
    #[cfg(test)]
    owner_started: Arc<AtomicBool>,
}

impl RuntimeOwner {
    fn run(self) {
        #[cfg(test)]
        self.owner_started.store(true, Ordering::SeqCst);
        let mut state = OwnerState::default();

        loop {
            self.drain_commands(&mut state);
            self.drain_curl_completions(&mut state);

            if state.closed && state.active_transfers == 0 {
                return;
            }

            if state.closed {
                self.wait_for_curl_completion(&mut state);
                continue;
            }

            crossbeam_channel::select! {
                recv(self.request_rx) -> command => self.handle_command_result(&mut state, command),
                recv(self.curl_completion_rx) -> completion => self.handle_completion_result(&mut state, completion),
            }
        }
    }

    fn drain_commands(&self, state: &mut OwnerState) {
        loop {
            match self.request_rx.try_recv() {
                Ok(command) => self.handle_command(state, command),
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.close(state);
                    break;
                }
            }
        }
    }

    fn drain_curl_completions(&self, state: &mut OwnerState) {
        loop {
            match self.curl_completion_rx.try_recv() {
                Ok(completion) => self.finish_active_transfer(state, completion),
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    state.active_transfers = 0;
                    break;
                }
            }
        }
    }

    fn wait_for_curl_completion(&self, state: &mut OwnerState) {
        match self.curl_completion_rx.recv() {
            Ok(completion) => self.finish_active_transfer(state, completion),
            Err(_) => state.active_transfers = 0,
        }
    }

    fn handle_command_result(
        &self,
        state: &mut OwnerState,
        command: std::result::Result<RuntimeCommand, crossbeam_channel::RecvError>,
    ) {
        match command {
            Ok(command) => self.handle_command(state, command),
            Err(_) => self.close(state),
        }
    }

    fn handle_completion_result(
        &self,
        state: &mut OwnerState,
        completion: std::result::Result<RuntimeCurlCompletion, crossbeam_channel::RecvError>,
    ) {
        match completion {
            Ok(completion) => self.finish_active_transfer(state, completion),
            Err(_) => state.active_transfers = 0,
        }
    }

    fn handle_command(&self, state: &mut OwnerState, command: RuntimeCommand) {
        match command {
            RuntimeCommand::Request(job) if state.closed => {
                send_response(
                    job.response_tx,
                    Err(anyhow!("fetch runtime is shutting down")),
                );
            }
            RuntimeCommand::Request(job) => self.start_job_or_reply(state, job),
            RuntimeCommand::StreamingHtmlRequest(job) if state.closed => {
                fail_streaming_job(job, anyhow!("fetch runtime is shutting down"));
            }
            RuntimeCommand::StreamingHtmlRequest(job) => {
                self.start_streaming_job_or_reply(state, job)
            }
            RuntimeCommand::StreamingRawRequest(job) if state.closed => {
                fail_raw_streaming_job(job, anyhow!("fetch runtime is shutting down"));
            }
            RuntimeCommand::StreamingRawRequest(job) => {
                self.start_raw_streaming_job_or_reply(state, job)
            }
            #[cfg(test)]
            RuntimeCommand::PanicForTesting(admitted) => {
                let _ = admitted.send(());
                panic!("deterministic fetch runtime panic");
            }
            RuntimeCommand::Shutdown => self.close(state),
        }
    }

    fn close(&self, state: &mut OwnerState) {
        if state.closed {
            return;
        }
        state.closed = true;
        self.shutdown_requested.store(true, Ordering::SeqCst);
        self.curl_runtime.shutdown();
    }

    fn start_job_or_reply(&self, state: &mut OwnerState, job: RuntimeJob) {
        #[cfg(test)]
        if request_panics_for_testing(&job.request) {
            let error = anyhow!(
                "fetch runtime owner panicked while handling {} {}: runtime owner panic requested by test",
                job.request.method,
                job.current_url
            );
            send_response(job.response_tx, Err(error));
            return;
        }

        match self.start_job_attempt(job) {
            Ok(JobOutcome::Submitted) => state.active_transfers += 1,
            Ok(JobOutcome::Complete(response_tx, response)) => {
                send_response(response_tx, Ok(*response))
            }
            Ok(JobOutcome::Retry(job)) => self.start_job_or_reply(state, *job),
            Err((response_tx, error)) => send_response(response_tx, Err(error)),
        }
    }

    fn start_job_attempt(
        &self,
        mut job: RuntimeJob,
    ) -> std::result::Result<JobOutcome, (RuntimeResponseTx, anyhow::Error)> {
        if job.cancel_handle.is_cancelled() {
            return Err((job.response_tx, anyhow!("fetch runtime request cancelled")));
        }
        let request_cookie_report = if job.request.allows_credentials_for_url(&job.current_url) {
            match cookie_access_report_for_request(
                &self.cookie_store,
                &job.current_url,
                job.current_cookie_context.clone(),
            ) {
                Ok(report) => report,
                Err(error) => return Err((job.response_tx, error)),
            }
        } else {
            None
        };
        let cookie_header = cookie_header_from_report(request_cookie_report.as_ref());
        let prepared_request = prepare_client_hint_request(
            &self.client_hint_preferences,
            &job.client_hint_navigation_restarts,
            &self.config,
            &job.request,
            &job.current_url,
        );
        let mut easy = Easy2::new(FetchTransferHandler::new_buffered(ResponseCollector::new(
            Some(job.cancel_handle.clone()),
        )));
        easy.get_mut()
            .buffered_mut()
            .expect("buffered request should use buffered collector")
            .begin_request(self.config.http_max_response_size());
        if let Err(error) = configure_network_observation(
            &mut easy,
            &job.request,
            request_cookie_report.as_ref(),
            self.config.http_proxy().is_some() && job.current_url.scheme() == "https",
        ) {
            return Err((job.response_tx, error));
        }
        let outgoing_headers = match configure_easy(
            &mut easy,
            &self.config,
            &prepared_request.request,
            &job.current_url,
            &job.redirect_chain,
            cookie_header.as_deref(),
            job.http_version,
            // Buffered transfers are now the auth/compatibility fallback and
            // do not participate in disk-cache validation. Cache IO stays on
            // the streaming reader/writer paths.
            None,
        )
        .with_context(|| anyhow!("failed to configure curl request for {}", job.current_url))
        {
            Ok(headers) => headers,
            Err(error) => return Err((job.response_tx, error)),
        };
        let request_extra_info = job.request.is_top_level_navigation_request().then(|| {
            network_request_extra_info_from_headers(
                &self.config,
                &outgoing_headers,
                request_cookie_report.as_ref(),
            )
        });
        attach_next_request_extra_info(
            &mut job.redirect_chain,
            request_cookie_report.clone(),
            request_extra_info.as_ref(),
        );

        let label = job.current_url.to_string();
        let dns_resolution = curl_dns_resolution(&self.config, &job.current_url);
        let context = ActiveBufferedTransferContext {
            job,
            request_cookie_report,
            request_extra_info,
            response_policy: prepared_request.response_policy,
        };
        let curl_job = CurlMultiJob {
            easy,
            origin: context.job.origin_key.clone(),
            dns_resolution,
            priority: request_fetch_priority_rank(&context.job.request),
            label,
            context: ActiveTransferContext::Buffered(Box::new(context)),
        };
        match self.curl_runtime.submit(curl_job) {
            Ok(()) => Ok(JobOutcome::Submitted),
            Err(error) => Err((
                error
                    .job
                    .context
                    .into_buffered()
                    .expect("buffered submit should return buffered context")
                    .job
                    .response_tx,
                anyhow!("failed to submit curl runtime job: {}", error.error),
            )),
        }
    }

    fn start_streaming_job_or_reply(&self, state: &mut OwnerState, job: StreamingRuntimeJob) {
        #[cfg(test)]
        if request_panics_for_testing(&job.request) {
            let error = anyhow!(
                "fetch runtime owner panicked while handling {} {}: runtime owner panic requested by test",
                job.request.method,
                job.current_url
            );
            fail_streaming_job(job, error);
            return;
        }

        match self.start_streaming_job_attempt(state, job) {
            Ok(StreamingJobOutcome::Submitted) => state.active_transfers += 1,
            Ok(StreamingJobOutcome::Complete) => {}
            Err((job, easy, error)) => fail_streaming_job_with_easy(*job, easy, error),
        }
    }

    fn start_streaming_job_attempt(
        &self,
        state: &mut OwnerState,
        mut job: StreamingRuntimeJob,
    ) -> std::result::Result<
        StreamingJobOutcome,
        (
            Box<StreamingRuntimeJob>,
            Option<Easy2<FetchTransferHandler>>,
            anyhow::Error,
        ),
    > {
        let credentials_allowed = job.request.allows_credentials_for_url(&job.current_url);
        let request_cookie_report = if credentials_allowed {
            match cookie_access_report_for_request(
                &self.cookie_store,
                &job.current_url,
                job.current_cookie_context.clone(),
            ) {
                Ok(report) => report,
                Err(error) => return Err((Box::new(job), None, error)),
            }
        } else {
            None
        };
        let cookie_header = cookie_header_from_report(request_cookie_report.as_ref());
        let prepared_request = prepare_client_hint_request(
            &self.client_hint_preferences,
            &job.client_hint_navigation_restarts,
            &self.config,
            &job.request,
            &job.current_url,
        );
        match load_cached_streaming_response_lookup(
            &self.config,
            &prepared_request.request,
            &job.current_url,
            cookie_header.as_deref(),
        ) {
            Ok(Some(cached_lookup)) if !cached_streaming_response_is_stale(&cached_lookup) => {
                if prepared_request
                    .response_policy
                    .observe_response(&job.current_url, &cached_lookup.headers)
                    == ClientHintResponseAction::RestartNavigation
                {
                    self.start_streaming_job_or_reply(state, job);
                    return Ok(StreamingJobOutcome::Complete);
                }
                self.complete_cached_streaming_html_redirect_or_response(
                    state,
                    job,
                    cached_lookup,
                    request_cookie_report,
                )
                .map_err(|(job, error)| (job, None, error))?;
                return Ok(StreamingJobOutcome::Complete);
            }
            Ok(Some(_)) | Ok(None) => {}
            Err(error) => return Err((Box::new(job), None, error)),
        }

        let mut easy = job.easy.take().unwrap_or_else(|| {
            Easy2::new(FetchTransferHandler::new_streaming(
                StreamingResponseCollector::new(
                    Arc::clone(&self.cookie_store),
                    job.started_tx
                        .take()
                        .expect("initial streaming job should have start sender"),
                    job.body_tx
                        .take()
                        .expect("initial streaming job should have body sender"),
                    job.cancel_handle.clone(),
                ),
            ))
        });
        easy.reset();
        let cache_plan = Some(StreamingCachePlan::new(
            self.config.clone(),
            prepared_request.request.clone(),
            job.current_url.clone(),
            cookie_header.clone(),
        ));

        if let Err(error) = configure_network_observation(
            &mut easy,
            &job.request,
            request_cookie_report.as_ref(),
            self.config.http_proxy().is_some() && job.current_url.scheme() == "https",
        ) {
            return Err((Box::new(job), Some(easy), error));
        }
        let outgoing_headers = match configure_easy(
            &mut easy,
            &self.config,
            &prepared_request.request,
            &job.current_url,
            &job.redirect_chain,
            cookie_header.as_deref(),
            job.http_version,
            None,
        )
        .with_context(|| anyhow!("failed to configure curl request for {}", job.current_url))
        {
            Ok(headers) => headers,
            Err(error) => return Err((Box::new(job), Some(easy), error)),
        };
        let request_extra_info = job.request.is_top_level_navigation_request().then(|| {
            network_request_extra_info_from_headers(
                &self.config,
                &outgoing_headers,
                request_cookie_report.as_ref(),
            )
        });
        attach_next_request_extra_info(
            &mut job.redirect_chain,
            request_cookie_report.clone(),
            request_extra_info.as_ref(),
        );
        let collector = easy
            .get_mut()
            .streaming_mut()
            .expect("streaming request should use streaming collector");
        collector.begin_request_with_cache_plan(
            self.config.http_max_response_size(),
            job.current_url.clone(),
            job.current_cookie_context.clone(),
            request_cookie_report.clone(),
            credentials_allowed,
            job.redirect_chain.clone(),
            request_extra_info.clone(),
            cache_plan,
        );
        collector.set_client_hint_response_policy(prepared_request.response_policy);

        let label = job.current_url.to_string();
        let dns_resolution = curl_dns_resolution(&self.config, &job.current_url);
        let context = ActiveStreamingTransferContext {
            job,
            request_cookie_report,
            request_extra_info,
            request_cookie_header: cookie_header,
            effective_request: prepared_request.request,
        };
        let curl_job = CurlMultiJob {
            easy,
            origin: context.job.origin_key.clone(),
            dns_resolution,
            priority: request_fetch_priority_rank(&context.job.request),
            label,
            context: ActiveTransferContext::Streaming(Box::new(context)),
        };
        match self.curl_runtime.submit(curl_job) {
            Ok(()) => Ok(StreamingJobOutcome::Submitted),
            Err(error) => {
                let context = error
                    .job
                    .context
                    .into_streaming()
                    .expect("streaming submit should return streaming context");
                Err((
                    Box::new(context.job),
                    Some(error.job.easy),
                    anyhow!("failed to submit curl runtime job: {}", error.error),
                ))
            }
        }
    }

    fn start_raw_streaming_job_or_reply(
        &self,
        state: &mut OwnerState,
        job: StreamingRawRuntimeJob,
    ) {
        #[cfg(test)]
        if request_panics_for_testing(&job.request) {
            let error = anyhow!(
                "fetch runtime owner panicked while handling {} {}: runtime owner panic requested by test",
                job.request.method,
                job.current_url
            );
            fail_raw_streaming_job(job, error);
            return;
        }

        match self.start_raw_streaming_job_attempt(state, job) {
            Ok(StreamingJobOutcome::Submitted) => state.active_transfers += 1,
            Ok(StreamingJobOutcome::Complete) => {}
            Err((job, easy, error)) => fail_raw_streaming_job_with_easy(*job, easy, error),
        }
    }

    fn start_raw_streaming_job_attempt(
        &self,
        state: &mut OwnerState,
        mut job: StreamingRawRuntimeJob,
    ) -> std::result::Result<
        StreamingJobOutcome,
        (
            Box<StreamingRawRuntimeJob>,
            Option<Easy2<FetchTransferHandler>>,
            anyhow::Error,
        ),
    > {
        let credentials_allowed = job.request.allows_credentials_for_url(&job.current_url);
        let request_cookie_report = if credentials_allowed {
            match cookie_access_report_for_request(
                &self.cookie_store,
                &job.current_url,
                job.current_cookie_context.clone(),
            ) {
                Ok(report) => report,
                Err(error) => return Err((Box::new(job), None, error)),
            }
        } else {
            None
        };
        let cookie_header = cookie_header_from_report(request_cookie_report.as_ref());
        let prepared_request = prepare_client_hint_request(
            &self.client_hint_preferences,
            &job.client_hint_navigation_restarts,
            &self.config,
            &job.request,
            &job.current_url,
        );
        let mut stale_cached_lookup = None;
        match load_cached_streaming_response_lookup(
            &self.config,
            &prepared_request.request,
            &job.current_url,
            cookie_header.as_deref(),
        ) {
            Ok(Some(cached_lookup)) if !cached_streaming_response_is_stale(&cached_lookup) => {
                if prepared_request
                    .response_policy
                    .observe_response(&job.current_url, &cached_lookup.headers)
                    == ClientHintResponseAction::RestartNavigation
                {
                    self.start_raw_streaming_job_or_reply(state, job);
                    return Ok(StreamingJobOutcome::Complete);
                }
                self.complete_cached_streaming_raw_redirect_or_response(
                    state,
                    job,
                    cached_lookup,
                    request_cookie_report,
                )
                .map_err(|(job, error)| (job, None, error))?;
                return Ok(StreamingJobOutcome::Complete);
            }
            Ok(Some(cached_lookup)) => {
                stale_cached_lookup = Some(cached_lookup);
            }
            Ok(None) => {}
            Err(error) => return Err((Box::new(job), None, error)),
        }

        let mut easy = job.easy.take().unwrap_or_else(|| {
            Easy2::new(FetchTransferHandler::new_raw_streaming(
                RawStreamingResponseCollector::new(
                    Arc::clone(&self.cookie_store),
                    job.started_tx
                        .take()
                        .expect("initial raw streaming job should have start sender"),
                    job.body_tx
                        .take()
                        .expect("initial raw streaming job should have body sender"),
                    job.cancel_handle.clone(),
                ),
            ))
        });
        easy.reset();
        let cache_plan = Some(StreamingCachePlan::new(
            self.config.clone(),
            prepared_request.request.clone(),
            job.current_url.clone(),
            cookie_header.clone(),
        ));
        if let Err(error) = configure_network_observation(
            &mut easy,
            &job.request,
            request_cookie_report.as_ref(),
            self.config.http_proxy().is_some() && job.current_url.scheme() == "https",
        ) {
            return Err((Box::new(job), Some(easy), error));
        }
        let outgoing_headers = match configure_easy(
            &mut easy,
            &self.config,
            &prepared_request.request,
            &job.current_url,
            &job.redirect_chain,
            cookie_header.as_deref(),
            job.http_version,
            stale_cached_lookup
                .as_ref()
                .map(validation_headers_for_cached_streaming_response_lookup),
        )
        .with_context(|| anyhow!("failed to configure curl request for {}", job.current_url))
        {
            Ok(headers) => headers,
            Err(error) => return Err((Box::new(job), Some(easy), error)),
        };
        let request_extra_info = job.request.is_top_level_navigation_request().then(|| {
            network_request_extra_info_from_headers(
                &self.config,
                &outgoing_headers,
                request_cookie_report.as_ref(),
            )
        });
        attach_next_request_extra_info(
            &mut job.redirect_chain,
            request_cookie_report.clone(),
            request_extra_info.as_ref(),
        );
        let collector = easy
            .get_mut()
            .raw_streaming_mut()
            .expect("raw streaming request should use raw streaming collector");
        collector.begin_request_with_cache_plan(
            self.config.http_max_response_size(),
            job.current_url.clone(),
            job.current_cookie_context.clone(),
            request_cookie_report.clone(),
            credentials_allowed,
            job.redirect_chain.clone(),
            request_extra_info.clone(),
            cache_plan,
            stale_cached_lookup.is_some(),
        );
        collector.set_client_hint_response_policy(prepared_request.response_policy);

        let label = job.current_url.to_string();
        let dns_resolution = curl_dns_resolution(&self.config, &job.current_url);
        let context = ActiveRawStreamingTransferContext {
            job,
            request_cookie_report,
            request_extra_info,
            request_cookie_header: cookie_header,
            stale_cached_lookup,
            effective_request: prepared_request.request,
        };
        let curl_job = CurlMultiJob {
            easy,
            origin: context.job.origin_key.clone(),
            dns_resolution,
            priority: request_fetch_priority_rank(&context.job.request),
            label,
            context: ActiveTransferContext::StreamingRaw(Box::new(context)),
        };
        match self.curl_runtime.submit(curl_job) {
            Ok(()) => Ok(StreamingJobOutcome::Submitted),
            Err(error) => {
                let context = error
                    .job
                    .context
                    .into_streaming_raw()
                    .expect("raw streaming submit should return raw streaming context");
                Err((
                    Box::new(context.job),
                    Some(error.job.easy),
                    anyhow!("failed to submit curl runtime job: {}", error.error),
                ))
            }
        }
    }

    fn finish_active_transfer(&self, state: &mut OwnerState, completion: RuntimeCurlCompletion) {
        state.active_transfers = state.active_transfers.saturating_sub(1);
        match completion.context {
            ActiveTransferContext::Buffered(context) => {
                match self.finish_buffered_transfer_inner(
                    completion.easy,
                    completion.result,
                    *context,
                ) {
                    Ok(JobOutcome::Submitted) => state.active_transfers += 1,
                    Ok(JobOutcome::Complete(response_tx, response)) => {
                        send_response(response_tx, Ok(*response))
                    }
                    Ok(JobOutcome::Retry(job)) => self.start_job_or_reply(state, *job),
                    Err((response_tx, error)) => send_response(response_tx, Err(error)),
                }
            }
            ActiveTransferContext::Streaming(context) => {
                self.finish_streaming_transfer(state, completion.easy, completion.result, *context);
            }
            ActiveTransferContext::StreamingRaw(context) => self.finish_raw_streaming_transfer(
                state,
                completion.easy,
                completion.result,
                *context,
            ),
        }
    }

    fn finish_buffered_transfer_inner(
        &self,
        mut easy: Option<Easy2<FetchTransferHandler>>,
        result: Result<()>,
        context: ActiveBufferedTransferContext,
    ) -> std::result::Result<JobOutcome, (RuntimeResponseTx, anyhow::Error)> {
        let ActiveBufferedTransferContext {
            mut job,
            request_cookie_report,
            request_extra_info,
            response_policy,
        } = context;
        if self.shutdown_requested.load(Ordering::SeqCst) {
            return Err((
                job.response_tx,
                anyhow!("fetch runtime request cancelled during shutdown"),
            ));
        }
        if let Err(error) = result {
            if job.cancel_handle.is_cancelled() {
                return Err((job.response_tx, anyhow!("fetch runtime request cancelled")));
            }
            if let Some(response) = easy.as_mut().and_then(take_failed_proxy_connect_response) {
                let response = proxy_connect_raw_response(
                    &job.current_url,
                    &job.redirect_chain,
                    request_cookie_report,
                    response,
                );
                return Ok(JobOutcome::Complete(
                    job.response_tx,
                    Box::new(CompletedBufferedResponse::Raw(response)),
                ));
            }
            let used_http2 = easy.as_ref().is_some_and(transfer_used_http2);
            if should_retry_http2_failure_over_http1(
                &job.request,
                job.http_version,
                used_http2,
                &error,
            ) {
                tracing::debug!(
                    url = %job.current_url,
                    error = %error,
                    "retrying safe request over HTTP/1.1 after HTTP/2 protocol error"
                );
                job.http_version = RequestHttpVersion::Http1Only;
                return Ok(JobOutcome::Retry(Box::new(job)));
            }
            if let Some(upgraded_url) = empty_http_navigation_https_upgrade_url(
                &job.request,
                &job.current_url,
                job.empty_http_https_upgrade_attempted,
                &error,
            ) {
                tracing::debug!(
                    from_url = %job.current_url,
                    to_url = %upgraded_url,
                    error = %error,
                    "retrying empty HTTP navigation response over HTTPS"
                );
                job.current_cookie_context = advance_cookie_request_context(
                    job.current_cookie_context,
                    &job.request.url,
                    &upgraded_url,
                );
                job.redirect_chain.push(https_upgrade_redirect_info(
                    job.current_url.clone(),
                    upgraded_url.clone(),
                    request_cookie_report,
                ));
                job.current_url = upgraded_url;
                job.origin_key = origin_key_for_url(&job.current_url);
                job.empty_http_https_upgrade_attempted = true;
                job.http_version = RequestHttpVersion::PreferHttp2;
                return Ok(JobOutcome::Retry(Box::new(job)));
            }
            return Err((job.response_tx, error));
        }
        let Some(mut easy) = easy else {
            return Err((
                job.response_tx,
                anyhow!(
                    "curl runtime completed {} without returning an easy handle",
                    job.current_url
                ),
            ));
        };

        let (mut response, transfer_metrics) =
            match collect_buffered_response(&mut easy, &job.current_url) {
                Ok(response) => response,
                Err(error) => return Err((job.response_tx, error)),
            };
        log_request_completion(
            &job.request.method,
            &job.current_url,
            &response.final_url,
            response.status,
            &transfer_metrics,
        );
        let cookie_set_reports = if job.request.allows_credentials_for_url(&response.final_url) {
            match store_response_cookies(
                &self.cookie_store,
                &response.final_url,
                &response.headers,
                &job.current_cookie_context,
            ) {
                Ok(cookie_set_reports) => cookie_set_reports,
                Err(error) => return Err((job.response_tx, error)),
            }
        } else {
            Vec::new()
        };
        if response_policy.observe_response(&response.final_url, &response.headers)
            == ClientHintResponseAction::RestartNavigation
        {
            tracing::debug!(
                url = %job.current_url,
                "restarting navigation before response commit for missing Critical-CH headers"
            );
            job.redirect_chain
                .push(critical_client_hint_restart_redirect_info(
                    response.final_url.clone(),
                    network_response_extra_info(
                        request_extra_info.expect(
                            "Critical-CH restart should only apply to top-level navigation",
                        ),
                        response.status,
                        response.headers.clone(),
                        cookie_set_reports,
                    ),
                ));
            return Ok(JobOutcome::Retry(Box::new(job)));
        }

        response.request_cookie_report = request_cookie_report;
        response.cookie_set_reports = cookie_set_reports;
        response = response.with_network_request_extra_info(request_extra_info.clone());

        let next_url = match next_followed_redirect_url_from_parts(
            &response.final_url,
            response.status,
            &response.headers,
            job.redirect_count,
            job.request.follow_redirects,
        ) {
            Ok(next_url) => next_url,
            Err(error) => return Err((job.response_tx, error)),
        };
        if let Some(next_url) = next_url
            && job.request.follow_redirects
        {
            let redirect_has_extra_info = request_extra_info.is_some() && !response.from_cache;
            job.redirect_chain.push(RedirectInfo {
                from_url: response.final_url.clone(),
                to_url: next_url.clone(),
                status: response.status,
                headers: response.headers.clone(),
                network_extra_info_available: redirect_has_extra_info,
                request_extra_info: None,
                response_extra_info: request_extra_info.map(|request_extra_info| {
                    network_response_extra_info(
                        request_extra_info,
                        response.status,
                        response.headers.clone(),
                        response.cookie_set_reports.clone(),
                    )
                }),
                redirect_has_extra_info,
                request_cookie_report: None,
                cookie_set_reports: response.cookie_set_reports.clone(),
                from_cache: response.from_cache,
                negotiated_http_version: response.negotiated_http_version,
            });
            job.current_cookie_context = advance_cookie_request_context(
                job.current_cookie_context,
                &job.request.url,
                &next_url,
            );
            job.request.apply_redirect_status(response.status);
            job.current_url = next_url;
            job.origin_key = origin_key_for_url(&job.current_url);
            job.redirect_count += 1;
            job.http_version = RequestHttpVersion::PreferHttp2;
            return Ok(JobOutcome::Retry(Box::new(job)));
        }

        response.redirected = !job.redirect_chain.is_empty();
        response.redirect_chain = job.redirect_chain;
        Ok(JobOutcome::Complete(
            job.response_tx,
            Box::new(CompletedBufferedResponse::Raw(response)),
        ))
    }

    fn finish_streaming_transfer(
        &self,
        state: &mut OwnerState,
        easy: Option<Easy2<FetchTransferHandler>>,
        result: Result<()>,
        context: ActiveStreamingTransferContext,
    ) {
        let ActiveStreamingTransferContext {
            mut job,
            request_cookie_report,
            request_extra_info,
            request_cookie_header,
            effective_request,
        } = context;

        let Some(mut easy) = easy else {
            fail_streaming_job(
                job,
                anyhow!(
                    "curl runtime completed streaming request without returning an easy handle"
                ),
            );
            return;
        };

        if easy.get_ref().streaming().is_none() {
            fail_streaming_job(
                job,
                anyhow!("curl runtime returned non-streaming easy for streaming request"),
            );
            return;
        }

        if self.shutdown_requested.load(Ordering::SeqCst) {
            fail_streaming_job_with_easy(
                job,
                Some(easy),
                anyhow!("fetch runtime streaming request cancelled during shutdown"),
            );
            return;
        }

        if !job.cancel_handle.is_cancelled()
            && easy
                .get_ref()
                .streaming()
                .is_some_and(StreamingResponseCollector::client_hint_restart_requested)
        {
            tracing::debug!(
                url = %job.current_url,
                "restarting streaming navigation before response commit for missing Critical-CH headers"
            );
            let (status, headers, cookie_set_reports) = {
                let collector = easy
                    .get_mut()
                    .streaming_mut()
                    .expect("streaming request should use streaming collector");
                (
                    collector.status(),
                    collector.headers().to_vec(),
                    collector.take_cookie_set_reports(),
                )
            };
            job.redirect_chain
                .push(critical_client_hint_restart_redirect_info(
                    job.current_url.clone(),
                    network_response_extra_info(
                        request_extra_info.expect(
                            "Critical-CH restart should only apply to top-level navigation",
                        ),
                        status,
                        headers,
                        cookie_set_reports,
                    ),
                ));
            job.http_version = RequestHttpVersion::PreferHttp2;
            job.easy = Some(easy);
            self.start_streaming_job_or_reply(state, job);
            return;
        }

        if let Some(limit) = easy
            .get_ref()
            .streaming()
            .and_then(StreamingResponseCollector::response_too_large_limit)
        {
            let current_url = job.current_url.clone();
            fail_streaming_job_with_easy(
                job,
                Some(easy),
                anyhow!(
                    "response exceeded configured limit of {limit} bytes for {}",
                    current_url
                ),
            );
            return;
        }

        if let Err(error) = result {
            let response_started = easy
                .get_ref()
                .streaming()
                .is_some_and(StreamingResponseCollector::started);
            if !response_started
                && !job.cancel_handle.is_cancelled()
                && let Some(response) = take_failed_proxy_connect_response(&mut easy)
            {
                complete_streaming_proxy_connect_response(
                    job,
                    easy,
                    request_cookie_report,
                    response,
                );
                return;
            }
            let used_http2 = transfer_used_http2(&easy);
            if !response_started
                && !job.cancel_handle.is_cancelled()
                && should_retry_http2_failure_over_http1(
                    &job.request,
                    job.http_version,
                    used_http2,
                    &error,
                )
            {
                tracing::debug!(
                    url = %job.current_url,
                    error = %error,
                    "retrying safe streaming request over HTTP/1.1 after HTTP/2 protocol error"
                );
                job.http_version = RequestHttpVersion::Http1Only;
                job.easy = Some(easy);
                self.start_streaming_job_or_reply(state, job);
                return;
            }
            if !response_started
                && !job.cancel_handle.is_cancelled()
                && let Some(upgraded_url) = empty_http_navigation_https_upgrade_url(
                    &job.request,
                    &job.current_url,
                    job.empty_http_https_upgrade_attempted,
                    &error,
                )
            {
                tracing::debug!(
                    from_url = %job.current_url,
                    to_url = %upgraded_url,
                    error = %error,
                    "retrying empty HTTP streaming navigation response over HTTPS"
                );
                job.current_cookie_context = advance_cookie_request_context(
                    job.current_cookie_context,
                    &job.request.url,
                    &upgraded_url,
                );
                job.redirect_chain.push(https_upgrade_redirect_info(
                    job.current_url.clone(),
                    upgraded_url.clone(),
                    request_cookie_report,
                ));
                job.current_url = upgraded_url;
                job.origin_key = origin_key_for_url(&job.current_url);
                job.empty_http_https_upgrade_attempted = true;
                job.http_version = RequestHttpVersion::PreferHttp2;
                job.easy = Some(easy);
                self.start_streaming_job_or_reply(state, job);
                return;
            }
            let header_terminated = easy
                .get_ref()
                .streaming()
                .is_some_and(StreamingResponseCollector::header_terminated);
            if !header_terminated {
                let error = easy
                    .get_mut()
                    .streaming_mut()
                    .and_then(StreamingResponseCollector::take_callback_error)
                    .map(anyhow::Error::msg)
                    .unwrap_or(error);
                fail_streaming_job_with_easy(job, Some(easy), error);
                return;
            }
        }

        let final_url = job.current_url.clone();
        let negotiated_http_version = negotiated_http_version_from_easy(&easy);
        let (status, headers, cookie_set_reports, collector_http_version) = {
            let streaming = easy
                .get_mut()
                .streaming_mut()
                .expect("streaming request should use streaming collector");
            (
                streaming.status(),
                streaming.headers().to_vec(),
                streaming.take_cookie_set_reports(),
                streaming.negotiated_http_version(),
            )
        };
        let negotiated_http_version = negotiated_http_version.or(collector_http_version);
        let transfer_metrics = transfer_metrics_from_easy(&easy, &headers);
        log_request_completion(
            &job.request.method,
            &job.current_url,
            &final_url,
            status,
            &transfer_metrics,
        );

        let next_url = match next_followed_redirect_url_from_parts(
            &final_url,
            status,
            &headers,
            job.redirect_count,
            job.request.follow_redirects,
        ) {
            Ok(next_url) => next_url,
            Err(error) => {
                fail_streaming_job_with_easy(job, Some(easy), error);
                return;
            }
        };
        if let Some(next_url) = next_url
            && job.request.follow_redirects
        {
            let cache_body_writer = easy
                .get_mut()
                .streaming_mut()
                .expect("streaming request should use streaming collector")
                .take_cache_body_writer();
            if let Some(cache_body_writer) = cache_body_writer
                && let Err(error) = finish_streaming_cached_response(
                    &self.config,
                    &effective_request,
                    &job.current_url,
                    request_cookie_header.as_deref(),
                    &final_url,
                    status,
                    &headers,
                    false,
                    cache_body_writer,
                )
            {
                tracing::debug!(url = %job.current_url, "failed to store streaming redirect response in disk cache: {error}");
            }
            let redirect_has_extra_info = request_extra_info.is_some();
            job.redirect_chain.push(RedirectInfo {
                from_url: final_url,
                to_url: next_url.clone(),
                status,
                headers: headers.clone(),
                network_extra_info_available: redirect_has_extra_info,
                request_extra_info: None,
                response_extra_info: request_extra_info.map(|request_extra_info| {
                    network_response_extra_info(
                        request_extra_info,
                        status,
                        headers,
                        cookie_set_reports.clone(),
                    )
                }),
                redirect_has_extra_info,
                request_cookie_report,
                cookie_set_reports,
                from_cache: false,
                negotiated_http_version,
            });
            job.current_cookie_context = advance_cookie_request_context(
                job.current_cookie_context,
                &job.request.url,
                &next_url,
            );
            job.request.apply_redirect_status(status);
            job.current_url = next_url;
            job.origin_key = origin_key_for_url(&job.current_url);
            job.redirect_count += 1;
            job.http_version = RequestHttpVersion::PreferHttp2;
            job.easy = Some(easy);
            self.start_streaming_job_or_reply(state, job);
            return;
        }

        let cache_body_writer = {
            let streaming = easy
                .get_mut()
                .streaming_mut()
                .expect("streaming request should use streaming collector");
            streaming.finish_streaming_body();
            streaming.take_cache_body_writer()
        };

        if let Some(cache_body_writer) = cache_body_writer
            && let Err(error) = finish_streaming_cached_response(
                &self.config,
                &effective_request,
                &job.current_url,
                request_cookie_header.as_deref(),
                &final_url,
                status,
                &headers,
                false,
                cache_body_writer,
            )
        {
            tracing::debug!(url = %job.current_url, "failed to store streaming response in disk cache: {error}");
        }
        let _ = job.completion_tx.send(Ok(()));
    }

    fn finish_raw_streaming_transfer(
        &self,
        state: &mut OwnerState,
        easy: Option<Easy2<FetchTransferHandler>>,
        result: Result<()>,
        context: ActiveRawStreamingTransferContext,
    ) {
        let ActiveRawStreamingTransferContext {
            mut job,
            request_cookie_report,
            request_extra_info,
            request_cookie_header,
            stale_cached_lookup,
            effective_request,
        } = context;

        let Some(mut easy) = easy else {
            fail_raw_streaming_job(
                job,
                anyhow!(
                    "curl runtime completed raw streaming request without returning an easy handle"
                ),
            );
            return;
        };

        if easy.get_ref().raw_streaming().is_none() {
            fail_raw_streaming_job(
                job,
                anyhow!("curl runtime returned non-raw-streaming easy for raw streaming request"),
            );
            return;
        }

        if self.shutdown_requested.load(Ordering::SeqCst) {
            fail_raw_streaming_job_with_easy(
                job,
                Some(easy),
                anyhow!("fetch runtime raw streaming request cancelled during shutdown"),
            );
            return;
        }

        if !job.cancel_handle.is_cancelled()
            && easy
                .get_ref()
                .raw_streaming()
                .is_some_and(RawStreamingResponseCollector::client_hint_restart_requested)
        {
            tracing::debug!(
                url = %job.current_url,
                "restarting raw navigation before response commit for missing Critical-CH headers"
            );
            let (status, headers, cookie_set_reports) = {
                let collector = easy
                    .get_mut()
                    .raw_streaming_mut()
                    .expect("raw streaming request should use raw streaming collector");
                (
                    collector.status(),
                    collector.headers().to_vec(),
                    collector.take_cookie_set_reports(),
                )
            };
            job.redirect_chain
                .push(critical_client_hint_restart_redirect_info(
                    job.current_url.clone(),
                    network_response_extra_info(
                        request_extra_info.expect(
                            "Critical-CH restart should only apply to top-level navigation",
                        ),
                        status,
                        headers,
                        cookie_set_reports,
                    ),
                ));
            job.http_version = RequestHttpVersion::PreferHttp2;
            job.easy = Some(easy);
            self.start_raw_streaming_job_or_reply(state, job);
            return;
        }

        if let Some(limit) = easy
            .get_ref()
            .raw_streaming()
            .and_then(RawStreamingResponseCollector::response_too_large_limit)
        {
            let current_url = job.current_url.clone();
            fail_raw_streaming_job_with_easy(
                job,
                Some(easy),
                anyhow!(
                    "response exceeded configured limit of {limit} bytes for {}",
                    current_url
                ),
            );
            return;
        }

        if let Err(error) = result {
            let response_started = easy
                .get_ref()
                .raw_streaming()
                .is_some_and(RawStreamingResponseCollector::started);
            if !response_started
                && !job.cancel_handle.is_cancelled()
                && let Some(response) = take_failed_proxy_connect_response(&mut easy)
            {
                complete_raw_streaming_proxy_connect_response(
                    job,
                    easy,
                    request_cookie_report,
                    response,
                );
                return;
            }
            let used_http2 = transfer_used_http2(&easy);
            if !response_started
                && !job.cancel_handle.is_cancelled()
                && should_retry_http2_failure_over_http1(
                    &job.request,
                    job.http_version,
                    used_http2,
                    &error,
                )
            {
                tracing::debug!(
                    url = %job.current_url,
                    error = %error,
                    "retrying safe raw streaming request over HTTP/1.1 after HTTP/2 protocol error"
                );
                job.http_version = RequestHttpVersion::Http1Only;
                job.easy = Some(easy);
                self.start_raw_streaming_job_or_reply(state, job);
                return;
            }
            if !response_started
                && !job.cancel_handle.is_cancelled()
                && let Some(upgraded_url) = empty_http_navigation_https_upgrade_url(
                    &job.request,
                    &job.current_url,
                    job.empty_http_https_upgrade_attempted,
                    &error,
                )
            {
                tracing::debug!(
                    from_url = %job.current_url,
                    to_url = %upgraded_url,
                    error = %error,
                    "retrying empty HTTP raw streaming navigation response over HTTPS"
                );
                job.current_cookie_context = advance_cookie_request_context(
                    job.current_cookie_context,
                    &job.request.url,
                    &upgraded_url,
                );
                job.redirect_chain.push(https_upgrade_redirect_info(
                    job.current_url.clone(),
                    upgraded_url.clone(),
                    request_cookie_report,
                ));
                job.current_url = upgraded_url;
                job.origin_key = origin_key_for_url(&job.current_url);
                job.empty_http_https_upgrade_attempted = true;
                job.http_version = RequestHttpVersion::PreferHttp2;
                job.easy = Some(easy);
                self.start_raw_streaming_job_or_reply(state, job);
                return;
            }
            let header_terminated = easy
                .get_ref()
                .raw_streaming()
                .is_some_and(RawStreamingResponseCollector::header_terminated);
            if !header_terminated {
                let error = easy
                    .get_mut()
                    .raw_streaming_mut()
                    .and_then(RawStreamingResponseCollector::take_callback_error)
                    .map(anyhow::Error::msg)
                    .unwrap_or(error);
                fail_raw_streaming_job_with_easy(job, Some(easy), error);
                return;
            }
        }

        let final_url = job.current_url.clone();
        let negotiated_http_version = negotiated_http_version_from_easy(&easy);
        let (status, headers, cookie_set_reports, collector_http_version) = {
            let streaming = easy
                .get_mut()
                .raw_streaming_mut()
                .expect("raw streaming request should use raw streaming collector");
            (
                streaming.status(),
                streaming.headers().to_vec(),
                streaming.take_cookie_set_reports(),
                streaming.negotiated_http_version(),
            )
        };
        let negotiated_http_version = negotiated_http_version.or(collector_http_version);
        let transfer_metrics = transfer_metrics_from_easy(&easy, &headers);
        log_request_completion(
            &job.request.method,
            &job.current_url,
            &final_url,
            status,
            &transfer_metrics,
        );

        if status == 304
            && let Some(cached_lookup) = stale_cached_lookup
        {
            if cached_streaming_response_body_exceeds_response_limit(&self.config, &cached_lookup) {
                if let Err(error) = remove_cached_response(
                    &self.config,
                    &effective_request,
                    &job.current_url,
                    request_cookie_header.as_deref(),
                ) {
                    tracing::debug!(url = %job.current_url, "failed to remove oversized revalidated disk cache entry: {error}");
                }
                job.easy = Some(easy);
                self.start_raw_streaming_job_or_reply(state, job);
                return;
            }
            // A 304 can update cache-control metadata without a response body.
            // Keep serving the old cached body for this request, but remove the
            // entry afterward if the revalidation response forbids storage.
            let should_remove_cache_entry = response_headers_forbid_cache_storage(&headers);
            let cached_lookup = match merge_cached_not_modified_streaming_response_lookup(
                &self.config,
                &effective_request,
                &job.current_url,
                request_cookie_header.as_deref(),
                cached_lookup,
                &headers,
            ) {
                Ok(cached_lookup) => cached_lookup,
                Err(error) => {
                    tracing::debug!(url = %job.current_url, "failed to merge streaming disk cache revalidation: {error}");
                    if let Err(error) = remove_cached_response(
                        &self.config,
                        &effective_request,
                        &job.current_url,
                        request_cookie_header.as_deref(),
                    ) {
                        tracing::debug!(url = %job.current_url, "failed to remove unreadable revalidated disk cache entry: {error}");
                    }
                    job.easy = Some(easy);
                    self.start_raw_streaming_job_or_reply(state, job);
                    return;
                }
            };
            if should_remove_cache_entry
                && let Err(error) = remove_cached_response(
                    &self.config,
                    &effective_request,
                    &job.current_url,
                    request_cookie_header.as_deref(),
                )
            {
                tracing::debug!(url = %job.current_url, "failed to remove no-store revalidated disk cache entry: {error}");
            }
            let (started_tx, body_tx) = easy
                .get_mut()
                .raw_streaming_mut()
                .expect("raw streaming request should use raw streaming collector")
                .take_response_channels();
            job.started_tx = started_tx;
            job.body_tx = body_tx;
            if let Err((job, error)) = self.complete_cached_streaming_raw_redirect_or_response(
                state,
                job,
                cached_lookup,
                request_cookie_report,
            ) {
                fail_raw_streaming_job(*job, error);
            }
            return;
        }

        let next_url = match next_followed_redirect_url_from_parts(
            &final_url,
            status,
            &headers,
            job.redirect_count,
            job.request.follow_redirects,
        ) {
            Ok(next_url) => next_url,
            Err(error) => {
                fail_raw_streaming_job_with_easy(job, Some(easy), error);
                return;
            }
        };
        if let Some(next_url) = next_url {
            if job.request.follow_redirects {
                let cache_body_writer = easy
                    .get_mut()
                    .raw_streaming_mut()
                    .expect("raw streaming request should use raw streaming collector")
                    .take_cache_body_writer();
                if let Some(cache_body_writer) = cache_body_writer
                    && let Err(error) = finish_streaming_cached_response(
                        &self.config,
                        &effective_request,
                        &job.current_url,
                        request_cookie_header.as_deref(),
                        &final_url,
                        status,
                        &headers,
                        false,
                        cache_body_writer,
                    )
                {
                    tracing::debug!(url = %job.current_url, "failed to store raw streaming redirect response in disk cache: {error}");
                }
                let redirect_has_extra_info = request_extra_info.is_some();
                job.redirect_chain.push(RedirectInfo {
                    from_url: final_url,
                    to_url: next_url.clone(),
                    status,
                    headers: headers.clone(),
                    network_extra_info_available: redirect_has_extra_info,
                    request_extra_info: None,
                    response_extra_info: request_extra_info.map(|request_extra_info| {
                        network_response_extra_info(
                            request_extra_info,
                            status,
                            headers,
                            cookie_set_reports.clone(),
                        )
                    }),
                    redirect_has_extra_info,
                    request_cookie_report,
                    cookie_set_reports,
                    from_cache: false,
                    negotiated_http_version,
                });
                job.current_cookie_context = advance_cookie_request_context(
                    job.current_cookie_context,
                    &job.request.url,
                    &next_url,
                );
                job.request.apply_redirect_status(status);
                job.current_url = next_url;
                job.origin_key = origin_key_for_url(&job.current_url);
                job.redirect_count += 1;
                job.http_version = RequestHttpVersion::PreferHttp2;
                job.easy = Some(easy);
                self.start_raw_streaming_job_or_reply(state, job);
                return;
            }

            job.cancel_handle.mark_response_terminal();
            let (started_tx, cookie_set_reports, cache_body_writer) = {
                let streaming = easy
                    .get_mut()
                    .raw_streaming_mut()
                    .expect("raw streaming request should use raw streaming collector");
                streaming.finish_streaming_body();
                let (started_tx, _) = streaming.take_response_channels();
                (
                    started_tx,
                    streaming.take_cookie_set_reports(),
                    streaming.take_cache_body_writer(),
                )
            };
            if let Some(cache_body_writer) = cache_body_writer
                && let Err(error) = finish_streaming_cached_response(
                    &self.config,
                    &effective_request,
                    &job.current_url,
                    request_cookie_header.as_deref(),
                    &final_url,
                    status,
                    &headers,
                    false,
                    cache_body_writer,
                )
            {
                tracing::debug!(url = %job.current_url, "failed to store raw streaming manual redirect response in disk cache: {error}");
            }
            if let Some(started_tx) = started_tx {
                let _ = started_tx.send(Ok(StreamingHtmlResponseStart {
                    final_url,
                    status,
                    headers,
                    request_cookie_report,
                    cookie_set_reports,
                    redirected: !job.redirect_chain.is_empty(),
                    redirect_chain: job.redirect_chain,
                    from_cache: false,
                    negotiated_http_version,
                    network_request_extra_info: request_extra_info,
                }));
            }
            let _ = job.completion_tx.send(Ok(()));
            return;
        }

        job.cancel_handle.mark_response_terminal();
        let cache_body_writer = {
            let streaming = easy
                .get_mut()
                .raw_streaming_mut()
                .expect("raw streaming request should use raw streaming collector");
            streaming.finish_streaming_body();
            streaming.take_cache_body_writer()
        };

        if let Some(cache_body_writer) = cache_body_writer
            && let Err(error) = finish_streaming_cached_response(
                &self.config,
                &effective_request,
                &job.current_url,
                request_cookie_header.as_deref(),
                &final_url,
                status,
                &headers,
                false,
                cache_body_writer,
            )
        {
            tracing::debug!(url = %job.current_url, "failed to store raw streaming response in disk cache: {error}");
        }
        let _ = job.completion_tx.send(Ok(()));
    }

    fn complete_cached_streaming_html_redirect_or_response(
        &self,
        state: &mut OwnerState,
        mut job: StreamingRuntimeJob,
        cached: CachedStreamingResponseLookup,
        request_cookie_report: Option<StoredCookieQueryReport>,
    ) -> std::result::Result<(), (Box<StreamingRuntimeJob>, anyhow::Error)> {
        let final_url = match Url::parse(&cached.final_url) {
            Ok(final_url) => final_url,
            Err(error) => {
                return Err((
                    Box::new(job),
                    anyhow!("failed to parse cached response final url: {error}"),
                ));
            }
        };
        let next_url = match next_followed_redirect_url_from_parts(
            &final_url,
            cached.status,
            &cached.headers,
            job.redirect_count,
            job.request.follow_redirects,
        ) {
            Ok(next_url) => next_url,
            Err(error) => return Err((Box::new(job), error)),
        };
        if let Some(next_url) = next_url
            && job.request.follow_redirects
        {
            job.redirect_chain.push(RedirectInfo {
                from_url: final_url,
                to_url: next_url.clone(),
                status: cached.status,
                headers: cached.headers,
                network_extra_info_available: false,
                request_extra_info: None,
                response_extra_info: None,
                redirect_has_extra_info: false,
                request_cookie_report,
                cookie_set_reports: Vec::new(),
                from_cache: true,
                negotiated_http_version: None,
            });
            job.current_cookie_context = advance_cookie_request_context(
                job.current_cookie_context,
                &job.request.url,
                &next_url,
            );
            job.request.apply_redirect_status(cached.status);
            job.current_url = next_url;
            job.origin_key = origin_key_for_url(&job.current_url);
            job.redirect_count += 1;
            job.http_version = RequestHttpVersion::PreferHttp2;
            self.start_streaming_job_or_reply(state, job);
            return Ok(());
        }

        complete_cached_streaming_html_job(job, cached, request_cookie_report);
        Ok(())
    }

    fn complete_cached_streaming_raw_redirect_or_response(
        &self,
        state: &mut OwnerState,
        mut job: StreamingRawRuntimeJob,
        cached: CachedStreamingResponseLookup,
        request_cookie_report: Option<StoredCookieQueryReport>,
    ) -> std::result::Result<(), (Box<StreamingRawRuntimeJob>, anyhow::Error)> {
        let final_url = match Url::parse(&cached.final_url) {
            Ok(final_url) => final_url,
            Err(error) => {
                return Err((
                    Box::new(job),
                    anyhow!("failed to parse cached raw response final url: {error}"),
                ));
            }
        };
        let next_url = match next_followed_redirect_url_from_parts(
            &final_url,
            cached.status,
            &cached.headers,
            job.redirect_count,
            job.request.follow_redirects,
        ) {
            Ok(next_url) => next_url,
            Err(error) => return Err((Box::new(job), error)),
        };
        if let Some(next_url) = next_url
            && job.request.follow_redirects
        {
            job.redirect_chain.push(RedirectInfo {
                from_url: final_url,
                to_url: next_url.clone(),
                status: cached.status,
                headers: cached.headers,
                network_extra_info_available: false,
                request_extra_info: None,
                response_extra_info: None,
                redirect_has_extra_info: false,
                request_cookie_report,
                cookie_set_reports: Vec::new(),
                from_cache: true,
                negotiated_http_version: None,
            });
            job.current_cookie_context = advance_cookie_request_context(
                job.current_cookie_context,
                &job.request.url,
                &next_url,
            );
            job.request.apply_redirect_status(cached.status);
            job.current_url = next_url;
            job.origin_key = origin_key_for_url(&job.current_url);
            job.redirect_count += 1;
            job.http_version = RequestHttpVersion::PreferHttp2;
            self.start_raw_streaming_job_or_reply(state, job);
            return Ok(());
        }

        complete_cached_streaming_raw_job(job, cached, request_cookie_report);
        Ok(())
    }
}

#[derive(Default)]
struct OwnerState {
    closed: bool,
    active_transfers: usize,
}

#[derive(Debug)]
struct RuntimeJob {
    request: Request,
    current_url: Url,
    current_cookie_context: NetworkCookieRequestContext,
    redirect_chain: Vec<RedirectInfo>,
    redirect_count: usize,
    origin_key: Option<CurlOriginKey>,
    response_tx: RuntimeResponseTx,
    cancel_handle: FetchCancelHandle,
    http_version: RequestHttpVersion,
    empty_http_https_upgrade_attempted: bool,
    client_hint_navigation_restarts: SharedNavigationClientHintRestarts,
}

impl RuntimeJob {
    fn new(
        request: Request,
        response_tx: RuntimeResponseTx,
        cancel_handle: FetchCancelHandle,
    ) -> Self {
        let origin_key = origin_key(&request);
        Self {
            current_url: request.url.clone(),
            current_cookie_context: request.cookie_context.clone(),
            request,
            redirect_chain: Vec::new(),
            redirect_count: 0,
            origin_key,
            response_tx,
            cancel_handle,
            http_version: RequestHttpVersion::PreferHttp2,
            empty_http_https_upgrade_attempted: false,
            client_hint_navigation_restarts: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }
}

struct StreamingRuntimeJob {
    request: Request,
    current_url: Url,
    current_cookie_context: NetworkCookieRequestContext,
    redirect_chain: Vec<RedirectInfo>,
    redirect_count: usize,
    origin_key: Option<CurlOriginKey>,
    started_tx: Option<oneshot::Sender<Result<StreamingHtmlResponseStart>>>,
    body_tx: Option<mpsc::UnboundedSender<String>>,
    completion_tx: RuntimeStreamingCompletionTx,
    cancel_handle: FetchCancelHandle,
    easy: Option<Easy2<FetchTransferHandler>>,
    http_version: RequestHttpVersion,
    empty_http_https_upgrade_attempted: bool,
    client_hint_navigation_restarts: SharedNavigationClientHintRestarts,
}

impl StreamingRuntimeJob {
    fn new(
        request: Request,
        started_tx: oneshot::Sender<Result<StreamingHtmlResponseStart>>,
        body_tx: mpsc::UnboundedSender<String>,
        completion_tx: RuntimeStreamingCompletionTx,
        cancel_handle: FetchCancelHandle,
    ) -> Self {
        let origin_key = origin_key(&request);
        Self {
            current_url: request.url.clone(),
            current_cookie_context: request.cookie_context.clone(),
            request,
            redirect_chain: Vec::new(),
            redirect_count: 0,
            origin_key,
            started_tx: Some(started_tx),
            body_tx: Some(body_tx),
            completion_tx,
            cancel_handle,
            easy: None,
            http_version: RequestHttpVersion::PreferHttp2,
            empty_http_https_upgrade_attempted: false,
            client_hint_navigation_restarts: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }
}

struct StreamingRawRuntimeJob {
    request: Request,
    current_url: Url,
    current_cookie_context: NetworkCookieRequestContext,
    redirect_chain: Vec<RedirectInfo>,
    redirect_count: usize,
    origin_key: Option<CurlOriginKey>,
    started_tx: Option<oneshot::Sender<Result<StreamingHtmlResponseStart>>>,
    body_tx: Option<mpsc::UnboundedSender<Vec<u8>>>,
    completion_tx: RuntimeStreamingCompletionTx,
    cancel_handle: FetchCancelHandle,
    easy: Option<Easy2<FetchTransferHandler>>,
    http_version: RequestHttpVersion,
    empty_http_https_upgrade_attempted: bool,
    client_hint_navigation_restarts: SharedNavigationClientHintRestarts,
}

impl StreamingRawRuntimeJob {
    fn new(
        request: Request,
        started_tx: oneshot::Sender<Result<StreamingHtmlResponseStart>>,
        body_tx: mpsc::UnboundedSender<Vec<u8>>,
        completion_tx: RuntimeStreamingCompletionTx,
        cancel_handle: FetchCancelHandle,
    ) -> Self {
        let origin_key = origin_key(&request);
        Self {
            current_url: request.url.clone(),
            current_cookie_context: request.cookie_context.clone(),
            request,
            redirect_chain: Vec::new(),
            redirect_count: 0,
            origin_key,
            started_tx: Some(started_tx),
            body_tx: Some(body_tx),
            completion_tx,
            cancel_handle,
            easy: None,
            http_version: RequestHttpVersion::PreferHttp2,
            empty_http_https_upgrade_attempted: false,
            client_hint_navigation_restarts: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }
}

enum ActiveTransferContext {
    Buffered(Box<ActiveBufferedTransferContext>),
    Streaming(Box<ActiveStreamingTransferContext>),
    StreamingRaw(Box<ActiveRawStreamingTransferContext>),
}

impl ActiveTransferContext {
    fn into_buffered(self) -> Option<ActiveBufferedTransferContext> {
        match self {
            Self::Buffered(context) => Some(*context),
            Self::Streaming(_) | Self::StreamingRaw(_) => None,
        }
    }

    fn into_streaming(self) -> Option<ActiveStreamingTransferContext> {
        match self {
            Self::Streaming(context) => Some(*context),
            Self::Buffered(_) => None,
            Self::StreamingRaw(_) => None,
        }
    }

    fn into_streaming_raw(self) -> Option<ActiveRawStreamingTransferContext> {
        match self {
            Self::StreamingRaw(context) => Some(*context),
            Self::Buffered(_) | Self::Streaming(_) => None,
        }
    }
}

struct ActiveBufferedTransferContext {
    job: RuntimeJob,
    request_cookie_report: Option<StoredCookieQueryReport>,
    request_extra_info: Option<NetworkRequestExtraInfo>,
    response_policy: ClientHintResponsePolicy,
}

struct ActiveStreamingTransferContext {
    job: StreamingRuntimeJob,
    request_cookie_report: Option<StoredCookieQueryReport>,
    request_extra_info: Option<NetworkRequestExtraInfo>,
    request_cookie_header: Option<String>,
    effective_request: Request,
}

struct ActiveRawStreamingTransferContext {
    job: StreamingRawRuntimeJob,
    request_cookie_report: Option<StoredCookieQueryReport>,
    request_extra_info: Option<NetworkRequestExtraInfo>,
    request_cookie_header: Option<String>,
    stale_cached_lookup: Option<CachedStreamingResponseLookup>,
    effective_request: Request,
}

struct FetchTransferHandler {
    response: FetchResponseCollector,
    network_observation_recorder: Option<NetworkObservationRecorder>,
    proxy_connect_response_recorder: ProxyConnectResponseRecorder,
}

enum FetchResponseCollector {
    Buffered(ResponseCollector),
    Streaming(StreamingResponseCollector),
    StreamingRaw(RawStreamingResponseCollector),
}

impl FetchTransferHandler {
    fn new_buffered(collector: ResponseCollector) -> Self {
        Self::new(FetchResponseCollector::Buffered(collector))
    }

    fn new_streaming(collector: StreamingResponseCollector) -> Self {
        Self::new(FetchResponseCollector::Streaming(collector))
    }

    fn new_raw_streaming(collector: RawStreamingResponseCollector) -> Self {
        Self::new(FetchResponseCollector::StreamingRaw(collector))
    }

    fn new(response: FetchResponseCollector) -> Self {
        Self {
            response,
            network_observation_recorder: None,
            proxy_connect_response_recorder: ProxyConnectResponseRecorder::default(),
        }
    }

    fn buffered(&self) -> Option<&ResponseCollector> {
        match &self.response {
            FetchResponseCollector::Buffered(collector) => Some(collector),
            FetchResponseCollector::Streaming(_) | FetchResponseCollector::StreamingRaw(_) => None,
        }
    }

    fn buffered_mut(&mut self) -> Option<&mut ResponseCollector> {
        match &mut self.response {
            FetchResponseCollector::Buffered(collector) => Some(collector),
            FetchResponseCollector::Streaming(_) | FetchResponseCollector::StreamingRaw(_) => None,
        }
    }

    fn streaming_mut(&mut self) -> Option<&mut StreamingResponseCollector> {
        match &mut self.response {
            FetchResponseCollector::Streaming(collector) => Some(collector),
            FetchResponseCollector::Buffered(_) | FetchResponseCollector::StreamingRaw(_) => None,
        }
    }

    fn streaming(&self) -> Option<&StreamingResponseCollector> {
        match &self.response {
            FetchResponseCollector::Streaming(collector) => Some(collector),
            FetchResponseCollector::Buffered(_) | FetchResponseCollector::StreamingRaw(_) => None,
        }
    }

    fn raw_streaming_mut(&mut self) -> Option<&mut RawStreamingResponseCollector> {
        match &mut self.response {
            FetchResponseCollector::StreamingRaw(collector) => Some(collector),
            FetchResponseCollector::Buffered(_) | FetchResponseCollector::Streaming(_) => None,
        }
    }

    fn raw_streaming(&self) -> Option<&RawStreamingResponseCollector> {
        match &self.response {
            FetchResponseCollector::StreamingRaw(collector) => Some(collector),
            FetchResponseCollector::Buffered(_) | FetchResponseCollector::Streaming(_) => None,
        }
    }

    fn begin_transfer(
        &mut self,
        network_observation_recorder: Option<NetworkObservationRecorder>,
        capture_proxy_connect_response: bool,
    ) {
        self.network_observation_recorder = network_observation_recorder;
        self.proxy_connect_response_recorder
            .begin_transfer(capture_proxy_connect_response);
    }

    fn take_failed_proxy_connect_response(
        &mut self,
        connect_status: u32,
    ) -> Option<ProxyConnectResponse> {
        let response = self
            .proxy_connect_response_recorder
            .take_failed_response(connect_status);
        if response.is_some()
            && let Some(recorder) = self.network_observation_recorder.as_ref()
        {
            recorder.record_failed_proxy_connect_terminal();
        }
        response
    }
}

impl Handler for FetchTransferHandler {
    fn write(&mut self, data: &[u8]) -> std::result::Result<usize, WriteError> {
        match &mut self.response {
            FetchResponseCollector::Buffered(collector) => collector.write(data),
            FetchResponseCollector::Streaming(collector) => collector.write(data),
            FetchResponseCollector::StreamingRaw(collector) => collector.write(data),
        }
    }

    fn header(&mut self, data: &[u8]) -> bool {
        if let Some(recorder) = self.network_observation_recorder.as_ref() {
            recorder.record_response_header_line(data);
        }
        match &mut self.response {
            FetchResponseCollector::Buffered(collector) => collector.header(data),
            FetchResponseCollector::Streaming(collector) => collector.header(data),
            FetchResponseCollector::StreamingRaw(collector) => collector.header(data),
        }
    }

    fn progress(&mut self, dltotal: f64, dlnow: f64, ultotal: f64, ulnow: f64) -> bool {
        match &mut self.response {
            FetchResponseCollector::Buffered(collector) => {
                collector.progress(dltotal, dlnow, ultotal, ulnow)
            }
            FetchResponseCollector::Streaming(collector) => {
                collector.progress(dltotal, dlnow, ultotal, ulnow)
            }
            FetchResponseCollector::StreamingRaw(collector) => {
                collector.progress(dltotal, dlnow, ultotal, ulnow)
            }
        }
    }

    fn debug(&mut self, kind: InfoType, data: &[u8]) {
        match kind {
            InfoType::HeaderOut => {
                let is_proxy_connect = self
                    .proxy_connect_response_recorder
                    .record_outgoing_header_block(data);
                if !is_proxy_connect
                    && let Some(recorder) = self.network_observation_recorder.as_ref()
                {
                    recorder.record_request_header_block(data);
                }
            }
            InfoType::HeaderIn => self
                .proxy_connect_response_recorder
                .record_incoming_header_line(data),
            _ => {}
        }
    }

    fn ssl_ctx(&mut self, ssl_ctx: *mut std::ffi::c_void) -> std::result::Result<(), curl::Error> {
        configure_openssl_tls_context(ssl_ctx)
    }
}

fn configure_network_observation(
    easy: &mut Easy2<FetchTransferHandler>,
    request: &Request,
    request_cookie_report: Option<&StoredCookieQueryReport>,
    capture_proxy_connect_response: bool,
) -> Result<()> {
    let recorder = request.network_observation_recorder().cloned();
    let verbose = recorder.is_some() || capture_proxy_connect_response;
    if let Some(recorder) = recorder.as_ref() {
        recorder.set_current_request_cookie_report(request_cookie_report.cloned());
    }
    easy.get_mut()
        .begin_transfer(recorder, capture_proxy_connect_response);
    easy.verbose(verbose)
        .context("failed to configure curl network observation")
}

enum JobOutcome {
    Submitted,
    Complete(RuntimeResponseTx, Box<CompletedBufferedResponse>),
    Retry(Box<RuntimeJob>),
}

enum StreamingJobOutcome {
    Submitted,
    Complete,
}

enum CompletedBufferedResponse {
    Raw(RawResponse),
}

impl CompletedBufferedResponse {
    fn into_text_response(self) -> Response {
        match self {
            Self::Raw(response) => response.into_lossy_materialized_text_response(),
        }
    }

    fn into_materialized_raw_response(self) -> RawResponse {
        match self {
            Self::Raw(response) => response,
        }
    }
}

#[cfg(test)]
fn complete_streaming_html_job(job: StreamingRuntimeJob, response: Response) {
    let network_request_extra_info = response.network_request_extra_info().cloned();
    let (head, body) = response.into_text_parts();
    if let Some(started_tx) = job.started_tx {
        let _ = started_tx.send(Ok(StreamingHtmlResponseStart {
            final_url: head.final_url,
            status: head.status,
            headers: head.headers,
            request_cookie_report: head.request_cookie_report,
            cookie_set_reports: head.cookie_set_reports,
            redirected: head.redirected,
            redirect_chain: head.redirect_chain,
            from_cache: head.from_cache,
            negotiated_http_version: head.negotiated_http_version,
            network_request_extra_info,
        }));
    }
    if let Some(body_tx) = job.body_tx
        && !body.is_empty()
    {
        let _ = body_tx.send(body);
    }
    let _ = job.completion_tx.send(Ok(()));
}

fn complete_cached_streaming_html_job(
    job: StreamingRuntimeJob,
    cached: CachedStreamingResponseLookup,
    request_cookie_report: Option<StoredCookieQueryReport>,
) {
    let redirected = !job.redirect_chain.is_empty();
    let redirect_chain = job.redirect_chain.clone();
    let CachedStreamingResponseLookup {
        final_url,
        status,
        headers,
        mut body,
        ..
    } = cached;
    let final_url = match Url::parse(&final_url) {
        Ok(final_url) => final_url,
        Err(error) => {
            if let Some(started_tx) = job.started_tx {
                let _ = started_tx.send(Err(anyhow!(
                    "failed to parse cached response final url: {error}"
                )));
            }
            let _ = job.completion_tx.send(Err(anyhow!(
                "failed to parse cached response final url: {error}"
            )));
            return;
        }
    };

    if let Some(started_tx) = job.started_tx {
        let _ = started_tx.send(Ok(StreamingHtmlResponseStart {
            final_url,
            status,
            headers,
            request_cookie_report,
            cookie_set_reports: Vec::new(),
            redirected,
            redirect_chain,
            from_cache: true,
            negotiated_http_version: None,
            network_request_extra_info: None,
        }));
    }

    let completion = if let Some(body_tx) = job.body_tx {
        send_cached_html_body_chunks(&mut body, &body_tx)
    } else {
        Ok(())
    };
    let _ = job.completion_tx.send(completion);
}

fn send_cached_html_body_chunks(
    body: &mut impl Read,
    body_tx: &mpsc::UnboundedSender<String>,
) -> Result<()> {
    let mut buffer = [0u8; 16 * 1024];
    let mut utf8_pending = Vec::new();
    loop {
        let read = body
            .read(&mut buffer)
            .context("failed to read cached streaming response body")?;
        if read == 0 {
            break;
        }
        utf8_pending.extend_from_slice(&buffer[..read]);
        if !drain_cached_utf8_chunks(&mut utf8_pending, body_tx) {
            return Ok(());
        }
    }
    if !utf8_pending.is_empty() {
        let tail = std::mem::take(&mut utf8_pending);
        let _ = body_tx.send(String::from_utf8_lossy(&tail).into_owned());
    }
    Ok(())
}

fn drain_cached_utf8_chunks(
    utf8_pending: &mut Vec<u8>,
    body_tx: &mpsc::UnboundedSender<String>,
) -> bool {
    loop {
        match std::str::from_utf8(utf8_pending) {
            Ok(valid) => {
                if !valid.is_empty() && body_tx.send(valid.to_owned()).is_err() {
                    return false;
                }
                utf8_pending.clear();
                return true;
            }
            Err(error) => {
                let valid_up_to = error.valid_up_to();
                if valid_up_to > 0 {
                    let valid = String::from_utf8_lossy(&utf8_pending[..valid_up_to]).into_owned();
                    if body_tx.send(valid).is_err() {
                        return false;
                    }
                }
                match error.error_len() {
                    Some(error_len) => {
                        let invalid_end = valid_up_to + error_len;
                        let invalid =
                            String::from_utf8_lossy(&utf8_pending[valid_up_to..invalid_end])
                                .into_owned();
                        if body_tx.send(invalid).is_err() {
                            return false;
                        }
                        utf8_pending.drain(..invalid_end);
                    }
                    None => {
                        utf8_pending.drain(..valid_up_to);
                        return true;
                    }
                }
            }
        }
    }
}

fn complete_cached_streaming_raw_job(
    job: StreamingRawRuntimeJob,
    cached: CachedStreamingResponseLookup,
    request_cookie_report: Option<StoredCookieQueryReport>,
) {
    job.cancel_handle.mark_response_terminal();
    let redirected = !job.redirect_chain.is_empty();
    let redirect_chain = job.redirect_chain.clone();
    let CachedStreamingResponseLookup {
        final_url,
        status,
        headers,
        mut body,
        ..
    } = cached;
    let final_url = match Url::parse(&final_url) {
        Ok(final_url) => final_url,
        Err(error) => {
            if let Some(started_tx) = job.started_tx {
                let _ = started_tx.send(Err(anyhow!(
                    "failed to parse cached raw response final url: {error}"
                )));
            }
            let _ = job.completion_tx.send(Err(anyhow!(
                "failed to parse cached raw response final url: {error}"
            )));
            return;
        }
    };

    if let Some(started_tx) = job.started_tx {
        let _ = started_tx.send(Ok(StreamingHtmlResponseStart {
            final_url,
            status,
            headers,
            request_cookie_report,
            cookie_set_reports: Vec::new(),
            redirected,
            redirect_chain,
            from_cache: true,
            negotiated_http_version: None,
            network_request_extra_info: None,
        }));
    }

    if let Some(body_tx) = job.body_tx {
        let completion_tx = job.completion_tx;
        // Keep cached raw-stream hits reader-backed past the cache boundary
        // instead of rebuilding a full RawResponse body in memory.
        thread::spawn(move || {
            let completion = send_cached_raw_body_chunks(&mut body, &body_tx);
            let _ = completion_tx.send(completion);
        });
    } else {
        let _ = job.completion_tx.send(Ok(()));
    }
}

fn send_cached_raw_body_chunks(
    body: &mut impl Read,
    body_tx: &mpsc::UnboundedSender<Vec<u8>>,
) -> Result<()> {
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let read = body
            .read(&mut buffer)
            .context("failed to read cached raw streaming response body")?;
        if read == 0 {
            break;
        }
        if body_tx.send(buffer[..read].to_vec()).is_err() {
            return Ok(());
        }
    }
    Ok(())
}

fn request_fetch_load_priority(request: &Request) -> crate::ResourceLoadPriority {
    let fetch_priority_hint = request.priority_hints.fetch_priority;
    let base_resource_priority = request_base_resource_priority(request);
    if request.subresource_request_metadata().is_none() {
        let priority = match request.browser_request_metadata() {
            Some(
                crate::BrowserRequestMetadata::AudioWorklet
                | crate::BrowserRequestMetadata::EventSource
                | crate::BrowserRequestMetadata::Fetch
                | crate::BrowserRequestMetadata::JsonModule
                | crate::BrowserRequestMetadata::Manifest
                | crate::BrowserRequestMetadata::StyleModule
                | crate::BrowserRequestMetadata::Xhr,
            ) => crate::RequestResourceType::Raw.default_load_priority(),
            Some(
                crate::BrowserRequestMetadata::Audio
                | crate::BrowserRequestMetadata::Beacon
                | crate::BrowserRequestMetadata::Font
                | crate::BrowserRequestMetadata::Image
                | crate::BrowserRequestMetadata::Ping
                | crate::BrowserRequestMetadata::Style
                | crate::BrowserRequestMetadata::TextTrack
                | crate::BrowserRequestMetadata::Video,
            )
            | None => base_resource_priority,
        };
        let author_priority =
            apply_fetch_priority_hint(priority, request.resource_type, fetch_priority_hint);
        let image_priority = apply_in_document_image_priority_boost(
            author_priority,
            request.resource_type,
            fetch_priority_hint,
            request.priority_hints.in_document_image_priority_boost,
        );
        return apply_subframe_priority_adjustment(
            image_priority,
            request.priority_hints.subframe_context,
        );
    }

    let author_priority = apply_fetch_priority_hint(
        base_resource_priority,
        request.resource_type,
        fetch_priority_hint,
    );
    let scheduler_priority = request
        .script_scheduler_priority()
        .map(script_fetch_scheduler_priority_rank)
        .unwrap_or(author_priority);
    let image_priority = apply_in_document_image_priority_boost(
        author_priority.max(scheduler_priority),
        request.resource_type,
        fetch_priority_hint,
        request.priority_hints.in_document_image_priority_boost,
    );
    apply_subframe_priority_adjustment(image_priority, request.priority_hints.subframe_context)
}

fn request_base_resource_priority(request: &Request) -> crate::ResourceLoadPriority {
    if request.priority_hints.link_preload
        && matches!(request.resource_type, crate::RequestResourceType::Font)
    {
        return crate::ResourceLoadPriority::High;
    }
    request.resource_type.default_load_priority()
}

fn request_fetch_priority_rank(request: &Request) -> u8 {
    request_fetch_load_priority(request).scheduler_rank()
}

fn apply_fetch_priority_hint(
    priority: crate::ResourceLoadPriority,
    resource_type: crate::RequestResourceType,
    fetch_priority: Option<crate::FetchPriorityHint>,
) -> crate::ResourceLoadPriority {
    match fetch_priority {
        Some(crate::FetchPriorityHint::High) => priority.max(crate::ResourceLoadPriority::High),
        Some(crate::FetchPriorityHint::Low) => {
            if matches!(resource_type, crate::RequestResourceType::CssStyleSheet)
                && priority == crate::ResourceLoadPriority::VeryHigh
            {
                crate::ResourceLoadPriority::High
            } else {
                priority.min(crate::ResourceLoadPriority::Low)
            }
        }
        _ => priority,
    }
}

fn apply_in_document_image_priority_boost(
    priority: crate::ResourceLoadPriority,
    resource_type: crate::RequestResourceType,
    fetch_priority: Option<crate::FetchPriorityHint>,
    in_document_image_priority_boost: bool,
) -> crate::ResourceLoadPriority {
    // Chromium's first-N in-document image boost is applied after the author
    // fetchpriority hint. It only promotes auto-priority images to at least
    // Medium; an explicit `fetchpriority=low` must remain Low, and an explicit
    // high hint already outranks the boost. Layout-visible and LCP predictor
    // boosts are separate Chromium mechanisms and are not modeled by this flag.
    if !in_document_image_priority_boost
        || !matches!(resource_type, crate::RequestResourceType::Image)
        || fetch_priority.is_some_and(|priority| priority != crate::FetchPriorityHint::Auto)
    {
        return priority;
    }
    priority.max(crate::ResourceLoadPriority::Medium)
}

fn apply_subframe_priority_adjustment(
    priority: crate::ResourceLoadPriority,
    subframe_context: bool,
) -> crate::ResourceLoadPriority {
    if !subframe_context {
        return priority;
    }
    if priority >= crate::ResourceLoadPriority::High {
        crate::ResourceLoadPriority::Low
    } else {
        crate::ResourceLoadPriority::VeryLow
    }
}

fn script_fetch_scheduler_priority_rank(
    priority: crate::ScriptFetchSchedulerPriority,
) -> crate::ResourceLoadPriority {
    match priority {
        crate::ScriptFetchSchedulerPriority::Low => crate::ResourceLoadPriority::Low,
        crate::ScriptFetchSchedulerPriority::Auto => crate::ResourceLoadPriority::High,
        crate::ScriptFetchSchedulerPriority::High => crate::ResourceLoadPriority::High,
        crate::ScriptFetchSchedulerPriority::VeryHigh => crate::ResourceLoadPriority::VeryHigh,
    }
}

fn should_retry_http2_failure_over_http1(
    request: &Request,
    http_version: RequestHttpVersion,
    used_http2: bool,
    error: &anyhow::Error,
) -> bool {
    if http_version != RequestHttpVersion::PreferHttp2
        || !(request.method.eq_ignore_ascii_case("GET")
            || request.method.eq_ignore_ascii_case("HEAD"))
    {
        return false;
    }

    // RFC 9110 section 9.2.2 permits replaying an idempotent request after a
    // communication failure before a response is exposed. The caller enforces
    // that response boundary, while Http1Only makes this a single compatibility
    // retry. A negotiated-H2 CURLE_SEND_ERROR is admitted because libcurl can
    // surface a failure while emitting the RST_STREAM for a malformed response
    // under that generic code. Use CURLINFO_HTTP_VERSION rather than unstable
    // CURLOPT_ERRORBUFFER text to prove that the failed transfer used H2.
    //
    // CURLE_HTTP2_STREAM remains terminal: a stream-scoped failure alone does
    // not show that changing the connection protocol would avoid the failure.
    error.chain().any(|cause| {
        cause
            .downcast_ref::<curl::Error>()
            .is_some_and(|error| error.is_http2_error() || (used_http2 && error.is_send_error()))
    })
}

fn transfer_used_http2<H: Handler>(easy: &Easy2<H>) -> bool {
    negotiated_http_version_from_easy(easy) == Some(NegotiatedHttpVersion::Http2)
}

fn negotiated_http_version_from_easy<H: Handler>(easy: &Easy2<H>) -> Option<NegotiatedHttpVersion> {
    let mut version: c_long = 0;
    let result =
        unsafe { curl_sys::curl_easy_getinfo(easy.raw(), CURLINFO_HTTP_VERSION, &mut version) };
    if result != curl_sys::CURLE_OK {
        return None;
    }
    match version {
        value if value == c_long::from(curl_sys::CURL_HTTP_VERSION_1_0) => {
            Some(NegotiatedHttpVersion::Http10)
        }
        value if value == c_long::from(curl_sys::CURL_HTTP_VERSION_1_1) => {
            Some(NegotiatedHttpVersion::Http11)
        }
        value if value == c_long::from(curl_sys::CURL_HTTP_VERSION_2_0) => {
            Some(NegotiatedHttpVersion::Http2)
        }
        value if value == c_long::from(curl_sys::CURL_HTTP_VERSION_3) => {
            Some(NegotiatedHttpVersion::Http3)
        }
        _ => None,
    }
}

fn empty_http_navigation_https_upgrade_url(
    request: &Request,
    current_url: &Url,
    already_attempted: bool,
    error: &anyhow::Error,
) -> Option<Url> {
    if already_attempted
        || !request.is_top_level_navigation_request()
        || !(request.method.eq_ignore_ascii_case("GET")
            || request.method.eq_ignore_ascii_case("HEAD"))
        || current_url.scheme() != "http"
        || !error.chain().any(|cause| {
            cause
                .downcast_ref::<curl::Error>()
                .is_some_and(curl::Error::is_got_nothing)
        })
    {
        return None;
    }

    let Host::Domain(domain) = current_url.host()? else {
        return None;
    };
    let domain = domain.trim_end_matches('.').to_ascii_lowercase();
    if !domain.contains('.') || is_special_use_domain_for_https_upgrade(&domain) {
        return None;
    }

    let had_explicit_http_port = current_url.port() == Some(80);
    let mut upgraded_url = current_url.clone();
    upgraded_url.set_scheme("https").ok()?;
    if had_explicit_http_port {
        upgraded_url.set_port(None).ok()?;
    }
    Some(upgraded_url)
}

fn is_special_use_domain_for_https_upgrade(domain: &str) -> bool {
    // ICANN resolution 2024.07.29.06 permanently reserves .internal for
    // private-use applications, where an implicit HTTPS retry is undesirable.
    [
        "localhost",
        "test",
        "invalid",
        "example",
        "local",
        "internal",
    ]
    .into_iter()
    .any(|suffix| domain == suffix || domain.ends_with(&format!(".{suffix}")))
}

fn https_upgrade_redirect_info(
    from_url: Url,
    to_url: Url,
    request_cookie_report: Option<StoredCookieQueryReport>,
) -> RedirectInfo {
    RedirectInfo {
        from_url,
        headers: vec![
            ("location".to_owned(), to_url.to_string()),
            (
                "non-authoritative-reason".to_owned(),
                "HttpsUpgrades".to_owned(),
            ),
        ],
        to_url,
        status: 307,
        network_extra_info_available: false,
        request_extra_info: None,
        response_extra_info: None,
        redirect_has_extra_info: false,
        request_cookie_report,
        cookie_set_reports: Vec::new(),
        from_cache: false,
        negotiated_http_version: Some(NegotiatedHttpVersion::Http11),
    }
}

fn attach_next_request_extra_info(
    redirect_chain: &mut [RedirectInfo],
    request_cookie_report: Option<StoredCookieQueryReport>,
    request_extra_info: Option<&NetworkRequestExtraInfo>,
) {
    if let Some(previous_redirect) = redirect_chain.last_mut() {
        previous_redirect.request_cookie_report = request_cookie_report;
        previous_redirect.request_extra_info = request_extra_info.cloned();
    }
}

fn network_response_extra_info(
    request_extra_info: NetworkRequestExtraInfo,
    status: u16,
    headers: Vec<(String, String)>,
    cookie_set_reports: Vec<moli_cookie_jar::StoredCookieSetReport>,
) -> NetworkResponseExtraInfo {
    NetworkResponseExtraInfo {
        request_extra_info,
        status,
        headers,
        cookie_set_reports,
    }
}

fn critical_client_hint_restart_redirect_info(
    url: Url,
    response_extra_info: NetworkResponseExtraInfo,
) -> RedirectInfo {
    RedirectInfo {
        from_url: url.clone(),
        to_url: url.clone(),
        status: 307,
        headers: vec![("Location".to_owned(), url.to_string())],
        network_extra_info_available: false,
        request_extra_info: None,
        response_extra_info: Some(response_extra_info),
        redirect_has_extra_info: false,
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        from_cache: false,
        negotiated_http_version: Some(NegotiatedHttpVersion::Http11),
    }
}

fn origin_key(request: &Request) -> Option<CurlOriginKey> {
    origin_key_for_url(&request.url)
}

fn origin_key_for_url(url: &Url) -> Option<CurlOriginKey> {
    Some(CurlOriginKey {
        scheme: url.scheme().to_owned(),
        host: url.host_str()?.to_owned(),
        port: url.port_or_known_default(),
    })
}

fn curl_runtime_config(config: &FetchConfig) -> CurlMultiRuntimeConfig {
    let max_active = NonZeroUsize::new(max_runtime_transfers(config))
        .expect("fetch runtime transfer count is non-zero");
    // Connection-pool limits are transport limits. They intentionally flow only
    // into curl's multi options, while `max_host_active` below remains an
    // optional scheduler cap for active transfers to one origin.
    let max_host_connections = config
        .effective_http_max_host_connections()
        .and_then(|value| NonZeroUsize::new(usize::from(value)));
    let max_host_active = config
        .http_max_host_open()
        .and_then(|value| NonZeroUsize::new(value.get() as usize));
    let max_total_connections = config
        .http_max_total_connections()
        .and_then(|value| NonZeroUsize::new(usize::from(value)));
    let max_concurrent_streams = config
        .http2_max_concurrent_streams()
        .and_then(|value| NonZeroUsize::new(usize::from(value)));
    CurlMultiRuntimeConfig {
        max_active,
        max_host_active,
        max_host_connections,
        max_total_connections,
        max_concurrent_streams,
        poll_interval: RUNTIME_POLL_INTERVAL,
        multiplex: true,
        thread_name: "lm-net-multi".to_owned(),
    }
}

fn max_runtime_transfers(config: &FetchConfig) -> usize {
    config
        .http_max_concurrent()
        .map(NonZeroU32::get)
        .map(|count| count as usize)
        .unwrap_or_else(default_runtime_transfer_count)
}

fn default_runtime_transfer_count() -> usize {
    DEFAULT_RUNTIME_TRANSFERS
}

fn send_response(response_tx: RuntimeResponseTx, response: Result<CompletedBufferedResponse>) {
    response_tx.send(response);
}

fn take_failed_proxy_connect_response(
    easy: &mut Easy2<FetchTransferHandler>,
) -> Option<ProxyConnectResponse> {
    let connect_status = easy.http_connectcode().ok()?;
    easy.get_mut()
        .take_failed_proxy_connect_response(connect_status)
}

fn proxy_connect_response_start(
    current_url: &Url,
    redirect_chain: &[RedirectInfo],
    request_cookie_report: Option<StoredCookieQueryReport>,
    response: ProxyConnectResponse,
) -> StreamingHtmlResponseStart {
    StreamingHtmlResponseStart {
        final_url: current_url.clone(),
        status: response.status,
        headers: response.headers,
        request_cookie_report,
        cookie_set_reports: Vec::new(),
        redirected: !redirect_chain.is_empty(),
        redirect_chain: redirect_chain.to_vec(),
        from_cache: false,
        negotiated_http_version: None,
        network_request_extra_info: None,
    }
}

fn proxy_connect_raw_response(
    current_url: &Url,
    redirect_chain: &[RedirectInfo],
    request_cookie_report: Option<StoredCookieQueryReport>,
    response: ProxyConnectResponse,
) -> RawResponse {
    RawResponse::from_head_and_body(
        proxy_connect_response_start(current_url, redirect_chain, request_cookie_report, response)
            .into_head(),
        Vec::new(),
    )
}

fn complete_streaming_proxy_connect_response(
    job: StreamingRuntimeJob,
    mut easy: Easy2<FetchTransferHandler>,
    request_cookie_report: Option<StoredCookieQueryReport>,
    response: ProxyConnectResponse,
) {
    let start = proxy_connect_response_start(
        &job.current_url,
        &job.redirect_chain,
        request_cookie_report,
        response,
    );
    let (started_tx, body_tx) = easy
        .get_mut()
        .streaming_mut()
        .expect("proxy CONNECT streaming response should use streaming collector")
        .take_response_channels();
    drop(body_tx);
    job.cancel_handle.mark_response_terminal();
    if let Some(started_tx) = started_tx {
        let _ = started_tx.send(Ok(start));
    }
    let _ = job.completion_tx.send(Ok(()));
}

fn complete_raw_streaming_proxy_connect_response(
    job: StreamingRawRuntimeJob,
    mut easy: Easy2<FetchTransferHandler>,
    request_cookie_report: Option<StoredCookieQueryReport>,
    response: ProxyConnectResponse,
) {
    let start = proxy_connect_response_start(
        &job.current_url,
        &job.redirect_chain,
        request_cookie_report,
        response,
    );
    let (started_tx, body_tx) = easy
        .get_mut()
        .raw_streaming_mut()
        .expect("proxy CONNECT raw response should use raw streaming collector")
        .take_response_channels();
    drop(body_tx);
    job.cancel_handle.mark_response_terminal();
    if let Some(started_tx) = started_tx {
        let _ = started_tx.send(Ok(start));
    }
    let _ = job.completion_tx.send(Ok(()));
}

fn collect_buffered_response(
    easy: &mut Easy2<FetchTransferHandler>,
    request_url: &Url,
) -> Result<(RawResponse, RequestTransferMetrics)> {
    let status = easy
        .response_code()
        .context("failed to read curl response code")? as u16;
    let final_url_text = easy
        .effective_url()?
        .unwrap_or(request_url.as_str())
        .to_owned();
    let final_url = Url::parse(&final_url_text)
        .with_context(|| anyhow!("failed to parse final response url `{final_url_text}`"))?;
    let negotiated_http_version = negotiated_http_version_from_easy(easy);
    let collector = easy
        .get_ref()
        .buffered()
        .ok_or_else(|| anyhow!("curl runtime returned non-buffered easy for buffered request"))?;
    let headers = collector.headers().to_vec();
    let body = collector.body().to_vec();
    let transfer_metrics = transfer_metrics_from_easy(easy, &headers);

    Ok((
        RawResponse::from_head_and_body(
            ResponseHead {
                final_url,
                status,
                headers,
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                negotiated_http_version,
            },
            body,
        ),
        transfer_metrics,
    ))
}

fn fail_streaming_job(job: StreamingRuntimeJob, error: anyhow::Error) {
    fail_streaming_job_with_easy(job, None, error);
}

fn fail_streaming_job_with_easy(
    job: StreamingRuntimeJob,
    mut easy: Option<Easy2<FetchTransferHandler>>,
    error: anyhow::Error,
) {
    let error = network_fetch_failure_for_request(
        &job.request,
        &job.current_url,
        &job.redirect_chain,
        error,
    );
    if let Some(easy) = easy.as_mut()
        && let Some(streaming) = easy.get_mut().streaming_mut()
    {
        if streaming.started() {
            streaming.abort_started_body();
        } else {
            streaming.fail(error);
            let _ = job.completion_tx.send(Err(anyhow!(
                "streaming html request failed before response start"
            )));
            return;
        }
    } else if let Some(started_tx) = job.started_tx {
        let _ = started_tx.send(Err(error));
        let _ = job.completion_tx.send(Err(anyhow!(
            "streaming html request failed before response start"
        )));
        return;
    }

    let _ = job.completion_tx.send(Err(error));
}

fn fail_raw_streaming_job(job: StreamingRawRuntimeJob, error: anyhow::Error) {
    fail_raw_streaming_job_with_easy(job, None, error);
}

fn fail_raw_streaming_job_with_easy(
    job: StreamingRawRuntimeJob,
    mut easy: Option<Easy2<FetchTransferHandler>>,
    error: anyhow::Error,
) {
    let error = network_fetch_failure_for_request(
        &job.request,
        &job.current_url,
        &job.redirect_chain,
        error,
    );
    job.cancel_handle.mark_response_terminal();
    if let Some(easy) = easy.as_mut()
        && let Some(streaming) = easy.get_mut().raw_streaming_mut()
    {
        if streaming.started() {
            streaming.abort_started_body();
        } else {
            streaming.fail(error);
            let _ = job.completion_tx.send(Err(anyhow!(
                "streaming raw request failed before response start"
            )));
            return;
        }
    } else if let Some(started_tx) = job.started_tx {
        let _ = started_tx.send(Err(error));
        let _ = job.completion_tx.send(Err(anyhow!(
            "streaming raw request failed before response start"
        )));
        return;
    }

    let _ = job.completion_tx.send(Err(error));
}

fn network_fetch_failure_for_request(
    request: &Request,
    current_url: &Url,
    redirect_chain: &[RedirectInfo],
    error: anyhow::Error,
) -> anyhow::Error {
    if error.is::<NetworkFetchFailureContext>() {
        return error;
    }
    let Some(recorder) = request.network_observation_recorder() else {
        return error;
    };
    NetworkFetchFailureContext::attach_with_request_context(
        error,
        recorder.snapshot(),
        NetworkFetchFailureRequestContext::new(
            current_url.clone(),
            request.method.clone(),
            request.body.clone(),
            request.request_headers.clone(),
            redirect_chain.to_vec(),
        ),
    )
}

#[cfg(test)]
fn request_panics_for_testing(request: &Request) -> bool {
    request.request_headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("x-moli-test-panic") && value == "runtime-worker"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ScriptFetchRequestMetadata;
    use moli_cookie_jar::new_shared_browser_cookie_store;

    fn request_with_priority(fetch_priority: Option<crate::FetchPriorityHint>) -> Request {
        let metadata = fetch_priority.map(|priority| ScriptFetchRequestMetadata {
            fetch_priority: Some(priority),
            ..ScriptFetchRequestMetadata::default()
        });
        let request = Request::new("GET", "https://example.test/script.js", None, Vec::new())
            .expect("test request should parse");
        match metadata {
            Some(metadata) => request.with_script_fetch_metadata(metadata),
            None => request,
        }
    }

    #[test]
    fn http2_fallback_policy_only_replays_safe_initial_attempts() {
        let http2_error = anyhow::Error::new(curl::Error::new(curl_sys::CURLE_HTTP2))
            .context("curl request failed");
        let send_error = anyhow::Error::new(curl::Error::new(curl_sys::CURLE_SEND_ERROR));
        let get = Request::new("GET", "https://example.test/", None, Vec::new()).unwrap();
        let head = Request::new("head", "https://example.test/", None, Vec::new()).unwrap();
        let post = Request::new("POST", "https://example.test/", None, Vec::new()).unwrap();

        assert!(should_retry_http2_failure_over_http1(
            &get,
            RequestHttpVersion::PreferHttp2,
            false,
            &http2_error
        ));
        assert!(should_retry_http2_failure_over_http1(
            &head,
            RequestHttpVersion::PreferHttp2,
            false,
            &http2_error
        ));
        assert!(!should_retry_http2_failure_over_http1(
            &post,
            RequestHttpVersion::PreferHttp2,
            false,
            &http2_error
        ));
        assert!(!should_retry_http2_failure_over_http1(
            &get,
            RequestHttpVersion::Http1Only,
            false,
            &http2_error
        ));
        assert!(should_retry_http2_failure_over_http1(
            &get,
            RequestHttpVersion::PreferHttp2,
            true,
            &send_error
        ));
        assert!(!should_retry_http2_failure_over_http1(
            &post,
            RequestHttpVersion::PreferHttp2,
            true,
            &send_error
        ));
        assert!(!should_retry_http2_failure_over_http1(
            &get,
            RequestHttpVersion::PreferHttp2,
            false,
            &send_error
        ));

        let stream_error = anyhow::Error::new(curl::Error::new(curl_sys::CURLE_HTTP2_STREAM));
        assert!(!should_retry_http2_failure_over_http1(
            &get,
            RequestHttpVersion::PreferHttp2,
            true,
            &stream_error
        ));
    }

    #[test]
    fn empty_http_navigation_upgrade_policy_is_narrow() {
        let empty_reply = anyhow::Error::new(curl::Error::new(curl_sys::CURLE_GOT_NOTHING))
            .context("curl request failed");
        let other_error = anyhow::Error::new(curl::Error::new(curl_sys::CURLE_RECV_ERROR));
        let get = Request::get("http://www.example.org/path").unwrap();
        let head = Request::new("HEAD", "http://www.example.org/path", None, Vec::new())
            .unwrap()
            .with_top_level_navigation_cookie_context();
        let post = Request::new("POST", "http://www.example.org/path", None, Vec::new())
            .unwrap()
            .with_top_level_navigation_cookie_context();
        let subresource =
            Request::new("GET", "http://www.example.org/path", None, Vec::new()).unwrap();

        assert_eq!(
            empty_http_navigation_https_upgrade_url(&get, &get.url, false, &empty_reply)
                .as_ref()
                .map(Url::as_str),
            Some("https://www.example.org/path")
        );
        assert!(
            empty_http_navigation_https_upgrade_url(&head, &head.url, false, &empty_reply)
                .is_some()
        );
        assert!(
            empty_http_navigation_https_upgrade_url(&post, &post.url, false, &empty_reply)
                .is_none()
        );
        assert!(
            empty_http_navigation_https_upgrade_url(
                &subresource,
                &subresource.url,
                false,
                &empty_reply
            )
            .is_none()
        );
        assert!(
            empty_http_navigation_https_upgrade_url(&get, &get.url, true, &empty_reply).is_none()
        );
        assert!(
            empty_http_navigation_https_upgrade_url(&get, &get.url, false, &other_error).is_none()
        );
    }

    #[test]
    fn empty_http_navigation_upgrade_rejects_non_public_hosts() {
        let empty_reply = anyhow::Error::new(curl::Error::new(curl_sys::CURLE_GOT_NOTHING));
        for url in [
            "http://localhost/path",
            "http://127.0.0.1/path",
            "http://host.test/path",
            "http://host.invalid/path",
            "http://host.example/path",
            "http://host.local/path",
            "http://app.internal/path",
            "http://intranet/path",
            "https://www.example.org/path",
        ] {
            let request = Request::get(url).unwrap();
            assert!(
                empty_http_navigation_https_upgrade_url(
                    &request,
                    &request.url,
                    false,
                    &empty_reply
                )
                .is_none(),
                "unexpected HTTPS upgrade for {url}"
            );
        }
    }

    #[test]
    fn script_fetch_priority_maps_to_scheduler_rank() {
        assert!(
            request_fetch_priority_rank(&request_with_priority(Some(
                crate::FetchPriorityHint::High
            ))) == request_fetch_priority_rank(&request_with_priority(None))
        );
        assert!(
            request_fetch_priority_rank(&request_with_priority(None))
                > request_fetch_priority_rank(&request_with_priority(Some(
                    crate::FetchPriorityHint::Low
                )))
        );
    }

    #[test]
    fn internal_script_scheduler_priority_can_promote_author_low_hint() {
        let request = Request::new("GET", "https://example.test/script.js", None, Vec::new())
            .expect("test request should parse")
            .with_script_fetch_metadata(ScriptFetchRequestMetadata {
                fetch_priority: Some(crate::FetchPriorityHint::Low),
                scheduler_priority: Some(crate::ScriptFetchSchedulerPriority::High),
                ..ScriptFetchRequestMetadata::default()
            });

        assert_eq!(
            request_fetch_load_priority(&request),
            crate::ResourceLoadPriority::High,
            "DCL-critical internal priority should not be lowered by an author hint"
        );
    }

    #[test]
    fn chromium_resource_types_map_to_load_priority() {
        let stylesheet = Request::new("GET", "https://example.test/app.css", None, Vec::new())
            .expect("test request should parse")
            .with_resource_type(crate::RequestResourceType::CssStyleSheet)
            .with_browser_request_metadata(crate::BrowserRequestMetadata::Style);
        let font = Request::new("GET", "https://example.test/font.woff2", None, Vec::new())
            .expect("test request should parse")
            .with_resource_type(crate::RequestResourceType::Font);
        let font_preload = Request::new("GET", "https://example.test/font.woff2", None, Vec::new())
            .expect("test request should parse")
            .with_resource_type(crate::RequestResourceType::Font)
            .with_link_preload();
        let fetch = Request::new("GET", "https://example.test/data.json", None, Vec::new())
            .expect("test request should parse")
            .with_browser_request_metadata(crate::BrowserRequestMetadata::Fetch);
        let raw = Request::new("GET", "https://example.test/data.json", None, Vec::new())
            .expect("test request should parse")
            .with_resource_type(crate::RequestResourceType::Raw);
        let script = Request::new("GET", "https://example.test/app.js", None, Vec::new())
            .expect("test request should parse")
            .with_script_fetch_metadata(ScriptFetchRequestMetadata::default());
        let async_script = Request::new("GET", "https://example.test/async.js", None, Vec::new())
            .expect("test request should parse")
            .with_resource_type(crate::RequestResourceType::ClassicAsyncOrDeferScript);
        let late_preload_script =
            Request::new("GET", "https://example.test/late.js", None, Vec::new())
                .expect("test request should parse")
                .with_resource_type(crate::RequestResourceType::LatePreloadScript);
        let late_preload_stylesheet =
            Request::new("GET", "https://example.test/late.css", None, Vec::new())
                .expect("test request should parse")
                .with_resource_type(crate::RequestResourceType::LatePreloadCssStyleSheet);
        let beacon = Request::new("POST", "https://example.test/beacon", None, Vec::new())
            .expect("test request should parse")
            .with_resource_type(crate::RequestResourceType::Beacon);
        let ping = Request::new("POST", "https://example.test/ping", None, Vec::new())
            .expect("test request should parse")
            .with_resource_type(crate::RequestResourceType::Ping);
        let csp_report = Request::new("POST", "https://example.test/csp-report", None, Vec::new())
            .expect("test request should parse")
            .with_resource_type(crate::RequestResourceType::CspReport);
        let link_prefetch = Request::new("GET", "https://example.test/next.html", None, Vec::new())
            .expect("test request should parse")
            .with_resource_type(crate::RequestResourceType::LinkPrefetch);
        let dictionary = Request::new("GET", "https://example.test/dict.bin", None, Vec::new())
            .expect("test request should parse")
            .with_resource_type(crate::RequestResourceType::Dictionary);

        assert_eq!(
            request_fetch_load_priority(&stylesheet),
            crate::ResourceLoadPriority::VeryHigh
        );
        assert_eq!(
            request_fetch_load_priority(&font),
            crate::ResourceLoadPriority::VeryHigh
        );
        assert_eq!(
            request_fetch_load_priority(&font_preload),
            crate::ResourceLoadPriority::High,
            "Chromium lowers link-preloaded fonts below critical CSS/scripts"
        );
        assert_eq!(
            request_fetch_load_priority(&fetch),
            crate::ResourceLoadPriority::High
        );
        assert_eq!(
            request_fetch_load_priority(&raw),
            crate::ResourceLoadPriority::High
        );
        assert_eq!(
            request_fetch_load_priority(&script),
            crate::ResourceLoadPriority::High
        );
        assert_eq!(
            request_fetch_load_priority(&async_script),
            crate::ResourceLoadPriority::Low
        );
        assert_eq!(
            request_fetch_load_priority(&late_preload_script),
            crate::ResourceLoadPriority::Medium
        );
        assert_eq!(
            request_fetch_load_priority(&late_preload_stylesheet),
            crate::ResourceLoadPriority::Medium,
            "Chromium lowers late in-document preload-scanner stylesheets"
        );
        assert_eq!(
            request_fetch_load_priority(&beacon),
            crate::ResourceLoadPriority::VeryLow,
            "Chromium lowers beacon request contexts"
        );
        assert_eq!(
            request_fetch_load_priority(&ping),
            crate::ResourceLoadPriority::VeryLow,
            "Chromium lowers ping request contexts"
        );
        assert_eq!(
            request_fetch_load_priority(&csp_report),
            crate::ResourceLoadPriority::VeryLow,
            "Chromium lowers CSP report request contexts"
        );
        assert_eq!(
            request_fetch_load_priority(&link_prefetch),
            crate::ResourceLoadPriority::VeryLow,
            "Chromium lowers link prefetch requests"
        );
        assert_eq!(
            request_fetch_load_priority(&dictionary),
            crate::ResourceLoadPriority::VeryLow,
            "Chromium lowers compression dictionary requests"
        );
    }

    #[test]
    fn fetch_priority_hints_apply_to_non_script_resources() {
        let boosted_image = Request::new("GET", "https://example.test/image.png", None, Vec::new())
            .expect("test request should parse")
            .with_resource_type(crate::RequestResourceType::Image)
            .with_fetch_priority_hint(Some(crate::FetchPriorityHint::High));
        let demoted_stylesheet =
            Request::new("GET", "https://example.test/app.css", None, Vec::new())
                .expect("test request should parse")
                .with_resource_type(crate::RequestResourceType::CssStyleSheet)
                .with_fetch_priority_hint(Some(crate::FetchPriorityHint::Low));

        assert_eq!(
            request_fetch_load_priority(&boosted_image),
            crate::ResourceLoadPriority::High,
            "Chromium treats fetchpriority as a generic ResourceRequest hint"
        );
        assert_eq!(
            request_fetch_load_priority(&demoted_stylesheet),
            crate::ResourceLoadPriority::High,
            "Chromium only lowers critical CSS from VeryHigh to High for low hints"
        );
    }

    #[test]
    fn in_document_image_boost_matches_chromium_first_n_auto_rule() {
        let auto_image = Request::new("GET", "https://example.test/hero.png", None, Vec::new())
            .expect("test request should parse")
            .with_resource_type(crate::RequestResourceType::Image)
            .with_in_document_image_priority_boost(true);
        let explicit_auto_image =
            Request::new("GET", "https://example.test/auto.png", None, Vec::new())
                .expect("test request should parse")
                .with_resource_type(crate::RequestResourceType::Image)
                .with_fetch_priority_hint(Some(crate::FetchPriorityHint::Auto))
                .with_in_document_image_priority_boost(true);
        let low_image = Request::new("GET", "https://example.test/low.png", None, Vec::new())
            .expect("test request should parse")
            .with_resource_type(crate::RequestResourceType::Image)
            .with_fetch_priority_hint(Some(crate::FetchPriorityHint::Low))
            .with_in_document_image_priority_boost(true);
        let high_image = Request::new("GET", "https://example.test/high.png", None, Vec::new())
            .expect("test request should parse")
            .with_resource_type(crate::RequestResourceType::Image)
            .with_fetch_priority_hint(Some(crate::FetchPriorityHint::High))
            .with_in_document_image_priority_boost(true);

        assert_eq!(
            request_fetch_load_priority(&auto_image),
            crate::ResourceLoadPriority::Medium,
            "Chromium boosts first-N in-document non-small auto-priority images"
        );
        assert_eq!(
            request_fetch_load_priority(&explicit_auto_image),
            crate::ResourceLoadPriority::Medium
        );
        assert_eq!(
            request_fetch_load_priority(&low_image),
            crate::ResourceLoadPriority::Low,
            "explicit low priority disables the first-N auto image boost"
        );
        assert_eq!(
            request_fetch_load_priority(&high_image),
            crate::ResourceLoadPriority::High,
            "explicit high priority already outranks the first-N image boost"
        );
    }

    #[test]
    fn subframe_context_deprioritizes_after_resource_and_author_priority() {
        let subframe_fetch =
            Request::new("GET", "https://example.test/data.json", None, Vec::new())
                .expect("test request should parse")
                .with_browser_request_metadata(crate::BrowserRequestMetadata::Fetch)
                .with_subframe_context(true);
        let subframe_boosted_image =
            Request::new("GET", "https://example.test/hero.png", None, Vec::new())
                .expect("test request should parse")
                .with_resource_type(crate::RequestResourceType::Image)
                .with_fetch_priority_hint(Some(crate::FetchPriorityHint::High))
                .with_subframe_context(true);
        let subframe_image =
            Request::new("GET", "https://example.test/thumb.png", None, Vec::new())
                .expect("test request should parse")
                .with_resource_type(crate::RequestResourceType::Image)
                .with_subframe_context(true);

        assert_eq!(
            request_fetch_load_priority(&subframe_fetch),
            crate::ResourceLoadPriority::Low,
            "Chromium lowers high-priority child-frame resources to low"
        );
        assert_eq!(
            request_fetch_load_priority(&subframe_boosted_image),
            crate::ResourceLoadPriority::Low,
            "subframe deprioritization runs after author priority hints"
        );
        assert_eq!(
            request_fetch_load_priority(&subframe_image),
            crate::ResourceLoadPriority::VeryLow,
            "delayable child-frame resources map to Moli's lowest priority"
        );
    }

    #[test]
    fn default_transfer_count_is_network_oriented_not_cpu_bound() {
        assert_eq!(default_runtime_transfer_count(), 256);
    }

    #[test]
    fn http_max_concurrent_only_sets_runtime_active_transfers() {
        let mut config = FetchConfig::default();
        config.set_connection_limits(NonZeroU32::new(8), None, None);

        let runtime_config = curl_runtime_config(&config);

        assert_eq!(runtime_config.max_active.get(), 8);
        assert_eq!(runtime_config.max_host_active, None);
        assert_eq!(
            runtime_config.max_host_connections.map(NonZeroUsize::get),
            Some(usize::from(FetchConfig::DEFAULT_HTTP_MAX_HOST_CONNECTIONS))
        );
        assert_eq!(runtime_config.max_total_connections, None);
        assert_eq!(runtime_config.max_concurrent_streams, None);
    }

    #[test]
    fn http_max_host_open_only_sets_host_active_cap() {
        let mut config = FetchConfig::default();
        config.set_connection_limits(None, NonZeroU32::new(3), None);

        let runtime_config = curl_runtime_config(&config);

        assert_eq!(
            runtime_config.max_host_active.map(NonZeroUsize::get),
            Some(3)
        );
        assert_eq!(
            runtime_config.max_host_connections.map(NonZeroUsize::get),
            Some(usize::from(FetchConfig::DEFAULT_HTTP_MAX_HOST_CONNECTIONS))
        );
    }

    #[test]
    fn explicit_transport_limits_configure_curl_connections_and_h2_streams() {
        let mut config = FetchConfig::default();
        config.set_connection_limits(NonZeroU32::new(8), None, None);
        config.set_transport_connection_limits(Some(3), Some(64), Some(100));

        let runtime_config = curl_runtime_config(&config);

        assert_eq!(runtime_config.max_active.get(), 8);
        assert_eq!(runtime_config.max_host_active, None);
        assert_eq!(
            runtime_config.max_host_connections.map(NonZeroUsize::get),
            Some(3)
        );
        assert_eq!(
            runtime_config.max_total_connections.map(NonZeroUsize::get),
            Some(64)
        );
        assert_eq!(
            runtime_config.max_concurrent_streams.map(NonZeroUsize::get),
            Some(100)
        );
    }

    #[tokio::test]
    async fn failing_started_streaming_job_closes_body_and_reports_completion_error() -> Result<()>
    {
        let (job_started_tx, _job_started_rx) = oneshot::channel();
        let (job_body_tx, _job_body_rx) = mpsc::unbounded_channel();
        let (completion_tx, completion_rx) = oneshot::channel();
        let cancel_handle = FetchCancelHandle::new();
        let job = StreamingRuntimeJob::new(
            Request::get("http://example.test/stream")?,
            job_started_tx,
            job_body_tx,
            completion_tx,
            cancel_handle.clone(),
        );
        let (start_tx, start_rx) = oneshot::channel();
        let (body_tx, mut body_rx) = mpsc::unbounded_channel();
        let mut collector = StreamingResponseCollector::new(
            new_shared_browser_cookie_store(),
            start_tx,
            body_tx,
            cancel_handle.clone(),
        );
        let final_url = Url::parse("http://example.test/stream")?;
        collector.begin_request(
            None,
            final_url.clone(),
            NetworkCookieRequestContext::top_level_navigation("GET"),
            None,
            true,
            vec![],
            None,
        );
        assert!(collector.header(b"HTTP/1.1 200 OK\r\n"));
        assert!(collector.header(b"Content-Type: text/html; charset=utf-8\r\n"));
        assert!(collector.header(b"\r\n"));
        assert!(collector.started());

        let easy = Easy2::new(FetchTransferHandler::new_streaming(collector));
        fail_streaming_job_with_easy(job, Some(easy), anyhow!("transfer failed after start"));

        let started = start_rx.await??;
        assert_eq!(started.status, 200);
        assert_eq!(started.final_url, final_url);
        assert!(body_rx.recv().await.is_none());
        assert!(completion_rx.await?.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn cached_streaming_html_completion_preserves_request_cookie_report() -> Result<()> {
        let (started_tx, started_rx) = oneshot::channel();
        let (body_tx, mut body_rx) = mpsc::unbounded_channel();
        let (completion_tx, completion_rx) = oneshot::channel();
        let job = StreamingRuntimeJob::new(
            Request::get("http://example.test/cache")?,
            started_tx,
            body_tx,
            completion_tx,
            FetchCancelHandle::new(),
        );
        let request_cookie_report = StoredCookieQueryReport::default();
        let final_url = Url::parse("http://example.test/cache")?;

        complete_streaming_html_job(
            job,
            Response::from_head_and_text_body(
                ResponseHead {
                    final_url: final_url.clone(),
                    status: 200,
                    headers: vec![("content-type".to_owned(), "text/html".to_owned())],
                    request_cookie_report: Some(request_cookie_report.clone()),
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                },
                "<!doctype html><html><body>cached</body></html>".to_owned(),
            ),
        );

        let started = started_rx.await??;
        assert_eq!(started.final_url, final_url);
        assert_eq!(started.request_cookie_report, Some(request_cookie_report));
        assert!(started.cookie_set_reports.is_empty());
        assert_eq!(
            body_rx.recv().await.as_deref(),
            Some("<!doctype html><html><body>cached</body></html>")
        );
        assert!(completion_rx.await?.is_ok());
        Ok(())
    }

    #[tokio::test]
    async fn cached_streaming_body_chunks_preserve_split_utf8() -> Result<()> {
        struct OneByteReader {
            bytes: Vec<u8>,
            offset: usize,
        }

        impl Read for OneByteReader {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                if self.offset >= self.bytes.len() {
                    return Ok(0);
                }
                out[0] = self.bytes[self.offset];
                self.offset += 1;
                Ok(1)
            }
        }

        let (body_tx, mut body_rx) = mpsc::unbounded_channel();
        let mut body = OneByteReader {
            bytes: "a\u{20ac}b".as_bytes().to_vec(),
            offset: 0,
        };

        send_cached_html_body_chunks(&mut body, &body_tx)?;
        drop(body_tx);

        let mut joined = String::new();
        while let Some(chunk) = body_rx.recv().await {
            joined.push_str(&chunk);
        }
        assert_eq!(joined, "a\u{20ac}b");
        Ok(())
    }
}
