use super::*;

#[test]
fn context_shutdown_aborts_pending_and_queued_register_jobs() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let first_version_id = ServiceWorkerVersionId(1);
    let scope_url = url("https://example.test/app/");
    let mut first_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let mut second_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

    insert_starting_version_with_register_job(
        &service,
        registration_id,
        first_version_id,
        url("https://example.test/app/worker-v1.js"),
        scope_url.clone(),
        11,
        first_queue.sender(),
    );
    push_queued_register_job(
        &service,
        registration_id,
        url("https://example.test/app/worker-v2.js"),
        scope_url,
        22,
        second_queue.sender(),
    );
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.queued_register_job_count, 1);

    service.terminate_all_for_context_shutdown();

    let first_completion = pop_register_completion(&mut first_queue);
    assert_eq!(first_completion.request_id, 11);
    assert_eq!(
        first_completion.result.err().as_deref(),
        Some(SERVICE_WORKER_JOB_ABORTED_ERROR)
    );
    let second_completion = pop_register_completion(&mut second_queue);
    assert_eq!(second_completion.request_id, 22);
    assert_eq!(
        second_completion.result.err().as_deref(),
        Some(SERVICE_WORKER_JOB_ABORTED_ERROR)
    );
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.queued_register_job_count, 0);
}
#[test]
fn context_shutdown_aborts_queued_unregister_jobs() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let first_version_id = ServiceWorkerVersionId(1);
    let scope_url = url("https://example.test/app/");
    let mut register_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let mut unregister_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

    insert_starting_version_with_register_job(
        &service,
        registration_id,
        first_version_id,
        url("https://example.test/app/worker-v1.js"),
        scope_url.clone(),
        11,
        register_queue.sender(),
    );

    assert_eq!(
        service.start_unregistration(&scope_url, 33, 1, unregister_queue.sender()),
        ServiceWorkerUnregisterStart::Queued
    );
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.queued_unregistration_job_count, 1);

    service.terminate_all_for_context_shutdown();

    let register_completion = pop_register_completion(&mut register_queue);
    assert_eq!(register_completion.request_id, 11);
    assert_eq!(
        register_completion.result.err().as_deref(),
        Some(SERVICE_WORKER_JOB_ABORTED_ERROR)
    );
    let unregister_completion = pop_unregister_completion(&mut unregister_queue);
    assert_eq!(unregister_completion.request_id, 33);
    assert!(!unregister_completion.result);
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.queued_unregistration_job_count, 0);
}
#[test]
fn navigation_preload_state_defaults_and_mutation_requires_active_worker() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let scope_url = url("https://example.test/app/");
    insert_inactive_registration(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        scope_url.clone(),
    );

    assert_eq!(service.navigation_preload_state_for_scope(&scope_url), None);
    assert_eq!(
        service.navigation_preload_state_for_scope(&url("https://example.test/other/")),
        None
    );
    assert_eq!(
        service.set_navigation_preload_enabled_for_scope(&scope_url, true),
        Err(ServiceWorkerNavigationPreloadStateError::InvalidState)
    );
    assert_eq!(
        service
            .set_navigation_preload_header_value_for_scope(&scope_url, "custom-preload".to_owned()),
        Err(ServiceWorkerNavigationPreloadStateError::InvalidState)
    );
    assert_eq!(
        snapshot_for_registration(&service, registration_id).navigation_preload_state(),
        &ServiceWorkerNavigationPreloadState::default()
    );
}
#[test]
fn navigation_preload_state_survives_initial_activation() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let scope_url = url("https://example.test/app/");
    insert_inactive_registration(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        scope_url.clone(),
    );
    make_version_persistable(&service, version_id);
    {
        let mut state = service.inner.state.lock();
        let registration = state.registrations.get_mut(&registration_id).unwrap();
        registration.installing_version_id = None;
        registration.waiting_version_id = Some(version_id);
        state.versions.get_mut(&version_id).unwrap().lifecycle_state =
            ServiceWorkerVersionLifecycleState::Installed;
    }
    assert_eq!(
        service.set_navigation_preload_enabled_for_scope(&scope_url, true),
        Err(ServiceWorkerNavigationPreloadStateError::InvalidState),
        "an installed worker that has not started activation is still inactive"
    );
    {
        let mut state = service.inner.state.lock();
        let version = state.versions.get_mut(&version_id).unwrap();
        version.lifecycle_state = ServiceWorkerVersionLifecycleState::Activating;
        version.in_flight_event_count = 1;
    }
    assert_eq!(
        service.navigation_preload_state_for_scope(&scope_url),
        Some(ServiceWorkerNavigationPreloadState::default())
    );
    service
        .set_navigation_preload_enabled_for_scope(&scope_url, true)
        .expect("enable must work during the initial activate event");
    service
        .set_navigation_preload_header_value_for_scope(&scope_url, "activate-preload".to_owned())
        .expect("setHeaderValue must work during the initial activate event");
    let expected = ServiceWorkerNavigationPreloadState {
        enabled: true,
        header_value: "activate-preload".to_owned(),
    };
    assert_eq!(
        service.navigation_preload_state_for_scope(&scope_url),
        Some(expected.clone())
    );
    let run = exact_version_run(&service, version_id);
    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(version_id, &run),
        kind: ServiceWorkerLifecycleEventKind::Activate,
        result: Ok(()),
    });
    assert_eq!(
        service.navigation_preload_state_for_scope(&scope_url),
        Some(expected.clone())
    );
    let stored = service
        .inner
        .resource_store
        .lock()
        .registrations()
        .into_iter()
        .find(|registration| registration.scope_url == scope_url)
        .expect("activation must persist the navigation preload state");
    assert_eq!(stored.navigation_preload_state, expected);
}

#[test]
fn navigation_preload_state_updates_active_registration_and_store() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let scope_url = url("https://example.test/app/");
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        scope_url.clone(),
        [],
    );
    make_version_persistable(&service, version_id);

    service
        .set_navigation_preload_enabled_for_scope(&scope_url, true)
        .expect("active navigation preload enable should resolve");
    service
        .set_navigation_preload_header_value_for_scope(&scope_url, "custom-preload".to_owned())
        .expect("active navigation preload header update should resolve");

    let expected = ServiceWorkerNavigationPreloadState {
        enabled: true,
        header_value: "custom-preload".to_owned(),
    };
    assert_eq!(
        service.navigation_preload_state_for_scope(&scope_url),
        Some(expected.clone())
    );
    assert_eq!(
        snapshot_for_registration(&service, registration_id).navigation_preload_state(),
        &expected
    );
    let registration_key = ServiceWorkerRegistrationKey {
        scope_url: scope_url.clone(),
        storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
    };
    let stored = service
        .inner
        .resource_store
        .lock()
        .registration_for_key(&registration_key)
        .expect("navigation preload update should persist active registration");
    assert_eq!(stored.navigation_preload_state, expected);
}
#[test]
fn navigation_preload_state_rolls_back_when_store_fails() {
    let temp_store = TempJsonStorePath::new("navigation-preload-state-failure");
    let resource_store =
        crate::new_shared_json_service_worker_resource_store(temp_store.store_path())
            .expect("json resource store should open");
    let service = new_service_worker_runtime_service_with_resource_store(
        resource_store.clone(),
        test_worker_context_runtime(),
    );
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let scope_url = url("https://example.test/app/");
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        scope_url.clone(),
        [],
    );
    make_version_persistable(&service, version_id);
    resource_store.lock().fail_next_persist_attempts_for_test(2);

    assert_eq!(
        service.set_navigation_preload_enabled_for_scope(&scope_url, true),
        Err(ServiceWorkerNavigationPreloadStateError::StorageFailure)
    );
    assert_eq!(
        service.navigation_preload_state_for_scope(&scope_url),
        Some(ServiceWorkerNavigationPreloadState::default())
    );
    assert_eq!(
        snapshot_for_registration(&service, registration_id).navigation_preload_state(),
        &ServiceWorkerNavigationPreloadState::default()
    );

    service
        .set_navigation_preload_enabled_for_scope(&scope_url, true)
        .expect("navigation preload update should succeed after transient store failure");
    assert!(
        service
            .navigation_preload_state_for_scope(&scope_url)
            .expect("registration should remain visible")
            .enabled
    );
}
#[test]
fn matching_controller_for_fetch_uses_controlled_document_scope() {
    let service = new_service_worker_runtime_service();
    let root = insert_registered_version(
        &service,
        ServiceWorkerRegistrationId(1),
        ServiceWorkerVersionId(1),
        url("https://example.test/root-worker.js"),
        url("https://example.test/"),
        [url("https://example.test/root.html")],
    );
    let app = insert_registered_version(
        &service,
        ServiceWorkerRegistrationId(2),
        ServiceWorkerVersionId(2),
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [url("https://example.test/app/page.html")],
    );

    assert_eq!(
        service.matching_controller_for_fetch(
            &url("https://example.test/app/page.html"),
            &url("https://example.test/app/api.json")
        ),
        Some(app.clone())
    );
    assert_eq!(
        service.matching_controller_for_fetch(
            &url("https://example.test/app/page.html"),
            &url("https://example.test/other/api.json")
        ),
        Some(app.clone())
    );
    assert_eq!(
        service.matching_controller_for_fetch(
            &url("https://example.test/app/page.html"),
            &url("https://other.test/api.json")
        ),
        Some(app)
    );
    assert_eq!(
        service.matching_controller_for_fetch(
            &url("https://example.test/root.html"),
            &url("https://example.test/other/api.json")
        ),
        Some(root)
    );
}
#[test]
fn pending_unregistration_waits_for_active_fetch_to_complete_before_delete() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let event_id = ServiceWorkerEventId(31);
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let document_url = url("https://example.test/app/page.html");
    let request_url = url("https://example.test/app/data.txt");
    let mut completion_queue = async_subresource_completion_queue();
    {
        let mut state = service.inner.state.lock();
        state.registrations.insert(
            registration_id,
            ServiceWorkerRegistration {
                id: registration_id,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
                scope_url: scope_url.clone(),
                script_url: script_url.clone(),
                installing_version_id: None,
                waiting_version_id: None,
                active_version_id: Some(version_id),
                pending_unregistration: true,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                last_update_check_time_ms: None,
                pending_register_jobs: HashMap::new(),
                controlled_client_ids: HashSet::new(),
            },
        );
        state.pending_fetch_jobs.insert(
            event_id,
            ServiceWorkerFetchJob {
                request: {
                    let origin = moli_url::WebOrigin::from_url(&document_url);
                    let initiator = (document_url).clone();
                    moli_fetch::Request::new_browser(
                        "GET",
                        request_url.clone(),
                        None,
                        Vec::new(),
                        origin,
                    )
                    .with_initiator_url(&initiator)
                    .with_request_mode(moli_fetch::RequestMode::Cors)
                    .with_credentials_mode(moli_fetch::RequestCredentialsMode::SameOrigin)
                    .with_redirect_mode(moli_fetch::RequestRedirectMode::Follow)
                    .with_fetch_priority_hint(None)
                },
                internal_id: 131,
                owner: Some(test_run_owner(version_id, &run)),
                cors_preflight_request_headers: Vec::new(),
                client_id: ServiceWorkerClientId::from_u64_for_test(0),
                resulting_client_id: None,
                destination: ServiceWorkerRequestDestination::Empty,
                is_reload: false,
                metadata: Default::default(),
                request_cookie_report: None,
                network_context: AsyncSubresourceNetworkContext {
                    frame_id: None,
                    request_origin: moli_url::WebOrigin::from_url(&document_url),
                    document_url,
                    resource_type: crate::types::SubresourceResourceType::Fetch,
                    policy_context: Default::default(),
                },
                completion_tx: completion_queue.sender(),
                request_client: test_request_client(&service),
                resource_task_runner: test_resource_task_runner(),
                cancel_handle: moli_fetch::FetchCancelHandle::new(),
                navigation_preload_cancel_handle: None,
                streaming_body_source_id: None,
                direct_completion_tx: None,
            },
        );
        state.versions.insert(
            version_id,
            ServiceWorkerVersion {
                id: version_id,
                registration_id,
                script_url: script_url.clone(),
                final_script_url: Some(script_url.clone()),
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: WorkerScriptKind::Classic,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: test_launch_config(&service, &script_url, &scope_url),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                running_state: ServiceWorkerVersionRunningState::Stopped,
                pending_start_events: VecDeque::new(),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 1,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.pending_unregistration_count, 1);
    assert_eq!(
        diagnostics.registrations[0].pending_clear_phase,
        Some("waiting-for-events")
    );

    service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
        event_id,
        owner: test_run_owner(version_id, &run),
        result: ServiceWorkerFetchResult::Failure("done".to_owned()),
    });

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 0);
    assert_eq!(diagnostics.version_count, 0);
    assert_eq!(diagnostics.pending_unregistration_count, 0);

    let completion = pop_async_subresource_completion(&mut completion_queue);
    assert_eq!(completion.internal_id, 131);
    assert_eq!(completion.result.err().as_deref(), Some("done"));
}
#[test]
fn register_aborts_pending_unregistration_and_starts_update() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let document_url = url("https://example.test/app/page.html");
    let scope_url = url("https://example.test/app/");
    let second_script_url = url("https://example.test/app/worker-v2.js");
    let browser_context_runtime = service.browser_context_runtime();
    let mut second_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let active_state = insert_registered_version(
        &service,
        registration_id,
        active_version_id,
        url("https://example.test/app/worker-v1.js"),
        scope_url.clone(),
        [document_url.clone()],
    );
    let client_id = client_id_for_document(&service, &document_url);

    assert!(service.mark_registration_unregistered(&scope_url));
    assert_eq!(
        service.matching_registration_for_client(&document_url),
        None
    );
    service.start_registration(
        second_script_url.clone(),
        scope_url.clone(),
        document_url.clone(),
        WorkerScriptKind::Classic,
        test_request_client(&service),
        WorkerNetworkPolicy::default(),
        browser_context_runtime,
        None,
        None,
        None,
        ServiceWorkerUpdateViaCache::Imports,
        22,
        1,
        second_queue.sender(),
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 2);
    assert_eq!(diagnostics.pending_unregistration_count, 0);
    assert_eq!(diagnostics.queued_register_job_count, 0);
    assert_eq!(diagnostics.starting_version_count, 1);
    assert_eq!(
        service
            .matching_registration_for_client(&document_url)
            .expect("register should abort pending unregister and restore visibility")
            .installing()
            .expect("different script should start installing immediately")
            .script_url(),
        &second_script_url
    );
    assert_eq!(
        service.matching_controller_for_fetch(
            &document_url,
            &url("https://example.test/app/data.json")
        ),
        Some(active_state)
    );
    assert!(!second_queue.has_ready_task());

    service.unregister_client(client_id);
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 2);
    assert_eq!(diagnostics.pending_unregistration_count, 0);
    assert_eq!(diagnostics.starting_version_count, 1);

    service.terminate_all_for_context_shutdown();
}
#[test]
fn same_options_register_aborts_pending_unregistration_fast_path() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let document_url = url("https://example.test/app/page.html");
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker-v1.js");
    let browser_context_runtime = service.browser_context_runtime();
    let mut completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    insert_registered_version(
        &service,
        registration_id,
        active_version_id,
        script_url.clone(),
        scope_url.clone(),
        [document_url.clone()],
    );

    assert!(service.mark_registration_unregistered(&scope_url));
    service.start_registration(
        script_url,
        scope_url.clone(),
        document_url.clone(),
        WorkerScriptKind::Classic,
        test_request_client(&service),
        WorkerNetworkPolicy::default(),
        browser_context_runtime,
        None,
        None,
        None,
        ServiceWorkerUpdateViaCache::Imports,
        33,
        1,
        completion_queue.sender(),
    );
    let completion = pop_register_completion(&mut completion_queue);
    assert_eq!(completion.request_id, 33);
    let snapshot = completion
        .result
        .expect("same-options register should resolve restored registration");
    assert_eq!(snapshot.registration_id(), registration_id);
    assert!(snapshot.installing().is_none());
    assert_eq!(
        snapshot
            .active()
            .expect("active version should remain visible")
            .version_id(),
        active_version_id
    );
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.pending_unregistration_count, 0);
    assert_eq!(diagnostics.queued_register_job_count, 0);
    assert_eq!(diagnostics.starting_version_count, 0);
    assert!(
        service
            .matching_registration_for_client(&document_url)
            .is_some()
    );
}
#[test]
fn resource_store_restores_no_fetch_handler_metadata_for_controlled_fetch() {
    let resource_store = new_shared_service_worker_resource_store();
    let first_service = new_service_worker_runtime_service_with_resource_store(
        resource_store.clone(),
        test_worker_context_runtime(),
    );
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let script_url = url("https://example.test/app/worker.js");
    let scope_url = url("https://example.test/app/");
    let document_url = url("https://example.test/app/page.html");
    let request_url = url("https://example.test/app/data.txt");
    insert_registered_version(
        &first_service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url.clone(),
        [],
    );
    {
        let mut state = first_service.inner.state.lock();
        let version = state
            .versions
            .get_mut(&version_id)
            .expect("version should exist");
        version.main_script_resource = Some(test_script_resource(&script_url));
        version.fetch_handler_existence = ServiceWorkerFetchHandlerExistence::DoesNotExist;
        version.fetch_handler_type = ServiceWorkerFetchHandlerType::NoHandler;
    }
    assert!(
        first_service
            .store_registration_resources_for_test(registration_id, version_id)
            .expect("stored no-handler registration should persist")
    );

    let second_service = new_service_worker_runtime_service_with_resource_store(
        resource_store,
        test_worker_context_runtime(),
    );
    let client_id =
        second_service.register_client(document_url.clone(), 1, test_completion_sender());
    let restored_version_id = {
        let state = second_service.inner.state.lock();
        let restored_registration = state
            .registrations
            .values()
            .find(|registration| registration.scope_url == scope_url)
            .expect("stored registration should restore at startup");
        assert!(
            restored_registration
                .controlled_client_ids
                .contains(&client_id),
            "new client should select the restored active controller"
        );
        let restored_version_id = restored_registration
            .active_version_id
            .expect("restored registration should be active");
        let version = state
            .versions
            .get(&restored_version_id)
            .expect("restored version should exist");
        assert_eq!(
            version.fetch_handler_existence,
            ServiceWorkerFetchHandlerExistence::DoesNotExist
        );
        assert_eq!(
            version.fetch_handler_type,
            ServiceWorkerFetchHandlerType::NoHandler
        );
        restored_version_id
    };

    let mut completion_queue = async_subresource_completion_queue();
    let (direct_completion_tx, mut direct_completion_rx) = tokio::sync::oneshot::channel();
    assert!(
        second_service.dispatch_controlled_fetch(ServiceWorkerFetchDispatch {
            redirect_check: None,
            internal_id: 89,
            request: ServiceWorkerFetchRequest {
                client_id,
                resulting_client_id: None,
                url: request_url,
                method: "GET".to_owned(),
                headers: Vec::new(),
                body: None,
                destination: ServiceWorkerRequestDestination::Empty,
                request_mode: moli_fetch::RequestMode::Cors,
                credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                priority: None,
                is_reload: false,
                metadata: Default::default(),
            },
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report: None,
            network_context: AsyncSubresourceNetworkContext {
                frame_id: None,
                request_origin: moli_url::WebOrigin::from_url(&document_url),
                document_url,
                resource_type: crate::types::SubresourceResourceType::Fetch,
                policy_context: Default::default(),
            },
            completion_tx: completion_queue.sender(),
            request_client: test_request_client(&second_service),
            resource_task_runner: test_resource_task_runner(),
            cancel_handle: moli_fetch::FetchCancelHandle::new(),
            direct_completion_tx: Some(direct_completion_tx),
        })
    );

    {
        let state = second_service.inner.state.lock();
        assert!(state.pending_fetch_jobs.is_empty());
        let version = state
            .versions
            .get(&restored_version_id)
            .expect("restored version should still exist");
        assert_eq!(version.in_flight_event_count, 0);
        assert!(matches!(
            version.running_state,
            ServiceWorkerVersionRunningState::Stopped
        ));
    }

    expect_direct_fetch_fallback(&mut direct_completion_rx);
    assert!(!completion_queue.has_ready_completion());
}
#[test]
fn resource_store_failure_aborts_identical_update_check_register_job() {
    let failing_store = FailingJsonStorePath::new("update-check");
    let resource_store =
        crate::new_shared_json_service_worker_resource_store(failing_store.store_path())
            .expect("failing json resource store should open before first write");
    let service = new_service_worker_runtime_service_with_resource_store(
        resource_store,
        test_worker_context_runtime(),
    );
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let script_url = url("https://example.test/app/worker.js");
    let scope_url = url("https://example.test/app/");
    insert_registered_version(
        &service,
        registration_id,
        active_version_id,
        script_url.clone(),
        scope_url.clone(),
        [],
    );
    {
        let mut state = service.inner.state.lock();
        state
            .versions
            .get_mut(&active_version_id)
            .expect("active version should exist")
            .main_script_resource = Some(test_script_resource(&script_url));
    }
    let mut completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    insert_pending_main_script_update_check(
        &service,
        registration_id,
        active_version_id,
        script_url.clone(),
        scope_url,
        77,
        completion_queue.sender(),
    );

    service.finish_main_script_update_check_completed(
        registration_id,
        Ok(test_update_check_result(
            &script_url,
            "self.skipWaiting();",
            false,
        )),
    );

    let completion = pop_register_completion(&mut completion_queue);
    assert_eq!(completion.request_id, 77);
    let error = completion
        .result
        .expect_err("store failure should reject update check register job");
    assert_eq!(
        error.kind,
        crate::service_worker_runtime::ServiceWorkerRegistrationErrorKind::Abort
    );
    assert!(
        error
            .message
            .contains("failed to store Service Worker registration resources")
    );
    let diagnostics = service.diagnostics_snapshot();
    let update_check = diagnostics.registrations[0]
        .last_main_script_update_check
        .as_ref()
        .expect("store failure should record update-check diagnostics");
    assert_eq!(update_check.result, "store-failed");
    assert_eq!(update_check.failure_status, Some("abort"));
}
#[test]
fn restart_start_failure_rejects_pending_fetch_job() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let document_url = url("https://example.test/app/page.html");
    let request_url = url("https://example.test/app/data.txt");
    let client_id = register_client_for_test(&service, document_url.clone());
    let event_id = ServiceWorkerEventId(7);
    let mut completion_queue = async_subresource_completion_queue();
    let host = new_loading_test_host(version_id, &run);
    {
        let mut state = service.inner.state.lock();
        state.registrations.insert(
            registration_id,
            ServiceWorkerRegistration {
                id: registration_id,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
                scope_url: scope_url.clone(),
                script_url: script_url.clone(),
                installing_version_id: None,
                waiting_version_id: None,
                active_version_id: Some(version_id),
                pending_unregistration: false,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                last_update_check_time_ms: None,
                pending_register_jobs: HashMap::new(),
                controlled_client_ids: HashSet::from([client_id]),
            },
        );
        state.pending_fetch_jobs.insert(
            event_id,
            ServiceWorkerFetchJob {
                request: {
                    let origin = moli_url::WebOrigin::from_url(&document_url);
                    let initiator = document_url.clone();
                    moli_fetch::Request::new_browser(
                        "GET",
                        request_url.clone(),
                        None,
                        Vec::new(),
                        origin,
                    )
                    .with_initiator_url(&initiator)
                    .with_request_mode(moli_fetch::RequestMode::Cors)
                    .with_credentials_mode(moli_fetch::RequestCredentialsMode::SameOrigin)
                    .with_redirect_mode(moli_fetch::RequestRedirectMode::Follow)
                    .with_fetch_priority_hint(None)
                },
                internal_id: 41,
                owner: Some(test_run_owner(version_id, &run)),
                cors_preflight_request_headers: Vec::new(),
                client_id: ServiceWorkerClientId::from_u64_for_test(0),
                resulting_client_id: None,
                destination: ServiceWorkerRequestDestination::Empty,
                is_reload: false,
                metadata: Default::default(),
                request_cookie_report: None,
                network_context: AsyncSubresourceNetworkContext {
                    frame_id: None,
                    request_origin: moli_url::WebOrigin::from_url(&document_url),
                    document_url: document_url.clone(),
                    resource_type: crate::types::SubresourceResourceType::Fetch,
                    policy_context: Default::default(),
                },
                completion_tx: completion_queue.sender(),
                request_client: test_request_client(&service),
                resource_task_runner: test_resource_task_runner(),
                cancel_handle: moli_fetch::FetchCancelHandle::new(),
                navigation_preload_cancel_handle: None,
                streaming_body_source_id: None,
                direct_completion_tx: None,
            },
        );
        state.versions.insert(
            version_id,
            ServiceWorkerVersion {
                id: version_id,
                registration_id,
                script_url: script_url.clone(),
                final_script_url: Some(script_url.clone()),
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: WorkerScriptKind::Classic,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: test_launch_config(&service, &script_url, &scope_url),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                running_state: ServiceWorkerVersionRunningState::Starting { host },
                pending_start_events: VecDeque::from([ServiceWorkerPendingStartEvent::Fetch(
                    ServiceWorkerFetchEvent {
                        event_id,
                        owner: test_run_owner(version_id, &run),
                        request: ServiceWorkerFetchRequest {
                            client_id,
                            resulting_client_id: None,
                            url: request_url,
                            method: "GET".to_owned(),
                            headers: Vec::new(),
                            body: None,
                            destination: ServiceWorkerRequestDestination::Empty,
                            request_mode: moli_fetch::RequestMode::Cors,
                            credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                            redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                            priority: None,
                            is_reload: false,
                            metadata: Default::default(),
                        },
                        navigation_preload_sent: false,
                    },
                )]),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 1,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    service.finish_worker_start_failed(
        version_id,
        run,
        ServiceWorkerVersionStartFailure::ScriptLoad {
            message: "restart script load failed".to_owned(),
        },
    );

    {
        let state = service.inner.state.lock();
        assert!(state.pending_fetch_jobs.is_empty());
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 0);
        assert!(matches!(
            version.running_state,
            ServiceWorkerVersionRunningState::Stopped
        ));
        assert_eq!(
            version.last_start_error.as_deref(),
            Some("restart script load failed")
        );
    }

    let completion = pop_async_subresource_completion(&mut completion_queue);
    assert_eq!(completion.internal_id, 41);
    assert_eq!(
        completion.result.err().as_deref(),
        Some("restart script load failed")
    );
}
#[test]
fn abort_controlled_fetch_clears_running_job_and_ignores_late_completion() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let event_id = ServiceWorkerEventId(17);
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let document_url = url("https://example.test/app/page.html");
    let request_url = url("https://example.test/app/data.txt");
    let mut completion_queue = async_subresource_completion_queue();
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url,
        scope_url,
        [document_url.clone()],
    );
    let client_id = client_id_for_document(&service, &document_url);
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    {
        let mut state = service.inner.state.lock();
        let version = state.versions.get_mut(&version_id).unwrap();
        version.run = run.clone();
        version.running_state = ServiceWorkerVersionRunningState::Running {
            host: new_running_test_host(version_id, &run),
        };
        version.in_flight_event_count = 1;
        state.pending_fetch_jobs.insert(
            event_id,
            test_fetch_job(
                &service,
                51,
                version_id,
                &run,
                client_id,
                document_url,
                request_url,
                completion_queue.sender(),
                cancel_handle.clone(),
            ),
        );
    }

    assert!(service.abort_controlled_fetch(51));
    assert!(cancel_handle.is_cancelled());
    {
        let state = service.inner.state.lock();
        assert!(state.pending_fetch_jobs.is_empty());
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 0);
        assert!(version.pending_start_events.is_empty());
        assert!(version.pending_activation_fetch_events.is_empty());
    }
    assert!(!completion_queue.has_ready_completion());

    service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
        event_id,
        owner: test_run_owner(version_id, &run),
        result: ServiceWorkerFetchResult::Fallback,
    });
    {
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(
            version.in_flight_event_count, 0,
            "late completion after abort must not decrement unrelated event accounting"
        );
        assert!(state.pending_fetch_jobs.is_empty());
    }
    assert!(!completion_queue.has_ready_completion());
}
#[test]
fn navigation_preload_fallback_completion_cancels_unstarted_preload_only() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let event_id = ServiceWorkerEventId(60);
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let document_url = url("https://example.test/app/page.html");
    let request_url = url("https://example.test/app/data.txt");
    let mut completion_queue = async_subresource_completion_queue();
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url,
        scope_url,
        [document_url.clone()],
    );
    let client_id = client_id_for_document(&service, &document_url);
    let fallback_cancel_handle = moli_fetch::FetchCancelHandle::new();
    let navigation_preload_cancel_handle = moli_fetch::FetchCancelHandle::new();
    insert_pending_navigation_preload_fetch_job(
        &service,
        event_id,
        301,
        version_id,
        &run,
        client_id,
        document_url,
        request_url,
        completion_queue.sender(),
        fallback_cancel_handle.clone(),
        navigation_preload_cancel_handle.clone(),
    );
    let (direct_completion_tx, mut direct_completion_rx) = tokio::sync::oneshot::channel();
    service
        .inner
        .state
        .lock()
        .pending_fetch_jobs
        .get_mut(&event_id)
        .expect("pending navigation preload fetch job")
        .direct_completion_tx = Some(direct_completion_tx);

    service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
        event_id,
        owner: test_run_owner(version_id, &run),
        result: ServiceWorkerFetchResult::Fallback,
    });

    assert!(
        navigation_preload_cancel_handle.is_cancelled(),
        "fallback completion must cancel the parallel preload before it has response-started"
    );
    assert!(
        !fallback_cancel_handle.is_cancelled(),
        "preload cancellation must not abort the main fallback fetch handle"
    );
    {
        let state = service.inner.state.lock();
        assert!(state.pending_fetch_jobs.is_empty());
        assert_eq!(
            state
                .versions
                .get(&version_id)
                .unwrap()
                .in_flight_event_count,
            0
        );
    }
    expect_direct_fetch_fallback(&mut direct_completion_rx);
    assert!(!completion_queue.has_ready_completion());
}
#[test]
fn navigation_preload_completion_keeps_response_started_preload_alive() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let event_id = ServiceWorkerEventId(61);
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let document_url = url("https://example.test/app/page.html");
    let request_url = url("https://example.test/app/data.txt");
    let mut completion_queue = async_subresource_completion_queue();
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url,
        scope_url,
        [document_url.clone()],
    );
    let client_id = client_id_for_document(&service, &document_url);
    let fetch_cancel_handle = moli_fetch::FetchCancelHandle::new();
    let navigation_preload_cancel_handle = moli_fetch::FetchCancelHandle::new();
    insert_pending_navigation_preload_fetch_job(
        &service,
        event_id,
        302,
        version_id,
        &run,
        client_id,
        document_url,
        request_url,
        completion_queue.sender(),
        fetch_cancel_handle,
        navigation_preload_cancel_handle.clone(),
    );

    assert!(
        service.mark_navigation_preload_response_started(
            event_id,
            &test_run_owner(version_id, &run),
        )
    );
    assert!(
        !navigation_preload_cancel_handle.is_cancelled(),
        "marking response-started should detach the runtime cancel handle, not cancel it"
    );

    service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
        event_id,
        owner: test_run_owner(version_id, &run),
        result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
            cors_exposed_header_names: None,
            status: 200,
            status_text: "OK".to_owned(),
            headers: vec![("content-type".to_owned(), b"text/plain".to_vec())],
            body: b"handled".to_vec(),
            final_url: None,
            response_type: "basic".to_owned(),
            redirected: false,
        }),
    });

    assert!(
        !navigation_preload_cancel_handle.is_cancelled(),
        "FetchEvent completion must not cancel a preload response already handed to the worker"
    );
    let completion = pop_async_subresource_completion(&mut completion_queue);
    assert_eq!(completion.internal_id, 302);
    assert!(completion.result.is_ok());
}
#[test]
fn abort_controlled_fetch_cancels_unstarted_navigation_preload() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let event_id = ServiceWorkerEventId(62);
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let document_url = url("https://example.test/app/page.html");
    let request_url = url("https://example.test/app/data.txt");
    let mut completion_queue = async_subresource_completion_queue();
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url,
        scope_url,
        [document_url.clone()],
    );
    let client_id = client_id_for_document(&service, &document_url);
    let fetch_cancel_handle = moli_fetch::FetchCancelHandle::new();
    let navigation_preload_cancel_handle = moli_fetch::FetchCancelHandle::new();
    insert_pending_navigation_preload_fetch_job(
        &service,
        event_id,
        303,
        version_id,
        &run,
        client_id,
        document_url,
        request_url,
        completion_queue.sender(),
        fetch_cancel_handle.clone(),
        navigation_preload_cancel_handle.clone(),
    );

    assert!(service.abort_controlled_fetch(303));

    assert!(fetch_cancel_handle.is_cancelled());
    assert!(navigation_preload_cancel_handle.is_cancelled());
    {
        let state = service.inner.state.lock();
        assert!(state.pending_fetch_jobs.is_empty());
        assert_eq!(
            state
                .versions
                .get(&version_id)
                .unwrap()
                .in_flight_event_count,
            0
        );
    }
    assert!(!completion_queue.has_ready_completion());
}
#[test]
fn abort_controlled_fetch_records_aborted_network_diagnostic() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let event_id = ServiceWorkerEventId(20);
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let document_url = url("https://example.test/app/page.html");
    let request_url = url("https://example.test/app/data.txt");
    let mut completion_queue = async_subresource_completion_queue();
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url.clone(),
        [document_url.clone()],
    );
    let client_id = client_id_for_document(&service, &document_url);
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    {
        let mut state = service.inner.state.lock();
        state.record_target_created(registration_id, version_id, script_url, scope_url);
        service.take_target_output_events_for_test();
        let version = state.versions.get_mut(&version_id).unwrap();
        version.run = run.clone();
        version.running_state = ServiceWorkerVersionRunningState::Running {
            host: new_running_test_host(version_id, &run),
        };
        version.in_flight_event_count = 1;
        state.pending_fetch_jobs.insert(
            event_id,
            test_fetch_job(
                &service,
                56,
                version_id,
                &run,
                client_id,
                document_url,
                request_url,
                completion_queue.sender(),
                cancel_handle.clone(),
            ),
        );
    }

    assert!(service.abort_controlled_fetch(56));
    assert!(cancel_handle.is_cancelled());
    assert!(!completion_queue.has_ready_completion());

    let target_events = service.take_target_output_events_for_test();
    let Some(crate::runtime::RendererServiceWorkerTargetEvent::FetchDiagnostic {
        version_id: diagnostic_version_id,
        diagnostic,
        ..
    }) = target_events.iter().find(|event| {
        matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::FetchDiagnostic { .. }
        )
    })
    else {
        panic!("expected fetch diagnostic event after abort, got {target_events:?}");
    };
    assert_eq!(*diagnostic_version_id, version_id.as_u64());
    assert_eq!(diagnostic.internal_id, 56);
    assert_eq!(
        diagnostic.result,
        crate::runtime::RendererServiceWorkerFetchDiagnosticResult::Failure {
            message: crate::network_host::ABORTED_ERROR_TEXT.to_owned()
        }
    );
}
#[test]
fn abort_controlled_fetch_cancels_running_stream_reader() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let event_id = ServiceWorkerEventId(18);
    let body_source_id = 77;
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let document_url = url("https://example.test/app/page.html");
    let request_url = url("https://example.test/app/data.txt");
    let completion_queue = async_subresource_completion_queue();
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url,
        scope_url,
        [document_url.clone()],
    );
    let client_id = client_id_for_document(&service, &document_url);
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let (worker_tx, mut worker_rx) = tokio::sync::mpsc::unbounded_channel();
    let (_parent_tx, parent_rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = crate::worker::WorkerHandle::new(
        worker_tx,
        parent_rx,
        std::thread::spawn(|| {}),
        Arc::new(parking_lot::Mutex::new(None)),
    );
    {
        let mut state = service.inner.state.lock();
        let version = state.versions.get_mut(&version_id).unwrap();
        version.run = run.clone();
        version.running_state = ServiceWorkerVersionRunningState::Running {
            host: new_running_test_host_with_handle(version_id, &run, handle),
        };
        version.in_flight_event_count = 1;
        let mut job = test_fetch_job(
            &service,
            55,
            version_id,
            &run,
            client_id,
            document_url,
            request_url,
            completion_queue.sender(),
            cancel_handle.clone(),
        );
        job.streaming_body_source_id = Some(body_source_id);
        state.pending_fetch_jobs.insert(event_id, job);
    }

    assert!(service.abort_controlled_fetch(55));
    assert!(cancel_handle.is_cancelled());
    match worker_rx.try_recv() {
        Ok(crate::worker::WorkerMessage::ServiceWorkerFetchRequestSignalAbort {
            event_id: actual_event_id,
            reason,
        }) => {
            assert_eq!(actual_event_id, event_id);
            assert!(reason.is_none());
        }
        other => panic!("expected request signal abort worker message, got {other:?}"),
    }
    match worker_rx.try_recv() {
        Ok(crate::worker::WorkerMessage::ServiceWorkerFetchStreamCancel {
            event_id: actual_event_id,
            body_source_id: actual_body_source_id,
        }) => {
            assert_eq!(actual_event_id, event_id);
            assert_eq!(actual_body_source_id, body_source_id);
        }
        other => panic!("expected stream cancel worker message, got {other:?}"),
    }
    {
        let state = service.inner.state.lock();
        assert!(state.pending_fetch_jobs.is_empty());
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 0);
    }
}
#[test]
fn abort_controlled_fetch_drops_direct_worker_completion_and_ignores_late_completion() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let event_id = ServiceWorkerEventId(19);
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let document_url = url("https://example.test/app/dedicated-worker.js");
    let request_url = url("https://example.test/app/data.txt");
    let mut completion_queue = async_subresource_completion_queue();
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url,
        scope_url,
        [document_url.clone()],
    );
    let client_id = client_id_for_document(&service, &document_url);
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let (direct_completion_tx, mut direct_completion_rx) = tokio::sync::oneshot::channel();
    {
        let mut state = service.inner.state.lock();
        let version = state.versions.get_mut(&version_id).unwrap();
        version.run = run.clone();
        version.running_state = ServiceWorkerVersionRunningState::Running {
            host: new_running_test_host(version_id, &run),
        };
        version.in_flight_event_count = 1;
        let mut job = test_fetch_job(
            &service,
            53,
            version_id,
            &run,
            client_id,
            document_url,
            request_url,
            completion_queue.sender(),
            cancel_handle.clone(),
        );
        job.direct_completion_tx = Some(direct_completion_tx);
        state.pending_fetch_jobs.insert(event_id, job);
    }

    assert!(service.abort_controlled_fetch(53));
    assert!(cancel_handle.is_cancelled());
    assert!(matches!(
        direct_completion_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed)
    ));
    {
        let state = service.inner.state.lock();
        assert!(state.pending_fetch_jobs.is_empty());
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 0);
        assert!(version.pending_start_events.is_empty());
        assert!(version.pending_activation_fetch_events.is_empty());
    }
    assert!(!completion_queue.has_ready_completion());

    service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
        event_id,
        owner: test_run_owner(version_id, &run),
        result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
            cors_exposed_header_names: None,
            status: 200,
            status_text: "OK".to_owned(),
            headers: vec![("content-type".to_owned(), b"text/plain".to_vec())],
            body: b"late".to_vec(),
            final_url: None,
            response_type: "basic".to_owned(),
            redirected: false,
        }),
    });
    {
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(
            version.in_flight_event_count, 0,
            "late direct completion after abort must not touch event accounting"
        );
        assert!(state.pending_fetch_jobs.is_empty());
    }
    assert!(!completion_queue.has_ready_completion());
}
#[test]
fn abort_controlled_fetch_removes_pending_start_event() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let event_id = ServiceWorkerEventId(18);
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let document_url = url("https://example.test/app/page.html");
    let request_url = url("https://example.test/app/data.txt");
    let mut completion_queue = async_subresource_completion_queue();
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url,
        scope_url,
        [document_url.clone()],
    );
    let client_id = client_id_for_document(&service, &document_url);
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    {
        let mut state = service.inner.state.lock();
        state.pending_fetch_jobs.insert(
            event_id,
            test_fetch_job(
                &service,
                52,
                version_id,
                &run,
                client_id,
                document_url.clone(),
                request_url.clone(),
                completion_queue.sender(),
                cancel_handle.clone(),
            ),
        );
        let version = state.versions.get_mut(&version_id).unwrap();
        version.run = run.clone();
        version.running_state = ServiceWorkerVersionRunningState::Starting {
            host: new_loading_test_host(version_id, &run),
        };
        version.pending_start_events = VecDeque::from([ServiceWorkerPendingStartEvent::Fetch(
            ServiceWorkerFetchEvent {
                event_id,
                owner: test_run_owner(version_id, &run),
                request: test_fetch_request(client_id, request_url),
                navigation_preload_sent: false,
            },
        )]);
        version.in_flight_event_count = 1;
    }

    assert!(service.abort_controlled_fetch(52));
    assert!(cancel_handle.is_cancelled());
    {
        let state = service.inner.state.lock();
        assert!(state.pending_fetch_jobs.is_empty());
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 0);
        assert!(version.pending_start_events.is_empty());
    }
    assert!(!completion_queue.has_ready_completion());

    service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
        event_id,
        owner: test_run_owner(version_id, &run),
        result: ServiceWorkerFetchResult::Fallback,
    });
    {
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 0);
        assert!(state.pending_fetch_jobs.is_empty());
    }
    assert!(!completion_queue.has_ready_completion());
}
#[test]
fn no_fetch_handler_controlled_fetch_falls_back_without_dispatching_event() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let document_url = url("https://example.test/app/page.html");
    let request_url = url("https://example.test/app/data.txt");
    let client_id = register_client_for_test(&service, document_url.clone());
    let mut completion_queue = async_subresource_completion_queue();
    let host = new_running_test_host(version_id, &run);
    {
        let mut state = service.inner.state.lock();
        state.registrations.insert(
            registration_id,
            ServiceWorkerRegistration {
                id: registration_id,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
                scope_url: scope_url.clone(),
                script_url: script_url.clone(),
                installing_version_id: None,
                waiting_version_id: None,
                active_version_id: Some(version_id),
                pending_unregistration: false,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                last_update_check_time_ms: None,
                pending_register_jobs: HashMap::new(),
                controlled_client_ids: HashSet::from([client_id]),
            },
        );
        state.versions.insert(
            version_id,
            ServiceWorkerVersion {
                id: version_id,
                registration_id,
                script_url: script_url.clone(),
                final_script_url: Some(script_url.clone()),
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: WorkerScriptKind::Classic,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::DoesNotExist,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: test_launch_config(&service, &script_url, &scope_url),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                running_state: ServiceWorkerVersionRunningState::Running { host },
                pending_start_events: VecDeque::new(),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 0,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    let (direct_completion_tx, mut direct_completion_rx) = tokio::sync::oneshot::channel();
    assert!(
        service.dispatch_controlled_fetch(ServiceWorkerFetchDispatch {
            redirect_check: None,
            internal_id: 88,
            request: ServiceWorkerFetchRequest {
                client_id,
                resulting_client_id: None,
                url: request_url.clone(),
                method: "GET".to_owned(),
                headers: Vec::new(),
                body: None,
                destination: ServiceWorkerRequestDestination::Empty,
                request_mode: moli_fetch::RequestMode::Cors,
                credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                priority: None,
                is_reload: false,
                metadata: Default::default(),
            },
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report: None,
            network_context: AsyncSubresourceNetworkContext {
                frame_id: None,
                request_origin: moli_url::WebOrigin::from_url(&document_url),
                document_url: document_url.clone(),
                resource_type: crate::types::SubresourceResourceType::Fetch,
                policy_context: Default::default(),
            },
            completion_tx: completion_queue.sender(),
            request_client: test_request_client(&service),
            resource_task_runner: test_resource_task_runner(),
            cancel_handle: moli_fetch::FetchCancelHandle::new(),
            direct_completion_tx: Some(direct_completion_tx),
        })
    );

    {
        let state = service.inner.state.lock();
        assert!(state.pending_fetch_jobs.is_empty());
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 0);
        assert!(matches!(
            version.running_state,
            ServiceWorkerVersionRunningState::Running { .. }
        ));
        assert!(
            state
                .registrations
                .get(&registration_id)
                .unwrap()
                .controlled_client_ids
                .contains(&client_id),
            "no-handler fallback must not clear the active controller"
        );
    }

    expect_direct_fetch_fallback(&mut direct_completion_rx);
    assert!(!completion_queue.has_ready_completion());
}
#[test]
fn controlled_fetch_waits_for_activating_active_version() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let document_url = url("https://example.test/app/page.html");
    let request_url = url("https://example.test/app/");
    let mut completion_queue = async_subresource_completion_queue();
    let (direct_completion_tx, mut main_resource_completion_rx) =
        tokio::sync::oneshot::channel();
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url,
        [document_url.clone()],
    );
    let client_id = client_id_for_document(&service, &document_url);
    let host = new_running_test_host(version_id, &run);
    {
        let mut state = service.inner.state.lock();
        let version = state.versions.get_mut(&version_id).unwrap();
        version.fetch_handler_existence = ServiceWorkerFetchHandlerExistence::DoesNotExist;
        version.fetch_handler_type = ServiceWorkerFetchHandlerType::NoHandler;
        version.lifecycle_state = ServiceWorkerVersionLifecycleState::Activating;
        version.running_state = ServiceWorkerVersionRunningState::Running { host };
        version.in_flight_event_count = 1;
        version.run = run.clone();
    }

    assert!(
        service.dispatch_controlled_fetch(ServiceWorkerFetchDispatch {
            redirect_check: None,
            internal_id: 91,
            request: ServiceWorkerFetchRequest {
                client_id,
                resulting_client_id: None,
                url: request_url,
                method: "GET".to_owned(),
                headers: Vec::new(),
                body: None,
                destination: ServiceWorkerRequestDestination::Document,
                request_mode: moli_fetch::RequestMode::Navigate,
                credentials_mode: moli_fetch::RequestCredentialsMode::Include,
                redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                priority: None,
                is_reload: false,
                metadata: Default::default(),
            },
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report: None,
            network_context: AsyncSubresourceNetworkContext {
                frame_id: None,
                request_origin: moli_url::WebOrigin::from_url(&document_url),
                document_url: document_url.clone(),
                resource_type: crate::types::SubresourceResourceType::Fetch,
                policy_context: Default::default(),
            },
            completion_tx: completion_queue.sender(),
            request_client: test_request_client(&service),
            resource_task_runner: test_resource_task_runner(),
            cancel_handle: moli_fetch::FetchCancelHandle::new(),
            direct_completion_tx: Some(direct_completion_tx),
        })
    );

    {
        let state = service.inner.state.lock();
        assert_eq!(state.pending_fetch_jobs.len(), 1);
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 1);
        assert!(version.pending_start_events.is_empty());
        assert_eq!(version.pending_activation_fetch_events.len(), 1);
        let event = version.pending_activation_fetch_events.front().unwrap();
        assert_eq!(event.owner, test_run_owner(version_id, &run));
        assert_eq!(
            event.request.destination,
            ServiceWorkerRequestDestination::Document
        );
    }
    assert!(matches!(
        main_resource_completion_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(77),
        owner: test_run_owner(version_id, &run),
        kind: ServiceWorkerLifecycleEventKind::Activate,
        result: Ok(()),
    });

    {
        let state = service.inner.state.lock();
        assert!(state.pending_fetch_jobs.is_empty());
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(
            version.lifecycle_state,
            ServiceWorkerVersionLifecycleState::Activated
        );
        assert_eq!(version.in_flight_event_count, 0);
        assert!(version.pending_activation_fetch_events.is_empty());
    }
    assert!(matches!(
        main_resource_completion_rx.try_recv(),
        Ok(ServiceWorkerDirectFetchResult::Fallback)
    ));
    assert!(!completion_queue.has_ready_completion());
}
#[test]
fn empty_fetch_handler_controlled_fetch_falls_back_without_dispatching_event() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let document_url = url("https://example.test/app/page.html");
    let request_url = url("https://example.test/app/data.txt");
    let mut completion_queue = async_subresource_completion_queue();
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url,
        [document_url.clone()],
    );
    let client_id = client_id_for_document(&service, &document_url);
    let host = new_running_test_host(version_id, &run);
    {
        let mut state = service.inner.state.lock();
        let version = state.versions.get_mut(&version_id).unwrap();
        version.fetch_handler_existence = ServiceWorkerFetchHandlerExistence::Exists;
        version.fetch_handler_type = ServiceWorkerFetchHandlerType::EmptyFetchHandler;
        version.running_state = ServiceWorkerVersionRunningState::Running { host };
        version.run = run.clone();
    }

    let (direct_completion_tx, mut direct_completion_rx) = tokio::sync::oneshot::channel();
    assert!(
        service.dispatch_controlled_fetch(ServiceWorkerFetchDispatch {
            redirect_check: None,
            internal_id: 89,
            request: ServiceWorkerFetchRequest {
                client_id,
                resulting_client_id: None,
                url: request_url,
                method: "GET".to_owned(),
                headers: Vec::new(),
                body: None,
                destination: ServiceWorkerRequestDestination::Empty,
                request_mode: moli_fetch::RequestMode::Cors,
                credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                priority: None,
                is_reload: false,
                metadata: Default::default(),
            },
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report: None,
            network_context: AsyncSubresourceNetworkContext {
                frame_id: None,
                request_origin: moli_url::WebOrigin::from_url(&document_url),
                document_url: document_url.clone(),
                resource_type: crate::types::SubresourceResourceType::Fetch,
                policy_context: Default::default(),
            },
            completion_tx: completion_queue.sender(),
            request_client: test_request_client(&service),
            resource_task_runner: test_resource_task_runner(),
            cancel_handle: moli_fetch::FetchCancelHandle::new(),
            direct_completion_tx: Some(direct_completion_tx),
        })
    );

    {
        let state = service.inner.state.lock();
        assert!(state.pending_fetch_jobs.is_empty());
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 0);
        assert!(matches!(
            version.running_state,
            ServiceWorkerVersionRunningState::Running { .. }
        ));
        assert!(
            state
                .registrations
                .get(&registration_id)
                .unwrap()
                .controlled_client_ids
                .contains(&client_id),
            "empty-handler fallback must not clear the active controller"
        );
    }

    expect_direct_fetch_fallback(&mut direct_completion_rx);
    assert!(!completion_queue.has_ready_completion());
}
#[test]
fn empty_fetch_handler_pending_start_fetch_falls_back_after_worker_start() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let document_url = url("https://example.test/app/page.html");
    let request_url = url("https://example.test/app/data.txt");
    let event_id = ServiceWorkerEventId(13);
    let mut completion_queue = async_subresource_completion_queue();
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url,
        [document_url.clone()],
    );
    let client_id = client_id_for_document(&service, &document_url);
    let host = new_loading_test_host(version_id, &run);
    let request = ServiceWorkerFetchRequest {
        client_id,
        resulting_client_id: None,
        url: request_url.clone(),
        method: "GET".to_owned(),
        headers: Vec::new(),
        body: None,
        destination: ServiceWorkerRequestDestination::Empty,
        request_mode: moli_fetch::RequestMode::Cors,
        credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
        redirect_mode: moli_fetch::RequestRedirectMode::Follow,
        priority: None,
        is_reload: false,
        metadata: Default::default(),
    };
    let (direct_completion_tx, mut direct_completion_rx) = tokio::sync::oneshot::channel();
    {
        let mut state = service.inner.state.lock();
        state.pending_fetch_jobs.insert(
            event_id,
            ServiceWorkerFetchJob {
                request: {
                    let origin = moli_url::WebOrigin::from_url(&document_url);
                    let initiator = (document_url).clone();
                    moli_fetch::Request::new_browser("GET", request_url, None, Vec::new(), origin)
                        .with_initiator_url(&initiator)
                        .with_request_mode(moli_fetch::RequestMode::Cors)
                        .with_credentials_mode(moli_fetch::RequestCredentialsMode::SameOrigin)
                        .with_redirect_mode(moli_fetch::RequestRedirectMode::Follow)
                        .with_fetch_priority_hint(None)
                },
                internal_id: 90,
                owner: Some(test_run_owner(version_id, &run)),
                cors_preflight_request_headers: Vec::new(),
                client_id: ServiceWorkerClientId::from_u64_for_test(0),
                resulting_client_id: None,
                destination: ServiceWorkerRequestDestination::Empty,
                is_reload: false,
                metadata: Default::default(),
                request_cookie_report: None,
                network_context: AsyncSubresourceNetworkContext {
                    frame_id: None,
                    request_origin: moli_url::WebOrigin::from_url(&document_url),
                    document_url,
                    resource_type: crate::types::SubresourceResourceType::Fetch,
                    policy_context: Default::default(),
                },
                completion_tx: completion_queue.sender(),
                request_client: test_request_client(&service),
                resource_task_runner: test_resource_task_runner(),
                cancel_handle: moli_fetch::FetchCancelHandle::new(),
                navigation_preload_cancel_handle: None,
                streaming_body_source_id: None,
                direct_completion_tx: Some(direct_completion_tx),
            },
        );
        let version = state.versions.get_mut(&version_id).unwrap();
        version.fetch_handler_existence = ServiceWorkerFetchHandlerExistence::Unknown;
        version.fetch_handler_type = ServiceWorkerFetchHandlerType::NoHandler;
        version.running_state = ServiceWorkerVersionRunningState::Starting { host };
        version.pending_start_events = VecDeque::from([ServiceWorkerPendingStartEvent::Fetch(
            ServiceWorkerFetchEvent {
                event_id,
                owner: test_run_owner(version_id, &run),
                request,
                navigation_preload_sent: false,
            },
        )]);
        version.in_flight_event_count = 1;
        version.run = run.clone();
    }

    service.finish_worker_start_completed_with_script_resource(
        version_id,
        run,
        script_url.to_string(),
        Some(test_script_resource(&script_url)),
        ServiceWorkerFetchHandlerType::EmptyFetchHandler,
    );

    {
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(
            version.fetch_handler_existence,
            ServiceWorkerFetchHandlerExistence::Exists
        );
        assert_eq!(
            version.fetch_handler_type,
            ServiceWorkerFetchHandlerType::EmptyFetchHandler
        );
        assert!(matches!(
            version.running_state,
            ServiceWorkerVersionRunningState::Running { .. }
        ));
        assert_eq!(version.in_flight_event_count, 1);
        assert!(state.pending_fetch_jobs.contains_key(&event_id));
    }
    assert_eq!(service.drain_service_lane(), 1);

    {
        let state = service.inner.state.lock();
        assert!(state.pending_fetch_jobs.is_empty());
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 0);
        assert!(version.pending_start_events.is_empty());
    }

    expect_direct_fetch_fallback(&mut direct_completion_rx);
    assert!(!completion_queue.has_ready_completion());
}
#[test]
fn fetch_completion_schedules_idle_timeout_for_running_active_worker() {
    let service = new_service_worker_runtime_service();
    service.set_idle_delay_for_test(Duration::ZERO);
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let document_url = url("https://example.test/app/page.html");
    let request_url = url("https://example.test/app/data.txt");
    let client_id = register_client_for_test(&service, document_url.clone());
    let event_id = ServiceWorkerEventId(11);
    let mut completion_queue = async_subresource_completion_queue();
    let host = new_running_test_host(version_id, &run);
    {
        let mut state = service.inner.state.lock();
        state.registrations.insert(
            registration_id,
            ServiceWorkerRegistration {
                id: registration_id,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
                scope_url: scope_url.clone(),
                script_url: script_url.clone(),
                installing_version_id: None,
                waiting_version_id: None,
                active_version_id: Some(version_id),
                pending_unregistration: false,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                last_update_check_time_ms: None,
                pending_register_jobs: HashMap::new(),
                controlled_client_ids: HashSet::from([client_id]),
            },
        );
        state.pending_fetch_jobs.insert(
            event_id,
            ServiceWorkerFetchJob {
                request: {
                    let origin = moli_url::WebOrigin::from_url(&document_url);
                    let initiator = (document_url).clone();
                    moli_fetch::Request::new_browser("GET", request_url, None, Vec::new(), origin)
                        .with_initiator_url(&initiator)
                        .with_request_mode(moli_fetch::RequestMode::Cors)
                        .with_credentials_mode(moli_fetch::RequestCredentialsMode::SameOrigin)
                        .with_redirect_mode(moli_fetch::RequestRedirectMode::Follow)
                        .with_fetch_priority_hint(None)
                },
                internal_id: 77,
                owner: Some(test_run_owner(version_id, &run)),
                cors_preflight_request_headers: Vec::new(),
                client_id: ServiceWorkerClientId::from_u64_for_test(0),
                resulting_client_id: None,
                destination: ServiceWorkerRequestDestination::Empty,
                is_reload: false,
                metadata: Default::default(),
                request_cookie_report: None,
                network_context: AsyncSubresourceNetworkContext {
                    frame_id: None,
                    request_origin: moli_url::WebOrigin::from_url(&document_url),
                    document_url,
                    resource_type: crate::types::SubresourceResourceType::Fetch,
                    policy_context: Default::default(),
                },
                completion_tx: completion_queue.sender(),
                request_client: test_request_client(&service),
                resource_task_runner: test_resource_task_runner(),
                cancel_handle: moli_fetch::FetchCancelHandle::new(),
                navigation_preload_cancel_handle: None,
                streaming_body_source_id: None,
                direct_completion_tx: None,
            },
        );
        state.versions.insert(
            version_id,
            ServiceWorkerVersion {
                id: version_id,
                registration_id,
                script_url: script_url.clone(),
                final_script_url: Some(script_url.clone()),
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: WorkerScriptKind::Classic,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: test_launch_config(&service, &script_url, &scope_url),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                running_state: ServiceWorkerVersionRunningState::Running { host },
                pending_start_events: VecDeque::new(),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 1,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
        state.record_target_created(
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
        );
    }

    service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
        event_id,
        owner: test_run_owner(version_id, &run),
        result: ServiceWorkerFetchResult::Failure("handled".to_owned()),
    });

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.in_flight_event_count, 0);
    assert_eq!(diagnostics.running_version_count, 1);
    assert_eq!(diagnostics.pending_service_lane_event_count, 1);

    assert_eq!(service.drain_service_lane(), 1);
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.running_version_count, 0);
    assert_eq!(diagnostics.running_host_count, 0);
    assert_eq!(diagnostics.stopped_version_count, 1);
    let target_events = service.take_target_output_events_for_test();
    assert!(
        target_events.iter().any(|event| matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::Stopped {
                version_id: stopped_version_id,
                reason,
                ..
            } if *stopped_version_id == version_id.as_u64()
                && reason == "idle_timeout"
        )),
        "idle timeout should enqueue a stopped target lifecycle event: {target_events:?}"
    );
    assert!(
        !target_events.iter().any(|event| matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::Destroyed {
                version_id: destroyed_version_id,
                ..
            } if *destroyed_version_id == version_id.as_u64()
        )),
        "idle timeout must retain the service worker target: {target_events:?}"
    );

    let completion = pop_async_subresource_completion(&mut completion_queue);
    assert_eq!(completion.internal_id, 77);
    assert_eq!(completion.result.err().as_deref(), Some("handled"));
}
#[test]
fn devtools_force_update_on_page_load_enqueues_update_for_controlled_page_load() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let pending_script_url = url("https://example.test/app/pending-worker.js");
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url.clone(),
        [],
    );
    {
        let mut state = service.inner.state.lock();
        let version = state.versions.get_mut(&version_id).unwrap();
        version.main_script_resource = Some(test_script_resource(&script_url));
    }
    let mut completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    insert_pending_main_script_update_check(
        &service,
        registration_id,
        version_id,
        pending_script_url,
        scope_url.clone(),
        44,
        completion_queue.sender(),
    );

    assert!(!service.force_update_on_page_load_for_devtools());
    assert!(
        !service
            .devtools_force_update_registration_for_page_load(
                &scope_url,
                service.browser_context_runtime()
            )
            .0
    );
    assert_eq!(service.diagnostics_snapshot().queued_register_job_count, 0);

    service.set_force_update_on_page_load_for_devtools(true);
    assert!(service.force_update_on_page_load_for_devtools());
    let (started, force_update_rx) = service.devtools_force_update_registration_for_page_load(
        &scope_url,
        service.browser_context_runtime(),
    );
    assert!(started);
    assert!(
        force_update_rx.is_some(),
        "force-update-on-page-load should expose a waiter"
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.pending_main_script_update_check_count, 1);
    assert_eq!(diagnostics.queued_register_job_count, 1);
    {
        let state = service.inner.state.lock();
        let registration_key = ServiceWorkerRegistrationKey::for_scope_and_storage_key(
            &scope_url,
            ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
        );
        assert_eq!(
            state
                .job_coordinator
                .queued_register_job_options(&registration_key),
            vec![(true, true, true)],
            "force-update-on-page-load should queue a forced update job that skips waiting"
        );
    }
    assert!(!completion_queue.has_ready_task());

    service.set_force_update_on_page_load_for_devtools(false);
    assert!(!service.force_update_on_page_load_for_devtools());
}
#[test]
fn active_fetch_completion_queues_waiting_activation_after_controllees_are_gone() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let waiting_version_id = ServiceWorkerVersionId(2);
    let active_run = RendererServiceWorkerRunIdentity::fresh();
    let waiting_run = RendererServiceWorkerRunIdentity::fresh();
    let scope_url = url("https://example.test/app/");
    let active_script_url = url("https://example.test/app/worker-v1.js");
    let waiting_script_url = url("https://example.test/app/worker-v2.js");
    let document_url = url("https://example.test/app/page.html");
    let request_url = url("https://example.test/app/data.txt");
    let event_id = ServiceWorkerEventId(19);
    let waiting_host = new_loading_test_host(waiting_version_id, &waiting_run);
    let mut completion_queue = async_subresource_completion_queue();
    {
        let mut state = service.inner.state.lock();
        state.registrations.insert(
            registration_id,
            ServiceWorkerRegistration {
                id: registration_id,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
                scope_url: scope_url.clone(),
                script_url: waiting_script_url.clone(),
                installing_version_id: None,
                waiting_version_id: Some(waiting_version_id),
                active_version_id: Some(active_version_id),
                pending_unregistration: false,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                last_update_check_time_ms: None,
                pending_register_jobs: HashMap::new(),
                controlled_client_ids: HashSet::new(),
            },
        );
        state.pending_fetch_jobs.insert(
            event_id,
            ServiceWorkerFetchJob {
                request: {
                    let origin = moli_url::WebOrigin::from_url(&document_url);
                    let initiator = (document_url).clone();
                    moli_fetch::Request::new_browser(
                        "GET",
                        request_url.clone(),
                        None,
                        Vec::new(),
                        origin,
                    )
                    .with_initiator_url(&initiator)
                    .with_request_mode(moli_fetch::RequestMode::Cors)
                    .with_credentials_mode(moli_fetch::RequestCredentialsMode::SameOrigin)
                    .with_redirect_mode(moli_fetch::RequestRedirectMode::Follow)
                    .with_fetch_priority_hint(None)
                },
                internal_id: 91,
                owner: Some(test_run_owner(active_version_id, &active_run)),
                cors_preflight_request_headers: Vec::new(),
                client_id: ServiceWorkerClientId::from_u64_for_test(0),
                resulting_client_id: None,
                destination: ServiceWorkerRequestDestination::Empty,
                is_reload: false,
                metadata: Default::default(),
                request_cookie_report: None,
                network_context: AsyncSubresourceNetworkContext {
                    frame_id: None,
                    request_origin: moli_url::WebOrigin::from_url(&document_url),
                    document_url,
                    resource_type: crate::types::SubresourceResourceType::Fetch,
                    policy_context: Default::default(),
                },
                completion_tx: completion_queue.sender(),
                request_client: test_request_client(&service),
                resource_task_runner: test_resource_task_runner(),
                cancel_handle: moli_fetch::FetchCancelHandle::new(),
                navigation_preload_cancel_handle: None,
                streaming_body_source_id: None,
                direct_completion_tx: None,
            },
        );
        state.versions.insert(
            active_version_id,
            ServiceWorkerVersion {
                id: active_version_id,
                registration_id,
                script_url: active_script_url.clone(),
                final_script_url: Some(active_script_url.clone()),
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: WorkerScriptKind::Classic,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: test_launch_config(&service, &active_script_url, &scope_url),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                running_state: ServiceWorkerVersionRunningState::Stopped,
                pending_start_events: VecDeque::new(),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 1,
                run: active_run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
        state.versions.insert(
            waiting_version_id,
            ServiceWorkerVersion {
                id: waiting_version_id,
                registration_id,
                script_url: waiting_script_url.clone(),
                final_script_url: Some(waiting_script_url.clone()),
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: WorkerScriptKind::Classic,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: test_launch_config(&service, &waiting_script_url, &scope_url),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Installed,
                running_state: ServiceWorkerVersionRunningState::Starting { host: waiting_host },
                pending_start_events: VecDeque::new(),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 0,
                run: waiting_run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
        event_id,
        owner: test_run_owner(active_version_id, &active_run),
        result: ServiceWorkerFetchResult::Failure("done".to_owned()),
    });

    let state = service.inner.state.lock();
    assert_eq!(
        state
            .versions
            .get(&active_version_id)
            .unwrap()
            .in_flight_event_count,
        0
    );
    let waiting = state.versions.get(&waiting_version_id).unwrap();
    assert_eq!(
        waiting.lifecycle_state,
        ServiceWorkerVersionLifecycleState::Activating
    );
    assert_eq!(waiting.in_flight_event_count, 1);
    assert_eq!(waiting.pending_start_events.len(), 1);
    drop(state);

    let completion = pop_async_subresource_completion(&mut completion_queue);
    assert_eq!(completion.internal_id, 91);
    assert_eq!(completion.result.err().as_deref(), Some("done"));
}
#[test]
fn activating_new_worker_fails_replaced_active_fetch_and_pending_start_events() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let waiting_version_id = ServiceWorkerVersionId(2);
    let active_run = RendererServiceWorkerRunIdentity::fresh();
    let waiting_run = RendererServiceWorkerRunIdentity::fresh();
    let dispatched_event_id = ServiceWorkerEventId(41);
    let pending_event_id = ServiceWorkerEventId(42);
    let activation_wait_event_id = ServiceWorkerEventId(43);
    let scope_url = url("https://example.test/app/");
    let active_script_url = url("https://example.test/app/worker-v1.js");
    let waiting_script_url = url("https://example.test/app/worker-v2.js");
    let document_url = url("https://example.test/app/page.html");
    let request_url = url("https://example.test/app/data.txt");
    let client_id = ServiceWorkerClientId::from_u64_for_test(1);
    let active_host = new_running_test_host(active_version_id, &active_run);
    let mut completion_queue = async_subresource_completion_queue();
    {
        let mut state = service.inner.state.lock();
        state.live_clients.insert(
            client_id,
            ServiceWorkerClient {
                id: client_id,
                exposed_id: service_worker_exposed_client_id(client_id),
                creation_url: document_url.clone(),
                document_url: document_url.clone(),
                client_type: ServiceWorkerClientType::Window,
                frame_type: ServiceWorkerClientFrameType::TopLevel,
                visibility_state: ServiceWorkerClientVisibilityState::Visible,
                storage_key: ServiceWorkerRegistrationKey::first_party_storage_key_for_url(
                    &document_url,
                ),
                secure_context: true,
                execution_ready: true,
                discarded_or_frozen: false,
                document_owner: Some(crate::native_bridge::WindowDocumentOwner::for_test(0)),
                endpoint: ServiceWorkerClientEndpoint::Page(test_completion_sender()),
                focused: false,
            },
        );
        state.registrations.insert(
            registration_id,
            ServiceWorkerRegistration {
                id: registration_id,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
                scope_url: scope_url.clone(),
                script_url: waiting_script_url.clone(),
                installing_version_id: None,
                waiting_version_id: Some(waiting_version_id),
                active_version_id: Some(active_version_id),
                pending_unregistration: false,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                last_update_check_time_ms: None,
                pending_register_jobs: HashMap::new(),
                controlled_client_ids: HashSet::from([client_id]),
            },
        );
        for (event_id, internal_id) in [
            (dispatched_event_id, 201),
            (pending_event_id, 202),
            (activation_wait_event_id, 203),
        ] {
            state.pending_fetch_jobs.insert(
                event_id,
                ServiceWorkerFetchJob {
                    request: {
                        let origin = moli_url::WebOrigin::from_url(&document_url);
                        let initiator = document_url.clone();
                        moli_fetch::Request::new_browser(
                            "GET",
                            request_url.clone(),
                            None,
                            Vec::new(),
                            origin,
                        )
                        .with_initiator_url(&initiator)
                        .with_request_mode(moli_fetch::RequestMode::Cors)
                        .with_credentials_mode(moli_fetch::RequestCredentialsMode::SameOrigin)
                        .with_redirect_mode(moli_fetch::RequestRedirectMode::Follow)
                        .with_fetch_priority_hint(None)
                    },
                    internal_id,
                    owner: Some(test_run_owner(active_version_id, &active_run)),
                    cors_preflight_request_headers: Vec::new(),
                    client_id: ServiceWorkerClientId::from_u64_for_test(0),
                    resulting_client_id: None,
                    destination: ServiceWorkerRequestDestination::Empty,
                    is_reload: false,
                    metadata: Default::default(),
                    request_cookie_report: None,
                    network_context: AsyncSubresourceNetworkContext {
                        frame_id: None,
                        request_origin: moli_url::WebOrigin::from_url(&document_url),
                        document_url: document_url.clone(),
                        resource_type: crate::types::SubresourceResourceType::Fetch,
                        policy_context: Default::default(),
                    },
                    completion_tx: completion_queue.sender(),
                    request_client: test_request_client(&service),
                    resource_task_runner: test_resource_task_runner(),
                    cancel_handle: moli_fetch::FetchCancelHandle::new(),
                    navigation_preload_cancel_handle: None,
                    streaming_body_source_id: None,
                    direct_completion_tx: None,
                },
            );
        }
        state.versions.insert(
            active_version_id,
            ServiceWorkerVersion {
                id: active_version_id,
                registration_id,
                script_url: active_script_url.clone(),
                final_script_url: Some(active_script_url.clone()),
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: WorkerScriptKind::Classic,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: test_launch_config(&service, &active_script_url, &scope_url),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                running_state: ServiceWorkerVersionRunningState::Running { host: active_host },
                pending_start_events: VecDeque::from([ServiceWorkerPendingStartEvent::Fetch(
                    ServiceWorkerFetchEvent {
                        event_id: pending_event_id,
                        owner: test_run_owner(active_version_id, &active_run),
                        request: ServiceWorkerFetchRequest {
                            client_id,
                            resulting_client_id: None,
                            url: request_url.clone(),
                            method: "GET".to_owned(),
                            headers: Vec::new(),
                            body: None,
                            destination: ServiceWorkerRequestDestination::Empty,
                            request_mode: moli_fetch::RequestMode::Cors,
                            credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                            redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                            priority: None,
                            is_reload: false,
                            metadata: Default::default(),
                        },
                        navigation_preload_sent: false,
                    },
                )]),
                pending_activation_fetch_events: VecDeque::from([ServiceWorkerFetchEvent {
                    event_id: activation_wait_event_id,
                    owner: test_run_owner(active_version_id, &active_run),
                    request: ServiceWorkerFetchRequest {
                        client_id,
                        resulting_client_id: None,
                        url: request_url,
                        method: "GET".to_owned(),
                        headers: Vec::new(),
                        body: None,
                        destination: ServiceWorkerRequestDestination::Document,
                        request_mode: moli_fetch::RequestMode::Navigate,
                        credentials_mode: moli_fetch::RequestCredentialsMode::Include,
                        redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                        priority: None,
                        is_reload: false,
                        metadata: Default::default(),
                    },
                    navigation_preload_sent: false,
                }]),
                in_flight_event_count: 2,
                run: active_run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
        state.versions.insert(
            waiting_version_id,
            ServiceWorkerVersion {
                id: waiting_version_id,
                registration_id,
                script_url: waiting_script_url.clone(),
                final_script_url: Some(waiting_script_url.clone()),
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: WorkerScriptKind::Classic,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: test_launch_config(&service, &waiting_script_url, &scope_url),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Activating,
                running_state: ServiceWorkerVersionRunningState::Stopped,
                pending_start_events: VecDeque::new(),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 1,
                run: waiting_run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: true,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(2),
        owner: test_run_owner(waiting_version_id, &waiting_run),
        kind: ServiceWorkerLifecycleEventKind::Activate,
        result: Ok(()),
    });

    {
        let state = service.inner.state.lock();
        assert!(state.pending_fetch_jobs.is_empty());
        let previous_active = state.versions.get(&active_version_id).unwrap();
        assert_eq!(
            previous_active.lifecycle_state,
            ServiceWorkerVersionLifecycleState::Redundant
        );
        assert!(matches!(
            previous_active.running_state,
            ServiceWorkerVersionRunningState::Stopped
        ));
        assert!(previous_active.pending_start_events.is_empty());
        assert!(previous_active.pending_activation_fetch_events.is_empty());
        assert_eq!(previous_active.in_flight_event_count, 0);
        let waiting = state.versions.get(&waiting_version_id).unwrap();
        assert_eq!(
            waiting.lifecycle_state,
            ServiceWorkerVersionLifecycleState::Activated
        );
        assert_eq!(waiting.in_flight_event_count, 0);
    }

    let mut results = Vec::new();
    for _ in 0..3 {
        let completion = pop_async_subresource_completion(&mut completion_queue);
        results.push((
            completion.internal_id,
            completion.result.err().unwrap_or_default(),
        ));
    }
    results.sort_by_key(|(internal_id, _)| *internal_id);
    assert_eq!(
        results,
        vec![
            (
                201,
                "service worker was replaced by a newer active worker".to_owned(),
            ),
            (
                202,
                "service worker was replaced by a newer active worker".to_owned(),
            ),
            (
                203,
                "service worker was replaced by a newer active worker".to_owned(),
            ),
        ]
    );
    assert!(!completion_queue.has_ready_completion());
}
