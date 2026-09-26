use super::*;

#[test]
fn registration_lifecycle_watchers_are_partitioned_by_storage_key() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let scope_url = url("https://lifecycle-partition.test/app/");
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://lifecycle-partition.test/app/worker.js"),
        scope_url.clone(),
        [],
    );

    let deliveries = {
        let mut state = service.inner.state.lock();
        let registration_storage_key = state
            .registrations
            .get(&registration_id)
            .expect("registered service worker")
            .storage_key
            .clone();
        state.lifecycle_watchers.extend([
            ServiceWorkerLifecycleWatcher {
                scope_url: scope_url.clone(),
                storage_key: registration_storage_key.clone(),
                document_owner: crate::native_bridge::WindowDocumentOwner::for_test(7),
                completion_tx: test_completion_sender(),
            },
            ServiceWorkerLifecycleWatcher {
                scope_url: scope_url.clone(),
                storage_key: "different-partition".to_owned(),
                document_owner: crate::native_bridge::WindowDocumentOwner::for_test(8),
                completion_tx: test_completion_sender(),
            },
        ]);
        lifecycle_notifications_for_registration_locked(
            &state,
            registration_id,
            vec![ServiceWorkerLifecycleClientEvent::UpdateFound],
        )
    };

    assert_eq!(deliveries.len(), 1);
    assert_eq!(
        deliveries[0].watcher.document_owner,
        crate::native_bridge::WindowDocumentOwner::for_test(7)
    );
    assert_ne!(deliveries[0].watcher.storage_key, "different-partition");
}
#[test]
fn queued_unregister_job_phase_transitions_to_complete() {
    let mut job = ServiceWorkerQueuedUnregisterJob::new(None);
    assert_eq!(job.phase(), ServiceWorkerUnregisterJobPhase::Initial);
    job.mark_pending();
    assert_eq!(job.phase(), ServiceWorkerUnregisterJobPhase::MarkPending);
    assert_eq!(
        job.send_all(true),
        ServiceWorkerUnregisterJobPhase::Complete
    );
}
#[test]
fn matching_controller_uses_longest_scope() {
    let service = new_service_worker_runtime_service();
    let root = insert_registered_version(
        &service,
        ServiceWorkerRegistrationId(1),
        ServiceWorkerVersionId(1),
        url("https://example.test/root-worker.js"),
        url("https://example.test/"),
        [url("https://example.test/other.html")],
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
        service.matching_controller_for_document(&url("https://example.test/app/page.html")),
        Some(app)
    );
    assert_eq!(
        service.matching_controller_for_document(&url("https://example.test/other.html")),
        Some(root)
    );
}
#[test]
fn matching_controller_respects_scope_path_boundary() {
    let service = new_service_worker_runtime_service();
    let app = insert_registered_version(
        &service,
        ServiceWorkerRegistrationId(1),
        ServiceWorkerVersionId(1),
        url("https://example.test/app-worker.js"),
        url("https://example.test/app"),
        [
            url("https://example.test/app"),
            url("https://example.test/app/page.html"),
            url("https://example.test/app?query"),
            url("https://example.test/app#fragment"),
        ],
    );

    assert_eq!(
        service.matching_controller_for_document(&url("https://example.test/app")),
        Some(app.clone())
    );
    assert_eq!(
        service.matching_controller_for_document(&url("https://example.test/app/page.html")),
        Some(app.clone())
    );
    assert_eq!(
        service.matching_controller_for_document(&url("https://example.test/app?query")),
        Some(app.clone())
    );
    assert_eq!(
        service.matching_controller_for_document(&url("https://example.test/app#fragment")),
        Some(app)
    );
    assert_eq!(
        service.matching_controller_for_document(&url("https://example.test/app2/page.html")),
        None
    );
}
#[test]
fn matching_controller_ignores_inactive_registration() {
    let service = new_service_worker_runtime_service();
    insert_inactive_registration(
        &service,
        ServiceWorkerRegistrationId(1),
        ServiceWorkerVersionId(1),
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
    );
    let inactive = snapshot_for_registration(&service, ServiceWorkerRegistrationId(1));

    assert_eq!(
        service.matching_registration_for_client(&url("https://example.test/app/page.html")),
        Some(inactive)
    );
    assert_eq!(
        service.matching_controller_for_document(&url("https://example.test/app/page.html")),
        None
    );
    assert_eq!(
        service.matching_controller_for_fetch(
            &url("https://example.test/app/page.html"),
            &url("https://example.test/app/api.json")
        ),
        None
    );
}
#[test]
fn pending_unregistration_without_controllees_deletes_immediately() {
    let service = new_service_worker_runtime_service();
    let scope_url = url("https://example.test/app/");
    insert_registered_version(
        &service,
        ServiceWorkerRegistrationId(1),
        ServiceWorkerVersionId(1),
        url("https://example.test/app/worker.js"),
        scope_url.clone(),
        [],
    );

    assert!(service.mark_registration_unregistered(&scope_url));

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 0);
    assert_eq!(diagnostics.version_count, 0);
    assert_eq!(diagnostics.pending_unregistration_count, 0);
    assert!(!service.mark_registration_unregistered(&scope_url));
}
#[test]
fn pending_unregistration_deletion_clears_owner_state() {
    let service = new_service_worker_runtime_service();
    let deleted_registration_id = ServiceWorkerRegistrationId(1);
    let kept_registration_id = ServiceWorkerRegistrationId(2);
    let deleted_version_id = ServiceWorkerVersionId(1);
    let kept_version_id = ServiceWorkerVersionId(2);
    let deleted_event_id = ServiceWorkerEventId(31);
    let kept_event_id = ServiceWorkerEventId(32);
    let deleted_scope_url = url("https://example.test/app/");
    let kept_scope_url = url("https://example.test/other/");
    let request_url = url("https://example.test/app/data.txt");
    let mut deleted_fetch_queue = async_subresource_completion_queue();
    let mut kept_fetch_queue = async_subresource_completion_queue();
    insert_registered_version(
        &service,
        deleted_registration_id,
        deleted_version_id,
        url("https://example.test/app/worker.js"),
        deleted_scope_url.clone(),
        [],
    );
    insert_registered_version(
        &service,
        kept_registration_id,
        kept_version_id,
        url("https://example.test/other/worker.js"),
        kept_scope_url,
        [],
    );
    {
        let mut state = service.inner.state.lock();
        for (event_id, version_id, completion_tx) in [
            (
                deleted_event_id,
                deleted_version_id,
                deleted_fetch_queue.sender(),
            ),
            (kept_event_id, kept_version_id, kept_fetch_queue.sender()),
        ] {
            let run = state
                .versions
                .get(&version_id)
                .expect("inserted version")
                .run
                .clone();
            state.pending_fetch_jobs.insert(
                event_id,
                ServiceWorkerFetchJob {
                    request: {
                        let origin = moli_url::WebOrigin::from_url(&request_url);
                        let initiator = request_url.clone();
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
                    internal_id: event_id.as_u64(),
                    owner: Some(ServiceWorkerRunOwner::new(version_id, run)),
                    cors_preflight_request_headers: Vec::new(),
                    client_id: ServiceWorkerClientId::from_u64_for_test(0),
                    resulting_client_id: None,
                    destination: ServiceWorkerRequestDestination::Empty,
                    is_reload: false,
                    metadata: Default::default(),
                    request_cookie_report: None,
                    network_context: AsyncSubresourceNetworkContext {
                        frame_id: None,
                        request_origin: moli_url::WebOrigin::from_url(&request_url),
                        document_url: request_url.clone(),
                        resource_type: crate::types::SubresourceResourceType::Fetch,
                        policy_context: Default::default(),
                    },
                    completion_tx,
                    request_client: test_request_client(&service),
                    resource_task_runner: test_resource_task_runner(),
                    cancel_handle: moli_fetch::FetchCancelHandle::new(),
                    navigation_preload_cancel_handle: None,
                    streaming_body_source_id: None,
                    direct_completion_tx: None,
                },
            );
        }
        for registration_id in [deleted_registration_id, kept_registration_id] {
            state.pending_ready_jobs.push(ServiceWorkerReadyJob {
                request_id: registration_id.as_u64(),
                document_owner: crate::native_bridge::WindowDocumentOwner::for_test(1),
                completion_tx: test_completion_sender(),
                registration_id,
            });
            state
                .notification_records
                .push(ServiceWorkerNotificationRecord {
                    id: registration_id.as_u64(),
                    registration_id,
                    title: format!("notification-{}", registration_id.as_u64()),
                    tag: "tag".to_owned(),
                    metadata: ServiceWorkerNotificationMetadata::default(),
                    actions: Vec::new(),
                    data: V8StructuredClonePayload::default(),
                });
            state.push_subscriptions.insert(
                registration_id,
                service_worker_push_subscription_snapshot(registration_id, true),
            );
        }
        state.sync_registrations.insert(
            (deleted_registration_id, "deleted-sync".to_owned()),
            ServiceWorkerSyncRegistrationRecord::default(),
        );
        state.sync_registrations.insert(
            (kept_registration_id, "kept-sync".to_owned()),
            ServiceWorkerSyncRegistrationRecord::default(),
        );
        state.periodic_sync_registrations.insert(
            (deleted_registration_id, "deleted-periodic".to_owned()),
            ServiceWorkerPeriodicSyncRegistrationRecord::new(10),
        );
        state.periodic_sync_registrations.insert(
            (kept_registration_id, "kept-periodic".to_owned()),
            ServiceWorkerPeriodicSyncRegistrationRecord::new(20),
        );
    }

    assert!(service.mark_registration_unregistered(&deleted_scope_url));

    {
        let state = service.inner.state.lock();
        assert!(!state.registrations.contains_key(&deleted_registration_id));
        assert!(!state.versions.contains_key(&deleted_version_id));
        assert!(state.registrations.contains_key(&kept_registration_id));
        assert!(state.versions.contains_key(&kept_version_id));
        assert!(!state.pending_fetch_jobs.contains_key(&deleted_event_id));
        assert!(state.pending_fetch_jobs.contains_key(&kept_event_id));
        assert!(
            state
                .pending_ready_jobs
                .iter()
                .all(|job| job.registration_id != deleted_registration_id)
        );
        assert!(
            state
                .pending_ready_jobs
                .iter()
                .any(|job| job.registration_id == kept_registration_id)
        );
        assert!(
            state
                .notification_records
                .iter()
                .all(|record| record.registration_id != deleted_registration_id)
        );
        assert!(
            state
                .notification_records
                .iter()
                .any(|record| record.registration_id == kept_registration_id)
        );
        assert!(
            !state
                .push_subscriptions
                .contains_key(&deleted_registration_id)
        );
        assert!(state.push_subscriptions.contains_key(&kept_registration_id));
        assert!(
            state
                .sync_registrations
                .keys()
                .all(|(registration_id, _)| *registration_id != deleted_registration_id)
        );
        assert!(
            state
                .periodic_sync_registrations
                .keys()
                .all(|(registration_id, _)| *registration_id != deleted_registration_id)
        );
        assert!(
            state
                .sync_registrations
                .contains_key(&(kept_registration_id, "kept-sync".to_owned()))
        );
        assert!(
            state
                .periodic_sync_registrations
                .contains_key(&(kept_registration_id, "kept-periodic".to_owned()))
        );
    }

    let completion = pop_async_subresource_completion(&mut deleted_fetch_queue);
    assert_eq!(completion.internal_id, deleted_event_id.as_u64());
    assert_eq!(
        completion.result.err().as_deref(),
        Some(SERVICE_WORKER_REGISTRATION_DELETED_FETCH_ERROR)
    );
    assert!(!kept_fetch_queue.has_ready_completion());
}
#[test]
fn queued_unregister_coalesces_repeated_callbacks() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker-v1.js");
    let mut register_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let mut first_unregister_queue =
        crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let mut second_unregister_queue =
        crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    insert_starting_version_with_register_job(
        &service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url.clone(),
        11,
        register_queue.sender(),
    );

    assert_eq!(
        service.start_unregistration(&scope_url, 21, 1, first_unregister_queue.sender()),
        ServiceWorkerUnregisterStart::Queued
    );
    assert_eq!(
        service.start_unregistration(&scope_url, 22, 2, second_unregister_queue.sender()),
        ServiceWorkerUnregisterStart::Queued
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.queued_unregistration_job_count, 1);
    assert_eq!(diagnostics.pending_unregistration_count, 0);
    {
        let state = service.inner.state.lock();
        let registration_key = ServiceWorkerRegistrationKey::for_scope_url(&scope_url);
        assert_eq!(
            state
                .job_coordinator
                .queued_unregistration_phases(&registration_key),
            vec![ServiceWorkerUnregisterJobPhase::Initial]
        );
    }

    let run = exact_version_run(&service, version_id);
    service.finish_worker_start_completed(version_id, run.clone(), script_url.to_string());
    assert!(!register_queue.has_ready_task());
    assert!(!first_unregister_queue.has_ready_task());
    assert!(!second_unregister_queue.has_ready_task());

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(version_id, &run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });

    let register_completion = pop_register_completion(&mut register_queue);
    assert_eq!(register_completion.request_id, 11);
    assert!(register_completion.result.is_ok());
    let first_completion = pop_unregister_completion(&mut first_unregister_queue);
    assert_eq!(first_completion.request_id, 21);
    assert_eq!(
        first_completion.document_owner,
        crate::native_bridge::WindowDocumentOwner::for_test(1)
    );
    assert!(first_completion.result);
    let second_completion = pop_unregister_completion(&mut second_unregister_queue);
    assert_eq!(second_completion.request_id, 22);
    assert_eq!(
        second_completion.document_owner,
        crate::native_bridge::WindowDocumentOwner::for_test(2)
    );
    assert!(second_completion.result);

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 0);
    assert_eq!(diagnostics.version_count, 0);
    assert_eq!(diagnostics.queued_unregistration_job_count, 0);
}
#[test]
fn queued_unregister_preserves_fifo_before_later_register() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let first_version_id = ServiceWorkerVersionId(1);
    let scope_url = url("https://example.test/app/");
    let first_script_url = url("https://example.test/app/worker-v1.js");
    let second_script_url = url("https://example.test/app/worker-v2.js");
    let document_url = url("https://example.test/app/page.html");
    let browser_context_runtime = service.browser_context_runtime();
    let mut first_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let second_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    insert_starting_version_with_register_job(
        &service,
        registration_id,
        first_version_id,
        first_script_url.clone(),
        scope_url.clone(),
        11,
        first_queue.sender(),
    );

    assert!(service.mark_registration_unregistered(&scope_url));
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
    {
        let mut state = service.inner.state.lock();
        state.pending_ready_jobs.push(ServiceWorkerReadyJob {
            request_id: 33,
            document_owner: crate::native_bridge::WindowDocumentOwner::for_test(1),
            completion_tx: test_completion_sender(),
            registration_id,
        });
        state
            .notification_records
            .push(ServiceWorkerNotificationRecord {
                id: 44,
                registration_id,
                title: "old notification".to_owned(),
                tag: "old".to_owned(),
                metadata: ServiceWorkerNotificationMetadata::default(),
                actions: Vec::new(),
                data: V8StructuredClonePayload::default(),
            });
        state.sync_registrations.insert(
            (registration_id, "old-sync".to_owned()),
            ServiceWorkerSyncRegistrationRecord::default(),
        );
        state.periodic_sync_registrations.insert(
            (registration_id, "old-periodic".to_owned()),
            ServiceWorkerPeriodicSyncRegistrationRecord::new(10),
        );
        state.push_subscriptions.insert(
            registration_id,
            service_worker_push_subscription_snapshot(registration_id, true),
        );
    }

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.queued_unregistration_job_count, 1);
    assert_eq!(diagnostics.queued_register_job_count, 1);
    assert_eq!(diagnostics.pending_unregistration_count, 0);

    let first_run = exact_version_run(&service, first_version_id);
    service.finish_worker_start_completed(
        first_version_id,
        first_run.clone(),
        first_script_url.to_string(),
    );
    assert!(!first_queue.has_ready_task());
    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(first_version_id, &first_run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });

    let first_completion = pop_register_completion(&mut first_queue);
    assert_eq!(first_completion.request_id, 11);
    assert!(first_completion.result.is_ok());
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.pending_unregistration_count, 0);
    assert_eq!(diagnostics.queued_unregistration_job_count, 0);
    assert_eq!(diagnostics.queued_register_job_count, 0);
    assert_eq!(diagnostics.starting_version_count, 1);
    assert_eq!(
        service
            .matching_registration_for_client(&document_url)
            .expect("later register should recreate visible registration")
            .installing()
            .expect("later register should start installing")
            .script_url(),
        &second_script_url
    );
    {
        let state = service.inner.state.lock();
        assert!(
            state
                .pending_ready_jobs
                .iter()
                .all(|job| job.registration_id != registration_id)
        );
        assert!(
            state
                .notification_records
                .iter()
                .all(|record| record.registration_id != registration_id)
        );
        assert!(
            state
                .sync_registrations
                .keys()
                .all(|(record_registration_id, _)| *record_registration_id != registration_id)
        );
        assert!(
            state
                .periodic_sync_registrations
                .keys()
                .all(|(record_registration_id, _)| *record_registration_id != registration_id)
        );
        assert!(!state.push_subscriptions.contains_key(&registration_id));
    }

    service.terminate_all_for_context_shutdown();
}
#[test]
fn queued_register_coalesces_with_last_identical_job() {
    let service = new_service_worker_runtime_service();
    let browser_context_runtime = service.browser_context_runtime();
    let document_url = url("https://example.test/app/page.html");
    let scope_url = url("https://example.test/app/");
    let first_script_url = url("https://example.test/app/worker-v1.js");
    let second_script_url = url("https://example.test/app/worker-v2.js");
    let mut first_completion_queue =
        crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let mut second_completion_queue =
        crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let mut third_completion_queue =
        crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

    service.start_registration(
        first_script_url.clone(),
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
        second_script_url.clone(),
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
        2,
        1,
        second_completion_queue.sender(),
    );
    service.start_registration(
        second_script_url.clone(),
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
        3,
        1,
        third_completion_queue.sender(),
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.queued_register_job_count, 1);

    let first_version_id = ServiceWorkerVersionId(1);
    let first_run = exact_version_run(&service, first_version_id);
    service.finish_worker_start_completed(
        first_version_id,
        first_run.clone(),
        first_script_url.to_string(),
    );
    assert!(!first_completion_queue.has_ready_task());
    assert!(!second_completion_queue.has_ready_task());
    assert!(!third_completion_queue.has_ready_task());

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(first_version_id, &first_run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });

    assert!(first_completion_queue.has_ready_task());
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.version_count, 2);
    assert_eq!(diagnostics.queued_register_job_count, 0);
    assert_eq!(diagnostics.starting_version_count, 1);

    let second_version_id = ServiceWorkerVersionId(2);
    let second_run = exact_version_run(&service, second_version_id);
    service.finish_worker_start_completed(
        second_version_id,
        second_run.clone(),
        second_script_url.to_string(),
    );
    assert!(!second_completion_queue.has_ready_task());
    assert!(!third_completion_queue.has_ready_task());
    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(2),
        owner: test_run_owner(second_version_id, &second_run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });
    assert!(second_completion_queue.has_ready_task());
    assert!(third_completion_queue.has_ready_task());

    service.terminate_all_for_context_shutdown();
}
#[test]
fn start_failure_starts_next_queued_register_job() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let first_version_id = ServiceWorkerVersionId(1);
    let scope_url = url("https://example.test/app/");
    let mut first_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let second_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

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

    let first_run = exact_version_run(&service, first_version_id);
    service.finish_worker_start_failed(
        first_version_id,
        first_run,
        ServiceWorkerVersionStartFailure::ScriptLoad {
            message: "first worker load failed".to_owned(),
        },
    );

    assert!(first_queue.has_ready_task());
    let first_completion = pop_register_completion(&mut first_queue);
    assert_eq!(first_completion.request_id, 11);
    assert!(first_completion.result.is_err());

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.queued_register_job_count, 0);
    assert_eq!(diagnostics.starting_version_count, 1);
    let snapshot = snapshot_for_registration(&service, registration_id);
    assert_eq!(
        snapshot
            .installing()
            .expect("queued version should start after failure")
            .script_url(),
        &url("https://example.test/app/worker-v2.js")
    );

    service.terminate_all_for_context_shutdown();
}
#[test]
fn same_options_register_uses_waiting_newest_worker_for_fast_path() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let waiting_version_id = ServiceWorkerVersionId(2);
    let scope_url = url("https://example.test/app/");
    let active_script_url = url("https://example.test/app/worker-v1.js");
    let waiting_script_url = url("https://example.test/app/worker-v2.js");
    let document_url = url("https://example.test/app/page.html");
    insert_registered_version(
        &service,
        registration_id,
        active_version_id,
        active_script_url,
        scope_url.clone(),
        [],
    );
    {
        let mut state = service.inner.state.lock();
        let registration = state
            .registrations
            .get_mut(&registration_id)
            .expect("registration should exist");
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
    }
    let browser_context_runtime = service.browser_context_runtime();
    let mut completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

    service.start_registration(
        waiting_script_url.clone(),
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
        43,
        1,
        completion_queue.sender(),
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.version_count, 2);
    assert_eq!(diagnostics.starting_version_count, 0);
    assert_eq!(diagnostics.queued_register_job_count, 0);

    let completion = pop_register_completion(&mut completion_queue);
    assert_eq!(completion.request_id, 43);
    let snapshot = completion
        .result
        .expect("same-options register should resolve existing registration");
    assert_eq!(snapshot.registration_id(), registration_id);
    assert_eq!(
        snapshot
            .active()
            .expect("active version should remain visible")
            .version_id(),
        active_version_id
    );
    assert_eq!(
        snapshot
            .waiting()
            .expect("waiting newest version should remain visible")
            .version_id(),
        waiting_version_id
    );

    service.terminate_all_for_context_shutdown();
}
#[test]
fn queued_register_does_not_coalesce_different_update_via_cache() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let first_version_id = ServiceWorkerVersionId(1);
    let scope_url = url("https://example.test/app/");
    let first_script_url = url("https://example.test/app/worker-v1.js");
    let second_script_url = url("https://example.test/app/worker-v2.js");
    let document_url = url("https://example.test/app/page.html");
    let browser_context_runtime = service.browser_context_runtime();
    let first_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let mut second_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let mut third_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    insert_starting_version_with_register_job(
        &service,
        registration_id,
        first_version_id,
        first_script_url,
        scope_url.clone(),
        11,
        first_queue.sender(),
    );

    service.start_registration(
        second_script_url.clone(),
        scope_url.clone(),
        document_url.clone(),
        WorkerScriptKind::Classic,
        test_request_client(&service),
        WorkerNetworkPolicy::default(),
        browser_context_runtime.clone(),
        None,
        None,
        None,
        ServiceWorkerUpdateViaCache::All,
        22,
        1,
        second_queue.sender(),
    );
    service.start_registration(
        second_script_url,
        scope_url,
        document_url,
        WorkerScriptKind::Classic,
        test_request_client(&service),
        WorkerNetworkPolicy::default(),
        browser_context_runtime,
        None,
        None,
        None,
        ServiceWorkerUpdateViaCache::None,
        33,
        1,
        third_queue.sender(),
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.queued_register_job_count, 2);
    assert!(!second_queue.has_ready_task());
    assert!(!third_queue.has_ready_task());

    service.terminate_all_for_context_shutdown();
}
#[test]
fn stored_registration_resources_use_serialized_storage_key() {
    let resource_store = new_shared_service_worker_resource_store();
    let service = new_service_worker_runtime_service_with_resource_store(
        resource_store.clone(),
        test_worker_context_runtime(),
    );
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
        [],
    );
    {
        let mut state = service.inner.state.lock();
        state
            .versions
            .get_mut(&version_id)
            .expect("version should exist")
            .main_script_resource = Some(test_script_resource(&script_url));
    }

    assert!(
        service
            .store_registration_resources_for_test(registration_id, version_id)
            .expect("stored registration resources should persist")
    );
    let registrations = resource_store.lock().registrations();
    assert_eq!(registrations.len(), 1);
    assert_eq!(
        registrations[0].storage_key,
        "storage-key:v1;origin=https://example.test;top-level-site=https://example.test"
    );
    assert_ne!(registrations[0].storage_key, "https://example.test");
    assert_eq!(
        registrations[0].storage_key,
        ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url)
    );
}
#[test]
fn devtools_unregister_scope_uses_owner_unregistration_path() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
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
    {
        let mut state = service.inner.state.lock();
        state.record_target_created(registration_id, version_id, script_url, scope_url.clone());
        service.take_target_output_events_for_test();
    }

    assert_eq!(
        service.devtools_unregister_scope(&scope_url),
        Ok(true),
        "DevTools unregister should accept an existing registration"
    );

    let state = service.inner.state.lock();
    assert!(!state.registrations.contains_key(&registration_id));
    assert!(!state.versions.contains_key(&version_id));
    drop(state);
    let target_events = service.take_target_output_events_for_test();
    assert!(
        target_events.iter().any(|event| matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::Destroyed {
                version_id: destroyed_version_id,
                active_run: None,
            } if *destroyed_version_id == version_id.as_u64()
        )),
        "DevTools unregister should destroy the runtime-owned target: {target_events:?}"
    );
}
#[test]
fn devtools_update_registration_enters_existing_registration_job_queue() {
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

    assert_eq!(
        service
            .devtools_update_registration_for_scope(&scope_url, service.browser_context_runtime()),
        Ok(true),
        "updateRegistration should enqueue through the runtime-owned job coordinator"
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.pending_main_script_update_check_count, 1);
    assert_eq!(diagnostics.queued_register_job_count, 1);
    assert!(!completion_queue.has_ready_task());
}
#[test]
fn devtools_functional_event_dispatch_uses_active_registration_owner() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let origin = url("https://example.test/");
    let other_origin = url("https://other.test/");
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/worker.js");
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url,
        scope_url,
        [],
    );
    {
        let mut state = service.inner.state.lock();
        let version = state.versions.get_mut(&version_id).unwrap();
        version.run = run.clone();
        version.running_state = ServiceWorkerVersionRunningState::Starting {
            host: new_loading_test_host(version_id, &run),
        };
    }

    assert_eq!(
        service.devtools_deliver_push_message(
            &other_origin,
            registration_id,
            Some(b"ignored".to_vec())
        ),
        Ok(false),
        "DevTools push dispatch should enforce the registration storage key"
    );
    assert_eq!(
        service.devtools_deliver_push_message(&origin, registration_id, Some(b"payload".to_vec())),
        Ok(true)
    );
    assert_eq!(
        service.devtools_dispatch_sync_event(&origin, registration_id, "sync-tag".to_owned(), true),
        Ok(true)
    );
    assert_eq!(
        service.devtools_dispatch_periodic_sync_event(
            &origin,
            registration_id,
            "periodic-tag".to_owned()
        ),
        Ok(true)
    );

    let state = service.inner.state.lock();
    let version = state.versions.get(&version_id).unwrap();
    assert_eq!(version.in_flight_event_count, 3);
    assert_eq!(version.pending_start_events.len(), 3);
    let ServiceWorkerPendingStartEvent::Push(push) = &version.pending_start_events[0] else {
        panic!("expected pending push event");
    };
    assert_eq!(push.owner, test_run_owner(version_id, &run));
    assert_eq!(push.data.as_deref(), Some(&b"payload"[..]));
    let ServiceWorkerPendingStartEvent::Sync(sync) = &version.pending_start_events[1] else {
        panic!("expected pending sync event");
    };
    assert_eq!(sync.registration_id, registration_id);
    assert_eq!(sync.owner, test_run_owner(version_id, &run));
    assert_eq!(sync.tag, "sync-tag");
    assert!(sync.last_chance);
    let ServiceWorkerPendingStartEvent::PeriodicSync(periodic) = &version.pending_start_events[2]
    else {
        panic!("expected pending periodic sync event");
    };
    assert_eq!(periodic.registration_id, registration_id);
    assert_eq!(periodic.owner, test_run_owner(version_id, &run));
    assert_eq!(periodic.tag, "periodic-tag");
}
