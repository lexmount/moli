use super::*;
use crate::service_worker_runtime::static_router::{
    RouterRule, RouterSource, within_registration_limit,
};

pub(super) enum StaticRouterDispatch {
    Network,
    Cache {
        origin: moli_url::WebOrigin,
        embedder_policy: crate::cross_origin_isolation::CrossOriginEmbedderPolicy,
        store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
        storage_key: String,
        name: Option<Vec<u16>>,
    },
}

impl StaticRouterDispatch {
    pub(super) fn complete(
        self,
        service: &ServiceWorkerRuntimeService,
        job: ServiceWorkerFetchJob,
    ) {
        let Self::Cache {
            origin,
            embedder_policy,
            store: Some(store),
            storage_key,
            name,
        } = self
        else {
            service.dispatch_fetch_fallback(job);
            return;
        };
        let response = {
            job.cancel_handle.mark_service_worker_cache_lookup();
            let store = store.lock();
            store
                .bucket_identity(
                    &storage_key,
                    moli_storage_service::IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME,
                )
                .and_then(|identity| {
                    let query = moli_storage_service::StorageBucketCacheQuery {
                        request_url: job.request.url.to_string(),
                        method: job.request.method.clone(),
                        headers: job.request.request_headers.to_byte_strings(),
                        ignore_search: false,
                        ignore_method: false,
                        ignore_vary: false,
                    };
                    let names = name.map_or_else(
                        || {
                            store
                                .cache_names_for_identity(&identity)
                                .unwrap_or_default()
                        },
                        |name| {
                            vec![moli_storage_service::StorageBucketCacheName::from_utf16(
                                name,
                            )]
                        },
                    );
                    names.into_iter().find_map(|name| {
                        store
                            .match_cache_entries_for_identity(&identity, name, &query)
                            .and_then(|matches| matches.into_iter().next())
                            .map(|matched| matched.response)
                    })
                })
        };
        if let Some(response) = response {
            if response.response_type == "opaque"
                && Url::parse(&response.url).ok().is_none_or(|url| {
                    crate::network_host::check_opaque_response_policy(
                        &origin,
                        &url,
                        &response.headers,
                        response.request_include_credentials,
                        embedder_policy,
                        crate::cross_origin_isolation::DocumentIsolationPolicy::None,
                    )
                    .is_err()
                })
            {
                service.dispatch_fetch_fallback(job);
                return;
            }
            job.cancel_handle.mark_service_worker_router_final_source(
                moli_fetch::ServiceWorkerRouterSource::Cache,
            );
            let body_is_null =
                response.body_is_null || matches!(response.status, 101 | 103 | 204 | 205 | 304);
            service.complete_fetch_with_service_worker_response(
                job,
                ServiceWorkerFetchResponse {
                    cors_exposed_header_names: response.cors_exposed_header_names,
                    request_include_credentials: response.request_include_credentials,
                    final_url: Url::parse(&response.url).ok(),
                    response_type: response.response_type,
                    redirected: response.redirected,
                    status: response.status,
                    status_text: response.status_text,
                    headers: response.headers,
                    body_is_null,
                    body: response.body,
                },
            );
        } else {
            service.dispatch_fetch_fallback(job);
        }
    }
}

pub(super) fn matching_router_source(
    version: &ServiceWorkerVersion,
    request: &super::super::ServiceWorkerFetchRequest,
) -> Option<RouterSource> {
    let running = matches!(
        version.running_state,
        ServiceWorkerVersionRunningState::Running { .. }
    );
    version
        .router_rules
        .iter()
        .find(|rule| {
            rule.condition.matches(
                &request.url,
                &request.method,
                request.request_mode.as_ref(),
                request.destination.as_str(),
                running,
            )
        })
        .map(|rule| rule.source.clone())
}

impl ServiceWorkerRuntimeService {
    pub(super) fn soft_update_for_router(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        document_url: Url,
        non_subresource: bool,
    ) {
        let job = {
            let state = self.inner.state.lock();
            let Some(registration) = state.registrations.get(&registration_id) else {
                return;
            };
            if registration.pending_unregistration
                || registration.installing_version_id.is_some()
                || state
                    .pending_main_script_update_checks
                    .contains_key(&registration_id)
            {
                return;
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
                .unwrap_or_default();
            let stale = registration
                .last_update_check_time_ms
                .is_some_and(|last| now.saturating_sub(last) > 86_400_000);
            if !non_subresource && !stale {
                return;
            }
            let Some(version) = registration
                .waiting_version_id
                .or(registration.active_version_id)
                .and_then(|id| state.versions.get(&id))
            else {
                return;
            };
            let config = &version.launch_config;
            ServiceWorkerQueuedRegisterJob {
                update_registration_id: Some(registration_id),
                script_url: version.script_url.clone(),
                scope_url: registration.scope_url.clone(),
                document_url,
                storage_key: registration.storage_key.clone(),
                script_kind: version.script_kind,
                update_via_cache: registration.update_via_cache,
                force_bypass_cache: false,
                skip_script_comparison: false,
                skip_waiting_after_install: false,
                force_update_page_load_waiter_ids: Vec::new(),
                request_client: config.request_client(),
                network_policy: config.network_policy.clone(),
                worker_context_runtime: config.worker_context_runtime.clone(),
                broadcast_channel_top_level_site: config.broadcast_channel_top_level_site.clone(),
                indexed_db_manager: config.indexed_db_manager.clone(),
                storage_bucket_store: config.storage_bucket_store.clone(),
                callbacks: Vec::new(),
            }
        };
        self.start_queued_register_job(job);
    }

    pub(super) fn mark_fetch_event_router_source(&self, job: &ServiceWorkerFetchJob) {
        if job.router_race_request_cancel_handle.is_none()
            && matches!(
                job.cancel_handle
                    .response_timing()
                    .service_worker_router()
                    .matched_source,
                Some(
                    moli_fetch::ServiceWorkerRouterSource::FetchEvent
                        | moli_fetch::ServiceWorkerRouterSource::RaceNetworkAndFetchHandler
                )
            )
        {
            job.cancel_handle.mark_service_worker_router_final_source(
                moli_fetch::ServiceWorkerRouterSource::FetchEvent,
            );
        }
    }

    pub(super) fn close_job_router_race(&self, job: &ServiceWorkerFetchJob) {
        if let Some(event_id) = job
            .direct_completion_tx
            .as_ref()
            .and_then(|completion| completion.router_race_event_id)
            && let Some(owner) = &job.owner
        {
            self.close_router_race(event_id, owner);
        }
    }

    pub(crate) fn has_router_race(
        &self,
        event_id: ServiceWorkerEventId,
        owner: &ServiceWorkerRunOwner,
    ) -> bool {
        self.inner
            .router_races
            .lock()
            .get(&event_id)
            .is_some_and(|entry| &entry.owner == owner)
    }

    pub(crate) fn lookup_router_race(
        &self,
        event_id: ServiceWorkerEventId,
        owner: &ServiceWorkerRunOwner,
        request: &super::super::ServiceWorkerFetchRequest,
    ) -> Option<super::super::static_router::race::RouterRaceResponseReader> {
        let races = self.inner.router_races.lock();
        let entry = races.get(&event_id)?;
        if &entry.owner != owner || !entry.matches_request(request) {
            return None;
        }
        entry.response.lookup()
    }

    pub(super) fn close_router_race(
        &self,
        event_id: ServiceWorkerEventId,
        owner: &ServiceWorkerRunOwner,
    ) {
        let mut races = self.inner.router_races.lock();
        if races
            .get(&event_id)
            .is_some_and(|entry| &entry.owner == owner)
            && let Some(entry) = races.remove(&event_id)
        {
            entry.response.close_lookups();
        }
    }

    pub(crate) fn static_router_has_fetch_handler(
        &self,
        version_id: ServiceWorkerVersionId,
    ) -> bool {
        self.inner
            .state
            .lock()
            .versions
            .get(&version_id)
            .is_some_and(|version| {
                version.fetch_handler_existence == ServiceWorkerFetchHandlerExistence::Exists
            })
    }

    pub(in crate::service_worker_runtime) fn register_router_rules(
        &self,
        owner: ServiceWorkerRunOwner,
        rules: Vec<RouterRule>,
    ) -> Result<(), String> {
        let mut state = self.inner.state.lock();
        let Some(version) = state.versions.get_mut(&owner.version_id()) else {
            return Err("The installing Service Worker version no longer exists".to_owned());
        };
        if version.run_owner() != owner
            || version.lifecycle_state != ServiceWorkerVersionLifecycleState::Installing
        {
            return Err("The installing Service Worker run is no longer current".to_owned());
        }
        if rules.iter().any(|rule| !rule.condition.verify()) {
            return Err("Invalid static router condition".to_owned());
        }
        let mut all_rules = version.router_rules.clone();
        all_rules.extend(rules);
        if !within_registration_limit(&all_rules) {
            return Err(
                "Static router registration exceeds the condition count or nesting limit"
                    .to_owned(),
            );
        }
        version.router_rules = all_rules;
        Ok(())
    }
}
