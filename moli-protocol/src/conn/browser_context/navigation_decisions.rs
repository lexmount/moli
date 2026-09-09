use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CdpSessionRoute, CommandOwnerScope, FetchRequestStage,
    NavigationDispatchState, NavigationResultProjection, NavigationSourceDocumentSecurityContext,
    PendingFetchNavigation, ResponseStageUrlMatchPolicy, TargetPageResidenceIdentity,
    monotonic_timestamp_seconds,
};
use moli_core::browser::web_contents::NavigationInterceptionPermit;
use moli_core::browser::{NavigationDecision, NavigationDecisionStage, WebContentsHandle};

impl CdpConnection {
    pub async fn project_browser_initial_document_inspection(
        &mut self,
        contents: WebContentsHandle,
        expected: Option<moli_core::browser::web_contents::InitialDocumentBuildKey>,
    ) -> Vec<BackgroundProtocolEvent> {
        use moli_core::browser::web_contents::InitialDocumentInspectionStage;
        let Ok(context) = self.browser.context_handle(contents.context()) else {
            return Vec::new();
        };
        let Ok(current) = context.initial_document_build_key(contents) else {
            return Vec::new();
        };
        let Some(projection) = self.browser_context_by_browser_id_mut(contents.context()) else {
            return Vec::new();
        };
        if let Some(expected) = expected
            && current != Some(expected)
        {
            projection.retire_initial_document_projection(expected);
            return Vec::new();
        }
        projection.reconcile_initial_document_projection(contents, current);
        let Some(key) = current else {
            return Vec::new();
        };
        let Some(target_id) = projection
            .target_id_for_web_contents(contents.id())
            .map(str::to_owned)
        else {
            return Vec::new();
        };
        let context_id = projection.id.clone();
        let Ok(Some(claim)) = context.claim_initial_document_inspection(contents, key) else {
            return Vec::new();
        };
        match &claim.stage {
            InitialDocumentInspectionStage::Reserved => {
                self.browser_context_by_id_mut(&context_id)
                    .expect("resolved Context")
                    .project_initial_document_build(&target_id, key);
                self.bind_renderer_page_output_owner(
                    key.renderer(),
                    TargetPageResidenceIdentity::new(context_id, Some(target_id), key.document()),
                );
            }
            InitialDocumentInspectionStage::Prepared(endpoint) => {
                let owner = CommandOwnerScope::for_route(CdpSessionRoute::PageTarget {
                    browser_context_id: context_id,
                    target_id,
                    session_key: moli_page_types::DevToolsSessionKey::Primary,
                });
                if let Err(error) = endpoint
                    .start_configure(self.prepared_document_inspection_for_owner(&owner))
                    .await
                {
                    tracing::warn!(%error, "initial document inspection configuration failed");
                }
            }
        }
        drop(claim);
        Vec::new()
    }

    pub(crate) fn start_created_web_contents_navigation(
        &self,
        contents: WebContentsHandle,
        url: String,
    ) -> Result<(), String> {
        let url = url::Url::parse(&url).map_err(|error| error.to_string())?;
        self.browser
            .context_handle(contents.context())?
            .navigate_initial_document(contents, url)?;
        Ok(())
    }

    pub async fn project_browser_navigation_responses(
        &mut self,
        contents: WebContentsHandle,
    ) -> Vec<BackgroundProtocolEvent> {
        let Ok(context) = self.browser.context_handle(contents.context()) else {
            return Vec::new();
        };
        let Ok(responses) = context.navigation_responses(contents) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for response in responses {
            let observed = self
                .browser_context_by_browser_id_mut(contents.context())
                .and_then(|context| {
                    let target = context
                        .target_id_for_web_contents(contents.id())?
                        .to_owned();
                    context.observe_native_navigation_response(
                        &target,
                        response.request.navigation,
                        response.body.is_some(),
                    )
                });
            let Some((pending, emit_response, metadata_emitted)) = observed else {
                continue;
            };
            // Stream loss may reveal body completion before its commit event.
            // Publish the exact committed Document fence before completing it.
            if response.body.is_some()
                && let Ok(Some(document)) = context.document_handle(contents)
                && document.id() == response.request.document
            {
                out.extend(self.project_browser_document_commit(document).await);
                if let Some(Ok(body)) = &response.body {
                    let state = &pending.navigation;
                    let _ = self.commit_main_document_resource_for_owner(
                        &state.owner,
                        state.frame_id.clone(),
                        state.loader_id.clone(),
                        response.response.final_url.clone(),
                        response.response.headers.clone(),
                        response.response.from_cache,
                        Some(body.clone()),
                    );
                }
            }
            out.extend(crate::domains::network::native_navigation_response_events(
                self,
                &pending.navigation,
                &response,
                emit_response,
                metadata_emitted,
            ));
            if response.body.is_some()
                && context.navigation_snapshot(contents).ok().is_some_and(|snapshot| {
                    matches!(snapshot.attempt, Some(moli_core::browser::NavigationAttempt::Failed {
                        request, reason: moli_core::browser::NavigationFailureReason::Download
                    }) if request == response.request)
                })
            {
                for session in self.page_event_session_ids_for_owner(&pending.navigation.owner) {
                    crate::domains::page::emit_navigation_frame_stop_after_download_background_events(
                        &mut out, session.as_deref(), &pending.navigation.frame_id, &pending.navigation.loader_id,
                    );
                }
            }
        }
        out
    }

    pub(crate) fn native_startup_allows_document_access(&self, owner: &CommandOwnerScope) -> bool {
        let Some((context_id, target_id)) = self.resolved_page_owner_identity_for_owner(owner)
        else {
            return false;
        };
        let Some(contents) = self
            .browser_context_by_id(&context_id)
            .and_then(|context| context.web_contents_handle_for_target(&target_id))
        else {
            return false;
        };
        let Ok(context) = self.browser.context_handle(contents.context()) else {
            return false;
        };
        let Ok(Some(navigation)) = context.native_initial_document_navigation(contents) else {
            return false;
        };
        // Initial-document access is for a real inspector pause, not for the
        // brief request-admission turns of an unpaused background navigation.
        let inspecting_initial = self.target_has_waiting_for_debugger_session(&target_id)
            || self
                .browser_context_by_id(&context_id)
                .and_then(|projection| {
                    projection.native_navigation_dispatch(&target_id, navigation)
                })
                .is_some_and(|pending| {
                    context
                        .navigation_interception_awaits_decision(
                            contents,
                            pending.navigation_permit,
                        )
                        .unwrap_or(false)
                });
        inspecting_initial
            && self
                .runtime_session_owner_slot_for_owner(owner)
                .is_ok_and(|slot| slot.allows_initial_document_access(navigation))
    }

    pub(crate) fn native_navigation_decision_for_target(
        &self,
        target_id: &str,
    ) -> Option<(
        WebContentsHandle,
        moli_core::browser::NavigationDecisionSnapshot,
    )> {
        let context_id = self.browser_context_id_for_target(target_id)?;
        let contents = self
            .browser_context_by_id(context_id)?
            .web_contents_handle_for_target(target_id)?;
        let paused = self
            .browser
            .context_handle(contents.context())
            .ok()?
            .navigation_decision(contents)
            .ok()??;
        Some((contents, paused))
    }

    pub async fn project_browser_navigation_decision(
        &mut self,
        contents: WebContentsHandle,
        expected_permit: Option<NavigationInterceptionPermit>,
    ) -> Vec<BackgroundProtocolEvent> {
        let Some((context_id, target_id)) = self
            .browser_context_by_browser_id(contents.context())
            .and_then(|context| {
                Some((
                    context.id.clone(),
                    context
                        .target_id_for_web_contents(contents.id())?
                        .to_owned(),
                ))
            })
        else {
            return Vec::new();
        };
        let Ok(context) = self.browser.context_handle(contents.context()) else {
            return Vec::new();
        };
        let Ok(Some(paused)) = context.navigation_decision(contents) else {
            return Vec::new();
        };
        if expected_permit.is_some_and(|expected| paused.permit != expected) {
            return Vec::new();
        }
        let owner = CommandOwnerScope::for_route(CdpSessionRoute::PageTarget {
            browser_context_id: context_id.clone(),
            target_id: target_id.clone(),
            session_key: moli_page_types::DevToolsSessionKey::Primary,
        });
        match paused.stage {
            NavigationDecisionStage::Auth { response, .. } => {
                let pending = self
                    .browser_context_by_id(&context_id)
                    .and_then(|context| {
                        context.native_navigation_dispatch(&target_id, paused.permit.navigation())
                    })
                    .cloned();
                if let Some(pending) = pending
                    && self.target_fetch_matches_auth_required_for_owner(
                        &pending.navigation.owner,
                        &pending.navigation.requested_url,
                    )
                    && let Some(mut challenge) =
                        crate::domains::fetch::extract_auth_challenge(&response.headers)
                {
                    if !self
                        .browser_context_by_id_mut(&context_id)
                        .expect("resolved Context")
                        .observe_native_auth_decision(&target_id, paused.permit)
                    {
                        return Vec::new();
                    }
                    crate::domains::fetch::populate_auth_challenge_origin(
                        self,
                        pending.navigation.owner.session_id(),
                        &response.final_url,
                        &mut challenge,
                    );
                    match crate::domains::fetch::register_navigation_auth_required_event_for_permit(
                        self,
                        &pending,
                        challenge,
                        response.request_cookie_report.clone(),
                        paused.permit,
                    ) {
                        Ok(event) => return vec![event],
                        Err(error) => {
                            tracing::warn!(%error, "native navigation authentication projection failed");
                            let _ = context.resolve_navigation_decision(
                                contents,
                                paused.permit,
                                NavigationDecision::Cancel,
                            );
                            return Vec::new();
                        }
                    }
                }
            }
            NavigationDecisionStage::Response {
                response,
                observations,
            } => {
                let pending = self
                    .browser_context_by_id(&context_id)
                    .and_then(|context| {
                        context.native_navigation_dispatch(&target_id, paused.permit.navigation())
                    })
                    .cloned();
                if let Some(mut pending) = pending
                    && crate::domains::fetch::prepare_navigation_response_stage(
                        self,
                        &mut pending,
                        &response.final_url,
                    )
                {
                    let event =
                        crate::domains::fetch::navigation_response_stage_request_paused_event(
                            self,
                            pending.interception_session_id.as_deref(),
                            &pending.fetch_request_id,
                            &pending.navigation,
                            &response.final_url,
                            response.request_cookie_report.as_ref(),
                            response.status,
                            &response.headers,
                        );
                    let mut out = Vec::new();
                    let progress = crate::domains::network::response_stage_main_document_navigation_network_progress(self, &pending.navigation, response.request_cookie_report.as_ref());
                    let method = pending.navigation.request_method.clone();
                    let headers = pending.navigation.request_headers.clone();
                    if self.register_native_fetch_response_for_owner(pending, paused.permit) {
                        progress.emit_response_extra_info_before_pause(
                            &mut out,
                            &method,
                            &headers,
                            response.request_cookie_report.as_ref(),
                            &response.redirect_chain,
                            response.status,
                            &response.headers,
                            &response.cookie_set_reports,
                            &observations,
                            !observations.is_empty(),
                        );
                        out.push(event);
                    }
                    return out;
                }
            }
            NavigationDecisionStage::PreparedDocument {
                inspection,
                renderer,
                ..
            } => {
                let request = context
                    .navigation_snapshot(contents)
                    .ok()
                    .and_then(|snapshot| match snapshot.attempt {
                        Some(moli_core::browser::NavigationAttempt::Started(request))
                            if request.navigation == paused.permit.navigation() =>
                        {
                            Some(request)
                        }
                        _ => None,
                    });
                let Some(request) = request else {
                    return Vec::new();
                };
                if self
                    .browser_context_by_id_mut(&context_id)
                    .expect("resolved Context")
                    .project_navigation_preparation_for_target(&target_id, request, renderer)
                    .is_err()
                {
                    return Vec::new();
                }
                self.bind_renderer_page_output_owner(
                    renderer,
                    TargetPageResidenceIdentity::new(
                        context_id.clone(),
                        Some(target_id.clone()),
                        request.document,
                    ),
                );
                // Browser's DocumentCommitted occurrence owns the frame event
                // and inspector rebind fence. Do not install the legacy renderer
                // commit publisher as a competing source of the same fact.
                if let Err(error) = inspection
                    .start_configure(self.prepared_document_inspection_for_owner(&owner))
                    .await
                {
                    tracing::warn!(%error, "native navigation inspection configuration failed");
                }
            }
            NavigationDecisionStage::Request {
                url,
                method,
                headers,
                opening,
            } => {
                if opening.strong_count() != 0
                    && !self
                        .browser_context_by_id(&context_id)
                        .is_some_and(|context| {
                            context
                                .popup_navigation_observed(&target_id, paused.permit.navigation())
                        })
                {
                    return Vec::new();
                }
                if self.target_has_waiting_for_debugger_session(&target_id) {
                    return Vec::new();
                }
                if self
                    .browser_context_by_id(&context_id)
                    .and_then(|context| {
                        context.native_navigation_dispatch(&target_id, paused.permit.navigation())
                    })
                    .is_some()
                {
                    return Vec::new();
                }
                return self.project_native_navigation_request(
                    &owner,
                    contents,
                    paused.permit,
                    url,
                    method,
                    headers,
                );
            }
        }
        let _ = context.resolve_navigation_decision(
            contents,
            paused.permit,
            NavigationDecision::Continue,
        );
        Vec::new()
    }

    pub(crate) async fn observe_popup_navigation(
        &mut self,
        admission: moli_core::browser::BrowserPopupAdmission,
    ) -> Vec<BackgroundProtocolEvent> {
        let Some(navigation) = admission.navigation else {
            return Vec::new();
        };
        // Allocate the existing navigation projection before marking this
        // exact source-FIFO observation. A late receipt cannot release a new attempt.
        let events = self
            .project_browser_navigation(admission.web_contents)
            .await;
        if let Some(context) =
            self.browser_context_by_browser_id_mut(admission.web_contents.context())
            && let Some(target_id) = context
                .target_id_for_web_contents(admission.web_contents.id())
                .map(str::to_owned)
        {
            context.observe_popup_navigation(&target_id, navigation);
            let context_id = context.id.clone();
            if self
                .native_navigation_decision_for_target(&target_id)
                .is_some_and(|(_, paused)| paused.permit.navigation() == navigation)
                && let Some(action) =
                    crate::conn::TargetStartupOwnerAction::capture(self, &context_id, &target_id)
            {
                self.publish_target_startup_owner_action(action);
            }
        }
        events
    }

    fn project_native_navigation_request(
        &mut self,
        owner: &CommandOwnerScope,
        contents: WebContentsHandle,
        permit: NavigationInterceptionPermit,
        url: url::Url,
        method: String,
        headers: Vec<(String, String)>,
    ) -> Vec<BackgroundProtocolEvent> {
        let Some(preflight) =
            self.prepare_navigation_request_for_owner(owner, &url, None, url.scheme() == "data")
        else {
            return Vec::new();
        };
        let Some((context_id, target_id)) = self.resolved_page_owner_identity_for_owner(owner)
        else {
            return Vec::new();
        };
        let mut request_headers = preflight.request_headers;
        for (name, value) in headers {
            request_headers.retain(|(previous, _)| !previous.eq_ignore_ascii_case(&name));
            request_headers.push((name, value));
        }
        let state = NavigationDispatchState {
            navigate_id: None,
            owner: owner.clone(),
            web_contents: contents,
            result_projection: NavigationResultProjection::Cdp(serde_json::json!({})),
            frame_id: preflight.frame_id,
            session_id: preflight.session_id,
            request_id: preflight.document_request_id,
            loader_id: preflight.document_loader_id,
            request_announced: true,
            requested_url: url.clone(),
            request_method: method.clone(),
            request_body: None,
            request_body_bytes: None,
            request_headers: request_headers.clone(),
            request_load_policy: moli_core::browser::NavigationRequestLoadPolicy::DocumentInitiated,
            timestamp: monotonic_timestamp_seconds(),
            source_document_security: NavigationSourceDocumentSecurityContext::new(
                preflight.inherited_security_origin,
                preflight.inherited_secure_context_type,
            ),
        };
        let mut out = Vec::new();
        let intercept_request =
            preflight.document_fetch_request_stage == Some(FetchRequestStage::Request);
        let fetch_id = preflight.fetch_navigation_request_id.unwrap_or_else(|| {
            self.allocate_fetch_navigation_request_id_for_owner(owner)
                .expect("resolved native navigation owner")
        });
        let pending = PendingFetchNavigation {
            fetch_request_id: fetch_id,
            interception_session_id: preflight
                .document_fetch_event_session_id
                .or_else(|| state.session_id.clone()),
            navigation_permit: permit,
            navigation: state,
            request_cookie_report: None,
            intercept_response: preflight.document_fetch_response_stage_candidate,
            response_stage_url_match_policy: ResponseStageUrlMatchPolicy::MatchFinalUrl,
            auth_required_blocked_intercepts: preflight.document_auth_required_blocked_intercepts,
        };
        self.browser_context_by_id_mut(&context_id)
            .expect("resolved Context")
            .record_native_navigation_dispatch(&target_id, permit.navigation(), pending.clone());
        for session in self.page_event_session_ids_for_owner(owner) {
            crate::domains::page::emit_navigation_started_background_events(
                &mut out,
                session.as_deref(),
                &pending.navigation.frame_id,
                &pending.navigation.loader_id,
                url.as_str(),
                crate::domains::page::NavigationStartInitiator::Renderer,
            );
        }
        crate::domains::network::emit_fetch_navigation_initial_request_for_pause_background_events(
            self,
            &mut out,
            &pending.navigation,
            None,
            intercept_request.then_some(pending.fetch_request_id.as_str()),
        );
        if intercept_request {
            out.push(crate::domains::fetch::request_paused_background_event(
                self,
                pending.interception_session_id.as_deref(),
                &pending,
            ));
            self.register_pending_fetch_navigation_request_for_owner(owner, pending);
        } else {
            let _ = self.resolve_native_navigation_decision(
                contents,
                permit,
                NavigationDecision::Request {
                    url,
                    method,
                    body: None,
                    headers: request_headers,
                },
            );
        }
        out
    }

    pub(crate) fn resolve_native_navigation_decision(
        &self,
        contents: WebContentsHandle,
        permit: NavigationInterceptionPermit,
        decision: NavigationDecision,
    ) -> bool {
        self.browser
            .context_handle(contents.context())
            .and_then(|context| context.resolve_navigation_decision(contents, permit, decision))
            .unwrap_or(false)
    }

    pub(crate) fn navigation_interception_awaits_decision(
        &self,
        contents: WebContentsHandle,
        permit: NavigationInterceptionPermit,
    ) -> bool {
        self.browser
            .context_handle(contents.context())
            .and_then(|context| context.navigation_interception_awaits_decision(contents, permit))
            .unwrap_or(false)
    }

    pub(crate) fn update_native_navigation_dispatch(&mut self, pending: &PendingFetchNavigation) {
        if let Some((context_id, target_id)) =
            self.resolved_page_owner_identity_for_owner(&pending.navigation.owner)
            && let Some(context) = self.browser_context_by_id_mut(&context_id)
        {
            context.record_native_navigation_dispatch(
                &target_id,
                pending.navigation_permit.navigation(),
                pending.clone(),
            );
        }
    }

    /// Popup creation has not exposed its Target yet. Drive only inspection
    /// decisions for its exact initial candidate while Browser builds/commits
    /// that candidate; request-stage debugger decisions run after attachment.
    pub(crate) async fn ensure_native_popup_initial_document(
        &mut self,
        contents: WebContentsHandle,
    ) -> Result<Vec<BackgroundProtocolEvent>, String> {
        let context = self.browser.context_handle(contents.context())?;
        let (_, mut events) = self.browser.subscribe()?;
        let mut out = Vec::new();
        loop {
            if let Some(document) = context.document_handle(contents)? {
                out.extend(self.project_browser_document_commit(document).await);
                return Ok(out);
            }
            if context.initial_document_build_key(contents)?.is_none() {
                return Err("native popup initial construction is unavailable".into());
            }
            out.extend(
                self.project_browser_initial_document_inspection(contents, None)
                    .await,
            );
            match events.recv().await {
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return Err("Browser stopped while preparing popup".into());
                }
            }
        }
    }
}
