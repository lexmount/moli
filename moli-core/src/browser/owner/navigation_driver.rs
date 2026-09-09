use tokio::sync::{oneshot, watch};
use url::Url;

use super::{Browser, BrowserContextHandle, BrowserLocalSender};
use crate::browser::navigation_decision::ResponseInterceptionStage;
use crate::browser::{
    CapturedBody, CapturedBodyWriter, NavigationDecision, NavigationDecisionStage,
    NavigationFailureReason, NavigationId, NavigationRequest, NavigationRequestLoadPolicy,
    NavigationResponseSnapshot, WebContentsHandle,
    web_contents::{
        DocumentBodySource, DocumentNavigationDestination, InheritedDocumentPolicy,
        NavigationInterceptionPermit, NavigationRequestInterception, PausedDocumentTransfer,
    },
};

struct PendingDecision {
    result: oneshot::Receiver<NavigationDecision>,
    provider: watch::Receiver<()>,
    permit: NavigationInterceptionPermit,
}
use crate::runtime::{
    CommittedDocumentResourceSource, ExternalRawDocumentBodyStream, PageVmInitStage,
    RendererReplyBoundary,
};

/// Observation of one admitted Browser task. Dropping it does not cancel the
/// request; cancellation is an operation on the original WebContents/navigation.
pub struct BrowserNavigationWaiter {
    request: NavigationRequest,
    completion: oneshot::Receiver<Result<BrowserNavigationOutcome, String>>,
}

pub enum BrowserNavigationOutcome {
    Document(Box<crate::browser::web_contents::DocumentCommitSnapshot>),
    Download { url: Url },
}

impl BrowserNavigationWaiter {
    pub fn request(&self) -> NavigationRequest {
        self.request
    }

    pub async fn wait(self) -> Result<BrowserNavigationOutcome, String> {
        self.completion
            .await
            .map_err(|_| "Browser navigation task stopped".to_owned())?
    }
}

impl BrowserContextHandle {
    pub fn navigate_document(
        &self,
        contents: WebContentsHandle,
        request: NavigationRequestInterception,
    ) -> Result<BrowserNavigationWaiter, String> {
        let context = self.id;
        self.browser.execute(move |browser| {
            browser.context(context)?.web_contents(contents)?;
            let policy = browser.native_navigation_policy(contents)?;
            browser.start_native_document_navigation(
                contents,
                request,
                policy,
                std::sync::Weak::new(),
            )
        })?
    }

    /// Admit the requested URL once against the original WebContents' initial
    /// Document. The Browser owns all subsequent loading and decisions; an
    /// observer is optional and cannot replace this request with a Target URL.
    pub fn navigate_initial_document(
        &self,
        contents: WebContentsHandle,
        url: Url,
    ) -> Result<Option<NavigationId>, String> {
        let context = self.id;
        self.browser.execute(move |browser| {
            let page = browser.context(context)?.web_contents(contents)?;
            if page.navigation().is_on_initial_empty_document() != Some(true)
                || page.navigation().has_pending_document_navigation()
                || page.navigation().initial_empty_document_url_if_current() == Some(url.as_str())
            {
                return Ok(None);
            }
            let policy = browser.native_navigation_policy(contents)?;
            browser
                .context_mut(context)?
                .web_contents_mut(contents)?
                .mark_next_navigation_history_replace_initial_empty_document();
            browser
                .start_native_document_navigation(
                    contents,
                    native_url_request(url),
                    policy,
                    std::sync::Weak::new(),
                )
                .map(|waiter| Some(waiter.request.navigation))
        })?
    }
}

impl Browser {
    pub(super) fn start_popup_navigation(
        &mut self,
        contents: WebContentsHandle,
        opening: &std::sync::Arc<crate::page::RendererPopupOpening>,
        created: bool,
    ) -> Result<Option<NavigationId>, String> {
        let url = Url::parse(opening.url()).map_err(|error| error.to_string())?;
        // Empty auxiliary documents and javascript: evaluation belong to the
        // initial document, not to a second fetched Document candidate.
        if (created && moli_url::is_about_blank(&url)) || url.scheme() == "javascript" {
            if created {
                let policy = self.native_navigation_policy(contents)?;
                self.start_initial_document(contents, policy)?;
            }
            return Ok(None);
        }
        let policy = self.native_navigation_policy(contents)?;
        self.start_native_document_navigation(
            contents,
            native_url_request(url),
            policy,
            std::sync::Arc::downgrade(opening),
        )
        .map(|waiter| Some(waiter.request.navigation))
    }

    fn start_native_document_navigation(
        &mut self,
        contents: WebContentsHandle,
        parameters: NavigationRequestInterception,
        policy: InheritedDocumentPolicy,
        opening: std::sync::Weak<crate::page::RendererPopupOpening>,
    ) -> Result<BrowserNavigationWaiter, String> {
        let navigation = self.start_navigation(contents)?;
        let initial = (|| {
            let context = self.context_mut(contents.context())?;
            context
                .web_contents_mut(contents)?
                .navigation_mut()
                .set_native_initial_document(navigation, true)?;
            // An auxiliary Window needs its initial Document even while its
            // requested URL is paused. Ordinary cross-document admission may
            // precede the first Document; it must not insert a separate blank
            // navigation/history entry as a side effect of loading the URL.
            if opening.strong_count() != 0 {
                self.start_initial_document(contents, policy)
            } else {
                Ok(None)
            }
        })();
        let initial = match initial {
            Ok(initial) => initial,
            Err(error) => {
                self.cancel_navigation(contents, navigation, NavigationFailureReason::Canceled)?;
                return Err(error);
            }
        };
        let decision = self.begin_navigation_decision(
            contents,
            navigation,
            parameters.decision_stage(opening),
        )?;
        let request = self
            .pending_navigation(contents)?
            .expect("admitted native navigation");
        let (completed, completion) = oneshot::channel();
        let owner = self.local_sender.clone();
        let failed_url = parameters.requested_url.clone();
        tokio::task::spawn_local(async move {
            let mut completed = Some(completed);
            if let Err(error) = navigate(
                &owner,
                contents,
                navigation,
                initial,
                parameters,
                decision,
                &mut completed,
            )
            .await
            {
                tracing::debug!(%error, "native document navigation did not commit");
                let failure = error.clone();
                let _ = on_owner(&owner, move |browser| {
                    if browser.pending_navigation(contents)? != Some(request) {
                        return Ok(());
                    }
                    let previous = browser
                        .context(contents.context())?
                        .web_contents(contents)?
                        .navigation()
                        .response_snapshots()
                        .into_iter()
                        .find(|response| response.request == request);
                    if previous.is_some() {
                        browser.complete_native_response(request, Err(failure.clone()))?;
                    } else {
                        browser.record_native_response(NavigationResponseSnapshot {
                            request,
                            response: Err(crate::browser::NavigationFetchFailure {
                                error: std::sync::Arc::new(crate::browser::NavigationError {
                                    unreachable_url: failed_url,
                                    error_text: failure.clone(),
                                }),
                                request: None,
                            }),
                            observations: Default::default(),
                            body: Some(Err(failure)),
                        })?;
                    }
                    browser.cancel_navigation(
                        contents,
                        navigation,
                        NavigationFailureReason::Canceled,
                    )?;
                    Ok(())
                })
                .await;
                if let Some(completed) = completed.take() {
                    let _ = completed.send(Err(error));
                }
            }
        });
        Ok(BrowserNavigationWaiter {
            request,
            completion,
        })
    }

    fn begin_navigation_decision(
        &mut self,
        contents: WebContentsHandle,
        navigation: NavigationId,
        stage: NavigationDecisionStage,
    ) -> Result<Option<PendingDecision>, String> {
        let Some(provider) = self
            .document_decision_provider
            .as_ref()
            .filter(|provider| provider.has_changed().is_ok())
            .cloned()
        else {
            return Ok(None);
        };
        self.install_navigation_decision(contents, provider, move |page| {
            page.navigation_mut()
                .pause_navigation_decision(contents.id(), navigation, stage)
        })
        .map(Some)
    }

    fn install_navigation_decision(
        &mut self,
        contents: WebContentsHandle,
        provider: watch::Receiver<()>,
        admit: impl FnOnce(
            &mut crate::browser::web_contents::WebContents,
        ) -> Result<oneshot::Receiver<NavigationDecision>, String>,
    ) -> Result<PendingDecision, String> {
        let page = self
            .context_mut(contents.context())?
            .web_contents_mut(contents)?;
        let result = admit(page)?;
        let permit = page
            .navigation()
            .navigation_decision()
            .expect("installed driver decision")
            .permit;
        let request = self
            .pending_navigation(contents)?
            .expect("pending driver decision");
        self.events
            .publish(crate::browser::BrowserEvent::NavigationAwaitingDecision(
                request,
            ));
        Ok(PendingDecision {
            result,
            provider,
            permit,
        })
    }

    fn native_navigation_policy(
        &self,
        contents: WebContentsHandle,
    ) -> Result<crate::browser::web_contents::InheritedDocumentPolicy, String> {
        let context = self.context(contents.context())?;
        let config = context
            .web_contents_navigation_fetch_config(contents)
            .cloned()
            .ok_or("navigation WebContents engine unavailable")?;
        Ok(context.inherited_document_policy(config, &self.permission_defaults, &[], None))
    }

    fn navigation_destination(
        &self,
        contents: WebContentsHandle,
        url: Url,
    ) -> Result<DocumentNavigationDestination, String> {
        let context = self.context(contents.context())?;
        let inherited = if moli_url::is_about_blank(&url) {
            context
                .document_handle(contents)?
                .and_then(|document| context.document(document).ok())
                .and_then(|host| host.commit.as_ref())
                .and_then(|commit| commit.info.as_ref())
                .map(|info| {
                    (
                        info.security_origin.clone(),
                        info.secure_context_type.clone(),
                    )
                })
                .or_else(|| {
                    context
                        .web_contents_initial_document_state(contents)
                        .ok()
                        .flatten()
                        .and_then(|initial| {
                            initial.creator().map(|creator| {
                                (
                                    creator.security_origin().to_owned(),
                                    creator.secure_context_type().to_owned(),
                                )
                            })
                        })
                })
        } else {
            None
        };
        let (security_origin, secure_context_type) = inherited.unwrap_or_else(|| {
            (
                moli_url::origin_ascii_serialization(&url),
                if moli_url::is_potentially_trustworthy_url(&url) {
                    "Secure"
                } else {
                    "InsecureScheme"
                }
                .into(),
            )
        });
        Ok(DocumentNavigationDestination::Document {
            url,
            security_origin,
            secure_context_type,
        })
    }
}

fn native_url_request(url: Url) -> NavigationRequestInterception {
    NavigationRequestInterception::new(
        url,
        "GET".into(),
        None,
        Vec::new(),
        NavigationRequestLoadPolicy::DocumentInitiated,
    )
}

async fn await_decision(
    owner: &BrowserLocalSender,
    contents: WebContentsHandle,
    pending: Option<PendingDecision>,
) -> Result<NavigationDecision, String> {
    let Some(PendingDecision {
        mut result,
        mut provider,
        permit,
    }) = pending
    else {
        return Ok(NavigationDecision::Continue);
    };
    let decision = tokio::select! {
        decision = &mut result => decision.map_err(|_| "native navigation decision canceled".to_owned())?,
        _ = provider.changed() => {
            on_owner(owner, move |browser| {
                let controller = browser.context_mut(contents.context())?.web_contents_mut(contents)?.navigation_mut();
                controller.resolve_navigation_decision(permit, NavigationDecision::Continue);
                Ok(())
            }).await?;
            result.await.map_err(|_| "native navigation decision canceled".to_owned())?
        }
    };
    on_owner(owner, move |browser| {
        let controller = browser
            .context_mut(contents.context())?
            .web_contents_mut(contents)?
            .navigation_mut();
        if !controller.finish_navigation_decision(permit) {
            return Err("native navigation decision canceled".into());
        }
        Ok(())
    })
    .await?;
    match decision {
        NavigationDecision::Cancel => Err("native navigation canceled by decision provider".into()),
        NavigationDecision::Fail { error_text } => Err(error_text),
        decision => Ok(decision),
    }
}

// These participants run on the Browser's LocalSet. Calling the external
// blocking BrowserHandle here would deadlock its own owner thread; only the
// move-owned result returns to an owner turn through this private mailbox.
pub(super) async fn on_owner<R: 'static>(
    sender: &BrowserLocalSender,
    operation: impl FnOnce(&mut Browser) -> Result<R, String> + 'static,
) -> Result<R, String> {
    let (reply, completed) = oneshot::channel();
    sender
        .send(Box::new(move |browser| {
            let _ = reply.send(operation(browser));
        }))
        .map_err(|_| "Browser owner stopped".to_owned())?;
    completed
        .await
        .map_err(|_| "Browser owner stopped".to_owned())?
}

async fn navigate(
    owner: &BrowserLocalSender,
    contents: WebContentsHandle,
    navigation: NavigationId,
    initial: Option<super::BrowserInitialDocumentWaiter>,
    parameters: NavigationRequestInterception,
    decision: Option<PendingDecision>,
    completed: &mut Option<oneshot::Sender<Result<BrowserNavigationOutcome, String>>>,
) -> Result<(), String> {
    if let Some(initial) = initial {
        initial.wait().await?;
    }
    let decision = await_decision(owner, contents, decision).await?;
    let NavigationRequestInterception {
        mut requested_url,
        mut method,
        body: mut request_body,
        headers: mut request_headers,
        policy: request_load_policy,
    } = parameters;
    let synthetic = match decision {
        NavigationDecision::Request {
            url,
            method: replacement,
            body,
            headers,
        } => {
            requested_url = url;
            method = replacement;
            request_body = body;
            request_headers = headers;
            None
        }
        NavigationDecision::Fulfill {
            status,
            headers,
            body,
        } => Some((status, headers, body)),
        NavigationDecision::Continue => None,
        NavigationDecision::Response { .. } | NavigationDecision::Authenticate { .. } => {
            return Err("response supplied for a request decision".into());
        }
        NavigationDecision::Cancel | NavigationDecision::Fail { .. } => {
            unreachable!("canceled decision returned as error")
        }
    };
    let mut load = on_owner(owner, move |browser| {
        browser
            .context_mut(contents.context())?
            .web_contents_mut(contents)?
            .navigation_mut()
            .set_native_initial_document(navigation, false)?;
        let policy = browser.native_navigation_policy(contents)?;
        browser
            .context_mut(contents.context())?
            .start_navigation_load(contents, navigation, request_load_policy, policy)
    })
    .await?;
    let request = NavigationRequest {
        web_contents: contents,
        navigation,
        document: load.document_id(),
    };
    let synthetic = synthetic.or_else(|| {
        moli_url::is_about_blank(&requested_url).then(|| {
            (
                200,
                vec![("Content-Type".into(), "text/html".into())],
                b"<!doctype html><html><head></head><body></body></html>".to_vec(),
            )
        })
    });
    let mut store_response_headers = synthetic.is_some();
    let (mut response, body_source, mut source, mut service_worker_client, observations) =
        if let Some((status, headers, body)) = synthetic {
            let head = moli_fetch::ResponseHead {
                final_url: requested_url.clone(),
                status,
                headers,
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                negotiated_http_version: None,
            };
            (
                head.clone(),
                DocumentBodySource::BufferedRaw {
                    requested_url: requested_url.clone(),
                    request_method: method.clone(),
                    request_headers: request_headers.clone(),
                    response: moli_fetch::RawResponse::from_head_and_body(head, body),
                    network_observation_journal: Default::default(),
                },
                CommittedDocumentResourceSource::Synthetic,
                None,
                Default::default(),
            )
        } else {
            let mut auth: Option<crate::page::SubresourceAuthCredentials> = None;
            let mut prior = moli_fetch::NetworkObservationJournal::default();
            loop {
                let (head, mut body, resource_source, reserved_client, observations) =
                    if let Some(credentials) = auth
                        .as_ref()
                        .filter(|auth| auth.scheme == crate::page::SubresourceAuthScheme::Digest)
                    {
                        // Keep Digest's existing buffered transport: libcurl
                        // consumes its intermediate challenges before returning.
                        let fetched = match load
                            .fetch_intercepted_auth_response(
                                &method,
                                requested_url.as_str(),
                                request_body.clone(),
                                request_headers.clone(),
                                credentials.clone(),
                            )
                            .await
                        {
                            Ok(fetched) => fetched,
                            Err(error) => {
                                return commit_fetch_error(
                                    owner,
                                    request,
                                    &mut load,
                                    requested_url,
                                    error,
                                    completed,
                                )
                                .await;
                            }
                        };
                        let (response, observations) =
                            fetched.into_parts_with_observation_journal();
                        (
                            response.head(),
                            DocumentBodySource::BufferedRaw {
                                requested_url: requested_url.clone(),
                                request_method: method.clone(),
                                request_headers: request_headers.clone(),
                                response,
                                network_observation_journal: observations.clone(),
                            },
                            CommittedDocumentResourceSource::Synthetic,
                            None,
                            observations,
                        )
                    } else {
                        let fetched = match load
                            .fetch_navigation_with_auth(
                                &method,
                                requested_url.as_str(),
                                request_body.clone(),
                                request_headers.clone(),
                                auth.clone(),
                            )
                            .await
                        {
                            Ok(fetched) => fetched,
                            Err(error) => {
                                return commit_fetch_error(
                                    owner,
                                    request,
                                    &mut load,
                                    requested_url,
                                    error,
                                    completed,
                                )
                                .await;
                            }
                        };
                        let (response, observations) =
                            fetched.fetch_result.into_parts_with_observation_journal();
                        (
                            response.head(),
                            DocumentBodySource::StreamingRaw {
                                requested_url: requested_url.clone(),
                                request_method: method.clone(),
                                request_headers: request_headers.clone(),
                                response,
                                network_observation_journal: observations.clone(),
                                prepared_document: None,
                            },
                            CommittedDocumentResourceSource::Navigation(Box::new(
                                fetched.document_fetch_context_seed,
                            )),
                            fetched.reserved_service_worker_client,
                            observations,
                        )
                    };
                prior.append(observations);
                match &mut body {
                    DocumentBodySource::BufferedRaw {
                        network_observation_journal,
                        ..
                    }
                    | DocumentBodySource::StreamingRaw {
                        network_observation_journal,
                        ..
                    }
                    | DocumentBodySource::CapturedRaw {
                        network_observation_journal,
                        ..
                    } => {
                        *network_observation_journal = prior.clone();
                    }
                }
                if matches!(head.status, 401 | 407) {
                    let decision = decide_transfer(
                        owner,
                        contents,
                        navigation,
                        ResponseInterceptionStage::Auth,
                        PausedDocumentTransfer::pending(request_load_policy, body),
                    )
                    .await?;
                    match decision {
                        NavigationDecision::Authenticate {
                            credentials,
                            response,
                        } => {
                            let (_, challenge) = response
                                .finish_body_stream_async()
                                .await
                                .map_err(|(_, error)| error)?;
                            if let DocumentBodySource::StreamingRaw { mut response, .. } = challenge
                            {
                                while response.next_chunk().await.is_some() {}
                                response.finish().await.map_err(|error| error.to_string())?;
                            }
                            auth = Some(credentials);
                            continue;
                        }
                        NavigationDecision::Response { transfer, .. } => {
                            body = transfer
                                .finish_body_stream_async()
                                .await
                                .map_err(|(_, error)| error)?
                                .1;
                        }
                        _ => return Err("invalid native authentication decision".into()),
                    }
                }
                break (head, body, resource_source, reserved_client, prior);
            }
        };
    let transfer = PausedDocumentTransfer::pending(request_load_policy, body_source);
    let decision = decide_transfer(
        owner,
        contents,
        navigation,
        ResponseInterceptionStage::Response,
        transfer,
    )
    .await?;
    let body_source = match decision {
        NavigationDecision::Response {
            transfer,
            status,
            headers,
        } => {
            if let Some(status) = status {
                response.status = status;
            }
            if !headers.is_empty() {
                response.headers = headers;
                store_response_headers = true;
            }
            transfer
                .finish_body_stream_async()
                .await
                .map_err(|(_, error)| error)?
                .1
        }
        NavigationDecision::Fulfill {
            status,
            headers,
            body,
        } => {
            response.status = status;
            response.headers = headers;
            store_response_headers = true;
            source = CommittedDocumentResourceSource::Synthetic;
            service_worker_client = None;
            DocumentBodySource::BufferedRaw {
                requested_url: requested_url.clone(),
                request_method: method,
                request_headers,
                response: moli_fetch::RawResponse::from_head_and_body(response.clone(), body),
                network_observation_journal: observations.clone(),
            }
        }
        _ => return Err("invalid native response decision".into()),
    };
    if store_response_headers {
        let url = response.final_url.clone();
        let headers = response.headers.clone();
        response.cookie_set_reports = on_owner(owner, move |browser| {
            if browser.pending_navigation(contents)? != Some(request) {
                return Err("stale native navigation response".into());
            }
            Ok(browser
                .context(contents.context())?
                .page_storage_handles(Some(contents))?
                .cookie_store
                .lock()
                .store_response_headers_with_reports(&url, &headers))
        })
        .await?;
    }
    let mut snapshot = NavigationResponseSnapshot {
        request,
        response: Ok(response.clone()),
        observations: observations.clone(),
        body: None,
    };
    if moli_web_mime::response_headers_indicate_attachment_download(&response.headers) {
        on_owner(owner, move |browser| {
            browser.record_native_response(snapshot)
        })
        .await?;
        let body = match body_source {
            DocumentBodySource::StreamingRaw { response, .. } => {
                crate::browser::DownloadBody::Streaming(Box::new(response))
            }
            DocumentBodySource::BufferedRaw { response, .. } => {
                crate::browser::DownloadBody::Buffered(
                    response
                        .into_body()
                        .1
                        .try_into_materialized_bytes()
                        .map_err(|_| {
                            "buffered native download has no materialized bytes".to_owned()
                        })?,
                )
            }
            DocumentBodySource::CapturedRaw { body, .. } => {
                crate::browser::DownloadBody::Captured(body)
            }
        };
        let renderer = load.renderer_page();
        let url = response.final_url.clone();
        on_owner(owner, move |browser| {
            browser.download_navigation_response(request, renderer, response, body)
        })
        .await?;
        if let Some(completed) = completed.take() {
            let _ = completed.send(Ok(BrowserNavigationOutcome::Download { url }));
        }
        return Ok(());
    }
    let PreparedNavigationBody::Document { body, capture } =
        prepare_response_body(body_source, &response).await?
    else {
        let error = std::sync::Arc::new(crate::browser::NavigationError {
            unreachable_url: response.final_url.clone(),
            error_text: "net::ERR_HTTP_RESPONSE_CODE_FAILURE".into(),
        });
        snapshot.body = Some(Err(error.error_text.clone()));
        on_owner(owner, move |browser| {
            browser.record_native_response(snapshot)
        })
        .await?;
        let html = http_error_page_html(&error.unreachable_url, response.status);
        return commit_error_document(owner, request, &mut load, error, html, completed).await;
    };
    on_owner(owner, move |browser| {
        browser.record_native_response(snapshot)
    })
    .await?;
    let final_url = response.final_url.clone();
    let destination = on_owner(owner, move |browser| {
        browser.navigation_destination(contents, final_url)
    })
    .await?;
    let snapshot = commit_response_document(
        owner,
        request,
        &mut load,
        requested_url,
        response,
        body,
        source,
        service_worker_client,
        destination,
    )
    .await?;
    if let Some(completed) = completed.take() {
        let _ = completed.send(Ok(BrowserNavigationOutcome::Document(Box::new(snapshot))));
    }
    let body = capture.finish().await;
    on_owner(owner, move |browser| {
        browser.complete_native_response(request, body)
    })
    .await?;
    Ok(())
}

async fn commit_response_document(
    owner: &BrowserLocalSender,
    request: NavigationRequest,
    load: &mut crate::browser::web_contents::AdmittedNavigationLoad,
    requested_url: Url,
    response: moli_fetch::ResponseHead,
    body: ExternalRawDocumentBodyStream,
    source: CommittedDocumentResourceSource,
    service_worker_client: Option<crate::runtime::RendererReservedServiceWorkerClient>,
    destination: DocumentNavigationDestination,
) -> Result<crate::browser::web_contents::DocumentCommitSnapshot, String> {
    let contents = request.web_contents;
    let navigation = request.navigation;
    let preparation = load.prepare_document_response_async(
        requested_url,
        response.final_url.clone(),
        response.redirected,
        response.redirect_chain.len(),
        response.status,
        response.headers.clone(),
        body,
        PageVmInitStage::DomContentLoaded,
        RendererReplyBoundary::DocumentCommit,
        source,
        service_worker_client,
    );
    let prepared = preparation.await.map_err(|error| error.to_string())?;
    let inspection = prepared.inspection_configuration_endpoint();
    let renderer = load.renderer_page();
    let decision = on_owner(owner, move |browser| {
        browser.begin_navigation_decision(
            contents,
            navigation,
            NavigationDecisionStage::PreparedDocument {
                renderer,
                inspection,
            },
        )
    })
    .await?;
    await_decision(owner, contents, decision).await?;
    let materialization = on_owner(owner, move |browser| {
        let policy = browser.native_navigation_policy(contents)?;
        browser
            .context_mut(contents.context())?
            .start_document_materialization(contents, navigation, prepared, destination, policy)
    })
    .await?;
    let built = materialization
        .materialize()
        .await
        .map_err(|error| error.to_string())?;
    let commit = on_owner(owner, move |browser| {
        browser.commit_navigation(contents, built.page)
    })
    .await?;
    // The physical commit is final, but the replacement parser must not
    // outrun cancellation and terminal publication from the outgoing Document.
    commit.retirement.close().await;
    if let Some(continuation) = commit.post_response_continuation {
        continuation.release();
    }
    Ok(commit.snapshot)
}

enum TransferAdmission {
    Unobserved(Box<PausedDocumentTransfer>),
    Paused(PendingDecision),
}

async fn commit_fetch_error(
    owner: &BrowserLocalSender,
    request: NavigationRequest,
    load: &mut crate::browser::web_contents::AdmittedNavigationLoad,
    requested_url: Url,
    failure: anyhow::Error,
    completed: &mut Option<oneshot::Sender<Result<BrowserNavigationOutcome, String>>>,
) -> Result<(), String> {
    let Some(failure) = failure.downcast_ref::<moli_fetch::NetworkFetchFailureContext>() else {
        return Err(failure.to_string());
    };
    let error = std::sync::Arc::new(crate::browser::NavigationError {
        unreachable_url: failure
            .request_context()
            .map(|request| request.current_url().clone())
            .unwrap_or(requested_url),
        error_text: failure.network_error_text().to_owned(),
    });
    let snapshot = NavigationResponseSnapshot {
        request,
        response: Err(crate::browser::NavigationFetchFailure {
            error: error.clone(),
            request: failure.request_context().cloned(),
        }),
        observations: failure.observation_journal().clone(),
        body: None,
    };
    on_owner(owner, move |browser| {
        browser.record_native_response(snapshot)
    })
    .await?;
    let html = network_error_page_html(&error.unreachable_url, &error.error_text);
    commit_error_document(owner, request, load, error, html, completed).await
}

async fn commit_error_document(
    owner: &BrowserLocalSender,
    request: NavigationRequest,
    load: &mut crate::browser::web_contents::AdmittedNavigationLoad,
    error: std::sync::Arc<crate::browser::NavigationError>,
    html: String,
    completed: &mut Option<oneshot::Sender<Result<BrowserNavigationOutcome, String>>>,
) -> Result<(), String> {
    let response = moli_fetch::ResponseHead {
        final_url: Url::parse("chrome-error://chromewebdata/").expect("Browser error URL"),
        status: 200,
        headers: vec![("content-type".into(), "text/html; charset=utf-8".into())],
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        redirected: false,
        redirect_chain: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    };
    let snapshot = commit_response_document(
        owner,
        request,
        load,
        error.unreachable_url.clone(),
        response,
        ExternalRawDocumentBodyStream::from_bytes(html.into_bytes()),
        CommittedDocumentResourceSource::Synthetic,
        None,
        DocumentNavigationDestination::Error(error.clone()),
    )
    .await?;
    if let Some(completed) = completed.take() {
        let _ = completed.send(Ok(BrowserNavigationOutcome::Document(Box::new(snapshot))));
    }
    on_owner(owner, move |browser| {
        browser.complete_native_response(request, Err(error.error_text.clone()))
    })
    .await
}

fn escape_error_page_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn network_error_page_html(url: &Url, error_text: &str) -> String {
    let title = escape_error_page_html(url.host_str().unwrap_or(url.as_str()));
    let url = escape_error_page_html(url.as_str());
    let error_text = escape_error_page_html(error_text);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head><body><main><h1>This site can’t be reached</h1><p>The webpage at <strong>{url}</strong> could not be loaded.</p><div>{error_text}</div></main></body></html>"
    )
}

fn http_error_page_html(url: &Url, status: u16) -> String {
    let title = escape_error_page_html(url.host_str().unwrap_or(url.as_str()));
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head><body><main><h1>This page isn't working</h1><p>If the problem continues, contact the site owner.</p><p>HTTP ERROR {status}</p></main></body></html>"
    )
}

async fn decide_transfer(
    owner: &BrowserLocalSender,
    contents: WebContentsHandle,
    navigation: NavigationId,
    stage: ResponseInterceptionStage,
    transfer: PausedDocumentTransfer,
) -> Result<NavigationDecision, String> {
    let admission = on_owner(owner, move |browser| {
        let Some(provider) = browser
            .document_decision_provider
            .as_ref()
            .filter(|provider| provider.has_changed().is_ok())
            .cloned()
        else {
            return Ok(TransferAdmission::Unobserved(Box::new(transfer)));
        };
        let pending = browser.install_navigation_decision(contents, provider, move |page| {
            page.navigation_mut().pause_response_decision(
                contents.id(),
                navigation,
                stage,
                transfer,
            )
        })?;
        Ok(TransferAdmission::Paused(pending))
    })
    .await?;
    match admission {
        TransferAdmission::Unobserved(transfer) => Ok(NavigationDecision::Response {
            transfer,
            status: None,
            headers: Vec::new(),
        }),
        TransferAdmission::Paused(pending) => await_decision(owner, contents, Some(pending)).await,
    }
}

struct NativeBodyCapture(Option<tokio::task::JoinHandle<Result<CapturedBody, String>>>);

impl NativeBodyCapture {
    async fn finish(mut self) -> Result<CapturedBody, String> {
        self.0
            .take()
            .expect("native body capture")
            .await
            .map_err(|error| error.to_string())?
    }
}

impl Drop for NativeBodyCapture {
    fn drop(&mut self) {
        if let Some(pump) = &self.0 {
            pump.abort();
        }
    }
}

enum PreparedNavigationBody {
    Document {
        body: ExternalRawDocumentBodyStream,
        capture: NativeBodyCapture,
    },
    EmptyErrorResponse,
}

async fn prepare_response_body(
    source: DocumentBodySource,
    head: &moli_fetch::ResponseHead,
) -> Result<PreparedNavigationBody, String> {
    let error_status = (400..600).contains(&head.status);
    let xml = moli_web_mime::response_document_content_type(&head.headers)
        .is_some_and(|mime| moli_web_mime::is_dom_parser_xml_mime(&mime));
    let captured = match source {
        DocumentBodySource::StreamingRaw { mut response, .. } => {
            let mut first = None;
            if error_status {
                while let Some(chunk) = response.next_chunk().await {
                    if !chunk.is_empty() {
                        first = Some(chunk);
                        break;
                    }
                }
                if first.is_none() {
                    response.finish().await.map_err(|error| error.to_string())?;
                    return Ok(PreparedNavigationBody::EmptyErrorResponse);
                }
            }
            if !xml {
                let (body, capture) = stream_raw_response(response, first);
                return Ok(PreparedNavigationBody::Document { body, capture });
            }
            // XML preparation has a single completed input, not the HTML
            // parser's early commit boundary. Keep the capture bounded/spooled.
            let mut writer = CapturedBodyWriter::default();
            if let Some(first) = first {
                writer.append(&first).map_err(|error| error.to_string())?;
            }
            while let Some(chunk) = response.next_chunk().await {
                writer.append(&chunk).map_err(|error| error.to_string())?;
            }
            response.finish().await.map_err(|error| error.to_string())?;
            writer.finish().map_err(|error| error.to_string())?
        }
        DocumentBodySource::BufferedRaw { response, .. } => {
            let bytes = response
                .into_body()
                .1
                .try_into_materialized_bytes()
                .map_err(|_| "buffered native response has no materialized bytes".to_owned())?;
            CapturedBody::from_bytes_spooled(bytes)
        }
        DocumentBodySource::CapturedRaw { body, .. } => body,
    };
    if error_status && captured.is_empty() {
        return Ok(PreparedNavigationBody::EmptyErrorResponse);
    }
    let (body, capture) = replay_captured_body(captured)?;
    Ok(PreparedNavigationBody::Document { body, capture })
}

fn stream_raw_response(
    mut response: moli_fetch::StreamingRawResponse,
    first: Option<Vec<u8>>,
) -> (ExternalRawDocumentBodyStream, NativeBodyCapture) {
    let (completion, completed) = oneshot::channel();
    let (sender, body) = ExternalRawDocumentBodyStream::channel(completed);
    let pump = tokio::task::spawn_local(async move {
        let result = async {
            let mut writer = CapturedBodyWriter::default();
            let mut sender = Some(sender);
            let mut first = first;
            loop {
                let chunk = match first.take() {
                    Some(chunk) => Some(chunk),
                    None => response.next_chunk().await,
                };
                let Some(chunk) = chunk else {
                    break;
                };
                writer.append(&chunk).map_err(|error| error.to_string())?;
                if let Some(output) = &sender
                    && output.send(chunk).await.is_err()
                {
                    sender = None;
                }
            }
            response.finish().await.map_err(|error| error.to_string())?;
            writer.finish().map_err(|error| error.to_string())
        }
        .await;
        let _ = completion.send(
            result
                .as_ref()
                .map(|_| ())
                .map_err(|error: &String| anyhow::anyhow!(error.clone())),
        );
        result
    });
    (body, NativeBodyCapture(Some(pump)))
}

fn replay_captured_body(
    captured: CapturedBody,
) -> Result<(ExternalRawDocumentBodyStream, NativeBodyCapture), String> {
    let mut reader = captured
        .chunk_reader(64 * 1024)
        .map_err(|error| error.to_string())?;
    let (completion, completed) = oneshot::channel();
    let (sender, body) = ExternalRawDocumentBodyStream::channel(completed);
    let pump = tokio::task::spawn_blocking(move || {
        let result = (|| {
            while let Some(chunk) = reader.next_chunk().map_err(|error| error.to_string())? {
                if sender.blocking_send(chunk).is_err() {
                    break;
                }
            }
            Ok(captured)
        })();
        let _ = completion.send(
            result
                .as_ref()
                .map(|_| ())
                .map_err(|error: &String| anyhow::anyhow!(error.clone())),
        );
        result
    });
    Ok((body, NativeBodyCapture(Some(pump))))
}
