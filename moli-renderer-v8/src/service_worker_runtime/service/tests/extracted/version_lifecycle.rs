use super::*;

#[test]
fn unregister_queues_behind_installing_registration_job() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker-v1.js");
    let mut completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    insert_starting_version_with_register_job(
        &service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url.clone(),
        11,
        completion_queue.sender(),
    );

    assert!(service.mark_registration_unregistered(&scope_url));
    assert!(!service.mark_registration_unregistered(&scope_url));

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.pending_unregistration_count, 0);
    assert_eq!(diagnostics.queued_unregistration_job_count, 1);
    assert_eq!(
        service
            .matching_registration_for_client(&url("https://example.test/app/page.html"))
            .expect("installing registration should remain visible before queued unregister")
            .installing()
            .expect("installing worker should remain current")
            .script_url(),
        &script_url
    );

    let run = exact_version_run(&service, version_id);
    service.finish_worker_start_completed(version_id, run.clone(), script_url.to_string());
    assert!(!completion_queue.has_ready_task());
    assert_eq!(
        service
            .diagnostics_snapshot()
            .queued_unregistration_job_count,
        1
    );

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(version_id, &run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });

    let completion = pop_register_completion(&mut completion_queue);
    assert_eq!(completion.request_id, 11);
    assert!(completion.result.is_ok());
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 0);
    assert_eq!(diagnostics.version_count, 0);
    assert_eq!(diagnostics.pending_unregistration_count, 0);
    assert_eq!(diagnostics.queued_unregistration_job_count, 0);
    assert_eq!(
        service.matching_registration_for_client(&url("https://example.test/app/page.html")),
        None
    );
}
#[test]
fn start_registration_queues_same_scope_job_while_installing() {
    let service = new_service_worker_runtime_service();
    let browser_context_runtime = service.browser_context_runtime();
    let document_url = url("https://example.test/app/page.html");
    let mut first_completion_queue =
        crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let mut second_completion_queue =
        crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    service.start_registration(
        url("https://example.test/app/worker-v1.js"),
        url("https://example.test/app/"),
        document_url.clone(),
        WorkerScriptKind::Classic,
        test_request_client(&service),
        WorkerNetworkPolicy::default(),
        browser_context_runtime.clone(),
        None,
        None,
        None,
        ServiceWorkerUpdateViaCache::Imports,
        1,
        1,
        first_completion_queue.sender(),
    );
    service.start_registration(
        url("https://example.test/app/worker-v2.js"),
        url("https://example.test/app/"),
        document_url,
        WorkerScriptKind::Module,
        test_request_client(&service),
        WorkerNetworkPolicy::default(),
        browser_context_runtime,
        None,
        None,
        None,
        ServiceWorkerUpdateViaCache::Imports,
        2,
        1,
        second_completion_queue.sender(),
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.queued_register_job_count, 1);
    assert!(!first_completion_queue.has_ready_task());
    assert!(!second_completion_queue.has_ready_task());
    assert_eq!(
        service
            .matching_registration_for_client(&url("https://example.test/app/client.html"))
            .expect("registration should exist")
            .installing()
            .expect("first registration version should keep installing")
            .script_url(),
        &url("https://example.test/app/worker-v1.js")
    );

    service.terminate_all_for_context_shutdown();
}
#[test]
fn start_registration_coalesces_same_installing_register_callbacks() {
    let service = new_service_worker_runtime_service();
    let browser_context_runtime = service.browser_context_runtime();
    let document_url = url("https://example.test/app/page.html");
    let script_url = url("https://example.test/app/worker.js");
    let scope_url = url("https://example.test/app/");
    let mut first_completion_queue =
        crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let mut second_completion_queue =
        crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

    service.start_registration(
        script_url.clone(),
        scope_url.clone(),
        document_url.clone(),
        WorkerScriptKind::Classic,
        test_request_client(&service),
        WorkerNetworkPolicy::default(),
        browser_context_runtime.clone(),
        None,
        None,
        None,
        ServiceWorkerUpdateViaCache::Imports,
        1,
        1,
        first_completion_queue.sender(),
    );
    service.start_registration(
        script_url.clone(),
        scope_url,
        document_url,
        WorkerScriptKind::Classic,
        test_request_client(&service),
        WorkerNetworkPolicy::default(),
        browser_context_runtime,
        None,
        None,
        None,
        ServiceWorkerUpdateViaCache::Imports,
        2,
        1,
        second_completion_queue.sender(),
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.queued_register_job_count, 0);
    assert!(!first_completion_queue.has_ready_task());
    assert!(!second_completion_queue.has_ready_task());

    let version_id = ServiceWorkerVersionId(1);
    let run = exact_version_run(&service, version_id);
    service.finish_worker_start_completed(version_id, run.clone(), script_url.to_string());

    assert!(!first_completion_queue.has_ready_task());
    assert!(!second_completion_queue.has_ready_task());
    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(version_id, &run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });

    assert!(first_completion_queue.has_ready_task());
    assert!(second_completion_queue.has_ready_task());
    service.terminate_all_for_context_shutdown();
}
#[test]
fn installing_register_coalesces_late_callback_until_install_completes() {
    let service = new_service_worker_runtime_service();
    let browser_context_runtime = service.browser_context_runtime();
    let document_url = url("https://example.test/app/page.html");
    let script_url = url("https://example.test/app/worker.js");
    let scope_url = url("https://example.test/app/");
    let mut first_completion_queue =
        crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let mut second_completion_queue =
        crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

    service.start_registration(
        script_url.clone(),
        scope_url.clone(),
        document_url.clone(),
        WorkerScriptKind::Classic,
        test_request_client(&service),
        WorkerNetworkPolicy::default(),
        browser_context_runtime.clone(),
        None,
        None,
        None,
        ServiceWorkerUpdateViaCache::Imports,
        1,
        1,
        first_completion_queue.sender(),
    );
    let version_id = ServiceWorkerVersionId(1);
    let run = exact_version_run(&service, version_id);
    service.finish_worker_start_completed(version_id, run.clone(), script_url.to_string());
    assert!(!first_completion_queue.has_ready_task());

    service.start_registration(
        script_url,
        scope_url,
        document_url,
        WorkerScriptKind::Classic,
        test_request_client(&service),
        WorkerNetworkPolicy::default(),
        browser_context_runtime,
        None,
        None,
        None,
        ServiceWorkerUpdateViaCache::Imports,
        2,
        1,
        second_completion_queue.sender(),
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.queued_register_job_count, 0);
    assert!(!first_completion_queue.has_ready_task());
    assert!(!second_completion_queue.has_ready_task());

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(version_id, &run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });

    assert!(first_completion_queue.has_ready_task());
    assert!(second_completion_queue.has_ready_task());

    service.terminate_all_for_context_shutdown();
}
#[test]
fn register_job_waits_for_install_completion_when_install_event_is_not_dispatched() {
    let service = new_service_worker_runtime_service();
    let browser_context_runtime = service.browser_context_runtime();
    let document_url = url("https://example.test/app/page.html");
    let script_url = url("https://example.test/app/worker.js");
    let scope_url = url("https://example.test/app/");
    let mut completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

    service.start_registration(
        script_url.clone(),
        scope_url,
        document_url,
        WorkerScriptKind::Classic,
        test_request_client(&service),
        WorkerNetworkPolicy::default(),
        browser_context_runtime,
        None,
        None,
        None,
        ServiceWorkerUpdateViaCache::Imports,
        1,
        1,
        completion_queue.sender(),
    );
    let version_id = ServiceWorkerVersionId(1);
    let run = exact_version_run(&service, version_id);
    service.finish_worker_start_completed(version_id, run.clone(), script_url.to_string());

    assert!(!completion_queue.has_ready_task());
    {
        let state = service.inner.state.lock();
        let registration = state
            .registrations
            .get(&ServiceWorkerRegistrationId(1))
            .expect("registration should exist");
        let pending_job = registration
            .pending_register_jobs
            .get(&ServiceWorkerVersionId(1))
            .expect("register job should wait for install completion");
        assert_eq!(pending_job.phase(), ServiceWorkerRegisterJobPhase::Update);
    }

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(version_id, &run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });

    let completion = pop_register_completion(&mut completion_queue);
    assert_eq!(completion.request_id, 1);
    let snapshot = completion
        .result
        .expect("register should resolve after install store succeeds");
    assert!(snapshot.installing().is_none());
    assert!(
        snapshot.waiting().is_some(),
        "installed worker should be visible as waiting"
    );
    {
        let state = service.inner.state.lock();
        let registration = state
            .registrations
            .get(&ServiceWorkerRegistrationId(1))
            .expect("registration should exist");
        assert!(
            !registration
                .pending_register_jobs
                .contains_key(&ServiceWorkerVersionId(1))
        );
    }
    service.terminate_all_for_context_shutdown();
}
#[test]
fn install_completion_starts_next_queued_register_job() {
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
        scope_url.clone(),
        22,
        second_queue.sender(),
    );

    let first_run = exact_version_run(&service, first_version_id);
    service.finish_worker_start_completed(
        first_version_id,
        first_run.clone(),
        "https://example.test/app/worker-v1.js".to_owned(),
    );
    assert!(!first_queue.has_ready_task());
    assert!(!second_queue.has_ready_task());

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(first_version_id, &first_run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });

    assert!(first_queue.has_ready_task());
    assert!(!second_queue.has_ready_task());
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.version_count, 2);
    assert_eq!(diagnostics.queued_register_job_count, 0);
    assert_eq!(diagnostics.starting_version_count, 1);
    let snapshot = snapshot_for_registration(&service, registration_id);
    assert_eq!(
        snapshot
            .installing()
            .expect("queued version should start installing")
            .script_url(),
        &url("https://example.test/app/worker-v2.js")
    );

    service.terminate_all_for_context_shutdown();
}
#[test]
fn start_registration_preserves_existing_active_version_for_same_scope() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let active_state = insert_registered_version(
        &service,
        registration_id,
        active_version_id,
        url("https://example.test/app/worker-v1.js"),
        url("https://example.test/app/"),
        [url("https://example.test/app/page.html")],
    );
    let browser_context_runtime = service.browser_context_runtime();
    let completion_tx = test_completion_sender();
    service.start_registration(
        url("https://example.test/app/worker-v2.js"),
        url("https://example.test/app/"),
        url("https://example.test/app/page.html"),
        WorkerScriptKind::Classic,
        test_request_client(&service),
        WorkerNetworkPolicy::default(),
        browser_context_runtime,
        None,
        None,
        None,
        ServiceWorkerUpdateViaCache::Imports,
        1,
        1,
        completion_tx,
    );

    let snapshot = service
        .matching_registration_for_client(&url("https://example.test/app/page.html"))
        .expect("registration should exist");
    assert_eq!(snapshot.registration_id(), registration_id);
    assert_eq!(
        snapshot
            .active()
            .expect("existing active version should be preserved")
            .version_id(),
        active_version_id
    );
    assert_eq!(
        service.matching_controller_for_document(&url("https://example.test/app/page.html")),
        Some(active_state)
    );

    service.terminate_all_for_context_shutdown();
}
#[test]
fn same_options_active_register_resolves_without_new_version() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let document_url = url("https://example.test/app/page.html");
    let active_state = insert_registered_version(
        &service,
        registration_id,
        active_version_id,
        script_url.clone(),
        scope_url.clone(),
        [document_url.clone()],
    );
    let browser_context_runtime = service.browser_context_runtime();
    let mut completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

    service.start_registration(
        script_url.clone(),
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
        42,
        1,
        completion_queue.sender(),
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.starting_version_count, 0);
    assert_eq!(diagnostics.queued_register_job_count, 0);

    let completion = pop_register_completion(&mut completion_queue);
    assert_eq!(completion.request_id, 42);
    let snapshot = completion
        .result
        .expect("same-options register should resolve existing registration");
    assert_eq!(snapshot.registration_id(), registration_id);
    assert!(snapshot.installing().is_none());
    assert!(snapshot.waiting().is_none());
    assert_eq!(
        snapshot
            .active()
            .expect("active version should be visible")
            .version_id(),
        active_version_id
    );
    assert_eq!(
        service.matching_controller_for_document(&document_url),
        Some(active_state)
    );

    service.terminate_all_for_context_shutdown();
}
#[test]
fn different_update_via_cache_register_starts_new_version() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    let document_url = url("https://example.test/app/page.html");
    insert_registered_version(
        &service,
        registration_id,
        active_version_id,
        script_url.clone(),
        scope_url.clone(),
        [document_url.clone()],
    );
    let browser_context_runtime = service.browser_context_runtime();
    let mut completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

    service.start_registration(
        script_url.clone(),
        scope_url,
        document_url,
        WorkerScriptKind::Classic,
        test_request_client(&service),
        WorkerNetworkPolicy::default(),
        browser_context_runtime,
        None,
        None,
        None,
        ServiceWorkerUpdateViaCache::All,
        44,
        1,
        completion_queue.sender(),
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 2);
    assert_eq!(diagnostics.starting_version_count, 1);
    assert_eq!(diagnostics.queued_register_job_count, 0);
    assert!(!completion_queue.has_ready_task());
    {
        let state = service.inner.state.lock();
        let registration = state
            .registrations
            .get(&registration_id)
            .expect("registration should exist");
        assert_eq!(
            registration.update_via_cache,
            ServiceWorkerUpdateViaCache::All
        );
        assert_eq!(registration.active_version_id, Some(active_version_id));
        assert!(registration.installing_version_id.is_some());
    }

    service.terminate_all_for_context_shutdown();
}
#[test]
fn worker_start_failed_prunes_empty_registration_and_destroys_installing_target() {
    let service = new_service_worker_runtime_service();
    let (script_url, scope_url, registration_id, version_id) = insert_starting_version(&service);
    let created_run = {
        let mut state = service.inner.state.lock();
        state.record_target_created(registration_id, version_id, script_url, scope_url);
        exact_created_target_run(&service.take_target_output_events_for_test(), version_id)
    };
    service.enqueue_worker_start_failed(
        test_run_owner(version_id, &created_run),
        ServiceWorkerVersionStartFailure::ScriptLoad {
            message: "service worker script load failed: network request client not available"
                .to_owned(),
        },
    );

    let pre_drain_diagnostics = service.diagnostics_snapshot();
    assert_eq!(pre_drain_diagnostics.pending_service_lane_event_count, 1);
    assert_eq!(pre_drain_diagnostics.starting_version_count, 1);
    assert_eq!(pre_drain_diagnostics.stopped_version_count, 0);

    assert_eq!(service.drain_service_lane(), 1);

    let state = service.inner.state.lock();
    assert_eq!(state.registrations.len(), 0);
    assert_eq!(state.versions.len(), 0);
    drop(state);

    let target_events = service.take_target_output_events_for_test();
    assert!(
        target_events.iter().any(|event| matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::Destroyed {
                version_id: destroyed_version_id,
                active_run: Some(active_run),
            } if *destroyed_version_id == version_id.as_u64()
                && active_run == &created_run
        )),
        "failed installing target should destroy its exact starting run: {target_events:?}"
    );
    assert!(
        !target_events.iter().any(|event| matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::Stopped {
                version_id: stopped_version_id,
                ..
            } if *stopped_version_id == version_id.as_u64()
        )),
        "failed installing target should not be retained as stopped: {target_events:?}"
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 0);
    assert_eq!(diagnostics.version_count, 0);
    assert_eq!(diagnostics.installing_version_count, 0);
    assert_eq!(diagnostics.redundant_version_count, 0);
    assert_eq!(diagnostics.stopped_version_count, 0);
    assert_eq!(diagnostics.failed_start_count, 0);
    assert_eq!(diagnostics.pending_service_lane_event_count, 0);
}
#[test]
fn install_completion_queues_service_worker_target_installed_version_update() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let script_url = url("https://example.test/app/sw.js");
    let scope_url = url("https://example.test/app/");
    let mut completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    insert_starting_version_with_register_job(
        &service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url.clone(),
        11,
        completion_queue.sender(),
    );
    {
        let mut state = service.inner.state.lock();
        state.record_target_created(
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
        );
        service.take_target_output_events_for_test();
    }

    let run = exact_version_run(&service, version_id);
    service.finish_worker_start_completed(version_id, run.clone(), script_url.to_string());
    service.take_target_output_events_for_test();

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(version_id, &run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });
    let install_events = service.take_target_output_events_for_test();
    assert!(
        install_events.iter().any(|event| matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::VersionUpdated {
                version_id: updated_version_id,
                status: crate::runtime::RendererServiceWorkerVersionStatus::Installed,
            } if *updated_version_id == version_id.as_u64()
        )),
        "install completion should refresh target status to installed: {install_events:?}"
    );

    assert!(
        pop_register_completion(&mut completion_queue)
            .result
            .is_ok()
    );
}
#[test]
fn stale_worker_start_completion_does_not_advance_version_state() {
    let service = new_service_worker_runtime_service();
    let (script_url, _, _, version_id) = insert_starting_version(&service);

    service.enqueue_worker_start_completed(
        ServiceWorkerRunOwner::fresh(version_id),
        "https://example.test/app/sw.js".to_owned(),
        test_script_resource(&script_url),
        ServiceWorkerFetchHandlerType::NotSkippable,
    );
    assert_eq!(service.pending_service_lane_event_count(), 1);
    assert_eq!(service.drain_service_lane(), 1);

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.running_version_count, 0);
    assert_eq!(diagnostics.starting_version_count, 1);
    assert_eq!(diagnostics.versions[0].final_script_url, None);
    assert_eq!(diagnostics.versions[0].main_script_status, None);
}
#[test]
fn devtools_pause_on_start_defers_initial_install_launch_after_target_creation() {
    let service = new_service_worker_runtime_service();
    service.set_pause_new_workers_on_start_for_devtools(true);
    let script_url = url("https://example.test/app/worker.js");
    let scope_url = url("https://example.test/app/");
    let document_url = url("https://example.test/app/page.html");
    let browser_context_runtime = service.browser_context_runtime();
    let mut completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

    service.start_registration(
        script_url,
        scope_url,
        document_url,
        WorkerScriptKind::Classic,
        test_request_client(&service),
        WorkerNetworkPolicy::default(),
        browser_context_runtime,
        None,
        None,
        None,
        ServiceWorkerUpdateViaCache::Imports,
        42,
        1,
        completion_queue.sender(),
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.installing_version_count, 1);
    assert_eq!(diagnostics.starting_version_count, 1);
    assert_eq!(diagnostics.running_host_count, 0);
    assert_eq!(diagnostics.pending_main_script_update_check_count, 0);
    assert!(!completion_queue.has_ready_task());

    let version_id = {
        let state = service.inner.state.lock();
        assert_eq!(state.pending_devtools_launches.len(), 1);
        let (version_id, launch) = state
            .pending_devtools_launches
            .iter()
            .next()
            .expect("install launch should wait for debugger");
        assert_eq!(launch.params.run_owner.version_id(), *version_id);
        assert_eq!(
            launch.params.run_owner.run_identity(),
            &state
                .versions
                .get(version_id)
                .expect("starting version")
                .run
        );
        assert!(launch.preloaded_script.is_none());
        *version_id
    };
    let target_events = service.take_target_output_events_for_test();
    let created_run = exact_created_target_run(&target_events, version_id);
    assert!(
        target_events.iter().any(|event| matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::Created { info, .. }
                if info.version_id == version_id.as_u64()
        )),
        "deferred install launch should still expose the installing target: {target_events:?}"
    );

    service.terminate_all_for_context_shutdown();
    let shutdown_events = service.take_target_output_events_for_test();
    assert!(
        shutdown_events.iter().any(|event| matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::Destroyed {
                version_id: destroyed_version_id,
                active_run: Some(active_run),
            } if *destroyed_version_id == version_id.as_u64()
                && active_run == &created_run
        )),
        "context shutdown should destroy the pending launch's exact run: {shutdown_events:?}"
    );
}
#[test]
fn force_update_page_load_install_reports_devtools_warning() {
    let service = new_service_worker_runtime_service();
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
    let mut queued_job = test_queued_register_job(&service, script_url.clone(), scope_url);
    queued_job.force_bypass_cache = true;
    queued_job.skip_script_comparison = true;
    queued_job.skip_waiting_after_install = true;
    queued_job.force_update_page_load_waiter_ids = vec![1];

    let new_version_id = {
        let (force_update_tx, _force_update_rx) = tokio::sync::oneshot::channel();
        let mut state = service.inner.state.lock();
        state.insert_force_update_page_load_waiter(1, force_update_tx);
        match service
            .start_main_script_update_check_locked(&mut state, registration_id, queued_job)
            .expect("force update should start update check")
        {
            ServiceWorkerMainScriptUpdateCheckStart::Start(_) => {}
            ServiceWorkerMainScriptUpdateCheckStart::WaitForDebugger => {
                panic!("force update should not wait for debugger by default")
            }
        }
        state
            .registrations
            .get(&registration_id)
            .expect("registration should exist")
            .installing_version_id
            .expect("update check should precreate an installing version")
    };

    let target_events = service.take_target_output_events_for_test();
    assert!(
        target_events.iter().any(|event| matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::Created { info, .. }
                if info.version_id == new_version_id.as_u64()
        )),
        "force-update update check should precreate a Service Worker target: {target_events:?}"
    );
    assert!(
        target_events.iter().all(|event| !matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::Console { .. }
        )),
        "force-update warning should wait until the install launch starts: {target_events:?}"
    );

    service.finish_main_script_update_check_completed(
        registration_id,
        Ok(ServiceWorkerScriptUpdateCheckResult {
            main_script: test_loaded_script(&script_url, "self.skipWaiting();"),
            change: ServiceWorkerScriptUpdateCheckChange::ScriptComparisonSkipped,
        }),
    );

    let target_events = service.take_target_output_events_for_test();
    assert!(
        target_events.iter().any(|event| matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::Console {
                version_id,
                message,
                ..
            } if *version_id == new_version_id.as_u64()
                && message.message == SERVICE_WORKER_FORCE_UPDATE_DEVTOOLS_CONSOLE_MESSAGE
                && message.args.is_empty()
                && message.stack.is_none()
        )),
        "force-update install should report Chromium DevTools warning: {target_events:?}"
    );
    service.terminate_all_for_context_shutdown();
}
#[test]
fn resource_store_transient_failure_retries_install_store() {
    let temp_store = TempJsonStorePath::new("install-transient");
    let resource_store =
        crate::new_shared_json_service_worker_resource_store(temp_store.store_path())
            .expect("json resource store should open");
    let service = new_service_worker_runtime_service_with_resource_store(
        resource_store.clone(),
        test_worker_context_runtime(),
    );
    let (registration_id, version_id) = insert_running_installing_version(&service);
    {
        let mut state = service.inner.state.lock();
        let script_url = state
            .versions
            .get(&version_id)
            .expect("installing version should exist")
            .script_url
            .clone();
        state
            .versions
            .get_mut(&version_id)
            .expect("installing version should exist")
            .main_script_resource = Some(test_script_resource(&script_url));
    }
    resource_store.lock().fail_next_persist_attempts_for_test(1);
    let run = exact_version_run(&service, version_id);

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(version_id, &run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });

    assert_eq!(resource_store.lock().registration_count(), 1);
    let state = service.inner.state.lock();
    let registration = state
        .registrations
        .get(&registration_id)
        .expect("registration should survive transient store failure");
    assert_eq!(registration.installing_version_id, None);
    assert_eq!(registration.waiting_version_id, Some(version_id));
    let version = state
        .versions
        .get(&version_id)
        .expect("version should survive transient store failure");
    assert_eq!(
        version.lifecycle_state,
        ServiceWorkerVersionLifecycleState::Installed
    );
}
#[test]
fn resource_store_failure_deletes_initial_install_registration() {
    let failing_store = FailingJsonStorePath::new("install");
    let resource_store =
        crate::new_shared_json_service_worker_resource_store(failing_store.store_path())
            .expect("failing json resource store should open before first write");
    let service = new_service_worker_runtime_service_with_resource_store(
        resource_store.clone(),
        test_worker_context_runtime(),
    );
    let (script_url, _, registration_id, version_id) = insert_starting_version(&service);
    {
        let mut state = service.inner.state.lock();
        state
            .versions
            .get_mut(&version_id)
            .expect("installing version should exist")
            .main_script_resource = Some(test_script_resource(&script_url));
    }
    let run = exact_version_run(&service, version_id);

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(version_id, &run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 0);
    assert_eq!(diagnostics.version_count, 0);
    assert_eq!(resource_store.lock().registration_count(), 0);
    let state = service.inner.state.lock();
    assert!(!state.registrations.contains_key(&registration_id));
    assert!(!state.versions.contains_key(&version_id));
}
#[test]
fn identical_main_script_update_check_destroys_precreated_installing_version() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let script_url = url("https://example.test/app/worker.js");
    let scope_url = url("https://example.test/app/");
    let document_url = url("https://example.test/app/page.html");
    insert_registered_version(
        &service,
        registration_id,
        active_version_id,
        script_url.clone(),
        scope_url.clone(),
        [document_url.clone()],
    );
    {
        let mut state = service.inner.state.lock();
        state
            .registrations
            .get_mut(&registration_id)
            .expect("registration should exist")
            .last_update_check_time_ms = Some(1);
        state
            .versions
            .get_mut(&active_version_id)
            .expect("active version should exist")
            .main_script_resource = Some(test_script_resource(&script_url));
    }
    let mut completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let new_version_id = insert_pending_main_script_update_check(
        &service,
        registration_id,
        active_version_id,
        script_url.clone(),
        scope_url,
        42,
        completion_queue.sender(),
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.version_count, 2);
    assert_eq!(diagnostics.installing_version_count, 1);
    assert_eq!(diagnostics.starting_version_count, 1);
    assert_eq!(diagnostics.pending_main_script_update_check_count, 1);
    let target_events = service.take_target_output_events_for_test();
    let created_run = exact_created_target_run(&target_events, new_version_id);
    assert!(
        target_events.iter().any(|event| matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::Created { info, .. }
                if info.version_id == new_version_id.as_u64()
                    && info.registration_id == registration_id.as_u64()
                    && info.script_url == script_url.as_str()
        )),
        "pending update check should create a Service Worker target: {target_events:?}"
    );
    assert!(!completion_queue.has_ready_task());

    service.finish_main_script_update_check_completed(
        registration_id,
        Ok(test_update_check_result(&script_url, "abc", false)),
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.installing_version_count, 0);
    assert_eq!(diagnostics.starting_version_count, 0);
    assert_eq!(diagnostics.pending_main_script_update_check_count, 0);
    let target_events = service.take_target_output_events_for_test();
    assert!(
        target_events.iter().any(|event| matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::Destroyed {
                version_id,
                active_run: Some(active_run),
            } if *version_id == new_version_id.as_u64()
                && active_run == &created_run
        )),
        "identical update check should destroy the precreated target's exact run: {target_events:?}"
    );
    let update_check = diagnostics.registrations[0]
        .last_main_script_update_check
        .as_ref()
        .expect("identical update check should be diagnosed");
    assert_eq!(update_check.script_url, script_url.as_str());
    assert_eq!(update_check.newest_version_id, active_version_id);
    assert_eq!(update_check.result, "identical");
    assert_eq!(update_check.failure_status, None);
    assert_eq!(update_check.message, None);
    assert_eq!(update_check.imported_script_url, None);
    assert!(
        diagnostics.registrations[0]
            .last_update_check_time_ms
            .is_some_and(|value| value > 1),
        "successful identical update check should bump last update check time"
    );

    let completion = pop_register_completion(&mut completion_queue);
    assert_eq!(completion.request_id, 42);
    let snapshot = completion
        .result
        .expect("identical update check should resolve existing registration");
    assert!(snapshot.installing().is_none());
    assert_eq!(
        snapshot
            .active()
            .expect("active version should remain")
            .version_id(),
        active_version_id
    );
}
#[test]
fn imported_script_update_check_change_creates_installing_version() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let script_url = url("https://example.test/app/worker.js");
    let scope_url = url("https://example.test/app/");
    let document_url = url("https://example.test/app/page.html");
    insert_registered_version(
        &service,
        registration_id,
        active_version_id,
        script_url.clone(),
        scope_url.clone(),
        [document_url],
    );
    {
        let mut state = service.inner.state.lock();
        state
            .registrations
            .get_mut(&registration_id)
            .expect("registration should exist")
            .last_update_check_time_ms = Some(1);
        state
            .versions
            .get_mut(&active_version_id)
            .expect("active version should exist")
            .main_script_resource = Some(test_script_resource(&script_url));
    }
    let mut completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let new_version_id = insert_pending_main_script_update_check(
        &service,
        registration_id,
        active_version_id,
        script_url.clone(),
        scope_url,
        42,
        completion_queue.sender(),
    );
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.version_count, 2);
    assert_eq!(diagnostics.installing_version_count, 1);
    assert_eq!(diagnostics.starting_version_count, 1);
    assert_eq!(
        diagnostics.registrations[0].installing_version_id,
        Some(new_version_id)
    );

    service.finish_main_script_update_check_completed(
        registration_id,
        Ok(test_update_check_result(&script_url, "abc", true)),
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.version_count, 2);
    assert_eq!(diagnostics.installing_version_count, 1);
    assert_eq!(diagnostics.pending_main_script_update_check_count, 0);
    let update_check = diagnostics.registrations[0]
        .last_main_script_update_check
        .as_ref()
        .expect("imported script change should be diagnosed");
    assert_eq!(update_check.result, "imported-script-different");
    assert_eq!(
        update_check.imported_script_url.as_deref(),
        Some("https://example.test/app/dep.js")
    );
    assert!(
        diagnostics.registrations[0]
            .last_update_check_time_ms
            .is_some_and(|value| value > 1),
        "successful changed update check should bump last update check time"
    );
    assert!(!completion_queue.has_ready_task());
}
#[test]
fn devtools_skip_waiting_scope_uses_activation_progress() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let waiting_version_id = ServiceWorkerVersionId(2);
    let scope_url = url("https://example.test/app/");
    let active_script_url = url("https://example.test/app/worker-v1.js");
    let waiting_script_url = url("https://example.test/app/worker-v2.js");
    let waiting_run = RendererServiceWorkerRunIdentity::fresh();
    insert_registered_version(
        &service,
        registration_id,
        active_version_id,
        active_script_url.clone(),
        scope_url.clone(),
        [],
    );
    let waiting_host = new_running_test_host(waiting_version_id, &waiting_run);
    {
        let mut state = service.inner.state.lock();
        let registration = state.registrations.get_mut(&registration_id).unwrap();
        registration.script_url = waiting_script_url.clone();
        registration.waiting_version_id = Some(waiting_version_id);
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
                running_state: ServiceWorkerVersionRunningState::Running { host: waiting_host },
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

    assert_eq!(
        service.devtools_skip_waiting_for_scope(&scope_url),
        Ok(true),
        "DevTools skipWaiting should accept the waiting worker"
    );

    let state = service.inner.state.lock();
    let registration = state.registrations.get(&registration_id).unwrap();
    assert_eq!(registration.active_version_id, Some(active_version_id));
    assert_eq!(registration.waiting_version_id, Some(waiting_version_id));
    let waiting = state.versions.get(&waiting_version_id).unwrap();
    assert!(waiting.skip_waiting_requested);
    assert_eq!(
        waiting.lifecycle_state,
        ServiceWorkerVersionLifecycleState::Activating
    );
    assert_eq!(waiting.in_flight_event_count, 1);
}
#[test]
fn install_completion_moves_version_to_waiting_and_decrements_event_count() {
    let service = new_service_worker_runtime_service();
    let (registration_id, version_id) = insert_running_installing_version(&service);
    let run = exact_version_run(&service, version_id);

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(version_id, &run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });

    let state = service.inner.state.lock();
    let registration = state.registrations.get(&registration_id).unwrap();
    assert_eq!(registration.installing_version_id, None);
    assert_eq!(registration.waiting_version_id, Some(version_id));
    assert_eq!(registration.active_version_id, None);
    let version = state.versions.get(&version_id).unwrap();
    assert_eq!(
        version.lifecycle_state,
        ServiceWorkerVersionLifecycleState::Installed
    );
    assert_eq!(version.in_flight_event_count, 0);
}
#[test]
fn install_rejection_deletes_initial_install_registration() {
    let service = new_service_worker_runtime_service();
    let (registration_id, version_id) = insert_running_installing_version(&service);
    let run = exact_version_run(&service, version_id);

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(version_id, &run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Err("service worker waitUntil promise rejected".to_owned()),
    });

    let state = service.inner.state.lock();
    assert!(!state.registrations.contains_key(&registration_id));
    assert!(!state.versions.contains_key(&version_id));
}
#[test]
fn activate_rejection_still_commits_active_version() {
    let service = new_service_worker_runtime_service();
    let (registration_id, version_id) = insert_running_installing_version(&service);
    let run = exact_version_run(&service, version_id);
    {
        let mut state = service.inner.state.lock();
        let registration = state.registrations.get_mut(&registration_id).unwrap();
        registration.installing_version_id = None;
        registration.waiting_version_id = Some(version_id);
        let version = state.versions.get_mut(&version_id).unwrap();
        version.lifecycle_state = ServiceWorkerVersionLifecycleState::Activating;
        version.in_flight_event_count = 1;
    }

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(2),
        owner: test_run_owner(version_id, &run),
        kind: ServiceWorkerLifecycleEventKind::Activate,
        result: Err("service worker waitUntil promise rejected".to_owned()),
    });

    let state = service.inner.state.lock();
    let registration = state.registrations.get(&registration_id).unwrap();
    assert_eq!(registration.installing_version_id, None);
    assert_eq!(registration.waiting_version_id, None);
    assert_eq!(registration.active_version_id, Some(version_id));
    let version = state.versions.get(&version_id).unwrap();
    assert_eq!(
        version.lifecycle_state,
        ServiceWorkerVersionLifecycleState::Activated
    );
    assert_eq!(version.in_flight_event_count, 0);
    assert_eq!(
        version.last_start_error.as_deref(),
        Some("service worker waitUntil promise rejected")
    );
}
#[test]
fn activate_completion_does_not_clear_newer_installing_version() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let previous_active_version_id = ServiceWorkerVersionId(1);
    let activating_version_id = ServiceWorkerVersionId(2);
    let installing_version_id = ServiceWorkerVersionId(3);
    let scope_url = url("https://example.test/app/");
    let previous_script_url = url("https://example.test/app/worker-v1.js");
    let activating_script_url = url("https://example.test/app/worker-v2.js");
    let installing_script_url = url("https://example.test/app/worker-v3.js");
    {
        let mut state = service.inner.state.lock();
        state.registrations.insert(
            registration_id,
            ServiceWorkerRegistration {
                id: registration_id,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
                scope_url: scope_url.clone(),
                script_url: installing_script_url.clone(),
                installing_version_id: Some(installing_version_id),
                waiting_version_id: Some(activating_version_id),
                active_version_id: Some(previous_active_version_id),
                pending_unregistration: false,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                last_update_check_time_ms: None,
                pending_register_jobs: HashMap::new(),
                controlled_client_ids: HashSet::new(),
            },
        );
        for (version_id, script_url, lifecycle_state, in_flight_event_count) in [
            (
                previous_active_version_id,
                previous_script_url,
                ServiceWorkerVersionLifecycleState::Activated,
                0,
            ),
            (
                activating_version_id,
                activating_script_url,
                ServiceWorkerVersionLifecycleState::Activating,
                1,
            ),
            (
                installing_version_id,
                installing_script_url,
                ServiceWorkerVersionLifecycleState::Installing,
                0,
            ),
        ] {
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
                    lifecycle_state,
                    running_state: ServiceWorkerVersionRunningState::Stopped,
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count,
                    run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }
    }
    let activating_run = exact_version_run(&service, activating_version_id);

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(2),
        owner: test_run_owner(activating_version_id, &activating_run),
        kind: ServiceWorkerLifecycleEventKind::Activate,
        result: Ok(()),
    });

    let state = service.inner.state.lock();
    let registration = state.registrations.get(&registration_id).unwrap();
    assert_eq!(
        registration.installing_version_id,
        Some(installing_version_id)
    );
    assert_eq!(registration.waiting_version_id, None);
    assert_eq!(registration.active_version_id, Some(activating_version_id));
    assert_eq!(
        state
            .versions
            .get(&previous_active_version_id)
            .unwrap()
            .lifecycle_state,
        ServiceWorkerVersionLifecycleState::Redundant
    );
    assert_eq!(
        state
            .versions
            .get(&installing_version_id)
            .unwrap()
            .lifecycle_state,
        ServiceWorkerVersionLifecycleState::Installing
    );
}
#[test]
fn activate_completion_does_not_clear_newer_waiting_version() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let previous_active_version_id = ServiceWorkerVersionId(1);
    let activating_version_id = ServiceWorkerVersionId(2);
    let waiting_version_id = ServiceWorkerVersionId(3);
    let scope_url = url("https://example.test/app/");
    let previous_script_url = url("https://example.test/app/worker-v1.js");
    let activating_script_url = url("https://example.test/app/worker-v2.js");
    let waiting_script_url = url("https://example.test/app/worker-v3.js");
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
                active_version_id: Some(previous_active_version_id),
                pending_unregistration: false,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                last_update_check_time_ms: None,
                pending_register_jobs: HashMap::new(),
                controlled_client_ids: HashSet::new(),
            },
        );
        for (version_id, script_url, lifecycle_state, in_flight_event_count) in [
            (
                previous_active_version_id,
                previous_script_url,
                ServiceWorkerVersionLifecycleState::Activated,
                0,
            ),
            (
                activating_version_id,
                activating_script_url,
                ServiceWorkerVersionLifecycleState::Activating,
                1,
            ),
            (
                waiting_version_id,
                waiting_script_url,
                ServiceWorkerVersionLifecycleState::Installed,
                0,
            ),
        ] {
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
                    lifecycle_state,
                    running_state: ServiceWorkerVersionRunningState::Stopped,
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count,
                    run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }
    }
    let activating_run = exact_version_run(&service, activating_version_id);

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(2),
        owner: test_run_owner(activating_version_id, &activating_run),
        kind: ServiceWorkerLifecycleEventKind::Activate,
        result: Ok(()),
    });

    let state = service.inner.state.lock();
    let registration = state.registrations.get(&registration_id).unwrap();
    assert_eq!(registration.installing_version_id, None);
    assert_eq!(registration.waiting_version_id, Some(waiting_version_id));
    assert_eq!(registration.active_version_id, Some(activating_version_id));
    assert_eq!(
        state
            .versions
            .get(&previous_active_version_id)
            .unwrap()
            .lifecycle_state,
        ServiceWorkerVersionLifecycleState::Redundant
    );
    assert_eq!(
        state
            .versions
            .get(&waiting_version_id)
            .unwrap()
            .lifecycle_state,
        ServiceWorkerVersionLifecycleState::Installed
    );
}
#[test]
fn pending_ready_job_waits_for_target_registration_activation() {
    let service = new_service_worker_runtime_service();
    let root_registration_id = ServiceWorkerRegistrationId(1);
    let root_version_id = ServiceWorkerVersionId(1);
    let admin_registration_id = ServiceWorkerRegistrationId(2);
    let admin_version_id = ServiceWorkerVersionId(2);
    let root_scope_url = url("https://example.test/app/");
    let admin_scope_url = url("https://example.test/app/admin/");
    let document_url = url("https://example.test/app/admin/page.html");
    insert_inactive_registration(
        &service,
        root_registration_id,
        root_version_id,
        url("https://example.test/app/root-sw.js"),
        root_scope_url,
    );
    insert_inactive_registration(
        &service,
        admin_registration_id,
        admin_version_id,
        url("https://example.test/app/admin/sw.js"),
        admin_scope_url.clone(),
    );

    let mut ready_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    assert!(service.watch_ready_registration(document_url, 91, 1, ready_queue.sender(),));
    assert!(!ready_queue.has_ready_task());

    {
        let mut state = service.inner.state.lock();
        state
            .registrations
            .get_mut(&root_registration_id)
            .unwrap()
            .installing_version_id = None;
        state
            .registrations
            .get_mut(&root_registration_id)
            .unwrap()
            .waiting_version_id = Some(root_version_id);
        let root_version = state.versions.get_mut(&root_version_id).unwrap();
        root_version.lifecycle_state = ServiceWorkerVersionLifecycleState::Activating;
        root_version.in_flight_event_count = 1;
    }
    let root_run = exact_version_run(&service, root_version_id);
    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(2),
        owner: test_run_owner(root_version_id, &root_run),
        kind: ServiceWorkerLifecycleEventKind::Activate,
        result: Ok(()),
    });
    assert!(
        !ready_queue.has_ready_task(),
        "broader scope activation must not resolve ready job for narrower scope"
    );

    {
        let mut state = service.inner.state.lock();
        state
            .registrations
            .get_mut(&admin_registration_id)
            .unwrap()
            .installing_version_id = None;
        state
            .registrations
            .get_mut(&admin_registration_id)
            .unwrap()
            .waiting_version_id = Some(admin_version_id);
        let admin_version = state.versions.get_mut(&admin_version_id).unwrap();
        admin_version.lifecycle_state = ServiceWorkerVersionLifecycleState::Activating;
        admin_version.in_flight_event_count = 1;
    }
    let admin_run = exact_version_run(&service, admin_version_id);
    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(3),
        owner: test_run_owner(admin_version_id, &admin_run),
        kind: ServiceWorkerLifecycleEventKind::Activate,
        result: Ok(()),
    });

    let completion = match ready_queue.pop_internal() {
        Some(crate::page_task_queue::RendererServiceWorkerInternalTask::Ready(completion)) => {
            completion
        }
        other => panic!("expected ready completion, got {other:?}"),
    };
    assert_eq!(completion.request_id, 91);
    assert_eq!(completion.registration.scope_url(), &admin_scope_url);
}
#[test]
fn update_install_waits_when_existing_active_version_has_not_been_skipped() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let installing_version_id = ServiceWorkerVersionId(2);
    let scope_url = url("https://example.test/app/");
    let active_script_url = url("https://example.test/app/worker-v1.js");
    let installing_script_url = url("https://example.test/app/worker-v2.js");
    let installing_run = RendererServiceWorkerRunIdentity::fresh();
    let host = new_running_test_host(installing_version_id, &installing_run);
    let (force_update_tx, mut force_update_rx) = tokio::sync::oneshot::channel();
    {
        let mut state = service.inner.state.lock();
        let client_id = ServiceWorkerClientId::from_u64_for_test(1);
        state.insert_force_update_page_load_waiter(1, force_update_tx);
        state.bind_force_update_page_load_waiters(installing_version_id, vec![1]);
        state.live_clients.insert(
            client_id,
            ServiceWorkerClient {
                id: client_id,
                exposed_id: service_worker_exposed_client_id(client_id),
                creation_url: scope_url.clone(),
                document_url: scope_url.clone(),
                client_type: ServiceWorkerClientType::Window,
                frame_type: ServiceWorkerClientFrameType::TopLevel,
                visibility_state: ServiceWorkerClientVisibilityState::Visible,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
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
                script_url: installing_script_url.clone(),
                installing_version_id: Some(installing_version_id),
                waiting_version_id: None,
                active_version_id: Some(active_version_id),
                pending_unregistration: false,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                last_update_check_time_ms: None,
                pending_register_jobs: HashMap::new(),
                controlled_client_ids: HashSet::from([client_id]),
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
            installing_version_id,
            ServiceWorkerVersion {
                id: installing_version_id,
                registration_id,
                script_url: installing_script_url.clone(),
                final_script_url: Some(installing_script_url.clone()),
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: WorkerScriptKind::Classic,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: test_launch_config(&service, &installing_script_url, &scope_url),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Installing,
                running_state: ServiceWorkerVersionRunningState::Running { host },
                pending_start_events: VecDeque::new(),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 1,
                run: installing_run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(installing_version_id, &installing_run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });

    {
        let state = service.inner.state.lock();
        let registration = state.registrations.get(&registration_id).unwrap();
        assert_eq!(registration.active_version_id, Some(active_version_id));
        assert_eq!(registration.waiting_version_id, Some(installing_version_id));
        let installed = state.versions.get(&installing_version_id).unwrap();
        assert_eq!(
            installed.lifecycle_state,
            ServiceWorkerVersionLifecycleState::Installed
        );
        assert_eq!(installed.in_flight_event_count, 0);
        let active = state.versions.get(&active_version_id).unwrap();
        assert_eq!(
            active.lifecycle_state,
            ServiceWorkerVersionLifecycleState::Activated
        );
    }

    service.finish_worker_skip_waiting_requested(registration_id, installing_version_id);

    let state = service.inner.state.lock();
    let registration = state.registrations.get(&registration_id).unwrap();
    assert_eq!(registration.active_version_id, Some(active_version_id));
    assert_eq!(registration.waiting_version_id, Some(installing_version_id));
    let activating = state.versions.get(&installing_version_id).unwrap();
    assert_eq!(
        activating.lifecycle_state,
        ServiceWorkerVersionLifecycleState::Activating
    );
    assert!(activating.skip_waiting_requested);
    assert_eq!(activating.in_flight_event_count, 1);
    assert_eq!(
        force_update_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty),
        "force-update page load must keep waiting until activation settles"
    );
    drop(state);

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(2),
        owner: test_run_owner(installing_version_id, &installing_run),
        kind: ServiceWorkerLifecycleEventKind::Activate,
        result: Ok(()),
    });

    let state = service.inner.state.lock();
    let registration = state.registrations.get(&registration_id).unwrap();
    assert_eq!(registration.active_version_id, Some(installing_version_id));
    assert_eq!(
        state
            .versions
            .get(&active_version_id)
            .unwrap()
            .lifecycle_state,
        ServiceWorkerVersionLifecycleState::Redundant
    );
    assert_eq!(force_update_rx.try_recv(), Ok(()));
}
#[test]
fn force_update_install_skips_waiting_even_with_active_controllee() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let installing_version_id = ServiceWorkerVersionId(2);
    let scope_url = url("https://example.test/app/");
    let active_script_url = url("https://example.test/app/worker-v1.js");
    let installing_script_url = url("https://example.test/app/worker-v2.js");
    let installing_run = RendererServiceWorkerRunIdentity::fresh();
    let host = new_running_test_host(installing_version_id, &installing_run);
    {
        let mut state = service.inner.state.lock();
        let client_id = ServiceWorkerClientId::from_u64_for_test(1);
        state.live_clients.insert(
            client_id,
            ServiceWorkerClient {
                id: client_id,
                exposed_id: service_worker_exposed_client_id(client_id),
                creation_url: url("https://example.test/app/page.html"),
                document_url: url("https://example.test/app/page.html"),
                client_type: ServiceWorkerClientType::Window,
                frame_type: ServiceWorkerClientFrameType::TopLevel,
                visibility_state: ServiceWorkerClientVisibilityState::Visible,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
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
                script_url: installing_script_url.clone(),
                installing_version_id: Some(installing_version_id),
                waiting_version_id: None,
                active_version_id: Some(active_version_id),
                pending_unregistration: false,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                last_update_check_time_ms: None,
                pending_register_jobs: HashMap::from([(
                    installing_version_id,
                    ServiceWorkerPendingRegisterJob::new_with_options(Vec::new(), true),
                )]),
                controlled_client_ids: HashSet::from([client_id]),
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
            installing_version_id,
            ServiceWorkerVersion {
                id: installing_version_id,
                registration_id,
                script_url: installing_script_url.clone(),
                final_script_url: Some(installing_script_url.clone()),
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: WorkerScriptKind::Classic,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: test_launch_config(&service, &installing_script_url, &scope_url),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Installing,
                running_state: ServiceWorkerVersionRunningState::Running { host },
                pending_start_events: VecDeque::new(),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 1,
                run: installing_run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(installing_version_id, &installing_run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });

    let state = service.inner.state.lock();
    let registration = state.registrations.get(&registration_id).unwrap();
    assert_eq!(registration.active_version_id, Some(active_version_id));
    assert_eq!(registration.waiting_version_id, Some(installing_version_id));
    let activating = state.versions.get(&installing_version_id).unwrap();
    assert_eq!(
        activating.lifecycle_state,
        ServiceWorkerVersionLifecycleState::Activating
    );
    assert!(activating.skip_waiting_requested);
    assert_eq!(activating.in_flight_event_count, 1);
}
#[test]
fn update_install_activates_when_existing_active_has_no_controllees() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let installing_version_id = ServiceWorkerVersionId(2);
    let scope_url = url("https://example.test/app/");
    let active_script_url = url("https://example.test/app/worker-v1.js");
    let installing_script_url = url("https://example.test/app/worker-v2.js");
    let installing_run = RendererServiceWorkerRunIdentity::fresh();
    let host = new_running_test_host(installing_version_id, &installing_run);
    {
        let mut state = service.inner.state.lock();
        state.registrations.insert(
            registration_id,
            ServiceWorkerRegistration {
                id: registration_id,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
                scope_url: scope_url.clone(),
                script_url: installing_script_url.clone(),
                installing_version_id: Some(installing_version_id),
                waiting_version_id: None,
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
            installing_version_id,
            ServiceWorkerVersion {
                id: installing_version_id,
                registration_id,
                script_url: installing_script_url.clone(),
                final_script_url: Some(installing_script_url.clone()),
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: WorkerScriptKind::Classic,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: test_launch_config(&service, &installing_script_url, &scope_url),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Installing,
                running_state: ServiceWorkerVersionRunningState::Running { host },
                pending_start_events: VecDeque::new(),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 1,
                run: installing_run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(installing_version_id, &installing_run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });

    let state = service.inner.state.lock();
    let registration = state.registrations.get(&registration_id).unwrap();
    assert_eq!(registration.active_version_id, Some(active_version_id));
    assert_eq!(registration.waiting_version_id, Some(installing_version_id));
    let installing = state.versions.get(&installing_version_id).unwrap();
    assert_eq!(
        installing.lifecycle_state,
        ServiceWorkerVersionLifecycleState::Activating
    );
    assert_eq!(installing.in_flight_event_count, 1);
    drop(state);
    assert_eq!(service.pending_service_lane_event_count(), 1);
}
#[test]
fn activate_completion_queues_service_worker_target_activated_version_update() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(2);
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
                waiting_version_id: Some(version_id),
                active_version_id: None,
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
                lifecycle_state: ServiceWorkerVersionLifecycleState::Activating,
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
        service.take_target_output_events_for_test();
    }

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(7),
        owner: test_run_owner(version_id, &run),
        kind: ServiceWorkerLifecycleEventKind::Activate,
        result: Ok(()),
    });

    let activation_events = service.take_target_output_events_for_test();
    assert!(
        activation_events.iter().any(|event| matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::VersionUpdated {
                version_id: updated_version_id,
                status: crate::runtime::RendererServiceWorkerVersionStatus::Activated,
            } if *updated_version_id == version_id.as_u64()
        )),
        "activate completion should refresh target status to activated: {activation_events:?}"
    );
}
#[test]
fn skip_waiting_update_activation_replaces_previous_active_version() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let waiting_version_id = ServiceWorkerVersionId(2);
    let scope_url = url("https://example.test/app/");
    let active_script_url = url("https://example.test/app/worker-v1.js");
    let waiting_script_url = url("https://example.test/app/worker-v2.js");
    let waiting_run = RendererServiceWorkerRunIdentity::fresh();
    let mut client_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let client_id = ServiceWorkerClientId::from_u64_for_test(1);
    {
        let mut state = service.inner.state.lock();
        state.live_clients.insert(
            client_id,
            ServiceWorkerClient {
                id: client_id,
                exposed_id: service_worker_exposed_client_id(client_id),
                creation_url: url("https://example.test/app/page.html"),
                document_url: url("https://example.test/app/page.html"),
                client_type: ServiceWorkerClientType::Window,
                frame_type: ServiceWorkerClientFrameType::TopLevel,
                visibility_state: ServiceWorkerClientVisibilityState::Visible,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
                secure_context: true,
                execution_ready: true,
                discarded_or_frozen: false,
                document_owner: Some(crate::native_bridge::WindowDocumentOwner::for_test(0)),
                endpoint: ServiceWorkerClientEndpoint::Page(client_queue.sender()),
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

    let state = service.inner.state.lock();
    let registration = state.registrations.get(&registration_id).unwrap();
    assert_eq!(registration.active_version_id, Some(waiting_version_id));
    assert_eq!(registration.waiting_version_id, None);
    assert_eq!(
        state
            .versions
            .get(&waiting_version_id)
            .unwrap()
            .lifecycle_state,
        ServiceWorkerVersionLifecycleState::Activated
    );
    assert_eq!(
        state
            .versions
            .get(&active_version_id)
            .unwrap()
            .lifecycle_state,
        ServiceWorkerVersionLifecycleState::Redundant
    );
    drop(state);
    let completion = match client_queue.pop_internal() {
        Some(crate::page_task_queue::RendererServiceWorkerInternalTask::ControllerChange(
            completion,
        )) => completion,
        other => panic!("expected replacement controllerchange completion, got {other:?}"),
    };
    assert_eq!(completion.target.client_id, client_id);
    assert!(!client_queue.has_ready_task());
}
