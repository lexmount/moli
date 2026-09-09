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
        InitialDocumentAdmission, NavigationInterceptionPermit, PausedDocumentTransfer,
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

impl BrowserContextHandle {
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
                .start_native_document_navigation(contents, url, policy, std::sync::Weak::new())
                .map(Some)
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
            return Ok(None);
        }
        let policy = self.native_navigation_policy(contents)?;
        self.start_native_document_navigation(
            contents,
            url,
            policy,
            std::sync::Arc::downgrade(opening),
        )
        .map(Some)
    }

    fn start_native_document_navigation(
        &mut self,
        contents: WebContentsHandle,
        url: Url,
        policy: InheritedDocumentPolicy,
        opening: std::sync::Weak<crate::page::RendererPopupOpening>,
    ) -> Result<NavigationId, String> {
        let navigation = self.start_navigation(contents)?;
        let initial = (|| {
            let context = self.context_mut(contents.context())?;
            context
                .web_contents_mut(contents)?
                .navigation_mut()
                .set_native_initial_document(navigation, true)?;
            let initial = context.start_initial_document(contents, policy)?;
            let decision = if let InitialDocumentAdmission::Build(build) = &initial {
                self.begin_navigation_decision(
                    contents,
                    navigation,
                    NavigationDecisionStage::InitialDocumentReserved { key: build.key() },
                )?
            } else {
                None
            };
            Ok::<_, String>((initial, decision))
        })();
        let (initial, decision) = match initial {
            Ok(initial) => initial,
            Err(error) => {
                self.cancel_navigation(contents, navigation, NavigationFailureReason::Canceled)?;
                return Err(error);
            }
        };
        let owner = self.local_sender.clone();
        tokio::task::spawn_local(async move {
            if let Err(error) = navigate(
                &owner, contents, navigation, initial, decision, url, opening,
            )
            .await
            {
                tracing::debug!(%error, "native document navigation did not commit");
                let _ = owner.send(Box::new(move |browser| {
                    let _ = browser.cancel_navigation(
                        contents,
                        navigation,
                        NavigationFailureReason::Canceled,
                    );
                }));
            }
        });
        Ok(navigation)
    }

    fn begin_navigation_decision(
        &mut self,
        contents: WebContentsHandle,
        navigation: NavigationId,
        stage: NavigationDecisionStage,
    ) -> Result<Option<PendingDecision>, String> {
        let Some(provider) = self
            .navigation_decision_provider
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
        decision => Ok(decision),
    }
}

// These participants run on the Browser's LocalSet. Calling the external
// blocking BrowserHandle here would deadlock its own owner thread; only the
// move-owned result returns to an owner turn through this private mailbox.
async fn on_owner<R: 'static>(
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
    initial: InitialDocumentAdmission,
    decision: Option<PendingDecision>,
    mut requested_url: Url,
    opening: std::sync::Weak<crate::page::RendererPopupOpening>,
) -> Result<(), String> {
    await_decision(owner, contents, decision).await?;
    match initial {
        InitialDocumentAdmission::Present => {}
        InitialDocumentAdmission::Join(waiter) => waiter.wait().await?,
        InitialDocumentAdmission::Build(mut build) => {
            build
                .start_preparation()
                .map_err(|error| error.to_string())?;
            let key = build.key();
            let inspection = build.inspection_endpoint();
            let decision = on_owner(owner, move |browser| {
                browser.begin_navigation_decision(
                    contents,
                    navigation,
                    NavigationDecisionStage::InitialDocument { key, inspection },
                )
            })
            .await?;
            await_decision(owner, contents, decision).await?;
            let built = build
                .materialize()
                .await
                .map_err(|error| error.to_string())?;
            on_owner(owner, move |browser| {
                let committed = match browser.context_mut(contents.context()) {
                    Ok(context) => context.commit_initial_document(built),
                    Err(_) => Err(Box::new(built)),
                };
                match committed {
                    Ok(committed) => {
                        browser.publish_initial_document_commit(contents, committed);
                        Ok(())
                    }
                    Err(stale) => {
                        tokio::task::spawn_local(stale.retire());
                        Err("native initial document superseded".into())
                    }
                }
            })
            .await?;
        }
    }
    let request_url = requested_url.clone();
    let decision = on_owner(owner, move |browser| {
        browser.begin_navigation_decision(
            contents,
            navigation,
            NavigationDecisionStage::Request {
                url: request_url,
                method: "GET".into(),
                headers: Vec::new(),
                opening,
            },
        )
    })
    .await?;
    let decision = await_decision(owner, contents, decision).await?;
    let mut method = "GET".to_owned();
    let mut request_body = None;
    let mut request_headers = Vec::new();
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
        NavigationDecision::Cancel => unreachable!("canceled decision returned as error"),
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
            .start_navigation_load(
                contents,
                navigation,
                NavigationRequestLoadPolicy::DocumentInitiated,
                policy,
            )
    })
    .await?;
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
                        let fetched = load
                            .fetch_intercepted_auth_response(
                                &method,
                                requested_url.as_str(),
                                request_body.clone(),
                                request_headers.clone(),
                                credentials.clone(),
                            )
                            .await
                            .map_err(|error| error.to_string())?;
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
                        let fetched = load
                            .fetch_navigation_with_auth(
                                &method,
                                requested_url.as_str(),
                                request_body.clone(),
                                request_headers.clone(),
                                auth.clone(),
                            )
                            .await
                            .map_err(|error| error.to_string())?;
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
                        PausedDocumentTransfer::pending(
                            NavigationRequestLoadPolicy::DocumentInitiated,
                            body,
                        ),
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
    let transfer = PausedDocumentTransfer::pending(
        NavigationRequestLoadPolicy::DocumentInitiated,
        body_source,
    );
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
    let request = NavigationRequest {
        web_contents: contents,
        navigation,
        document: load.document_id(),
    };
    let snapshot = NavigationResponseSnapshot {
        request,
        response: response.clone(),
        observations: observations.clone(),
        body: None,
    };
    on_owner(owner, move |browser| {
        browser.record_native_response(snapshot)
    })
    .await?;
    if moli_web_mime::response_headers_indicate_attachment_download(&response.headers) {
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
        return on_owner(owner, move |browser| {
            browser.download_navigation_response(request, renderer, response, body)
        })
        .await;
    }
    let (body, capture) = stream_response_body(body_source)?;
    let destination = DocumentNavigationDestination {
        url: response.final_url.clone(),
        security_origin: moli_url::origin_ascii_serialization(&response.final_url),
        secure_context_type: if moli_url::is_potentially_trustworthy_url(&response.final_url) {
            "Secure"
        } else {
            "InsecureScheme"
        }
        .to_owned(),
    };
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
    if let Some(continuation) = commit.post_response_continuation {
        continuation.release();
    }
    let body = capture.finish().await;
    on_owner(owner, move |browser| {
        browser.complete_native_response(request, body)
    })
    .await?;
    Ok(())
}

enum TransferAdmission {
    Unobserved(Box<PausedDocumentTransfer>),
    Paused(PendingDecision),
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
            .navigation_decision_provider
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

fn stream_response_body(
    source: DocumentBodySource,
) -> Result<(ExternalRawDocumentBodyStream, NativeBodyCapture), String> {
    match source {
        DocumentBodySource::BufferedRaw { response, .. } => {
            let bytes = response
                .into_body()
                .1
                .try_into_materialized_bytes()
                .map_err(|_| "buffered native response has no materialized bytes".to_owned())?;
            replay_captured_body(CapturedBody::from_bytes_spooled(bytes))
        }
        DocumentBodySource::StreamingRaw { mut response, .. } => {
            let (completion, completed) = oneshot::channel();
            let (sender, body) = ExternalRawDocumentBodyStream::channel(completed);
            let pump = tokio::task::spawn_local(async move {
                let result = async {
                    let mut writer = CapturedBodyWriter::default();
                    let mut sender = Some(sender);
                    while let Some(chunk) = response.next_chunk().await {
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
            Ok((body, NativeBodyCapture(Some(pump))))
        }
        DocumentBodySource::CapturedRaw { body, .. } => replay_captured_body(body),
    }
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
