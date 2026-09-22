use super::*;
use crate::runtime::owner_local_store::{
    reconcile_navigation_lifecycle_observation,
    take_staged_auxiliary_page_on_bound_owner_local_store,
};

impl RendererOwnerHandle {
    pub(super) async fn run_owner_lane_local_task<R, F>(&self, future: F) -> Result<R>
    where
        R: 'static,
        F: Future<Output = Result<R>> + 'static,
    {
        run_named_owner_local_task(
            self.state.local_executor.clone(),
            "owner-lane local task channel closed",
            future,
        )
        .await
    }

    pub(super) async fn install_page_vm_on_owner_lane(
        &self,
        owner_local_context: RendererOwnerLocalContext,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        page_vm: PageVm,
        pending_download: Option<RendererPendingDownloadActivation>,
        lifecycle_decision: Option<PageVmInitStage>,
    ) -> Result<RendererPendingPageCreation> {
        self.run_owner_lane_local_task(async move {
            install_page_vm_on_bound_owner_local_store(
                &owner_local_context,
                requested_url,
                navigation_initiator_url,
                navigation_redirected,
                navigation_redirect_count,
                response_status,
                response_headers,
                page_vm,
                pending_download,
                lifecycle_decision,
            )
        })
        .await
    }

    pub(super) async fn install_phase_one_blocked_page_on_owner_lane(
        &self,
        owner_local_context: RendererOwnerLocalContext,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        pending_navigation: PageVmPendingPhaseOneNavigation,
        lifecycle_decision: Option<PageVmInitStage>,
    ) -> Result<RendererPendingPageCreation> {
        self.run_owner_lane_local_task(async move {
            install_phase_one_blocked_page_on_bound_owner_local_store(
                &owner_local_context,
                requested_url,
                navigation_initiator_url,
                navigation_redirected,
                navigation_redirect_count,
                response_status,
                response_headers,
                pending_navigation,
                lifecycle_decision,
            )
        })
        .await
    }

    pub(super) async fn finalize_pending_page_creation_on_owner_lane(
        &self,
        pending: RendererPendingPageCreation,
    ) -> Result<RendererAttachedPage> {
        let token = pending.token;
        let commit = self
            .run_owner_lane_local_task(async move {
                Ok(finalize_pending_page_creation_on_bound_owner_local_store(
                    pending,
                ))
            })
            .await?;
        let finalized =
            commit.publish_then_finalize(|output| self.publish_renderer_output(output))?;
        if finalized.resume_parked_page_turn {
            self.signal_internal_page_turn_source(
                token,
                RendererOwnerWakeSource::SchedulerContinuation,
            );
        }
        Ok(finalized.attached_page)
    }

    pub(super) async fn resolve_pending_page_creation_on_owner_lane(
        &self,
        pending: RendererPendingPageCreation,
        document: RendererDocumentLifecycleIdentity,
        target_stage: PageVmInitStage,
        navigation_reply_policy: NavigationReplyPolicy,
    ) -> Result<RendererPageCreationResolution> {
        self.run_owner_lane_local_task(async move {
            Ok(resolve_pending_page_creation_on_bound_owner_local_store(
                pending,
                document,
                target_stage,
                navigation_reply_policy,
            ))
        })
        .await
    }

    pub(super) async fn install_page_vm_and_begin_post_parse_lifecycle(
        &self,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        page_vm: PageVm,
        page_tasks: Vec<PostParsePageOwnedWork>,
        stage: PageVmInitStage,
        started: Instant,
        reply_boundary: crate::RendererReplyBoundary,
        lifecycle_decider: Option<RendererLifecycleDecider>,
        top_level_navigation_dispatch: RendererTopLevelNavigationDispatch,
    ) -> Result<(RendererPendingPageCreation, DocumentLifecycleTurnOutcome)> {
        let owner_local_context = self.owner_local_context()?;
        let pending = self
            .install_page_vm_on_owner_lane(
                owner_local_context,
                requested_url,
                navigation_initiator_url,
                navigation_redirected,
                navigation_redirect_count,
                response_status,
                response_headers,
                page_vm,
                None,
                reply_boundary.waits_for_stage().then_some(stage),
            )
            .await?;
        let pending = pending.with_lifecycle_decider(stage, lifecycle_decider);
        let token = pending.token;
        let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => {
                remove_page_on_bound_owner_local_store(token);
                return Err(error);
            }
        };
        entry.set_top_level_navigation_dispatch(top_level_navigation_dispatch);
        let (entry, begin_result) = begin_post_parse_lifecycle_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            page_tasks,
            stage,
            started,
        )
        .await;
        self.restore_live_page_entry(token, entry);
        match begin_result {
            Ok(outcome) => Ok((pending, outcome)),
            Err(error) => {
                remove_page_on_bound_owner_local_store(token);
                Err(error)
            }
        }
    }

    pub(super) async fn continue_pending_phase_one_page_creation(
        &self,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        pending_navigation: PageVmPendingPhaseOneNavigation,
        stage: PageVmInitStage,
        reply_boundary: crate::RendererReplyBoundary,
        lifecycle_decider: Option<RendererLifecycleDecider>,
        top_level_navigation_dispatch: RendererTopLevelNavigationDispatch,
        navigation_reply_policy: NavigationReplyPolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let owner_local_context = match self.owner_local_context() {
            Ok(context) => context,
            Err(error) => return Err(error).into(),
        };
        let pending = match self
            .install_phase_one_blocked_page_on_owner_lane(
                owner_local_context,
                requested_url,
                navigation_initiator_url,
                navigation_redirected,
                navigation_redirect_count,
                response_status,
                response_headers,
                pending_navigation,
                reply_boundary.waits_for_stage().then_some(stage),
            )
            .await
        {
            Ok(pending) => pending,
            Err(error) => return Err(error).into(),
        };
        let pending = pending.with_lifecycle_decider(stage, lifecycle_decider);
        let token = pending.token;
        let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        entry.set_top_level_navigation_dispatch(top_level_navigation_dispatch);
        let vm_creation_id = entry.page_vm().creation_id;
        self.restore_live_page_entry(token, entry);

        if matches!(reply_boundary, crate::RendererReplyBoundary::DocumentCommit) {
            self.publish_pending_page_creation_and_continue(
                pending,
                RenderRuntimePageCreationContinuation::AfterCommittedDocumentResponse {
                    turn: Box::new(
                        RenderRuntimeTurn::ContinueLivePagePendingLocationNavigationPhaseOne {
                            token,
                            vm_creation_id,
                            follow_count: 0,
                            completion:
                                LivePagePendingNavigationCompletion::PublishedPageCreation {
                                    navigation_reply_policy,
                                },
                        },
                    ),
                    wake_token: token,
                },
            )
            .await
        } else {
            let admission =
                pending_phase_one_admission_after_restore_on_bound_owner_local_store(token);
            self.signal_pending_phase_one_admission(token, admission);
            RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                turn: Box::new(
                    RenderRuntimeTurn::ContinueLivePagePendingLocationNavigationPhaseOne {
                        token,
                        vm_creation_id,
                        follow_count: 0,
                        completion: LivePagePendingNavigationCompletion::CompletePageCreation {
                            pending,
                            navigation_reply_policy,
                        },
                    },
                ),
                wake_token: token,
            }
        }
    }

    pub(super) async fn continue_page_creation_with_pending_navigation(
        &self,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        page_vm: PageVm,
        stage: PageVmInitStage,
        reply_boundary: crate::RendererReplyBoundary,
        lifecycle_decider: Option<RendererLifecycleDecider>,
        top_level_navigation_dispatch: RendererTopLevelNavigationDispatch,
        navigation_reply_policy: NavigationReplyPolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let owner_local_context = match self.owner_local_context() {
            Ok(context) => context,
            Err(error) => return Err(error).into(),
        };
        let pending = match self
            .install_page_vm_on_owner_lane(
                owner_local_context,
                requested_url,
                navigation_initiator_url,
                navigation_redirected,
                navigation_redirect_count,
                response_status,
                response_headers,
                page_vm,
                None,
                reply_boundary.waits_for_stage().then_some(stage),
            )
            .await
        {
            Ok(pending) => pending,
            Err(error) => return Err(error).into(),
        };

        let pending = pending.with_lifecycle_decider(stage, lifecycle_decider);
        let token = pending.token;
        let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        entry.set_top_level_navigation_dispatch(top_level_navigation_dispatch);

        if navigation_reply_policy.returns_with_pending_navigation() {
            self.restore_live_page_entry(token, entry);
            return self.finish_pending_page_creation(pending).await;
        }

        if matches!(reply_boundary, crate::RendererReplyBoundary::DocumentCommit) {
            self.restore_live_page_entry(token, entry);
            self.publish_pending_page_creation_and_continue(
                pending,
                RenderRuntimePageCreationContinuation::next_turn(
                    RenderRuntimeTurn::FollowLivePagePendingLocationNavigation {
                        token,
                        stage,
                        follow_count: 0,
                        completion: LivePagePendingNavigationCompletion::PublishedPageCreation {
                            navigation_reply_policy,
                        },
                    },
                ),
            )
            .await
        } else {
            self.continue_live_page_pending_navigation(
                token,
                entry,
                stage,
                0,
                LivePagePendingNavigationCompletion::CompletePageCreation {
                    pending,
                    navigation_reply_policy,
                },
            )
        }
    }

    pub(super) async fn finish_pending_page_creation(
        &self,
        mut pending: RendererPendingPageCreation,
    ) -> RenderRuntimeDispatchOutcome {
        if let Some((target_stage, decider)) = pending.take_lifecycle_decider() {
            return self
                .apply_lifecycle_decision(pending, target_stage, decider)
                .await;
        }

        self.finalize_pending_page_creation_reply(pending).await
    }

    pub(super) async fn finalize_pending_page_creation_reply(
        &self,
        pending: RendererPendingPageCreation,
    ) -> RenderRuntimeDispatchOutcome {
        let token = pending.token;
        match self
            .finalize_pending_page_creation_on_owner_lane(pending)
            .await
        {
            Ok(attached_page) => {
                self.reply_with_attached_page_and_resume_lifecycle_if_needed(attached_page)
            }
            Err(error) => self.retire_failed_page_creation(token, error).await,
        }
    }

    pub(super) fn reply_with_attached_page_and_resume_lifecycle_if_needed(
        &self,
        attached_page: RendererAttachedPage,
    ) -> RenderRuntimeDispatchOutcome {
        let token = attached_page.token;
        let lifecycle = attached_page.creation_artifacts.lifecycle_snapshot;
        let document = lifecycle.into();
        let should_resume_lifecycle = lifecycle.load.is_none() && lifecycle.terminated.is_none();
        if should_resume_lifecycle {
            RenderRuntimeDispatchOutcome::PageCreatedAndContinueNavigation {
                page: Box::new(attached_page),
                continuation: RenderRuntimePageCreationContinuation::next_turn(
                    RenderRuntimeTurn::ResumeLivePageDocumentLifecycleAfterReply {
                        token,
                        document,
                    },
                ),
            }
        } else {
            Ok(RendererOwnerReply::PageCreated(Box::new(attached_page))).into()
        }
    }

    pub(super) async fn retire_failed_page_creation(
        &self,
        token: RendererPageToken,
        error: anyhow::Error,
    ) -> RenderRuntimeDispatchOutcome {
        let _ = remove_page_on_bound_owner_local_store_via_local_task(
            self.state.local_executor.clone(),
            token,
        )
        .await;
        Err(error).into()
    }

    pub(super) async fn publish_pending_page_creation_and_continue(
        &self,
        pending: RendererPendingPageCreation,
        continuation: RenderRuntimePageCreationContinuation,
    ) -> RenderRuntimeDispatchOutcome {
        let token = pending.token;
        if matches!(
            continuation.turn(),
            RenderRuntimeTurn::FollowLivePagePendingLocationNavigation {
                follow_count: 0,
                ..
            }
        ) {
            let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
                Ok(entry) => entry,
                Err(error) => return Err(error).into(),
            };
            entry.begin_renderer_navigation_follow();
            self.restore_live_page_entry(token, entry);
        }
        match self
            .finalize_pending_page_creation_on_owner_lane(pending)
            .await
        {
            Ok(attached_page) => RenderRuntimeDispatchOutcome::PageCreatedAndContinueNavigation {
                page: Box::new(attached_page),
                continuation,
            },
            Err(error) => self.retire_failed_page_creation(token, error).await,
        }
    }

    pub(super) async fn continue_attached_page_creation_lifecycle_turn(
        &self,
        pending: RendererPendingPageCreation,
        document: RendererDocumentLifecycleIdentity,
        target_stage: PageVmInitStage,
        navigation_reply_policy: NavigationReplyPolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let token = pending.token;
        let resolution = self
            .resolve_pending_page_creation_on_owner_lane(
                pending,
                document,
                target_stage,
                navigation_reply_policy,
            )
            .await;
        let resolution = match resolution {
            Ok(resolution) => resolution,
            Err(error) => return self.retire_failed_page_creation(token, error).await,
        };
        let (resolution, retire_page_after_publication) =
            resolution.publish_then_resolve(|output| self.publish_renderer_output(output));
        match resolution {
            PageCreationResolution::Finalized {
                attached,
                resume_parked_page_turn,
            } => {
                debug_assert!(!retire_page_after_publication);
                if resume_parked_page_turn {
                    self.signal_internal_page_turn_source(
                        token,
                        RendererOwnerWakeSource::SchedulerContinuation,
                    );
                }
                self.reply_with_attached_page_and_resume_lifecycle_if_needed(attached)
            }
            PageCreationResolution::Waiting { pending, document } => {
                debug_assert!(!retire_page_after_publication);
                RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                    turn: Box::new(RenderRuntimeTurn::ContinueAttachedPageCreationLifecycle {
                        pending,
                        document,
                        target_stage,
                        navigation_reply_policy,
                    }),
                    wake_token: token,
                }
            }
            PageCreationResolution::LifecycleDecisionRequired { pending } => {
                debug_assert!(!retire_page_after_publication);
                self.finish_pending_page_creation(pending).await
            }
            PageCreationResolution::Retired { failure } => {
                debug_assert!(retire_page_after_publication);
                let _ = remove_page_on_bound_owner_local_store_via_local_task(
                    self.state.local_executor.clone(),
                    token,
                )
                .await;
                Err(failure.into_error()).into()
            }
            PageCreationResolution::EntryUnavailable { error } => {
                debug_assert!(!retire_page_after_publication);
                Err(error).into()
            }
        }
    }

    pub(super) async fn continue_live_page_navigation_post_parse_lifecycle_turn(
        &self,
        token: RendererPageToken,
        document: RendererDocumentLifecycleIdentity,
        target_stage: PageVmInitStage,
        follow_count: usize,
        completion: LivePagePendingNavigationCompletion,
    ) -> RenderRuntimeDispatchOutcome {
        let retire_page_on_failure = completion.retires_page_on_navigation_failure();
        let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        let observation = observe_document_lifecycle_on_entry(&mut entry, document, target_stage);
        let observation = reconcile_navigation_lifecycle_observation(
            observation,
            entry.page_vm().vm().has_pending_location_navigation(),
        );
        match observation {
            DocumentLifecycleObserverOutcome::NavigationPending
                if completion.returns_with_pending_location_navigation() =>
            {
                self.finish_live_page_navigation_completion(token, entry, completion)
                    .await
            }
            DocumentLifecycleObserverOutcome::Reached => {
                self.finish_live_page_navigation_completion(token, entry, completion)
                    .await
            }
            DocumentLifecycleObserverOutcome::NavigationPending => self
                .continue_live_page_pending_navigation(
                    token,
                    entry,
                    target_stage,
                    follow_count + 1,
                    completion,
                ),
            DocumentLifecycleObserverOutcome::Pending => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                    turn: Box::new(
                        RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle {
                            token,
                            document,
                            target_stage,
                            follow_count,
                            completion,
                        },
                    ),
                    wake_token: token,
                }
            }
            DocumentLifecycleObserverOutcome::DocumentReplaced { document } => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                    turn: Box::new(
                        RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle {
                            token,
                            document,
                            target_stage,
                            follow_count,
                            completion,
                        },
                    ),
                    wake_token: token,
                }
            }
            DocumentLifecycleObserverOutcome::Interrupted(termination) => {
                let error = anyhow!(
                    "renderer document lifecycle was interrupted before {target_stage:?}: {:?}",
                    termination.reason
                );
                self.finish_live_page_navigation_failure(
                    token,
                    entry,
                    retire_page_on_failure,
                    LivePageNavigationFailureDisposition::ReturnToInitiator(error),
                )
                .await
            }
            DocumentLifecycleObserverOutcome::MissingResident => {
                let error = anyhow!(
                    "renderer document lifecycle resident disappeared while exact Document {document:?} was still pending before {target_stage:?}"
                );
                self.finish_live_page_navigation_failure(
                    token,
                    entry,
                    retire_page_on_failure,
                    LivePageNavigationFailureDisposition::ReturnToInitiator(error),
                )
                .await
            }
        }
    }

    pub fn build_create_html_page_request(
        &self,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        loader: &ResourceRequestClient,
        web_storage: crate::RendererWebStorageHandles,
        final_url: Url,
        html: String,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        extra_http_headers: moli_fetch::RequestHeaders,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<crate::SubresourceResourceType>,
        stage: PageVmInitStage,
    ) -> RendererCreateHtmlPageRequest {
        self.build_create_html_page_request_with_env(
            self.allocate_page_reservation_token(),
            requested_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            loader,
            web_storage,
            final_url,
            html,
            stage,
            crate::RendererDocumentOptions {
                document_start_scripts,
                runtime_bindings,
                runtime_inspector_session_restore_snapshots,
                extra_http_headers,
                network_offline,
                blocked_url_patterns,
                fetch_subresource_interception_enabled,
                fetch_subresource_interception_resource_type,
                ..Default::default()
            },
        )
    }

    pub fn build_create_html_page_request_with_env(
        &self,
        page_reservation: RendererPageReservationToken,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        loader: &ResourceRequestClient,
        web_storage: crate::RendererWebStorageHandles,
        final_url: Url,
        html: String,
        stage: PageVmInitStage,
        options: crate::RendererDocumentOptions,
    ) -> RendererCreateHtmlPageRequest {
        let layout_configuration = self.seal_layout_configuration_for_page_creation();
        RendererCreateHtmlPageRequest {
            web_mcp_invocation: None,
            page_reservation,
            root_frame_id: options.root_frame_id,
            main_document_commit: options.main_document_commit,
            navigation_history: options.navigation_history,
            top_level_storage_key: None,
            requested_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            loader: loader.clone(),
            navigator_identity: loader.browser_identity().clone(),
            web_storage,
            final_url,
            html,
            document_start_scripts: options.document_start_scripts,
            runtime_bindings: options.runtime_bindings,
            runtime_inspector_session_restore_snapshots: options
                .runtime_inspector_session_restore_snapshots,
            runtime_isolated_worlds: Vec::new(),
            permission_overrides: Vec::new(),
            extra_http_headers: options.extra_http_headers,
            document_settings: options.document_settings,
            network_offline: options.network_offline,
            blocked_url_patterns: options.blocked_url_patterns,
            indexed_db_manager: options.indexed_db_manager,
            storage_bucket_store: options.storage_bucket_store,
            fetch_subresource_interception_enabled: options.fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type: options
                .fetch_subresource_interception_resource_type,
            layout_configuration,
            wpt_extensions_enabled: false,
            stage,
            reply_boundary: crate::RendererReplyBoundary::Stage,
            lifecycle_decider: None,
            top_level_navigation_dispatch:
                RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            reserved_service_worker_client: None,
        }
    }

    pub fn build_create_streaming_raw_page_request(
        &self,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        navigation_redirect_chain: Vec<crate::protocol_types::NavigationRedirect>,
        response_status: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        loader: &ResourceRequestClient,
        web_storage: crate::RendererWebStorageHandles,
        raw_body: ExternalRawDocumentBodyStream,
        stage: PageVmInitStage,
        options: crate::RendererDocumentOptions,
    ) -> RendererCreateStreamingRawPageRequest {
        let layout_configuration = self.seal_layout_configuration_for_page_creation();
        RendererCreateStreamingRawPageRequest {
            web_mcp_invocation: None,
            document_replacement: None,
            root_frame_id: options.root_frame_id,
            main_document_commit: options.main_document_commit,
            navigation_history: options.navigation_history,
            requested_url,
            final_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            navigation_redirect_chain,
            response_status,
            response_headers,
            loader: loader.clone(),
            navigator_identity: loader.browser_identity().clone(),
            web_storage,
            raw_body,
            document_start_scripts: options.document_start_scripts,
            runtime_bindings: options.runtime_bindings,
            runtime_inspector_session_restore_snapshots: options
                .runtime_inspector_session_restore_snapshots,
            runtime_isolated_worlds: Vec::new(),
            permission_overrides: Vec::new(),
            extra_http_headers: options.extra_http_headers,
            document_settings: options.document_settings,
            network_offline: options.network_offline,
            blocked_url_patterns: options.blocked_url_patterns,
            indexed_db_manager: options.indexed_db_manager,
            storage_bucket_store: options.storage_bucket_store,
            fetch_subresource_interception_enabled: options.fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type: options
                .fetch_subresource_interception_resource_type,
            layout_configuration,
            wpt_extensions_enabled: false,
            stage,
            reply_boundary: crate::RendererReplyBoundary::Stage,
            lifecycle_decider: None,
            top_level_navigation_dispatch:
                RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            navigation_reply_policy: NavigationReplyPolicy::FollowBeforeReply,
            reserved_service_worker_client: None,
        }
    }

    pub fn materialize_page_created_reply_parts(
        &self,
        reply: RendererOwnerReply,
    ) -> Result<(
        RendererPageHandle,
        Arc<RendererPageState>,
        RendererPageCreationDiagnostics,
        RendererPageCreationArtifacts,
        Option<RendererPendingDownloadActivation>,
    )> {
        match reply {
            RendererOwnerReply::PageCreated(reply) => Ok((*reply).into_parts(
                self.state.local_executor.clone(),
                self.render_runtime.clone(),
            )),
            _ => Err(anyhow!(
                "renderer owner returned non-page-creation reply for page creation request"
            )),
        }
    }

    pub(super) async fn create_page_reply_from_html_request_on_owner_local_store(
        &self,
        request: RendererCreateHtmlPageRequest,
        _owner_local_store: &mut RendererOwnerLocalStore,
    ) -> RenderRuntimeDispatchOutcome {
        let RendererCreateHtmlPageRequest {
            web_mcp_invocation,
            page_reservation,
            root_frame_id,
            main_document_commit,
            navigation_history,
            top_level_storage_key,
            requested_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            loader,
            navigator_identity,
            web_storage,
            final_url,
            html,
            document_start_scripts,
            runtime_bindings,
            runtime_inspector_session_restore_snapshots,
            runtime_isolated_worlds,
            permission_overrides,
            extra_http_headers,
            document_settings,
            network_offline,
            blocked_url_patterns,
            indexed_db_manager,
            storage_bucket_store,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            layout_configuration,
            wpt_extensions_enabled,
            stage,
            reply_boundary,
            lifecycle_decider,
            top_level_navigation_dispatch,
            reserved_service_worker_client,
        } = request;
        if page_reservation.local_host_id() != self.state.owner_local_host_id {
            return Err(anyhow!(
                "page reservation belongs to renderer owner {}, not {}",
                page_reservation.local_host_id().as_u64(),
                self.state.owner_local_host_id.as_u64()
            ))
            .into();
        }
        if lifecycle_decider.is_some()
            && (!matches!(reply_boundary, crate::RendererReplyBoundary::Stage)
                || !matches!(
                    top_level_navigation_dispatch,
                    RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter
                ))
        {
            return Err(anyhow!(
                "a lifecycle decider requires a standalone lifecycle-boundary page creation"
            ))
            .into();
        }
        let loader = loader_for_new_document(
            &loader,
            &extra_http_headers,
            network_offline,
            &blocked_url_patterns,
        );
        if response_headers_indicate_download(&response_headers) {
            return Err(anyhow!(
                "external raw streaming page request received download headers; CDP navigation must branch downloads before renderer page creation"
            ))
            .into();
        }
        let document_policy_container = DocumentPolicyContainer::from_navigation_response_headers(
            &response_headers,
            &final_url,
        )
        .with_content_security_policy_bypass(document_settings.bypass_content_security_policy);
        let document_default_language =
            crate::document_language::document_default_language_from_headers(&response_headers);
        let document_last_modified =
            crate::document_last_modified::document_last_modified_from_headers(&response_headers);
        let owner = self.clone();
        let phase_one_result = self
            .run_owner_lane_local_task(async move {
                let page_id = page_reservation.page_id();
                let owner_local_context = owner.owner_local_context()?;
                let owner_wake = owner.owner_wake_sender_for_page(&owner_local_context, page_id);
                let renderer_document_isolate_allocator =
                    RendererDocumentIsolateAllocator::new(owner_local_context.clone(), page_id);
                let runtime_hooks = PageVmRuntimeHooks::with_owner_wake(
                    owner_wake,
                    owner.state.browser_context_runtime.clone(),
                )
                .with_renderer_document_isolate_allocator(renderer_document_isolate_allocator);
                let runtime_hooks = runtime_hooks.with_web_mcp_navigation(web_mcp_invocation);
                let local_executor = owner.state.local_executor.clone();
                debug!(stage = ?stage, %final_url, "starting page VM creation from html");
                let staged_auxiliary_page =
                    take_staged_auxiliary_page_on_bound_owner_local_store(page_reservation);
                let reserved_service_worker_client_id = if staged_auxiliary_page.is_some() {
                    // Adoption keeps the initial Document and its resource authority.
                    // Release the unused provisional navigation client through its guard.
                    drop(reserved_service_worker_client);
                    None
                } else {
                    reserved_service_worker_client.map(RendererReservedServiceWorkerClient::release)
                };
                let mut env = PageVmEnvConfig {
                    web_storage,
                    document_start_scripts,
                    runtime_bindings,
                    runtime_inspector_session_restore_snapshots,
                    runtime_isolated_worlds,
                    permission_overrides,
                    extra_http_headers,
                    navigator_identity,
                    document_policy_container,
                    document_default_language,
                    document_last_modified,
                    document_settings,
                    network_offline,
                    blocked_url_patterns,
                    indexed_db_manager,
                    storage_bucket_store,
                    fetch_subresource_interception_enabled,
                    fetch_subresource_interception_resource_type,
                    layout_configuration,
                    wpt_extensions_enabled,
                    root_frame_id,
                    main_document_commit,
                    top_level_storage_key,
                    navigation_bootstrap_entry: navigation_history
                        .as_ref()
                        .map(|request| request.resolve(&final_url))
                        .transpose()?,
                    navigation_history_source: navigation_history
                        .as_ref()
                        .map(|request| request.source_history()),
                    reserved_service_worker_client_id,
                };
                env.apply_main_document_commit_referrer();
                if let Some(mut page_vm) = staged_auxiliary_page {
                    ensure!(
                        moli_url::is_about_blank(&final_url),
                        "a staged auxiliary Page only accepts its initial blank adoption"
                    );
                    let started = Instant::now();
                    page_vm.adopt_initial_auxiliary_page(&loader, &env)?;
                    return Ok(if page_vm.vm_mut().has_pending_location_navigation() {
                        ParseTimePageVmCreationOutcome::TriggeredNavigation { page_vm, stage }
                    } else {
                        ParseTimePageVmCreationOutcome::ContinuePhaseTwo {
                            page_vm,
                            page_tasks: Vec::new(),
                            stage,
                            started,
                        }
                    });
                }
                let bootstrap = Box::pin(async move {
                    let started = Instant::now();
                    ConcurrentParseTimeRuntime::finish_creation_from_html_bootstrap(
                        page_id,
                        local_executor,
                        &loader,
                        &env,
                        runtime_hooks,
                        final_url,
                        stage,
                        html,
                        started,
                    )
                    .await
                });
                PageVm::run_bootstrap_future_on_fresh_local_task(
                    owner.state.local_executor.clone(),
                    "create-page bootstrap local task channel closed",
                    bootstrap,
                )
                .await
            })
            .await;
        match phase_one_result {
            Ok(ParseTimePageVmCreationOutcome::PendingPhaseOne(residence))
                if !matches!(&residence, PendingPhaseOneResidence::OpenStreaming(_)) =>
            {
                self.continue_pending_phase_one_page_creation(
                    requested_url,
                    navigation_initiator_url,
                    navigation_redirected,
                    navigation_redirect_count,
                    response_status,
                    response_headers,
                    PageVmPendingPhaseOneNavigation::new(
                        residence,
                        PageVmFollowedNavigationMetadata::default(),
                    ),
                    stage,
                    reply_boundary,
                    lifecycle_decider,
                    top_level_navigation_dispatch,
                    page_creation_navigation_reply_policy(top_level_navigation_dispatch),
                )
                .await
            }
            Ok(ParseTimePageVmCreationOutcome::PendingPhaseOne(_)) => Err(anyhow!(
                "full-body page creation cannot retain an open Document stream"
            ))
            .into(),
            Ok(ParseTimePageVmCreationOutcome::TriggeredNavigation { page_vm, stage }) => {
                self.continue_page_creation_with_pending_navigation(
                    requested_url,
                    navigation_initiator_url,
                    navigation_redirected,
                    navigation_redirect_count,
                    response_status,
                    response_headers,
                    page_vm,
                    stage,
                    reply_boundary,
                    lifecycle_decider,
                    top_level_navigation_dispatch,
                    page_creation_navigation_reply_policy(top_level_navigation_dispatch),
                )
                .await
            }
            Ok(ParseTimePageVmCreationOutcome::ContinuePhaseTwo {
                page_vm,
                page_tasks,
                stage,
                started,
            }) => RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                RenderRuntimeTurn::FinishHtmlCreatePage {
                    requested_url,
                    navigation_initiator_url,
                    navigation_redirected,
                    navigation_redirect_count,
                    response_status,
                    response_headers,
                    page_vm: Box::new(page_vm),
                    page_tasks,
                    stage,
                    started,
                    reply_boundary,
                    lifecycle_decider,
                    top_level_navigation_dispatch,
                    navigation_reply_policy: page_creation_navigation_reply_policy(
                        top_level_navigation_dispatch,
                    ),
                },
            )),
            Err(error) => Err(error).into(),
        }
    }

    pub(super) async fn prepare_renderer_document_on_owner_local_store(
        &self,
        token: RendererPageReservationToken,
        request: RendererCreateStreamingRawPageRequest,
        owner_local_store: &mut RendererOwnerLocalStore,
    ) -> RenderRuntimeDispatchOutcome {
        if token.local_host_id() != self.state.owner_local_host_id {
            return Err(anyhow!(
                "prepared document belongs to renderer owner {}, not {}",
                token.local_host_id().as_u64(),
                self.state.owner_local_host_id.as_u64()
            ))
            .into();
        }
        if token.replacement.is_some()
            && let Err(error) = owner_local_store.validate_live_page_replacement_reservation(token)
        {
            return Err(error).into();
        }
        let owner = self.clone();
        let residence = self
            .run_owner_lane_local_task(async move {
                let owner_local_context = owner.owner_local_context()?;
                let owner_wake =
                    owner.owner_wake_sender_for_page(&owner_local_context, token.page_id());
                let page_runtime_task_source =
                    crate::page_task_queue::PageRuntimeTaskSource::new(Some(owner_wake));
                let isolate_allocator =
                    RendererDocumentIsolateAllocator::new(owner_local_context, token.page_id());
                let (isolate_bootstrap, isolate_reservation) = isolate_allocator
                    .reserve_renderer_document_isolate(page_runtime_task_source)?;
                Ok(RendererPreparedDocumentResidence {
                    request,
                    isolate_allocator,
                    isolate_bootstrap,
                    isolate_reservation,
                })
            })
            .await;
        match residence {
            Ok(residence) => {
                let renderer_devtools_agent_token =
                    residence.isolate_bootstrap.renderer_devtools_agent_token();
                match owner_local_store.store_prepared_document(token, residence) {
                    Ok(()) => Ok(RendererOwnerReply::PreparedRendererDocumentStored {
                        renderer_devtools_agent_token,
                    })
                    .into(),
                    Err(error) => Err(error).into(),
                }
            }
            Err(error) => Err(error).into(),
        }
    }

    pub(super) async fn create_page_reply_from_prepared_document_on_owner_local_store(
        &self,
        page_id: PageId,
        residence: RendererPreparedDocumentResidence,
        owner_local_store: &mut RendererOwnerLocalStore,
    ) -> RenderRuntimeDispatchOutcome {
        let RendererPreparedDocumentResidence {
            request,
            isolate_allocator,
            isolate_bootstrap,
            isolate_reservation,
        } = residence;
        self.create_page_reply_from_streaming_raw_request_on_owner_local_store(
            page_id,
            request,
            isolate_allocator,
            isolate_bootstrap,
            isolate_reservation,
            owner_local_store,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn create_page_reply_from_streaming_raw_request_on_owner_local_store(
        &self,
        page_id: PageId,
        request: RendererCreateStreamingRawPageRequest,
        isolate_allocator: RendererDocumentIsolateAllocator,
        isolate_bootstrap: RendererDocumentIsolateBootstrap,
        isolate_reservation: RendererDocumentIsolateReservation,
        _owner_local_store: &mut RendererOwnerLocalStore,
    ) -> RenderRuntimeDispatchOutcome {
        let mut request = request;
        if let Err(error) = request.validate_bootstrap_configuration() {
            return Err(error).into();
        }
        let requested_url = request.requested_url.clone();
        let navigation_initiator_url = request.navigation_initiator_url.clone();
        let navigation_redirected = request.navigation_redirected;
        let navigation_redirect_count = request.navigation_redirect_count;
        let navigation_redirect_chain = request.navigation_redirect_chain.clone();
        let stage = request.stage;
        let reply_boundary = request.reply_boundary;
        let lifecycle_decider = request.lifecycle_decider.take();
        let top_level_navigation_dispatch = request.top_level_navigation_dispatch;
        let navigation_reply_policy = request.navigation_reply_policy;
        let owner = self.clone();
        let phase_one_result = self
            .run_owner_lane_local_task(async move {
                let owner_local_context = owner.owner_local_context()?;
                let owner_wake = owner.owner_wake_sender_for_page(&owner_local_context, page_id);
                let runtime_hooks = PageVmRuntimeHooks::with_owner_wake(
                    owner_wake,
                    owner.state.browser_context_runtime.clone(),
                )
                .with_renderer_document_isolate_allocator(isolate_allocator)
                .with_prepared_renderer_document_isolate(isolate_bootstrap, isolate_reservation)?;
                request
                    .bootstrap(page_id, owner.state.local_executor.clone(), runtime_hooks)
                    .await
            })
            .await;
        match phase_one_result {
            Ok(StreamingNavigationPageCreationResult::Download(_)) => Err(anyhow!(
                "external raw streaming page request produced a download; CDP navigation must branch downloads before renderer page creation"
            ))
            .into(),
            Ok(StreamingNavigationPageCreationResult::Html(result)) => {
                let StreamingHtmlPageCreationResult {
                    response_status,
                    response_headers,
                    outcome,
                } = *result;
                match outcome {
                    ParseTimePageVmCreationOutcome::PendingPhaseOne(residence) => {
                        let committed_navigation_response = Some(PageVmNavigationResponse {
                            requested_url: requested_url.clone(),
                            redirected: navigation_redirected,
                            redirect_count: navigation_redirect_count,
                            redirect_chain: navigation_redirect_chain.clone(),
                            status: response_status,
                            headers: response_headers.clone(),
                        });
                        self.continue_pending_phase_one_page_creation(
                            requested_url,
                            navigation_initiator_url,
                            navigation_redirected,
                            navigation_redirect_count,
                            response_status,
                            response_headers,
                            PageVmPendingPhaseOneNavigation::new(
                                residence,
                                PageVmFollowedNavigationMetadata {
                                    committed_navigation_response,
                                    ..Default::default()
                                },
                            ),
                            stage,
                            reply_boundary,
                            lifecycle_decider,
                            top_level_navigation_dispatch,
                            navigation_reply_policy,
                        )
                        .await
                    }
                    ParseTimePageVmCreationOutcome::TriggeredNavigation {
                        mut page_vm,
                        stage,
                    } => {
                        page_vm::attach_navigation_response_to_page_vm(
                            &mut page_vm,
                            PageVmNavigationResponse {
                                requested_url: requested_url.clone(),
                                redirected: navigation_redirected,
                                redirect_count: navigation_redirect_count,
                                redirect_chain: navigation_redirect_chain.clone(),
                                status: response_status,
                                headers: response_headers.clone(),
                            },
                        );
                        self.continue_page_creation_with_pending_navigation(
                            requested_url,
                            navigation_initiator_url,
                            navigation_redirected,
                            navigation_redirect_count,
                            response_status,
                            response_headers,
                            page_vm,
                            stage,
                            reply_boundary,
                            lifecycle_decider,
                            top_level_navigation_dispatch,
                            navigation_reply_policy,
                        )
                        .await
                    }
                    ParseTimePageVmCreationOutcome::ContinuePhaseTwo {
                        mut page_vm,
                        page_tasks,
                        stage,
                        started,
                    } => {
                        page_vm::attach_navigation_response_to_page_vm(
                            &mut page_vm,
                            PageVmNavigationResponse {
                                requested_url: requested_url.clone(),
                                redirected: navigation_redirected,
                                redirect_count: navigation_redirect_count,
                                redirect_chain: navigation_redirect_chain,
                                status: response_status,
                                headers: response_headers.clone(),
                            },
                        );
                        RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                            RenderRuntimeTurn::FinishHtmlCreatePage {
                                requested_url,
                                navigation_initiator_url,
                                navigation_redirected,
                                navigation_redirect_count,
                                response_status,
                                response_headers,
                                page_vm: Box::new(page_vm),
                                page_tasks,
                                stage,
                                started,
                                reply_boundary,
                                lifecycle_decider,
                                top_level_navigation_dispatch,
                                navigation_reply_policy,
                            },
                        ))
                    }
                }
            }
            Err(error) => Err(error).into(),
        }
    }

    pub fn record(&self, page_id: PageId) -> Option<RendererPageRecord> {
        self.state.page_table.record(page_id)
    }

    pub fn len(&self) -> usize {
        self.state.page_table.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn command_epoch(&self, page_id: PageId) -> Option<u64> {
        self.state.page_table.command_epoch(page_id)
    }

    pub fn in_flight_command_epoch(&self, page_id: PageId) -> Option<u64> {
        self.state.page_table.in_flight_command_epoch(page_id)
    }
}
