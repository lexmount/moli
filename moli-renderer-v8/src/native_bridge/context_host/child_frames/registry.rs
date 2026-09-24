use super::super::child_documents::ChildInitialEmptyDocumentInit;
use super::*;
use crate::custom_elements::{CustomElementRegistryAssociation, CustomElementRegistryKey};
use crate::document_script_scheduler::FrameDocumentClassicScriptSchedulerWork;

/// Identity captured before author callbacks. The DOM handle can be reused by
/// a newly attached browsing context while the old Window is being canceled.
struct ChildWindowRetirement {
    handle: DomHandle,
    frame_id: String,
    local_window: Option<crate::frame_owner_model::LocalWindowId>,
}

impl JsContextHost {
    fn remove_child_browsing_context_entry(
        &mut self,
        handle: DomHandle,
    ) -> Option<ChildBrowsingContextEntry> {
        self.session_histories
            .detach(super::super::OwnerDispatchScope::Child(handle));
        self.history_update_limits
            .remove(super::super::OwnerDispatchScope::Child(handle));
        self.child_browsing_contexts.shift_remove(&handle)
    }

    pub(crate) fn clear_child_browsing_context_current_document(
        &mut self,
        handle: DomHandle,
    ) -> Option<crate::frame_owner_model::FrameDocumentOwnerTransition> {
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.clear_current_document_loader_id();
            entry.clear_document_internal_ancestor_origins();
        }
        let transition = self.frame_owner_store.detach_current_child_document(handle);
        let retired_owner = transition.and_then(|item| item.retired_owner());
        self.remove_child_browsing_context_current_document_storage(handle, retired_owner);
        transition
    }

    pub(crate) fn take_pending_child_document_owner_retirements(
        &mut self,
    ) -> Vec<crate::frame_owner_model::FrameDocumentOwnerTransition> {
        self.frame_owner_store
            .take_pending_document_owner_retirements()
    }

    pub(in crate::native_bridge::context_host) fn retire_child_document_external_state(
        &mut self,
        handle: DomHandle,
        retired_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        document_handle: DomHandle,
    ) {
        self.claimed_child_histories.remove(&document_handle);
        let _ = self.retire_document_resource_loader(
            crate::native_bridge::WindowDocumentOwner::Frame(retired_owner),
        );
        self.cancel_stylesheet_subresource_fetches_for_document_owner(retired_owner);
        self.retire_image_state_for_document(document_handle);
        self.cancel_pending_media_loads_for_document(document_handle);
        self.cancel_pending_text_track_loads_for_document(document_handle);
        self.drop_child_browsing_context_subtree(document_handle);
        self.frame_document_blocking_stylesheets
            .remove_document(retired_owner.document_owner());
        self.cancel_child_document_script_work_for_owner(handle, retired_owner);
        self.retire_child_frame_realm_materialization_request(handle, retired_owner);
        self.note_style_subtree_context_change(document_handle);
        self.dom_host_mut()
            .mark_subtree_disconnected_preserving_owner_document(document_handle);
        // The retired child realm exposes no further rendered style surface.
        // Drop its heavyweight Stylist/source state after disconnecting the DOM;
        // the global retirement-generation barrier keeps stale
        // CSSStyleDeclaration wrappers invalid without retaining the world.
        self.style_engine
            .retire_document_style_world(document_handle);
    }

    fn remove_child_browsing_context_current_document_storage(
        &mut self,
        handle: DomHandle,
        retired_owner: Option<crate::frame_owner_model::FrameDocumentTaskOwner>,
    ) -> Option<DomHandle> {
        let document_handle = self
            .child_browsing_context_document_handles
            .remove(&handle)?;
        if let Some(owner) = retired_owner {
            self.retire_child_document_external_state(handle, owner, document_handle);
            return Some(document_handle);
        }
        self.retire_image_state_for_document(document_handle);
        self.cancel_pending_media_loads_for_document(document_handle);
        self.cancel_pending_text_track_loads_for_document(document_handle);
        self.drop_child_browsing_context_subtree(document_handle);
        self.cancel_child_classic_document_script_work(handle);
        self.note_style_subtree_context_change(document_handle);
        self.dom_host_mut()
            .mark_subtree_disconnected_preserving_owner_document(document_handle);
        self.style_engine
            .retire_document_style_world(document_handle);
        Some(document_handle)
    }

    pub(crate) fn refresh_child_browsing_context(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
    ) -> Option<FrameDocumentClassicScriptSchedulerWork> {
        // Reinsertion from an abort/navigateerror handler creates a new Window.
        // Finish only the retiring context before installing its successor.
        if self
            .child_browsing_contexts
            .get(&handle)
            .is_some_and(|entry| entry.retiring)
        {
            self.drop_child_browsing_context_handles(vec![handle]);
        }
        match self.child_browsing_context_bootstrap_for_handle(handle) {
            Some(attribute_bootstrap) => {
                self.object_fallback_bootstraps.remove(&handle);
                let existing = self.child_browsing_contexts.get(&handle).cloned();
                let is_new = existing.is_none();
                let ignored_attribute_bootstrap = (existing.as_ref().is_some_and(|entry| {
                    entry.ignored_attribute_bootstrap.as_ref() == Some(&attribute_bootstrap)
                }) || self
                    .child_frame_attribute_bootstrap_repeats_ancestor(
                        handle,
                        &attribute_bootstrap,
                    ))
                .then(|| attribute_bootstrap.clone());
                let attribute_bootstrap = if ignored_attribute_bootstrap.is_some() {
                    existing
                        .as_ref()
                        .map_or(ChildBrowsingContextBootstrap::AboutBlank, |entry| {
                            entry.attribute_bootstrap().clone()
                        })
                } else {
                    attribute_bootstrap
                };
                let initial_attribute_navigation_ignored =
                    is_new && ignored_attribute_bootstrap.is_some();
                let history_identity = existing
                    .as_ref()
                    .and_then(|entry| entry.history_identity.clone())
                    .or_else(|| self.child_history_identity(scope, handle));
                let restored_history = if is_new {
                    history_identity
                        .as_ref()
                        .and_then(|identity| self.child_history_for_restoration(handle, identity))
                } else {
                    None
                };
                let attribute_bootstrap_changed = existing
                    .as_ref()
                    .is_some_and(|entry| entry.attribute_bootstrap_changed(&attribute_bootstrap));
                let stale_pending_service_worker_client_id = existing.as_ref().and_then(|entry| {
                    entry.stale_pending_service_worker_client_id_for_refresh(
                        attribute_bootstrap_changed,
                    )
                });
                let frame_id = existing
                    .as_ref()
                    .map(|entry| entry.frame_id().to_owned())
                    .unwrap_or_else(|| self.next_child_browsing_context_frame_id());
                let live_bootstrap = existing
                    .as_ref()
                    .map(|entry| entry.live_bootstrap())
                    .unwrap_or_else(|| {
                        Self::child_browsing_context_initial_live_bootstrap(&attribute_bootstrap)
                    });
                let srcdoc_attribute_changed = attribute_bootstrap_changed
                    && matches!(
                        attribute_bootstrap,
                        ChildBrowsingContextBootstrap::Srcdoc { .. }
                    );
                let attribute_pushes_history = srcdoc_attribute_changed
                    && Self::child_browsing_context_navigation_entry_url(&attribute_bootstrap)
                        .is_some_and(|url| {
                            !self.child_frame_attribute_navigation_replaces_current_entry(
                                handle, &url,
                            )
                        });
                let navigation_entry_seed = if attribute_bootstrap_changed {
                    let mut seed = existing
                        .as_ref()
                        .map(|entry| {
                            if srcdoc_attribute_changed {
                                entry.committed_navigation_entry_seed()
                            } else {
                                entry.navigation_entry_seed()
                            }
                        })
                        .unwrap_or_else(|| {
                            Self::child_browsing_context_single_entry_seed(&live_bootstrap)
                        });
                    if let Some(url) =
                        Self::child_browsing_context_navigation_entry_url(&attribute_bootstrap)
                    {
                        if attribute_pushes_history {
                            Self::apply_child_browsing_context_navigation_to_entry_seed(
                                &mut seed, &url,
                            );
                        } else {
                            // Srcdoc inherits its creator's origin. URL-origin
                            // comparison alone treats about:srcdoc as opaque
                            // and would lose a same-origin replacement's key.
                            let replacement_key = (srcdoc_attribute_changed
                                && self.child_frame_attribute_initiator_has_same_origin(handle)
                                && !self
                                    .child_browsing_context_sandbox_policy_from_owner(handle)
                                    .forces_opaque_origin)
                                .then(|| {
                                    seed.entries
                                        .iter()
                                        .find(|entry| entry.history_index == seed.current_index)
                                        .map(|entry| entry.key.clone())
                                })
                                .flatten();
                            Self::replace_child_browsing_context_navigation_in_entry_seed(
                                &mut seed, &url,
                            );
                            if let Some(key) = replacement_key {
                                if let Some(entry) = seed
                                    .entries
                                    .iter_mut()
                                    .find(|entry| entry.history_index == seed.current_index)
                                {
                                    entry.key = key.clone();
                                }
                                if let Some(activation) = seed.activation.as_mut() {
                                    activation.entry.key = key;
                                }
                            }
                        }
                    }
                    seed
                } else {
                    existing
                        .as_ref()
                        .map(|entry| entry.navigation_entry_seed())
                        .unwrap_or_else(|| {
                            Self::child_browsing_context_single_entry_seed(&attribute_bootstrap)
                        })
                };
                let committed_navigation_entry_seed = if is_new {
                    // The target entry is planned when a frame owner with a
                    // non-blank navigation attribute is discovered, but its
                    // live Document is still the synchronous initial empty
                    // document. Keep the committed projection pinned to that
                    // actual Document until NavigationCommit installs the
                    // fetched/srcdoc target. Otherwise a pending iframe can
                    // expose the target URL together with the old complete
                    // about:blank Document.
                    Self::child_browsing_context_single_entry_seed(&live_bootstrap)
                } else if attribute_bootstrap_changed {
                    existing
                        .as_ref()
                        .map(|entry| entry.committed_navigation_entry_seed())
                        .unwrap_or_else(|| {
                            Self::child_browsing_context_single_entry_seed(&live_bootstrap)
                        })
                } else {
                    existing
                        .as_ref()
                        .map(|entry| entry.committed_navigation_entry_seed())
                        .unwrap_or_else(|| navigation_entry_seed.clone())
                };
                let performance_time_origin = if is_new {
                    ChildPerformanceTimeOrigin::now()
                } else {
                    existing
                        .as_ref()
                        .map(|entry| entry.performance_time_origin())
                        .unwrap_or_else(ChildPerformanceTimeOrigin::now)
                };
                let cached_snapshot = existing.as_ref().and_then(|entry| entry.cached_snapshot());
                let initial_empty_policy_container = is_new
                    .then(|| self.initial_child_about_blank_policy_container_from_parent(handle));
                let existing_document_policy = existing
                    .as_ref()
                    .map(|entry| entry.document_policy_container_snapshot());
                // The owner element's `name` attribute seeds the navigable target name only
                // when the child navigable is created. Preserve later `window.name` writes
                // across ordinary DOM-to-runtime record refreshes.
                let name = existing
                    .as_ref()
                    .map(|entry| entry.name.clone())
                    .unwrap_or_else(|| self.dom_host().get_attribute(handle, "name"))
                    .filter(|value| !value.is_empty());
                let id = self.dom_host().get_attribute(handle, "id");
                let credentialless = self
                    .dom_host()
                    .get_attribute(handle, "credentialless")
                    .is_some();
                let sandbox_policy_from_owner = super::document_sandbox_policy_from_attribute(
                    self.dom_host().get_attribute(handle, "sandbox").as_deref(),
                );
                let ancestor_origins_referrer_policy_snapshot =
                    if is_new || attribute_bootstrap_changed {
                        self.child_ancestor_origins_referrer_policy_from_owner_attribute(handle)
                    } else {
                        existing
                            .as_ref()
                            .map(|entry| entry.ancestor_origins_referrer_policy_snapshot())
                            .unwrap_or_default()
                    };
                let document_internal_ancestor_origins = existing
                    .as_ref()
                    .map(|entry| entry.document_internal_ancestor_origins())
                    .unwrap_or_default();
                let document_credentialless = if is_new {
                    self.child_browsing_context_document_credentialless_for_owner(
                        handle,
                        credentialless,
                    )
                } else {
                    existing_document_policy
                        .as_ref()
                        .map(|policy| policy.credentialless)
                        .unwrap_or_else(|| {
                            self.child_browsing_context_document_credentialless_for_owner(
                                handle,
                                credentialless,
                            )
                        })
                };
                let credentialless_storage_nonce = if attribute_bootstrap_changed || is_new {
                    self.child_document_credentialless_storage_nonce(document_credentialless)
                } else {
                    existing_document_policy
                        .as_ref()
                        .and_then(|policy| policy.credentialless_storage_nonce)
                        .or_else(|| {
                            self.child_document_credentialless_storage_nonce(
                                document_credentialless,
                            )
                        })
                };
                let parent_frame_id = self.child_browsing_context_parent_frame_id(handle);
                let frame_identity_changed = self
                    .frame_owner_store
                    .frame_id_for_child_handle(handle)
                    .is_some_and(|current| current.0.as_str() != frame_id);
                self.frame_owner_store.ensure_child_frame(
                    handle,
                    frame_id.clone(),
                    parent_frame_id.clone(),
                );
                self.apply_pending_parent_descendant_completions();
                self.rebind_active_child_frame_load_to_parent(handle);
                if is_new {
                    self.queue_child_frame_attachment_event(ChildFrameAttachmentSnapshot {
                        frame_id: frame_id.clone(),
                        parent_frame_id: parent_frame_id.clone(),
                    });
                }
                let initial_about_blank_document_is_complete = is_new
                    && !initial_attribute_navigation_ignored
                    && restored_history.is_none()
                    && child_browsing_context_bootstrap_is_initial_about_blank(
                        &attribute_bootstrap,
                    );
                let initial_navigation_uses_initial_empty_load = is_new
                    && restored_history.is_none()
                    && child_browsing_context_bootstrap_uses_initial_empty_load(
                        &attribute_bootstrap,
                    );
                let pending_attribute_bootstrap_commit = !initial_attribute_navigation_ignored
                    && ChildBrowsingContextEntry::pending_attribute_bootstrap_commit_for_refresh(
                        existing.as_ref(),
                        is_new,
                        attribute_bootstrap_changed,
                        initial_about_blank_document_is_complete,
                    );
                let pending_attribute_permissions_policy =
                    pending_attribute_bootstrap_commit.then(|| {
                        self.child_browsing_context_permissions_policy_for_navigation(
                            handle,
                            &attribute_bootstrap,
                        )
                    });
                if is_new || attribute_bootstrap_changed || frame_identity_changed {
                    self.note_child_frame_load_started_for_parent(handle);
                }
                let creator_document_url = self.document_url_for_child_context(handle);
                let refresh_policy_source = if is_new {
                    initial_empty_policy_container.as_ref()
                } else if attribute_bootstrap_changed {
                    cached_snapshot
                        .as_ref()
                        .map(|snapshot| &snapshot.policy_container)
                } else {
                    existing_document_policy.as_ref()
                };
                // The internal initial empty Document starts with the creator
                // URL. A later explicit about:blank navigation must run that
                // URL through Referrer Policy even though it sends no request.
                let document_referrer = if !is_new
                    && attribute_bootstrap_changed
                    && let ChildBrowsingContextBootstrap::Url(target) = &attribute_bootstrap
                    && target.as_str() == "about:blank"
                {
                    let creator_policy =
                        self.initial_child_about_blank_policy_container_from_parent(handle);
                    moli_fetch::referrer_value(
                        &creator_document_url,
                        target,
                        None,
                        creator_policy.referrer_policy.as_deref(),
                    )
                    .unwrap_or_default()
                } else if attribute_bootstrap_changed || is_new {
                    creator_document_url.to_string()
                } else {
                    refresh_policy_source
                        .map(|policy| policy.document_referrer.clone())
                        .unwrap_or_else(|| creator_document_url.to_string())
                };
                let document_policy_container = ChildDocumentPolicyContainer {
                    document_referrer,
                    referrer_policy: refresh_policy_source
                        .and_then(|policy| policy.referrer_policy.clone()),
                    cross_origin_embedder_policy: refresh_policy_source
                        .map(|policy| policy.cross_origin_embedder_policy)
                        .unwrap_or_default(),
                    document_isolation_policy: refresh_policy_source
                        .map(|policy| policy.document_isolation_policy)
                        .unwrap_or_default(),
                    cross_origin_isolated: refresh_policy_source
                        .is_some_and(|policy| policy.cross_origin_isolated),
                    document_content_security_policies: refresh_policy_source
                        .map(|policy| policy.document_content_security_policies.clone())
                        .unwrap_or_default(),
                    credentialless: document_credentialless,
                    credentialless_storage_nonce,
                    sandbox: if attribute_bootstrap_changed || is_new {
                        refresh_policy_source
                            .map(|policy| {
                                policy.sandbox.with_response_content_security_policy(
                                    sandbox_policy_from_owner,
                                )
                            })
                            .unwrap_or(sandbox_policy_from_owner)
                    } else {
                        refresh_policy_source
                            .map(|policy| policy.sandbox)
                            .unwrap_or(sandbox_policy_from_owner)
                    },
                    response_content_security_policies: refresh_policy_source
                        .map(|policy| policy.response_content_security_policies.clone())
                        .unwrap_or_default(),
                    response_content_security_report_only_policies: refresh_policy_source
                        .map(|policy| {
                            policy
                                .response_content_security_report_only_policies
                                .clone()
                        })
                        .unwrap_or_default(),
                    content_security_reporting_endpoints: refresh_policy_source
                        .map(|policy| policy.content_security_reporting_endpoints.clone())
                        .unwrap_or_default(),
                    inherited_meta_content_security_policies: refresh_policy_source
                        .map(|policy| policy.inherited_meta_content_security_policies.clone())
                        .unwrap_or_default(),
                    content_security_policy_self_url: refresh_policy_source
                        .and_then(|policy| policy.content_security_policy_self_url.clone()),
                    permissions_policy: if is_new {
                        // The synchronous initial about:blank Document is
                        // already subject to the iframe's container policy.
                        // Use its live inherited origin, not a pending target
                        // navigation's origin, when applying the allowlist.
                        self.child_browsing_context_permissions_policy_for_navigation(
                            handle,
                            &live_bootstrap,
                        )
                    } else {
                        refresh_policy_source
                            .map(|policy| policy.permissions_policy)
                            .unwrap_or_default()
                    },
                };
                let initial_empty_document_init: Option<ChildInitialEmptyDocumentInit> = is_new
                    .then(|| {
                        let document_url =
                            Self::child_browsing_context_bootstrap_url(&live_bootstrap)
                                .filter(moli_url::is_about_blank)
                                .expect("an initial live child bootstrap must match about:blank");
                        self.capture_child_initial_empty_document_init(
                            handle,
                            document_url,
                            document_policy_container.clone(),
                        )
                    });
                self.child_browsing_contexts.insert(
                    handle,
                    ChildBrowsingContextEntry {
                        frame_id,
                        retiring: false,
                        history_identity,
                        restoring_history: existing
                            .as_ref()
                            .is_some_and(|entry| entry.restoring_history),
                        restored_history_positions: existing
                            .as_ref()
                            .and_then(|entry| entry.restored_history_positions.clone()),
                        current_document_loader_id: existing.as_ref().and_then(|entry| {
                            entry.current_document_loader_id().map(ToOwned::to_owned)
                        }),
                        name,
                        id: id.filter(|value| !value.is_empty()),
                        attribute_bootstrap,
                        ignored_attribute_bootstrap,
                        pending_attribute_bootstrap_commit,
                        pending_live_navigation: existing.as_ref().and_then(|entry| {
                            entry.pending_live_navigation_for_refresh(attribute_bootstrap_changed)
                        }),
                        pending_live_navigation_initiator_url: existing.as_ref().and_then(
                            |entry| {
                                entry.pending_live_navigation_initiator_url_for_refresh(
                                    attribute_bootstrap_changed,
                                )
                            },
                        ),
                        pending_live_navigation_reflects_window_state: existing
                            .as_ref()
                            .is_some_and(|entry| {
                                entry.pending_live_navigation_reflects_window_state_for_refresh(
                                    attribute_bootstrap_changed,
                                )
                            }),
                        live_bootstrap,
                        navigation_entry_seed,
                        committed_navigation_entry_seed,
                        cached_snapshot,
                        srcdoc_history: existing
                            .as_ref()
                            .map(|entry| entry.srcdoc_history.clone())
                            .unwrap_or_default(),
                        document_policy_container,
                        can_navigate_top_without_user_gesture: existing
                            .as_ref()
                            .is_some_and(|entry| entry.can_navigate_top_without_user_gesture),
                        document_internal_ancestor_origins,
                        ancestor_origins_referrer_policy_snapshot,
                        completed_document_network: existing.as_ref().and_then(|entry| {
                            entry
                                .completed_document_network_for_refresh(attribute_bootstrap_changed)
                        }),
                        completed_frame_owner_resource_timing: existing.as_ref().and_then(
                            |entry| {
                                entry.completed_frame_owner_resource_timing_for_refresh(
                                    attribute_bootstrap_changed,
                                )
                            },
                        ),
                        performance_time_origin,
                        pending_document_load_id: existing.as_ref().and_then(|entry| {
                            entry.pending_document_load_id_for_refresh(attribute_bootstrap_changed)
                        }),
                        classic_script_document_state: existing
                            .as_ref()
                            .map(|entry| entry.classic_script_document_state.clone())
                            .unwrap_or_default(),
                        // Navigation attributes do not replace the current Document.
                        // Keep its domain through unload or a cancelled navigation;
                        // committing the new Document resets this state.
                        document_domain_override: existing
                            .as_ref()
                            .map(|entry| entry.document_domain_override.clone())
                            .unwrap_or_default(),
                        credentialless,
                        service_worker_client_id: existing
                            .as_ref()
                            .and_then(|entry| entry.service_worker_client_id()),
                        pending_service_worker_client_id: existing.as_ref().and_then(|entry| {
                            entry.pending_service_worker_client_id_for_refresh(
                                attribute_bootstrap_changed,
                            )
                        }),
                        pending_service_worker_client_navigation: existing
                            .as_ref()
                            .and_then(|entry| entry.pending_service_worker_client_navigation()),
                    },
                );
                if attribute_bootstrap_changed {
                    let _ = self.replace_child_navigation_load(handle);
                }
                if let Some(client_id) = stale_pending_service_worker_client_id {
                    self.browser_context_runtime
                        .unregister_service_worker_client(client_id);
                }
                if attribute_bootstrap_changed {
                    self.reject_replaced_service_worker_child_client_navigation(
                        handle,
                        "The navigation was canceled.".to_owned(),
                    );
                }
                let pending_attribute_url =
                    self.child_browsing_contexts.get(&handle).and_then(|entry| {
                        entry
                            .pending_attribute_bootstrap_commit()
                            .then(|| {
                                Self::child_browsing_context_navigation_entry_url(
                                    entry.attribute_bootstrap(),
                                )
                            })
                            .flatten()
                    });
                if let Some(url) = pending_attribute_url.as_ref() {
                    self.register_reserved_service_worker_child_client_for_navigation(handle, url);
                }
                if is_new {
                    self.cancel_child_meta_refresh_navigation(handle);
                    self.disconnect_shared_worker_clients_for_child_context(handle);
                    self.clear_custom_element_registry_associations_for_child_context(handle);
                    self.clear_child_browsing_context_current_document(handle);
                    self.child_custom_elements.remove(&handle);
                    let initialization = self.initialize_child_frame_with_initial_empty_document(
                        handle,
                        initial_empty_document_init.expect(
                            "new child frame refresh must capture initial-empty init inputs",
                        ),
                    );
                    if initial_about_blank_document_is_complete {
                        let _ = self.dispatch_ready_child_initial_empty_load_synchronously(
                            scope,
                            initialization.suppressed_load_delivery,
                        );
                    } else if initial_attribute_navigation_ignored
                        || !initial_navigation_uses_initial_empty_load
                    {
                        let suppressed = self.suppress_ready_child_initial_empty_load(
                            initialization.suppressed_load_delivery,
                        );
                        assert!(
                            suppressed,
                            "a pending child navigation must consume the initial-empty load delivery"
                        );
                    }
                    if !self
                        .child_browsing_contexts
                        .get(&handle)
                        .is_some_and(|entry| entry.pending_attribute_bootstrap_commit())
                    {
                        self.register_or_update_service_worker_child_client(handle);
                    }
                }
                if let Some(policy) = pending_attribute_permissions_policy
                    && let Some(entry) = self.child_browsing_contexts.get_mut(&handle)
                {
                    entry.set_document_permissions_policy(policy);
                }
                if let Some(restored) = restored_history {
                    self.restore_child_history_after_initial_empty(handle, restored);
                }
                if attribute_bootstrap_changed {
                    self.cancel_child_meta_refresh_navigation(handle);
                }
                if is_new || attribute_bootstrap_changed {
                    if self
                        .child_browsing_contexts
                        .get(&handle)
                        .is_some_and(|entry| entry.pending_attribute_bootstrap_commit())
                    {
                        self.queue_child_browsing_context_navigation_commit(handle);
                    } else if !initial_about_blank_document_is_complete {
                        let _ = self.queue_child_document_complete_lifecycle_if_ready(handle);
                    }
                }
                None
            }
            None => {
                let document_handle = self.child_browsing_context_document_handle(handle);
                self.cancel_child_meta_refresh_navigation(handle);
                self.clear_pending_child_document_loads_for_handle(handle);
                self.unregister_service_worker_child_client(handle);
                self.clear_child_browsing_context_current_document(handle);
                self.remove_child_browsing_context_entry(handle);
                self.detach_child_frame_owner_and_wake_parent(handle);
                self.clear_live_child_window_proxy_records(handle);
                if let Some(document_handle) = document_handle {
                    self.set_custom_element_registry_association(
                        document_handle,
                        CustomElementRegistryAssociation::Registry(
                            CustomElementRegistryKey::Child(handle),
                        ),
                    );
                }
                self.child_custom_elements.remove(&handle);
                self.clear_child_window_event_listeners(handle);
                self.close_broadcast_channels_for_child_context(handle);
                self.disconnect_shared_worker_clients_for_child_context(handle);
                self.child_web_storage_opaque_context_nonces.remove(&handle);
                self.retire_current_child_navigation_commit_task(handle);
                self.cancel_child_document_script_work(handle);
                None
            }
        }
    }

    pub(crate) fn cancel_child_browsing_context_attribute_navigation(&mut self, handle: DomHandle) {
        let Some(attribute_bootstrap) = self.child_browsing_context_bootstrap_for_handle(handle)
        else {
            return;
        };
        if !self.clear_child_browsing_context_pending_navigation(handle) {
            return;
        }
        let Some(entry) = self.child_browsing_contexts.get_mut(&handle) else {
            return;
        };
        entry.replace_attribute_bootstrap(attribute_bootstrap);
        entry.clear_pending_document_load();
        self.clear_pending_service_worker_child_client(handle);
        self.retire_current_child_navigation_commit_task(handle);
        self.cancel_child_document_script_work(handle);
        let navigation_load = self.current_child_navigation_load(handle);
        if let Some(navigation_load) = navigation_load {
            let _ =
                self.finish_child_frame_navigation_without_load_dispatch(handle, navigation_load);
        }
        self.clear_pending_child_document_loads_for_handle(handle);
    }

    fn child_browsing_context_subtree_ready_work(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        root: DomHandle,
    ) -> (bool, Vec<FrameDocumentClassicScriptSchedulerWork>) {
        let mut handles = Vec::new();
        self.collect_child_browsing_context_host_handles(root, &mut handles);
        if handles.is_empty() {
            return (false, Vec::new());
        }
        let ready_work = handles
            .into_iter()
            .filter_map(|handle| self.refresh_child_browsing_context(scope, handle))
            .collect();
        (true, ready_work)
    }

    pub(crate) fn sync_child_browsing_context_subtree_into_ready_work(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        root: DomHandle,
    ) -> Vec<FrameDocumentClassicScriptSchedulerWork> {
        self.child_browsing_context_subtree_ready_work(scope, root)
            .1
    }

    pub(crate) fn sync_child_browsing_context_subtree(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        root: DomHandle,
    ) -> bool {
        let (had_handles, ready_work) = self.child_browsing_context_subtree_ready_work(scope, root);
        for work in ready_work {
            self.push_child_document_script_ready_input(work);
        }
        had_handles
    }

    pub(crate) fn drop_child_browsing_context_subtree(&mut self, root: DomHandle) {
        let mut handles = Vec::new();
        self.collect_child_browsing_context_host_handles(root, &mut handles);
        self.drop_child_browsing_context_handles(handles);
    }

    /// Orchestrate synchronous cancellation without borrowing the host across
    /// author JS. Callers must also release their host borrow before entering.
    pub(crate) fn drop_child_browsing_context_subtree_with_window_realm(
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut Self,
        root: DomHandle,
    ) {
        let retirements = {
            let host = unsafe { &mut *host_ptr };
            let mut handles = Vec::new();
            host.collect_child_browsing_context_host_handles(root, &mut handles);
            let mut retirements = Vec::new();
            // Mark the whole batch before the first callback: a handler for A
            // can reattach B before B reaches its own cancellation boundary.
            for handle in handles {
                let Some(entry) = host.child_browsing_contexts.get_mut(&handle) else {
                    continue;
                };
                if entry.retiring {
                    // Nested removal must not dispatch the same cancellation twice.
                    host.drop_child_browsing_context_handles(vec![handle]);
                    continue;
                }
                entry.retiring = true;
                let frame_id = entry.frame_id.clone();
                let retirement = ChildWindowRetirement {
                    handle,
                    frame_id,
                    local_window: host
                        .current_child_document_task_owner(handle)
                        .map(|owner| owner.local_window_id),
                };
                host.prepare_child_window_retirement(handle);
                let window = host.child_window_proxy_records.live_window(scope, handle);
                retirements.push((retirement, window));
            }
            retirements
        };
        for (retirement, window) in retirements {
            // Settle the captured old Window even if an earlier callback has
            // already replaced it. Looking up by handle here would cancel its
            // successor, or leave the old Window's promises pending forever.
            if let Some(window) = window
                && let Some(context) = window.get_creation_context(scope)
            {
                let scope = &mut v8::ContextScope::new(scope, context);
                crate::context_bootstrap::inform_about_canceled_navigation_for_window(
                    scope,
                    window,
                    crate::context_bootstrap::NavigationCancellationReason::LocalWindowRetirement,
                );
            }
            if unsafe { &*host_ptr }.child_window_retirement_is_current(&retirement) {
                Self::dispatch_child_document_tree_unload_without_beforeunload(
                    scope, host_ptr, retirement.handle, false,
                );
            }
            if unsafe { &*host_ptr }.child_window_retirement_is_current(&retirement) {
                crate::context_bootstrap::finish_joint_history_without_document_commit(
                    scope, host_ptr, super::super::OwnerDispatchScope::Child(retirement.handle),
                );
            }
            let host = unsafe { &mut *host_ptr };
            if host.child_window_retirement_is_current(&retirement) {
                host.drop_child_browsing_context_handles(vec![retirement.handle]);
            }
        }
    }

    fn child_window_retirement_is_current(&self, retirement: &ChildWindowRetirement) -> bool {
        self.child_browsing_contexts
            .get(&retirement.handle)
            .is_some_and(|entry| entry.frame_id == retirement.frame_id)
            && self
                .current_child_document_task_owner(retirement.handle)
                .map(|owner| owner.local_window_id)
                == retirement.local_window
    }

    fn prepare_child_window_retirement(&mut self, handle: DomHandle) {
        if let Some(owner) = self.current_child_document_task_owner(handle) {
            self.pending_history_traversal_admissions.retire_owner(
                super::super::WindowExecutionContextOwner::Frame(owner.local_window_id),
            );
        }
        self.cancel_child_meta_refresh_navigation(handle);
        self.clear_pending_child_document_loads_for_handle(handle);
        self.retire_current_child_navigation_commit_task(handle);
        self.unregister_service_worker_child_client(handle);
    }

    pub(in crate::native_bridge::context_host) fn enter_object_fallback_state(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
    ) {
        let Some(attribute_bootstrap) = self
            .child_browsing_contexts
            .get(&handle)
            .map(|entry| entry.attribute_bootstrap().clone())
        else {
            return;
        };
        Self::drop_child_browsing_context_subtree_with_window_realm(scope, self, handle);
        if self.child_browsing_context_host_is_object_element(handle)
            && self
                .child_browsing_context_bootstrap_for_handle(handle)
                .as_ref()
                == Some(&attribute_bootstrap)
        {
            self.object_fallback_bootstraps
                .insert(handle, attribute_bootstrap);
        }
    }

    fn drop_child_browsing_context_handles(&mut self, handles: Vec<DomHandle>) {
        for handle in handles {
            self.object_fallback_bootstraps.remove(&handle);
            let document_handle_before_drop = self.child_browsing_context_document_handle(handle);
            let frame_id = self
                .child_browsing_contexts
                .get(&handle)
                .map(|entry| entry.frame_id().to_owned());
            #[cfg(test)]
            if let Some(frame_id) = frame_id.as_deref() {
                self.completed_child_browsing_context_loads
                    .retain(|load| load.frame_id != frame_id);
            }
            self.prepare_child_window_retirement(handle);
            self.clear_child_parser_classic_runner_for_current_document(handle);
            let removed = self.remove_child_browsing_context_entry(handle).is_some();
            self.clear_child_browsing_context_current_document(handle);
            self.detach_child_frame_owner_and_wake_parent(handle);
            if removed && let Some(frame_id) = frame_id {
                self.queue_child_frame_detachment_event(frame_id);
            }
            self.clear_live_child_window_proxy_records(handle);
            // Registry associations belong to retained DOM nodes, not to the
            // execution context being retired. Snapshot the document default
            // before subsequent lookups can no longer infer it from the frame.
            if let Some(document_handle) = document_handle_before_drop {
                self.set_custom_element_registry_association(
                    document_handle,
                    CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::Child(
                        handle,
                    )),
                );
            }
            self.child_custom_elements.remove(&handle);
            self.clear_child_window_event_listeners(handle);
            self.close_broadcast_channels_for_child_context(handle);
            self.disconnect_shared_worker_clients_for_child_context(handle);
            self.child_web_storage_opaque_context_nonces.remove(&handle);
        }
    }

    pub(crate) fn drop_child_browsing_contexts_moved_into_own_document_subtree(
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut Self,
        root: DomHandle,
    ) {
        let handles = {
            let host = unsafe { &*host_ptr };
            let mut handles = Vec::new();
            host.collect_child_browsing_context_host_handles(root, &mut handles);
            handles
        };
        for handle in handles {
            let should_drop = {
                let host = unsafe { &*host_ptr };
                host.dom_host()
                    .owner_document_handle(handle)
                    .is_some_and(|document| {
                        host.child_browsing_context_host_is_ancestor_of_document(handle, document)
                    })
            };
            if should_drop {
                Self::drop_child_browsing_context_subtree_with_window_realm(
                    scope, host_ptr, handle,
                );
            }
        }
    }

    pub(crate) fn resync_child_browsing_contexts(&mut self, scope: &mut v8::PinScope<'_, '_>) {
        let ready_work = self.resync_child_browsing_contexts_into_ready_work(scope);
        for work in ready_work {
            self.push_child_document_script_ready_input(work);
        }
    }

    pub(crate) fn resync_child_browsing_contexts_into_ready_work(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Vec<FrameDocumentClassicScriptSchedulerWork> {
        self.dom_host()
            .compact_child_browsing_context_host_candidates();
        let handles = self
            .dom_host()
            .child_browsing_context_host_candidate_handles()
            .into_iter()
            .filter(|handle| {
                self.is_child_browsing_context_host_handle(*handle)
                    && self.child_browsing_context_host_is_active(*handle)
            })
            .collect::<Vec<_>>();
        let mut handles = handles;
        for popup_id in self.open_lightweight_popup_ids() {
            if let Some(document_handle) = self.lightweight_popup_document_handle(popup_id) {
                self.collect_child_browsing_context_host_handles(document_handle, &mut handles);
            }
        }
        handles.retain(|handle| {
            self.is_child_browsing_context_host_handle(*handle)
                && self.child_browsing_context_host_is_active(*handle)
        });
        let live_handles = handles.iter().copied().collect::<HashSet<_>>();
        let stale_meta_refresh_handles = self
            .child_meta_refresh_navigations
            .keys()
            .copied()
            .filter(|handle| !live_handles.contains(handle))
            .collect::<Vec<_>>();
        for handle in stale_meta_refresh_handles {
            self.cancel_child_meta_refresh_navigation(handle);
        }
        let stale_shared_worker_client_handles = self
            .child_shared_worker_client_owner_ids
            .keys()
            .copied()
            .filter(|handle| !live_handles.contains(handle))
            .collect::<Vec<_>>();
        for handle in stale_shared_worker_client_handles {
            self.disconnect_shared_worker_clients_for_child_context(handle);
        }
        let stale_registry_context_handles = self
            .child_browsing_context_document_handles
            .keys()
            .chain(self.child_custom_elements.keys())
            .copied()
            .filter(|handle| !live_handles.contains(handle))
            .collect::<HashSet<_>>();
        for handle in stale_registry_context_handles {
            let document_handle = self.child_browsing_context_document_handle(handle);
            if let Some(document_handle) = document_handle {
                self.set_custom_element_registry_association(
                    document_handle,
                    CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::Child(
                        handle,
                    )),
                );
            }
        }
        let stale_document_handles = self
            .child_browsing_context_document_handles
            .keys()
            .copied()
            .filter(|handle| !live_handles.contains(handle))
            .collect::<Vec<_>>();
        for handle in stale_document_handles {
            self.clear_child_browsing_context_current_document(handle);
        }
        let stale_child_context_handles = self
            .child_browsing_contexts
            .keys()
            .copied()
            .filter(|handle| !live_handles.contains(handle))
            .collect::<Vec<_>>();
        for &handle in &stale_child_context_handles {
            self.clear_child_parser_classic_runner_for_current_document(handle);
            self.detach_child_frame_owner_and_wake_parent(handle);
        }
        for handle in stale_child_context_handles {
            let Some(entry) = self.remove_child_browsing_context_entry(handle) else {
                continue;
            };
            self.queue_child_frame_detachment_event(entry.frame_id().to_owned());
        }
        self.object_fallback_bootstraps
            .retain(|handle, _| live_handles.contains(handle));
        self.retain_live_child_window_proxy_records(&live_handles);
        self.child_browsing_context_document_handles
            .retain(|handle, _| live_handles.contains(handle));
        self.child_custom_elements
            .retain(|handle, _| live_handles.contains(handle));
        self.child_window_event_listeners
            .retain(|handle, _| live_handles.contains(handle));
        self.child_web_storage_opaque_context_nonces
            .retain(|handle, _| live_handles.contains(handle));
        self.pending_child_document_navigations
            .retain(|_, pending| live_handles.contains(&pending.target.child_handle()));
        live_handles
            .iter()
            .copied()
            .filter_map(|handle| self.refresh_child_browsing_context(scope, handle))
            .collect()
    }

    pub(crate) fn refresh_child_browsing_context_and_queue_ready_work(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
    ) {
        if let Some(work) = self.refresh_child_browsing_context(scope, handle) {
            self.push_child_document_script_ready_input(work);
        }
    }
}

fn child_browsing_context_bootstrap_is_initial_about_blank(
    bootstrap: &ChildBrowsingContextBootstrap,
) -> bool {
    matches!(bootstrap, ChildBrowsingContextBootstrap::AboutBlank)
        || matches!(
            bootstrap,
            ChildBrowsingContextBootstrap::Url(url) if moli_url::is_about_blank(url)
        )
}

fn child_browsing_context_bootstrap_uses_initial_empty_load(
    bootstrap: &ChildBrowsingContextBootstrap,
) -> bool {
    matches!(
        bootstrap,
        ChildBrowsingContextBootstrap::Url(url)
            if moli_url::is_about_blank(url) || url.scheme() == "javascript"
    )
}
