use super::super::{ChildBrowsingContextBootstrap, JsContextHost, OwnerDispatchScope};
use super::{
    ChildDocumentNavigationInitiator, PendingChildDocumentNavigation,
    configure_child_document_navigation_request,
    snapshots::{child_document_content_type_for_url, child_document_content_type_from_headers},
};
#[cfg(test)]
use crate::referrer_policy::response_referrer_policy_from_headers;
use crate::{
    content_security_policy::{
        ContentSecurityPolicyViolationEventFields, send_content_security_policy_reports,
    },
    document_runtime::{DocumentNavigationEmbeddingContext, DocumentPolicyContainer, DomHandle},
    document_script_scheduler::FrameDocumentClassicScriptSchedulerWork,
    frame_owner_model::{
        ChildDocumentNavigationFetchTarget, DocumentCreationKind,
        FrameDocumentInteractiveLifecycleAction, FrameRequestKind,
    },
    types::{
        ChildDocumentLoadCompletion, ChildDocumentLoadNetworkAttribution, ChildDocumentLoadOutcome,
        LoadedChildDocument, SubresourceResponseBody,
    },
};
use moli_encoding::decode_html_document_with_fallback;

pub(crate) struct AppliedChildDocumentLoadCompletion {
    /// Initial parser-classic work produced by the committed child document.
    /// The ScriptVm owner queues this directly into the document-script
    /// scheduler; it should not round-trip through the host ready-input bridge.
    initial_classic_ready_work: Option<FrameDocumentClassicScriptSchedulerWork>,
    parser_stop_action: Option<FrameDocumentInteractiveLifecycleAction>,
    owner_transition: Option<crate::frame_owner_model::FrameDocumentOwnerTransition>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ChildDocumentLoadBodyActivity {
    NoPageCodeOrEventDispatch,
    PageCodeOrEventDispatch,
}

pub(crate) enum ChildDocumentLoadApplication {
    Applied {
        followup: Option<Box<AppliedChildDocumentLoadCompletion>>,
        body_activity: ChildDocumentLoadBodyActivity,
    },
    /// The terminal was authorized before entering JS, but an unload handler
    /// replaced its navigation target. The returned terminal may still carry a
    /// historical Network fact, but must not commit or clean up the replacement.
    SupersededDuringApplication {
        completion: ChildDocumentLoadCompletion,
        body_activity: ChildDocumentLoadBodyActivity,
    },
}

impl AppliedChildDocumentLoadCompletion {
    fn new(
        initial_classic_ready_work: Option<FrameDocumentClassicScriptSchedulerWork>,
        parser_stop_action: Option<FrameDocumentInteractiveLifecycleAction>,
        owner_transition: Option<crate::frame_owner_model::FrameDocumentOwnerTransition>,
    ) -> Self {
        Self {
            initial_classic_ready_work,
            parser_stop_action,
            owner_transition,
        }
    }

    pub(crate) fn into_followups(
        self,
    ) -> (
        Option<FrameDocumentClassicScriptSchedulerWork>,
        Option<FrameDocumentInteractiveLifecycleAction>,
        Option<crate::frame_owner_model::FrameDocumentOwnerTransition>,
    ) {
        (
            self.initial_classic_ready_work,
            self.parser_stop_action,
            self.owner_transition,
        )
    }
}

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn clear_pending_child_document_loads_for_handle(
        &mut self,
        handle: DomHandle,
    ) {
        let pending_ids = self
            .pending_child_document_navigations
            .iter()
            .filter_map(|(load_id, pending)| {
                (pending.target.child_handle() == handle).then_some(*load_id)
            })
            .collect::<Vec<_>>();
        for load_id in pending_ids {
            if let Some(pending) = self.pending_child_document_navigations.remove(&load_id) {
                self.finish_pending_child_document_navigation_owner_request(&pending);
                let _ = self.settle_child_navigation_load(
                    handle,
                    pending.target.navigation_load(),
                    false,
                );
            }
        }
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.clear_pending_document_load();
        }
    }

    pub(in crate::native_bridge::context_host::child_documents) fn start_child_document_load(
        &mut self,
        handle: DomHandle,
        bootstrap: ChildBrowsingContextBootstrap,
        navigation_load: crate::frame_owner_model::FrameDocumentNavigationLoadBinding,
        initiator: ChildDocumentNavigationInitiator,
    ) -> Option<u64> {
        let target_url = Self::child_browsing_context_bootstrap_url(&bootstrap)?;
        let initiating_loader = self.document_resource_loader_for_owner(navigation_load.owner())?;
        let resource_loader =
            crate::network::navigation::NavigationResourceLoader::new_for_child_document(
                initiating_loader.request_client().clone(),
                target_url.clone(),
                initiating_loader.task_runner(),
            );
        let frame_id = self
            .child_browsing_contexts
            .get(&handle)?
            .frame_id()
            .to_owned();
        let parent_frame_id = self.child_browsing_context_parent_frame_id(handle);
        let completion_tx = self.resource_completion_tx.clone();
        let load_id = self.next_child_document_load_id;
        self.next_child_document_load_id = self.next_child_document_load_id.wrapping_add(1);
        let (owner_document_id, owner_request_id) = self
            .frame_owner_store
            .begin_child_document_request(handle)?;
        if owner_document_id != navigation_load.owner().document_id {
            let _ = self
                .frame_owner_store
                .finish_document_request(owner_document_id, owner_request_id);
            tracing::warn!(
                ?handle,
                ?navigation_load,
                ?owner_document_id,
                "refusing child document fetch whose request owner differs from its navigation binding"
            );
            return None;
        }
        let target = ChildDocumentNavigationFetchTarget::new(
            handle,
            load_id,
            navigation_load,
            owner_request_id,
        );
        let loader_id = self.allocate_child_document_loader_id();
        let network_attribution =
            ChildDocumentLoadNetworkAttribution::new(frame_id, parent_frame_id, loader_id);
        self.note_child_frame_load_started_for_parent(handle);
        let owner_credentialless = self
            .child_browsing_contexts
            .get(&handle)
            .is_some_and(|entry| entry.owner_credentialless());
        let document_credentialless = self
            .child_browsing_context_document_credentialless_for_owner(handle, owner_credentialless);
        let credentialless_storage_nonce =
            self.child_document_credentialless_storage_nonce(document_credentialless);
        let network_partition_key = self.child_browsing_context_navigation_network_partition_key(
            handle,
            &bootstrap,
            credentialless_storage_nonce,
        );
        let browser_context_runtime = self.browser_context_runtime();
        let service_worker_client_id = self
            .child_browsing_contexts
            .get(&handle)
            .and_then(|entry| entry.pending_service_worker_client_id());
        let frame_owner_resource_timing =
            self.pending_frame_owner_resource_timing(handle, &target_url, initiator);
        let initiator_url = self.document_url_for_child_context(handle);
        let browser_context = self.host_document().cookie_browser_context();
        self.pending_child_document_navigations.insert(
            load_id,
            PendingChildDocumentNavigation {
                target,
                target_url: target_url.clone(),
                resource_loader: resource_loader.clone(),
                reserved_service_worker_client_id: service_worker_client_id,
                document_credentialless,
                credentialless_storage_nonce,
                frame_owner_resource_timing,
            },
        );
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.mark_pending_document_load(load_id);
        }
        let parent_character_set = self.document_character_set().to_owned();
        let task_resource_loader = resource_loader.clone();
        resource_loader.spawn_resource_task(async move {
            let result = async {
                let request = configure_child_document_navigation_request(
                    child_document_load_request(bootstrap).ok_or_else(|| {
                        "unsupported child document navigation bootstrap".to_owned()
                    })?,
                    &initiator_url,
                    &browser_context,
                )
                .with_page_network_policy()
                .with_network_partition_key(network_partition_key);
                let request_url = request.url.as_str().to_owned();
                let request_method = request.method.clone();
                let request_headers = request.request_headers.clone();
                if let Some(client_id) = service_worker_client_id
                    && let Some(response) = browser_context_runtime
                        .fetch_service_worker_child_main_resource_for_reserved_client(
                            client_id,
                            &request,
                            task_resource_loader.request_client(),
                            task_resource_loader.task_runner(),
                            completion_tx.clone(),
                        )
                        .await
                        .map_err(|error| error.to_string())?
                {
                    task_resource_loader
                        .note_service_worker_response_ready()
                        .map_err(|error| error.to_string())?;
                    let (head, body) = response.into_body();
                    return child_document_load_outcome_from_response(
                        request_url,
                        request_method,
                        request_headers,
                        head,
                        body,
                        &parent_character_set,
                    );
                }
                let response = task_resource_loader
                    .fetch_raw(request)
                    .await
                    .map_err(|error| error.to_string())?;
                let (head, body) = response.into_body();
                child_document_load_outcome_from_response(
                    request_url,
                    request_method,
                    request_headers,
                    head,
                    body,
                    &parent_character_set,
                )
            }
            .await;
            let _ = completion_tx.send_child_document(ChildDocumentLoadCompletion::new(
                target,
                network_attribution,
                result,
            ));
        });
        Some(load_id)
    }

    pub(crate) fn has_pending_child_document_loads(&self) -> bool {
        !self.pending_child_document_navigations.is_empty()
    }

    pub(crate) fn has_pending_child_document_lifecycle(&self) -> bool {
        self.has_pending_child_document_loads()
            // Reserving a navigation commit ends resource preparation, but
            // the current Document is not replaced until the frame-lane task
            // runs. Lifecycle observers must keep waiting across that gap.
            || self.has_pending_child_navigation_commit_task()
            || self
                .frame_owner_store
                .has_pending_current_child_document_lifecycle()
    }

    pub(in crate::native_bridge::context_host) fn child_document_load_is_pending(
        &self,
        handle: DomHandle,
    ) -> bool {
        self.child_browsing_contexts
            .get(&handle)
            .and_then(|entry| entry.pending_document_load_id())
            .is_some_and(|load_id| {
                self.pending_child_document_navigations
                    .contains_key(&load_id)
            })
    }

    pub(crate) fn current_child_document_navigation_fetch_target(
        &self,
        handle: DomHandle,
    ) -> Option<ChildDocumentNavigationFetchTarget> {
        if !self.child_browsing_context_host_is_active(handle) {
            return None;
        }
        let entry = self.child_browsing_contexts.get(&handle)?;
        let load_id = entry.pending_document_load_id()?;
        if !entry.pending_document_navigation_owner_is_current(load_id) {
            return None;
        }
        let pending = self.pending_child_document_navigations.get(&load_id)?;
        let target = pending.target;
        if target.child_handle() != handle
            || self.current_child_navigation_load(handle) != Some(target.navigation_load())
            || !self.frame_owner_store.document_request_is_current(
                target.task_owner().document_id,
                target.request_id(),
                FrameRequestKind::DocumentNavigation,
            )
        {
            return None;
        }
        Some(target)
    }

    pub(crate) fn record_historical_child_document_load_network(
        &mut self,
        completion: &ChildDocumentLoadCompletion,
    ) -> bool {
        let Some(snapshot) = completion.document_network().cloned() else {
            return false;
        };
        let attribution = completion.network_attribution();
        let event = crate::protocol_types::ChildFrameDocumentNetworkActivitySnapshot {
            frame_id: attribution.frame_id().to_owned(),
            parent_frame_id: attribution.parent_frame_id().map(ToOwned::to_owned),
            loader_id: attribution.loader_id().to_owned(),
            snapshot,
        };
        if let Some(source_document) = self.root_document_lifecycle_identity()
            && self.append_live_turn_owner_action(
                crate::runtime::RendererOwnerAction::ChildFrameDocumentNetwork {
                    source_document,
                    event: event.clone(),
                },
            )
        {
            return true;
        }
        #[cfg(test)]
        {
            self.completed_child_document_networks.push(event);
            true
        }
        #[cfg(not(test))]
        {
            let _ = event;
            panic!(
                "a production child Document network event must have a concrete renderer output sink"
            );
        }
    }

    pub(crate) fn discard_stale_child_document_load_completion(
        &mut self,
        target: ChildDocumentNavigationFetchTarget,
    ) {
        let Some(pending) = self
            .pending_child_document_navigations
            .get(&target.load_id())
            .filter(|pending| pending.target == target)
            .cloned()
        else {
            return;
        };
        self.pending_child_document_navigations
            .remove(&target.load_id());
        self.finish_pending_child_document_navigation_owner_request(&pending);
        self.clear_child_browsing_context_pending_document_load_if_matches(
            target.child_handle(),
            target.load_id(),
        );
        self.clear_pending_service_worker_child_client_if_matches(
            target.child_handle(),
            pending.reserved_service_worker_client_id,
        );
        let _ = self.finish_child_frame_navigation_without_load_dispatch(
            target.child_handle(),
            target.navigation_load(),
        );
    }

    pub(crate) fn apply_current_child_document_load_completion(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        completion: ChildDocumentLoadCompletion,
    ) -> ChildDocumentLoadApplication {
        let (target, network_attribution, result) = completion.into_application_parts();
        let loader_id = network_attribution.loader_id().to_owned();
        let mut pending = self
            .pending_child_document_navigations
            .remove(&target.load_id())
            .expect("authorized child document terminal must retain its pending navigation");
        assert_eq!(
            pending.target, target,
            "authorized child document terminal must consume its exact pending navigation"
        );
        self.finish_pending_child_document_navigation_owner_request(&pending);
        let handle = target.child_handle();
        self.clear_child_browsing_context_pending_document_load_if_matches(
            handle,
            target.load_id(),
        );
        let result = match result {
            Ok(ChildDocumentLoadOutcome::Loaded(mut loaded)) => {
                if self.bypass_content_security_policy() {
                    loaded
                        .policy_container
                        .clear_content_security_policy_for_bypass();
                }
                let ancestor_origins = self.child_document_frame_ancestor_origins(handle);
                let (report_only_violation, enforced_violation) = loaded
                    .policy_container
                    .navigation_response_frame_ancestors_check(
                        &loaded.final_url,
                        DocumentNavigationEmbeddingContext::Nested(&ancestor_origins),
                    )
                    .into_violations();
                for violation in report_only_violation
                    .iter()
                    .chain(enforced_violation.iter())
                {
                    // The protected response never receives a Document when enforcement blocks
                    // it, so there is no target on which to dispatch a DOM event. Keep the
                    // violation observable to DevTools on the embedding frame owner instead.
                    // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
                    unsafe { &mut *self.runtime }
                        .record_content_security_policy_inspector_issue(Some(handle), violation);
                    let fields =
                        ContentSecurityPolicyViolationEventFields::from_url_violation(violation);
                    send_content_security_policy_reports(
                        pending.resource_loader.request_client(),
                        &fields,
                        &violation.report_uri_endpoints,
                        &violation.report_to_endpoints,
                    );
                }
                if let Some(violation) = enforced_violation {
                    Err(format!(
                        "child document response blocked by Content Security Policy `{}` for `{}`",
                        violation.effective_directive, violation.blocked_uri
                    ))
                } else {
                    Ok(ChildDocumentLoadOutcome::Loaded(loaded))
                }
            }
            result => result,
        };
        let document_credentialless = pending.document_credentialless;
        let credentialless_storage_nonce = pending.credentialless_storage_nonce;
        let replaces_existing_document = self
            .child_browsing_context_document_handle(handle)
            .is_some()
            || self
                .child_browsing_contexts
                .get(&handle)
                .is_some_and(|entry| entry.has_cached_snapshot());
        let window_commit_preflight = self.capture_child_document_window_commit_preflight(handle);
        let mut completed_document_network = None;
        let mut body_activity = ChildDocumentLoadBodyActivity::NoPageCodeOrEventDispatch;
        let snapshot_to_install = match result {
            Ok(ChildDocumentLoadOutcome::IgnoredNavigation) => {
                self.clear_child_browsing_context_pending_navigation(handle);
                if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
                    entry.restore_navigation_entry_seed_from_committed();
                    entry.clear_pending_top_level_history_length_increment();
                }
                self.sync_existing_child_browsing_context_window_state(scope, handle);
                self.reject_replaced_service_worker_child_client_navigation(
                    handle,
                    "Cannot navigate to URL.".to_owned(),
                );
                let _ = self.finish_child_frame_navigation_without_load_dispatch(
                    handle,
                    target.navigation_load(),
                );
                return ChildDocumentLoadApplication::Applied {
                    followup: None,
                    body_activity,
                };
            }
            Ok(ChildDocumentLoadOutcome::Loaded(loaded)) => {
                let loaded = *loaded;
                if self.dispatch_child_browsing_context_unload_lifecycle_if_needed(scope, handle) {
                    body_activity = ChildDocumentLoadBodyActivity::PageCodeOrEventDispatch;
                }
                if !self.child_document_window_commit_preflight_is_current(
                    handle,
                    &window_commit_preflight,
                ) || self.current_child_navigation_load(handle) != Some(target.navigation_load())
                {
                    self.clear_pending_service_worker_child_client_if_matches(
                        handle,
                        pending.reserved_service_worker_client_id,
                    );
                    let _ = self.finish_child_frame_navigation_without_load_dispatch(
                        handle,
                        target.navigation_load(),
                    );
                    return ChildDocumentLoadApplication::SupersededDuringApplication {
                        completion: ChildDocumentLoadCompletion::new(
                            target,
                            network_attribution,
                            Ok(ChildDocumentLoadOutcome::Loaded(Box::new(loaded))),
                        ),
                        body_activity,
                    };
                }
                let sandbox = self
                    .child_browsing_context_sandbox_policy_from_owner(handle)
                    .with_response_content_security_policy(loaded.policy_container.sandbox);
                let final_url = child_document_final_url_with_request_fragment(
                    loaded.final_url.clone(),
                    &pending.target_url,
                );
                self.clear_child_browsing_context_pending_navigation(handle);
                let Some(entry) = self.child_browsing_contexts.get_mut(&handle) else {
                    self.clear_pending_service_worker_child_client_if_matches(
                        handle,
                        pending.reserved_service_worker_client_id,
                    );
                    let _ = self.finish_child_frame_navigation_without_load_dispatch(
                        handle,
                        target.navigation_load(),
                    );
                    return ChildDocumentLoadApplication::SupersededDuringApplication {
                        completion: ChildDocumentLoadCompletion::new(
                            target,
                            network_attribution,
                            Ok(ChildDocumentLoadOutcome::Loaded(Box::new(loaded))),
                        ),
                        body_activity,
                    };
                };
                entry.commit_pending_child_document_load(
                    &final_url,
                    &loaded.policy_container,
                    sandbox,
                    document_credentialless,
                    credentialless_storage_nonce,
                );
                let resource_was_cached = loaded
                    .document_network
                    .as_ref()
                    .is_some_and(|network| network.from_cache);
                completed_document_network = loaded.document_network;
                let snapshot = super::super::ChildBrowsingContextSnapshot::with_character_set(
                    final_url,
                    loaded.markup,
                    loaded.content_type,
                    loaded.character_set,
                )
                .with_resource_was_cached(resource_was_cached);
                self.cache_child_snapshot_with_current_document_policy(handle, snapshot)
            }
            Err(error) => {
                if self.dispatch_child_browsing_context_unload_lifecycle_if_needed(scope, handle) {
                    body_activity = ChildDocumentLoadBodyActivity::PageCodeOrEventDispatch;
                }
                if !self.child_document_window_commit_preflight_is_current(
                    handle,
                    &window_commit_preflight,
                ) || self.current_child_navigation_load(handle) != Some(target.navigation_load())
                {
                    self.clear_pending_service_worker_child_client_if_matches(
                        handle,
                        pending.reserved_service_worker_client_id,
                    );
                    let _ = self.finish_child_frame_navigation_without_load_dispatch(
                        handle,
                        target.navigation_load(),
                    );
                    return ChildDocumentLoadApplication::SupersededDuringApplication {
                        completion: ChildDocumentLoadCompletion::new(
                            target,
                            network_attribution,
                            Err(error),
                        ),
                        body_activity,
                    };
                }
                tracing::debug!(
                    ?handle,
                    url = %pending.target_url,
                    error,
                    "child document load failed"
                );
                self.clear_child_browsing_context_pending_navigation(handle);
                let Some(entry) = self.child_browsing_contexts.get_mut(&handle) else {
                    self.clear_pending_service_worker_child_client_if_matches(
                        handle,
                        pending.reserved_service_worker_client_id,
                    );
                    let _ = self.finish_child_frame_navigation_without_load_dispatch(
                        handle,
                        target.navigation_load(),
                    );
                    return ChildDocumentLoadApplication::SupersededDuringApplication {
                        completion: ChildDocumentLoadCompletion::new(
                            target,
                            network_attribution,
                            Err(error),
                        ),
                        body_activity,
                    };
                };
                entry.clear_cached_snapshot();
                entry.clear_completed_document_network();
                self.reject_replaced_service_worker_child_client_navigation(
                    handle,
                    format!("Cannot navigate to URL: {error}"),
                );
                let _ = self.finish_child_frame_navigation_without_load_dispatch(
                    handle,
                    target.navigation_load(),
                );
                None
            }
        };
        let mut initial_classic_ready_work = None;
        let mut parser_stop_action = None;
        let mut owner_transition = None;
        if let Some(snapshot) = snapshot_to_install.as_ref() {
            let window_commit = self.plan_child_document_window_commit(
                handle,
                snapshot,
                window_commit_preflight,
                DocumentCreationKind::Navigation,
                Some(loader_id),
            );
            let Some(install) = self.install_child_browsing_context_current_document_from_snapshot(
                scope,
                handle,
                snapshot,
                window_commit,
                false,
                Some(pending.resource_loader.clone()),
            ) else {
                return ChildDocumentLoadApplication::Applied {
                    followup: None,
                    body_activity,
                };
            };
            initial_classic_ready_work = install.initial_classic_ready_work;
            parser_stop_action = install.parser_stop_action;
            owner_transition = Some(install.owner_transition);
            if let Some(current_owner) = install.owner_transition.current_owner()
                && let Some(entry) = self.child_browsing_contexts.get_mut(&handle)
            {
                let frame_owner_resource_timing = pending
                    .frame_owner_resource_timing
                    .take()
                    .zip(completed_document_network.clone())
                    .map(|(timing, network)| timing.complete(current_owner, network));
                entry.bind_completed_frame_owner_resource_timing(frame_owner_resource_timing);
                entry.bind_completed_document_network(current_owner, completed_document_network);
            }
            self.promote_pending_service_worker_child_client(handle);
        } else {
            if replaces_existing_document {
                self.disconnect_shared_worker_clients_for_child_context(handle);
            }
            self.clear_child_window_document_event_state(scope, handle);
            self.replace_child_custom_elements_registry_for_document_commit(scope, handle);
            self.clear_child_browsing_context_current_document(handle);
            self.clear_pending_service_worker_child_client(handle);
        }
        self.register_or_update_service_worker_child_client(handle);
        self.complete_pending_service_worker_child_client_navigation(handle);
        self.sync_existing_child_browsing_context_window_state(scope, handle);
        if parser_stop_action.is_none() && initial_classic_ready_work.is_none() {
            let _ = self.queue_child_document_complete_lifecycle_if_ready(handle);
        }
        ChildDocumentLoadApplication::Applied {
            followup: Some(Box::new(AppliedChildDocumentLoadCompletion::new(
                initial_classic_ready_work,
                parser_stop_action,
                owner_transition,
            ))),
            body_activity,
        }
    }

    fn finish_pending_child_document_navigation_owner_request(
        &mut self,
        pending: &PendingChildDocumentNavigation,
    ) {
        let _ = self.frame_owner_store.finish_document_request(
            pending.target.task_owner().document_id,
            pending.target.request_id(),
        );
    }

    fn child_document_frame_ancestor_origins(&self, handle: DomHandle) -> Vec<Option<url::Url>> {
        let mut ancestors = Vec::new();
        let mut current = handle;
        for _ in 0..=self.child_browsing_contexts.len() {
            let parent = self.child_browsing_context_parent_handle(current);
            let owner_scope = match parent {
                Some(parent) => OwnerDispatchScope::Child(parent),
                None => self.child_browsing_context_popup_owner_id(current).map_or(
                    OwnerDispatchScope::Top,
                    OwnerDispatchScope::LightweightPopup,
                ),
            };
            let origin = self
                .window_access_origin_for_dispatch_scope(owner_scope)
                .and_then(|origin| {
                    let serialized = origin.serialized_origin();
                    (serialized != "null")
                        .then(|| url::Url::parse(&serialized).ok())
                        .flatten()
                });
            ancestors.push(origin);
            let Some(parent) = parent else {
                break;
            };
            current = parent;
        }
        ancestors
    }
}

fn child_document_fallback_character_set(
    parent_character_set: &str,
    content_type: Option<&str>,
) -> String {
    if content_type.is_some_and(moli_web_mime::is_dom_parser_xml_mime) {
        "UTF-8".to_owned()
    } else {
        parent_character_set.to_owned()
    }
}

fn child_document_load_outcome_from_response(
    request_url: String,
    request_method: String,
    request_headers: Vec<(String, String)>,
    head: moli_fetch::ResponseHead,
    body: moli_fetch::ResponseBody,
    parent_character_set: &str,
) -> Result<ChildDocumentLoadOutcome, String> {
    if child_document_response_should_ignore_navigation(head.status, &head.headers) {
        return Ok(ChildDocumentLoadOutcome::IgnoredNavigation);
    }
    let response_body = SubresourceResponseBody::from_bytes(
        body.try_into_materialized_bytes()
            .map_err(|_| "child document response body should remain materialized".to_owned())?,
    );
    let encoded_data_length = response_body.len();
    let content_type = child_document_content_type_from_headers(&head.headers)
        .or_else(|| child_document_content_type_for_url(&head.final_url));
    let fallback =
        child_document_fallback_character_set(parent_character_set, content_type.as_deref());
    let (markup, character_set) = {
        let body_bytes = response_body
            .try_bytes()
            .map_err(|error| format!("failed to read child document response body: {error}"))?;
        let (markup, character_set) =
            decode_html_document_with_fallback(&body_bytes, &head.headers, Some(&fallback));
        (markup, character_set.to_owned())
    };
    let policy_container =
        DocumentPolicyContainer::from_navigation_response_headers(&head.headers, &head.final_url);
    Ok(ChildDocumentLoadOutcome::Loaded(Box::new(
        LoadedChildDocument {
            final_url: head.final_url.clone(),
            policy_container,
            content_type,
            character_set,
            markup,
            document_network: Some(crate::protocol_types::ChildFrameDocumentNetworkSnapshot {
                request_url,
                request_method,
                request_headers,
                final_url: head.final_url.as_str().to_owned(),
                status: head.status,
                response_headers: head.headers.clone(),
                encoded_data_length,
                response_body: Some(response_body),
                from_cache: head.from_cache,
            }),
        },
    )))
}

#[cfg(test)]
fn response_referrer_policy(headers: &[(String, String)]) -> Option<String> {
    response_referrer_policy_from_headers(headers)
}

fn child_document_response_should_ignore_navigation(
    status: u16,
    headers: &[(String, String)],
) -> bool {
    matches!(status, 204 | 205)
        || moli_web_mime::response_headers_indicate_attachment_download(headers)
        || child_document_content_type_from_headers(headers)
            .is_some_and(|mime| !moli_web_mime::is_supported_document_mime_type(&mime))
}

fn child_document_final_url_with_request_fragment(
    mut final_url: url::Url,
    target_url: &url::Url,
) -> url::Url {
    if final_url.fragment().is_none() {
        final_url.set_fragment(target_url.fragment());
    }
    final_url
}

fn child_document_load_request(
    bootstrap: ChildBrowsingContextBootstrap,
) -> Option<moli_fetch::Request> {
    match bootstrap {
        ChildBrowsingContextBootstrap::Url(url) => Some(moli_fetch::Request::get_with_url(url)),
        ChildBrowsingContextBootstrap::Request(request) => moli_fetch::Request::new_bytes(
            &request.method,
            request.url.as_str(),
            request.body,
            request.request_headers,
        )
        .ok(),
        ChildBrowsingContextBootstrap::AboutBlank
        | ChildBrowsingContextBootstrap::Srcdoc { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loaded_child_document_retains_exact_network_response_body() {
        let body_bytes = b"<!doctype html><p>child network body \xff</p>".to_vec();
        let outcome = child_document_load_outcome_from_response(
            "https://example.test/child".to_owned(),
            "GET".to_owned(),
            Vec::new(),
            moli_fetch::ResponseHead {
                final_url: url::Url::parse("https://example.test/child").unwrap(),
                status: 200,
                headers: vec![("Content-Type".to_owned(), "text/html".to_owned())],
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                negotiated_http_version: None,
            },
            moli_fetch::ResponseBody::materialized_bytes(body_bytes.clone()),
            "UTF-8",
        )
        .expect("child document response should load");
        let ChildDocumentLoadOutcome::Loaded(document) = outcome else {
            panic!("successful HTML response should load a child document");
        };
        let network = document
            .document_network
            .as_ref()
            .expect("loaded child document should retain Network metadata");
        assert_eq!(network.encoded_data_length, body_bytes.len());
        assert_eq!(
            network
                .response_body
                .as_ref()
                .expect("loaded child document should retain its response body")
                .try_bytes()
                .unwrap()
                .as_ref(),
            body_bytes
        );
    }

    #[test]
    fn child_document_final_url_preserves_request_fragment() {
        let final_url = url::Url::parse("https://example.test/frame.html").unwrap();
        let target_url = url::Url::parse("https://example.test/frame.html#target").unwrap();

        assert_eq!(
            child_document_final_url_with_request_fragment(final_url, &target_url).as_str(),
            "https://example.test/frame.html#target"
        );
    }

    #[test]
    fn child_document_final_url_keeps_response_fragment() {
        let final_url = url::Url::parse("https://example.test/frame.html#response").unwrap();
        let target_url = url::Url::parse("https://example.test/frame.html#target").unwrap();

        assert_eq!(
            child_document_final_url_with_request_fragment(final_url, &target_url).as_str(),
            "https://example.test/frame.html#response"
        );
    }

    #[test]
    fn child_document_response_referrer_policy_uses_last_valid_token() {
        let headers = vec![(
            "Referrer-Policy".to_owned(),
            "not-yet-standardized, no-referrer".to_owned(),
        )];

        assert_eq!(
            response_referrer_policy(&headers),
            Some("no-referrer".to_owned())
        );
    }

    #[test]
    fn child_document_response_referrer_policy_combines_header_instances() {
        let headers = vec![
            ("Referrer-Policy".to_owned(), "no-referrer".to_owned()),
            ("referrer-policy".to_owned(), "future-policy".to_owned()),
        ];

        assert_eq!(
            response_referrer_policy(&headers),
            Some("no-referrer".to_owned())
        );
    }

    #[test]
    fn child_document_response_referrer_policy_ignores_invalid_later_header() {
        let headers = vec![
            ("Referrer-Policy".to_owned(), "no-referrer".to_owned()),
            (
                "Referrer-Policy".to_owned(),
                "not-yet-standardized".to_owned(),
            ),
        ];

        assert_eq!(
            response_referrer_policy(&headers),
            Some("no-referrer".to_owned())
        );
    }

    #[test]
    fn child_document_response_disposition_rejects_unhandled_download_mime() {
        for content_type in [
            "application/octet-stream",
            "application/pdf",
            "application/zip",
            "font/woff2",
        ] {
            assert!(child_document_response_should_ignore_navigation(
                200,
                &[("Content-Type".to_owned(), content_type.to_owned())],
            ));
        }
    }

    #[test]
    fn child_document_response_disposition_keeps_supported_documents() {
        for content_type in [
            "text/html",
            "text/plain",
            "application/xml",
            "application/json",
            "image/png",
            "video/webm",
        ] {
            assert!(!child_document_response_should_ignore_navigation(
                200,
                &[("Content-Type".to_owned(), content_type.to_owned())],
            ));
        }
        assert!(!child_document_response_should_ignore_navigation(200, &[]));
    }
}
