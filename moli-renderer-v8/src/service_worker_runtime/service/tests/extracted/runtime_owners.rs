use super::*;

#[test]
fn restored_launch_config_resolves_the_current_browser_resource_runtime() {
    let service = new_service_worker_runtime_service();
    let launch_config = test_launch_config(
        &service,
        &url("https://example.test/sw.js"),
        &url("https://example.test/"),
    );
    let first_client = launch_config.request_client();

    let replacement_registration = crate::network::BrowserResourceRuntimeOwner::new(
        &moli_fetch::FetchConfig::default(),
        moli_cookie_jar::new_shared_browser_cookie_store(),
    );
    let replacement_runtime = service.replace_browser_resource_runtime(replacement_registration);
    let replacement_client = launch_config.request_client();
    assert!(
        replacement_client
            .browser_resource_runtime()
            .shares_state_with(&replacement_runtime)
    );
    assert!(!replacement_client.shares_resource_runtime_with(&first_client));
}
#[test]
fn devtools_pause_on_start_marks_non_released_launch_for_evaluation_pause() {
    let service = new_service_worker_runtime_service();
    service.set_pause_new_workers_on_start_for_devtools(true);
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let script_url = url("https://example.test/app/worker.js");
    let scope_url = url("https://example.test/app/");
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url.clone(),
        Vec::<Url>::new(),
    );
    {
        let mut state = service.inner.state.lock();
        let version = state
            .versions
            .get_mut(&version_id)
            .expect("inserted version");
        version.lifecycle_state = ServiceWorkerVersionLifecycleState::Installing;
        version.should_pause_on_start_for_devtools = true;
    }
    let mut launch =
        test_queued_launch(&service, registration_id, version_id, script_url, scope_url);

    service.apply_devtools_evaluation_pause_to_launch_if_needed(&mut launch, false);

    assert!(
        launch.params.pause_evaluation_until_debugger,
        "launches that did not consume a debugger release should pause top-level evaluation"
    );
    service.terminate_all_for_context_shutdown();
}
#[test]
fn devtools_pause_on_start_pauses_attached_stopped_worker_restart() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let script_url = url("https://example.test/app/worker.js");
    let scope_url = url("https://example.test/app/");
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url.clone(),
        Vec::<Url>::new(),
    );
    {
        let mut state = service.inner.state.lock();
        state
            .versions
            .get_mut(&version_id)
            .expect("inserted version")
            .should_pause_on_start_for_devtools = true;
    }
    service.set_devtools_attached_for_version(version_id, true);
    let mut launch =
        test_queued_launch(&service, registration_id, version_id, script_url, scope_url);

    service.apply_devtools_evaluation_pause_to_launch_if_needed(&mut launch, false);

    assert!(
        launch.params.pause_evaluation_until_debugger,
        "Chromium pauses stopped Service Worker restart only when the retained target is attached"
    );
    service.terminate_all_for_context_shutdown();
}
#[test]
fn devtools_pause_on_start_does_not_pause_unattached_stopped_worker_restart() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let script_url = url("https://example.test/app/worker.js");
    let scope_url = url("https://example.test/app/");
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url.clone(),
        Vec::<Url>::new(),
    );
    {
        let mut state = service.inner.state.lock();
        state
            .versions
            .get_mut(&version_id)
            .expect("inserted version")
            .should_pause_on_start_for_devtools = true;
    }
    let mut launch =
        test_queued_launch(&service, registration_id, version_id, script_url, scope_url);

    service.apply_devtools_evaluation_pause_to_launch_if_needed(&mut launch, false);

    assert!(
        !launch.params.pause_evaluation_until_debugger,
        "stopped Service Worker restart should not pause when DevTools retained the target but no session is attached"
    );
    service.terminate_all_for_context_shutdown();
}
#[test]
fn devtools_pause_on_start_does_not_retroactively_pause_existing_stopped_worker() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let script_url = url("https://example.test/app/worker.js");
    let scope_url = url("https://example.test/app/");
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url.clone(),
        Vec::<Url>::new(),
    );
    service.set_pause_new_workers_on_start_for_devtools(true);
    let mut launch =
        test_queued_launch(&service, registration_id, version_id, script_url, scope_url);

    service.apply_devtools_evaluation_pause_to_launch_if_needed(&mut launch, false);

    assert!(
        !launch.params.pause_evaluation_until_debugger,
        "a DevTools wait policy enabled after target creation must not pause an existing target"
    );
    service.terminate_all_for_context_shutdown();
}
#[test]
fn devtools_released_launch_does_not_double_pause_evaluation() {
    let service = new_service_worker_runtime_service();
    service.set_pause_new_workers_on_start_for_devtools(true);
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let script_url = url("https://example.test/app/worker.js");
    let scope_url = url("https://example.test/app/");
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url.clone(),
        Vec::<Url>::new(),
    );
    let mut launch =
        test_queued_launch(&service, registration_id, version_id, script_url, scope_url);

    service.apply_devtools_evaluation_pause_to_launch_if_needed(&mut launch, true);

    assert!(
        !launch.params.pause_evaluation_until_debugger,
        "pending launch released by Runtime.runIfWaitingForDebugger must not wait a second time"
    );
    service.terminate_all_for_context_shutdown();
}
#[test]
fn devtools_run_if_prereleases_starting_worker_evaluation_before_host_running() {
    let service = new_service_worker_runtime_service();
    service.set_pause_new_workers_on_start_for_devtools(true);
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let script_url = url("https://example.test/app/worker.js");
    let scope_url = url("https://example.test/app/");
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url.clone(),
        Vec::<Url>::new(),
    );
    {
        let mut state = service.inner.state.lock();
        let version = state.versions.get_mut(&version_id).unwrap();
        version.run = run.clone();
        version.should_pause_on_start_for_devtools = true;
        version.running_state = ServiceWorkerVersionRunningState::Starting {
            host: new_loading_test_host(version_id, &run),
        };
    }
    service.set_devtools_attached_for_version(version_id, true);

    assert!(
        service.devtools_run_if_waiting_for_debugger(version_id),
        "runIf should be remembered when the worker host is starting but not yet routable"
    );
    {
        let state = service.inner.state.lock();
        assert!(
            state
                .pending_devtools_evaluation_releases
                .contains(&version_id)
        );
    }

    let mut launch =
        test_queued_launch(&service, registration_id, version_id, script_url, scope_url);
    service.apply_devtools_evaluation_pause_to_launch_if_needed(&mut launch, false);

    assert!(
        !launch.params.pause_evaluation_until_debugger,
        "pre-released starting workers must not pause evaluation again"
    );
    {
        let state = service.inner.state.lock();
        assert!(
            !state
                .pending_devtools_evaluation_releases
                .contains(&version_id)
        );
    }
    service.terminate_all_for_context_shutdown();
}
#[test]
fn devtools_stop_worker_records_target_stopped() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url.clone(),
        [],
    );
    let host = new_running_test_host(version_id, &run);
    {
        let mut state = service.inner.state.lock();
        let version = state.versions.get_mut(&version_id).unwrap();
        version.run = run.clone();
        version.running_state = ServiceWorkerVersionRunningState::Running { host };
        state.record_target_created(registration_id, version_id, script_url, scope_url);
        service.take_target_output_events_for_test();
    }

    assert_eq!(
        service.devtools_stop_worker_version(version_id),
        Ok(true),
        "DevTools stopWorker should accept a live version"
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.running_version_count, 0);
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
                && reason == "devtools_stop"
        )),
        "DevTools stopWorker should enqueue a stopped target event: {target_events:?}"
    );
}
#[test]
fn idle_timeout_is_ignored_after_new_event_invalidates_timeout_token() {
    let service = new_service_worker_runtime_service();
    service.set_idle_delay_for_test(Duration::ZERO);
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
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
                controlled_client_ids: HashSet::new(),
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
                in_flight_event_count: 0,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    let timeout = {
        let mut state = service.inner.state.lock();
        service
            .maybe_schedule_idle_timeout_locked(&mut state, version_id)
            .expect("active idle worker should schedule timeout")
    };
    {
        let mut state = service.inner.state.lock();
        let version = state.versions.get_mut(&version_id).unwrap();
        ServiceWorkerRuntimeService::begin_version_event_locked(version);
    }
    service.schedule_idle_timeout(timeout);
    assert_eq!(service.drain_service_lane(), 1);

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.running_version_count, 1);
    assert_eq!(diagnostics.stopped_version_count, 0);
    assert_eq!(diagnostics.in_flight_event_count, 1);
}
#[test]
fn stopped_waiting_activation_queues_lifecycle_event_and_starts_worker() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let waiting_version_id = ServiceWorkerVersionId(2);
    let scope_url = url("https://example.test/app/");
    let active_script_url = url("https://example.test/app/worker-v1.js");
    let waiting_script_url = url("https://example.test/app/worker-v2.js");
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
                in_flight_event_count: 0,
                run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
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
                running_state: ServiceWorkerVersionRunningState::Stopped,
                pending_start_events: VecDeque::new(),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 0,
                run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
                idle_timeout_token: None,
                skip_waiting_requested: true,
                clients_claim_requested: false,
                last_start_error: Some("previous idle stop".to_owned()),
            },
        );
        state.record_target_created(
            registration_id,
            waiting_version_id,
            waiting_script_url.clone(),
            scope_url.clone(),
        );
        service.take_target_output_events_for_test();
    }

    let previous_waiting_run = exact_version_run(&service, waiting_version_id);
    let start = {
        let mut state = service.inner.state.lock();
        service
            .try_activate_waiting_version_locked(&mut state, registration_id, waiting_version_id)
            .expect("stopped waiting version should be startable")
    };
    let activation_start_events = service.take_target_output_events_for_test();
    assert!(
        activation_start_events.iter().any(|event| matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::VersionUpdated {
                version_id: updated_version_id,
                status: crate::runtime::RendererServiceWorkerVersionStatus::Activating,
            } if *updated_version_id == waiting_version_id.as_u64()
        )),
        "activation start should refresh target status to activating: {activation_start_events:?}"
    );
    let ServiceWorkerLifecycleStart::Start(launch) = start else {
        panic!("expected stopped waiting activation to start worker");
    };
    assert_eq!(launch.params.registration_id, registration_id);
    assert_eq!(launch.params.run_owner.version_id(), waiting_version_id);
    let restarted_run = launch.params.run_owner.cloned_run_identity();
    assert_ne!(restarted_run, previous_waiting_run);
    assert_eq!(launch.params.script_url, waiting_script_url);
    assert_eq!(launch.params.scope_url, scope_url);
    assert_eq!(launch.host.version_id(), waiting_version_id);
    assert_eq!(launch.host.run_identity(), restarted_run);

    let state = service.inner.state.lock();
    let registration = state.registrations.get(&registration_id).unwrap();
    assert_eq!(registration.active_version_id, Some(active_version_id));
    assert_eq!(registration.waiting_version_id, Some(waiting_version_id));
    let waiting = state.versions.get(&waiting_version_id).unwrap();
    assert_eq!(
        waiting.lifecycle_state,
        ServiceWorkerVersionLifecycleState::Activating
    );
    assert_eq!(waiting.run, restarted_run);
    assert_eq!(waiting.in_flight_event_count, 1);
    assert_eq!(waiting.last_start_error, None);
    assert!(matches!(
        waiting.running_state,
        ServiceWorkerVersionRunningState::Starting { .. }
    ));
    assert_eq!(waiting.pending_start_events.len(), 1);
    let ServiceWorkerPendingStartEvent::Lifecycle(event) =
        waiting.pending_start_events.front().unwrap()
    else {
        panic!("expected pending activate lifecycle event");
    };
    assert_eq!(
        event.owner,
        test_run_owner(waiting_version_id, &restarted_run)
    );
    assert_eq!(event.kind, ServiceWorkerLifecycleEventKind::Activate);
}
#[test]
fn stale_start_completion_does_not_dispatch_pending_lifecycle_activation() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let waiting_version_id = ServiceWorkerVersionId(2);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let scope_url = url("https://example.test/app/");
    let active_script_url = url("https://example.test/app/worker-v1.js");
    let waiting_script_url = url("https://example.test/app/worker-v2.js");
    let event_id = ServiceWorkerEventId(7);
    let host = new_loading_test_host(waiting_version_id, &run);
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
                in_flight_event_count: 0,
                run: RendererServiceWorkerRunIdentity::fresh(),
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
                running_state: ServiceWorkerVersionRunningState::Starting { host },
                pending_start_events: VecDeque::from([ServiceWorkerPendingStartEvent::Lifecycle(
                    ServiceWorkerLifecycleEvent {
                        event_id,
                        owner: test_run_owner(waiting_version_id, &run),
                        kind: ServiceWorkerLifecycleEventKind::Activate,
                    },
                )]),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 1,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: true,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    let stale_run = RendererServiceWorkerRunIdentity::fresh();
    assert_ne!(stale_run, run);
    service.finish_worker_start_completed(
        waiting_version_id,
        stale_run,
        waiting_script_url.to_string(),
    );

    assert_eq!(service.pending_service_lane_event_count(), 0);
    let state = service.inner.state.lock();
    let registration = state.registrations.get(&registration_id).unwrap();
    assert_eq!(registration.active_version_id, Some(active_version_id));
    assert_eq!(registration.waiting_version_id, Some(waiting_version_id));
    let waiting = state.versions.get(&waiting_version_id).unwrap();
    assert_eq!(
        waiting.lifecycle_state,
        ServiceWorkerVersionLifecycleState::Activating
    );
    assert_eq!(waiting.in_flight_event_count, 1);
    assert_eq!(waiting.pending_start_events.len(), 1);
    assert!(matches!(
        waiting.running_state,
        ServiceWorkerVersionRunningState::Starting { .. }
    ));
}
#[test]
fn activating_new_worker_destroys_replaced_active_target() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let waiting_version_id = ServiceWorkerVersionId(2);
    let active_run = RendererServiceWorkerRunIdentity::fresh();
    let waiting_run = RendererServiceWorkerRunIdentity::fresh();
    let scope_url = url("https://example.test/app/");
    let active_script_url = url("https://example.test/app/sw-v1.js");
    let waiting_script_url = url("https://example.test/app/sw-v2.js");
    insert_registered_version(
        &service,
        registration_id,
        active_version_id,
        active_script_url.clone(),
        scope_url.clone(),
        [],
    );
    {
        let mut state = service.inner.state.lock();
        let active_host = new_running_test_host(active_version_id, &active_run);
        let active = state
            .versions
            .get_mut(&active_version_id)
            .expect("active version should exist");
        active.run = active_run.clone();
        active.running_state = ServiceWorkerVersionRunningState::Running { host: active_host };
        state.record_target_created(
            registration_id,
            active_version_id,
            active_script_url,
            scope_url.clone(),
        );
        service.take_target_output_events_for_test();
        state
            .registrations
            .get_mut(&registration_id)
            .expect("registration should exist")
            .waiting_version_id = Some(waiting_version_id);
        state.versions.insert(
            waiting_version_id,
            ServiceWorkerVersion {
                id: waiting_version_id,
                registration_id,
                script_url: waiting_script_url.clone(),
                final_script_url: Some(waiting_script_url.clone()),
                main_script_resource: Some(test_script_resource(&waiting_script_url)),
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

    let target_events = service.take_target_output_events_for_test();
    assert_eq!(target_events.len(), 2);
    let crate::runtime::RendererServiceWorkerTargetEvent::Stopped {
        version_id,
        run: _,
        reason,
    } = &target_events[0]
    else {
        panic!("active replacement must first retire its exact run: {target_events:?}");
    };
    assert_eq!(*version_id, active_version_id.as_u64());
    assert_eq!(reason, "replaced_by_newer_active_worker");
    let crate::runtime::RendererServiceWorkerTargetEvent::Destroyed {
        version_id,
        active_run,
    } = &target_events[1]
    else {
        panic!("active replacement must then destroy its version: {target_events:?}");
    };
    assert_eq!(*version_id, active_version_id.as_u64());
    assert!(
        active_run.is_none(),
        "the preceding stop must leave no live run on version destruction"
    );
    let state = service.inner.state.lock();
    assert_eq!(
        state
            .versions
            .get(&active_version_id)
            .expect("replaced version should remain diagnosable")
            .lifecycle_state,
        ServiceWorkerVersionLifecycleState::Redundant
    );
    assert!(
        !state
            .service_worker_target_infos
            .contains_key(&active_version_id),
        "doomed active version should no longer be exposed as a Service Worker target"
    );
}
