use super::*;

#[test]
fn worker_start_completion_records_main_script_resource_metadata() {
    let service = new_service_worker_runtime_service();
    let (script_url, _, _, version_id) = insert_starting_version(&service);

    let run = exact_version_run(&service, version_id);
    service.finish_worker_start_completed_with_script_resource(
        version_id,
        run,
        script_url.to_string(),
        Some(test_script_resource(&script_url)),
        ServiceWorkerFetchHandlerType::NotSkippable,
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.running_version_count, 1);
    assert_eq!(
        diagnostics.versions[0].final_script_url.as_deref(),
        Some(script_url.as_str())
    );
    assert_eq!(diagnostics.versions[0].main_script_status, Some(200));
    assert_eq!(diagnostics.versions[0].main_script_body_len, Some(3));
    assert_eq!(
        diagnostics.versions[0].main_script_body_sha256.as_deref(),
        Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    );
    assert_eq!(
        diagnostics.versions[0].main_script_mime_type.as_deref(),
        Some("text/javascript")
    );
    assert!(
        diagnostics.registrations[0]
            .last_update_check_time_ms
            .is_some(),
        "installing version script load should initialize last update check time"
    );
}
#[test]
fn imported_script_loaded_records_resource_metadata() {
    let service = new_service_worker_runtime_service();
    let (_, _, registration_id, version_id) = insert_starting_version(&service);
    let import_url = url("https://example.test/app/dep.js");

    let run = exact_version_run(&service, version_id);
    service.finish_imported_script_loaded(
        registration_id,
        version_id,
        RendererServiceWorkerRunIdentity::fresh(),
        test_worker_script_resource(&import_url),
    );
    assert_eq!(
        service.diagnostics_snapshot().versions[0].imported_script_count,
        0
    );

    service.finish_imported_script_loaded(
        registration_id,
        version_id,
        run,
        test_worker_script_resource(&import_url),
    );

    let diagnostics = service.diagnostics_snapshot();
    let version = &diagnostics.versions[0];
    assert_eq!(version.imported_script_count, 1);
    assert_eq!(version.imported_scripts.len(), 1);
    let imported = &version.imported_scripts[0];
    assert_eq!(imported.request_url, import_url.as_str());
    assert_eq!(imported.final_url, import_url.as_str());
    assert_eq!(imported.status, 200);
    assert_eq!(imported.body_len, 3);
    assert_eq!(
        imported.body_sha256,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(imported.mime_type.as_deref(), Some("text/javascript"));
}
#[test]
fn resource_store_restores_imported_resources_for_update_check() {
    let resource_store = new_shared_service_worker_resource_store();
    let first_service = new_service_worker_runtime_service_with_resource_store(
        resource_store.clone(),
        test_worker_context_runtime(),
    );
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let script_url = url("https://example.test/app/worker.js");
    let dep_url = url("https://example.test/app/dep.js");
    let scope_url = url("https://example.test/app/");
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
        state
            .registrations
            .get_mut(&registration_id)
            .expect("registration should exist")
            .last_update_check_time_ms = Some(5);
        let version = state
            .versions
            .get_mut(&version_id)
            .expect("version should exist");
        version.main_script_resource = Some(test_script_resource(&script_url));
        version
            .imported_script_resources
            .insert(dep_url.to_string(), test_script_resource(&dep_url));
    }
    assert!(
        first_service
            .store_registration_resources_for_test(registration_id, version_id)
            .expect("stored registration resources should persist")
    );
    assert_eq!(resource_store.lock().registration_count(), 1);

    let second_service = new_service_worker_runtime_service_with_resource_store(
        resource_store.clone(),
        test_worker_context_runtime(),
    );
    let queued_job = test_queued_register_job(&second_service, script_url.clone(), scope_url);
    let (restored_registration_id, update_check_params) = {
        let mut state = second_service.inner.state.lock();
        let restored_registration_id = second_service
            .restore_stored_registration_for_queued_job_locked(&mut state, &queued_job)
            .expect("stored registration should restore");
        let update_check_params = match second_service
            .start_main_script_update_check_locked(&mut state, restored_registration_id, queued_job)
            .expect("restored active version should start update check")
        {
            ServiceWorkerMainScriptUpdateCheckStart::Start(update_check) => {
                let (_, update_check_params) = *update_check;
                update_check_params
            }
            ServiceWorkerMainScriptUpdateCheckStart::WaitForDebugger => {
                panic!("update check should not wait for debugger by default")
            }
        };
        (restored_registration_id, update_check_params)
    };

    assert_eq!(
        update_check_params.newest_main_body_sha256,
        test_script_resource(&script_url).body_sha256
    );
    assert_eq!(update_check_params.imported_scripts.len(), 1);
    let imported_script = &update_check_params.imported_scripts[0];
    assert_eq!(imported_script.request_url, dep_url);
    assert_eq!(imported_script.final_url, dep_url);
    assert_eq!(
        imported_script.body_sha256,
        test_script_resource(&dep_url).body_sha256
    );

    let diagnostics = second_service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 2);
    assert_eq!(diagnostics.installing_version_count, 1);
    assert_eq!(diagnostics.starting_version_count, 1);
    assert_eq!(diagnostics.pending_main_script_update_check_count, 1);
    assert_eq!(
        diagnostics.registrations[0].last_update_check_time_ms,
        Some(5)
    );
    assert_eq!(
        diagnostics.registrations[0].active_version_id,
        Some(ServiceWorkerVersionId(1))
    );
    assert_eq!(diagnostics.registrations[0].id, restored_registration_id);
}
#[test]
fn devtools_pause_on_start_defers_main_script_update_check_after_target_creation() {
    let service = new_service_worker_runtime_service_with_resource_store(
        new_shared_service_worker_resource_store(),
        test_worker_context_runtime(),
    );
    service.set_pause_new_workers_on_start_for_devtools(true);
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
    let queued_job = test_queued_register_job(&service, script_url, scope_url);

    let start = {
        let mut state = service.inner.state.lock();
        service
            .start_main_script_update_check_locked(&mut state, registration_id, queued_job)
            .expect("active version should be eligible for update check")
    };

    assert!(matches!(
        start,
        ServiceWorkerMainScriptUpdateCheckStart::WaitForDebugger
    ));
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.pending_main_script_update_check_count, 1);
    assert_eq!(diagnostics.installing_version_count, 1);
    assert_eq!(diagnostics.starting_version_count, 1);
    let target_events = service.take_target_output_events_for_test();
    assert!(
        target_events.iter().any(|event| matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::Created { info, .. }
                if info.version_id != active_version_id.as_u64()
        )),
        "deferred update check should still expose the precreated installing target: {target_events:?}"
    );
}
#[test]
fn devtools_related_pause_on_start_defers_matching_update_check() {
    let service = new_service_worker_runtime_service_with_resource_store(
        new_shared_service_worker_resource_store(),
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
    service.set_related_pause_on_start_policies_for_devtools(vec![(
        registration_id.as_u64(),
        active_version_id.as_u64(),
        script_url.to_string(),
        scope_url.to_string(),
    )]);
    let queued_job = test_queued_register_job(&service, script_url, scope_url);

    let start = {
        let mut state = service.inner.state.lock();
        service
            .start_main_script_update_check_locked(&mut state, registration_id, queued_job)
            .expect("active version should be eligible for update check")
    };

    assert!(matches!(
        start,
        ServiceWorkerMainScriptUpdateCheckStart::WaitForDebugger
    ));
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.pending_main_script_update_check_count, 1);
    assert_eq!(diagnostics.installing_version_count, 1);
    let state = service.inner.state.lock();
    assert!(
        state.versions.iter().any(|(version_id, version)| {
            *version_id != active_version_id && version.should_pause_on_start_for_devtools
        }),
        "matching related policy should set the sticky version pause flag"
    );
}
#[test]
fn forced_update_check_uses_validate_cache_and_skips_script_comparison() {
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
        let registration = state.registrations.get_mut(&registration_id).unwrap();
        registration.update_via_cache = ServiceWorkerUpdateViaCache::All;
        let version = state
            .versions
            .get_mut(&active_version_id)
            .expect("active version should exist");
        version.main_script_resource = Some(test_script_resource(&script_url));
    }
    let mut queued_job = test_queued_register_job(&service, script_url, scope_url);
    queued_job.update_via_cache = ServiceWorkerUpdateViaCache::All;
    queued_job.force_bypass_cache = true;
    queued_job.skip_script_comparison = true;
    queued_job.skip_waiting_after_install = true;

    let update_check_params = {
        let mut state = service.inner.state.lock();
        match service
            .start_main_script_update_check_locked(&mut state, registration_id, queued_job)
            .expect("force update should start update check")
        {
            ServiceWorkerMainScriptUpdateCheckStart::Start(update_check) => {
                let (_, update_check_params) = *update_check;
                update_check_params
            }
            ServiceWorkerMainScriptUpdateCheckStart::WaitForDebugger => {
                panic!("force update should not wait for debugger by default")
            }
        }
    };

    assert_eq!(
        update_check_params.main_script.cache_mode,
        moli_fetch::RequestCacheMode::Validate
    );
    assert_eq!(
        update_check_params.imported_script_cache_mode,
        moli_fetch::RequestCacheMode::Validate
    );
    assert!(update_check_params.skip_script_comparison);
}
#[test]
fn resource_store_lazily_restores_registration_added_after_startup() {
    let resource_store = new_shared_service_worker_resource_store();
    let observer_service = new_service_worker_runtime_service_with_resource_store(
        resource_store.clone(),
        test_worker_context_runtime(),
    );
    assert_eq!(
        observer_service.diagnostics_snapshot().registration_count,
        0
    );
    {
        let state = observer_service.inner.state.lock();
        assert_eq!(state.stored_registration_cache_revision, Some(0));
        assert_eq!(state.stored_registration_cache.len(), 0);
    }

    let writer_service = new_service_worker_runtime_service_with_resource_store(
        resource_store.clone(),
        test_worker_context_runtime(),
    );
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let script_url = url("https://example.test/app/worker.js");
    let scope_url = url("https://example.test/app/");
    let document_url = url("https://example.test/app/page.html");
    insert_registered_version(
        &writer_service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url.clone(),
        [],
    );
    {
        let mut state = writer_service.inner.state.lock();
        state
            .versions
            .get_mut(&version_id)
            .expect("version should exist")
            .main_script_resource = Some(test_script_resource(&script_url));
    }
    assert!(
        writer_service
            .store_registration_resources_for_test(registration_id, version_id)
            .expect("stored registration resources should persist")
    );
    let store_revision_after_write = resource_store.lock().revision();
    assert_eq!(store_revision_after_write, 1);
    assert_eq!(
        observer_service.diagnostics_snapshot().registration_count,
        0
    );
    {
        let state = observer_service.inner.state.lock();
        assert_eq!(state.stored_registration_cache_revision, Some(0));
        assert_eq!(state.stored_registration_cache.len(), 0);
    }

    let restored = observer_service
        .matching_registration_for_client(&document_url)
        .expect("matching client query should lazily restore later stored registration");
    assert_eq!(restored.scope_url(), &scope_url);
    assert_eq!(
        observer_service.diagnostics_snapshot().registration_count,
        1
    );
    {
        let state = observer_service.inner.state.lock();
        assert_eq!(
            state.stored_registration_cache_revision,
            Some(store_revision_after_write)
        );
        assert_eq!(state.stored_registration_cache.len(), 1);
    }
    assert!(
        observer_service
            .matching_registration_for_client(&document_url)
            .is_some()
    );
    {
        let state = observer_service.inner.state.lock();
        assert_eq!(
            state.stored_registration_cache_revision,
            Some(store_revision_after_write)
        );
        assert_eq!(state.stored_registration_cache.len(), 1);
    }
}
#[test]
fn resource_store_deletes_registration_on_unregistration_clear() {
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
    assert_eq!(resource_store.lock().registration_count(), 1);

    assert!(service.mark_registration_unregistered(&scope_url));

    assert_eq!(resource_store.lock().registration_count(), 0);
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 0);
}
#[test]
fn resource_store_failure_starts_next_queued_register_job() {
    let failing_store = FailingJsonStorePath::new("install-queued");
    let resource_store =
        crate::new_shared_json_service_worker_resource_store(failing_store.store_path())
            .expect("failing json resource store should open before first write");
    let service = new_service_worker_runtime_service_with_resource_store(
        resource_store,
        test_worker_context_runtime(),
    );
    let registration_id = ServiceWorkerRegistrationId(1);
    let first_version_id = ServiceWorkerVersionId(1);
    let scope_url = url("https://example.test/app/");
    let first_script_url = url("https://example.test/app/worker-v1.js");
    let second_script_url = url("https://example.test/app/worker-v2.js");
    let mut first_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let mut second_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

    insert_starting_version_with_register_job(
        &service,
        registration_id,
        first_version_id,
        first_script_url.clone(),
        scope_url.clone(),
        11,
        first_queue.sender(),
    );
    push_queued_register_job(
        &service,
        registration_id,
        second_script_url.clone(),
        scope_url,
        22,
        second_queue.sender(),
    );
    {
        let mut state = service.inner.state.lock();
        state
            .versions
            .get_mut(&first_version_id)
            .expect("first installing version should exist")
            .main_script_resource = Some(test_script_resource(&first_script_url));
    }
    let first_run = exact_version_run(&service, first_version_id);

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(first_version_id, &first_run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });

    let first_completion = pop_register_completion(&mut first_queue);
    assert_eq!(first_completion.request_id, 11);
    let error = first_completion
        .result
        .expect_err("store failure should reject the first register job");
    assert_eq!(
        error.kind,
        crate::service_worker_runtime::ServiceWorkerRegistrationErrorKind::Abort
    );
    assert!(!second_queue.has_ready_task());

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.queued_register_job_count, 0);
    assert_eq!(diagnostics.starting_version_count, 1);
    let snapshot = snapshot_for_registration(&service, registration_id);
    assert_eq!(
        snapshot
            .installing()
            .expect("queued version should start after failed store")
            .script_url(),
        &second_script_url
    );

    service.terminate_all_for_context_shutdown();
}
#[test]
fn resource_store_failure_preserves_existing_active_registration() {
    let failing_store = FailingJsonStorePath::new("install-update");
    let resource_store =
        crate::new_shared_json_service_worker_resource_store(failing_store.store_path())
            .expect("failing json resource store should open before first write");
    let service = new_service_worker_runtime_service_with_resource_store(
        resource_store,
        test_worker_context_runtime(),
    );
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let installing_version_id = ServiceWorkerVersionId(2);
    let old_script_url = url("https://example.test/app/worker-v1.js");
    let new_script_url = url("https://example.test/app/worker-v2.js");
    let scope_url = url("https://example.test/app/");
    let document_url = url("https://example.test/app/page.html");
    let active_state = insert_registered_version(
        &service,
        registration_id,
        active_version_id,
        old_script_url.clone(),
        scope_url.clone(),
        [document_url.clone()],
    );
    let mut completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    insert_starting_version_with_register_job(
        &service,
        registration_id,
        installing_version_id,
        new_script_url.clone(),
        scope_url,
        42,
        completion_queue.sender(),
    );
    {
        let mut state = service.inner.state.lock();
        state
            .versions
            .get_mut(&installing_version_id)
            .expect("installing update version should exist")
            .main_script_resource = Some(test_script_resource(&new_script_url));
    }
    let installing_run = exact_version_run(&service, installing_version_id);

    service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
        event_id: ServiceWorkerEventId(1),
        owner: test_run_owner(installing_version_id, &installing_run),
        kind: ServiceWorkerLifecycleEventKind::Install,
        result: Ok(()),
    });

    let completion = pop_register_completion(&mut completion_queue);
    assert_eq!(completion.request_id, 42);
    assert!(
        completion
            .result
            .expect_err("store failure should reject installing update")
            .message
            .contains("failed to store Service Worker registration resources")
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.installing_version_count, 0);
    assert_eq!(diagnostics.redundant_version_count, 0);
    assert_eq!(
        diagnostics.registrations[0].active_version_id,
        Some(active_version_id)
    );
    assert_eq!(diagnostics.registrations[0].waiting_version_id, None);
    assert_eq!(
        service.matching_controller_for_document(&document_url),
        Some(active_state)
    );
    let state = service.inner.state.lock();
    assert!(state.versions.contains_key(&active_version_id));
    assert!(!state.versions.contains_key(&installing_version_id));
}
#[test]
fn failed_main_script_update_check_records_diagnostics_and_rejects_register_job() {
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
        42,
        completion_queue.sender(),
    );

    service.finish_main_script_update_check_completed(
        registration_id,
        Err(ServiceWorkerScriptUpdateCheckFailure::script_load(
            "script fetch failed".to_owned(),
        )),
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.installing_version_count, 0);
    assert_eq!(diagnostics.pending_main_script_update_check_count, 0);
    assert_eq!(
        diagnostics.registrations[0].active_version_id,
        Some(active_version_id)
    );
    let update_check = diagnostics.registrations[0]
        .last_main_script_update_check
        .as_ref()
        .expect("failed update check should be diagnosed");
    assert_eq!(update_check.result, "failed");
    assert_eq!(update_check.failure_status, Some("script-load-failed"));
    assert_eq!(update_check.message.as_deref(), Some("script fetch failed"));
    assert_eq!(diagnostics.registrations[0].last_update_check_time_ms, None);

    let completion = pop_register_completion(&mut completion_queue);
    assert_eq!(completion.request_id, 42);
    let error = completion
        .result
        .expect_err("script load update check should reject");
    assert_eq!(
        error.kind,
        crate::service_worker_runtime::ServiceWorkerRegistrationErrorKind::Type
    );
    assert_eq!(error.message, "script fetch failed");
}
#[test]
fn stale_main_script_update_check_records_diagnostics_and_rejects_register_job() {
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
        let resource = state
            .versions
            .get_mut(&active_version_id)
            .expect("active version should exist")
            .main_script_resource
            .get_or_insert_with(|| test_script_resource(&script_url));
        resource.body_sha256 = "changed-after-check-started".to_owned();
    }
    let mut completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    insert_pending_main_script_update_check(
        &service,
        registration_id,
        active_version_id,
        script_url.clone(),
        scope_url,
        42,
        completion_queue.sender(),
    );

    service.finish_main_script_update_check_completed(
        registration_id,
        Ok(test_update_check_result(&script_url, "abc", false)),
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.installing_version_count, 0);
    assert_eq!(diagnostics.pending_main_script_update_check_count, 0);
    let update_check = diagnostics.registrations[0]
        .last_main_script_update_check
        .as_ref()
        .expect("stale update check should be diagnosed");
    assert_eq!(update_check.result, "stale");
    assert_eq!(update_check.failure_status, Some("stale"));
    assert_eq!(
        update_check.message.as_deref(),
        Some("service worker main script update check became stale")
    );
    assert_eq!(diagnostics.registrations[0].last_update_check_time_ms, None);

    let completion = pop_register_completion(&mut completion_queue);
    let error = completion
        .result
        .expect_err("stale update check should reject");
    assert_eq!(
        error.kind,
        crate::service_worker_runtime::ServiceWorkerRegistrationErrorKind::Abort
    );
    assert_eq!(
        error.message,
        "service worker main script update check became stale"
    );
}
#[test]
fn identical_main_script_update_resolves_existing_registration_without_starting_worker() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let installing_version_id = ServiceWorkerVersionId(2);
    let script_url = url("https://example.test/app/worker.js");
    let scope_url = url("https://example.test/app/");
    let document_url = url("https://example.test/app/page.html");
    let active_state = insert_registered_version(
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
            .versions
            .get_mut(&active_version_id)
            .expect("active version should exist")
            .main_script_resource = Some(test_script_resource(&script_url));
    }
    let mut completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    insert_starting_version_with_register_job(
        &service,
        registration_id,
        installing_version_id,
        script_url.clone(),
        scope_url,
        42,
        completion_queue.sender(),
    );
    let installing_run = exact_version_run(&service, installing_version_id);

    let mut different_resource = test_script_resource(&script_url);
    different_resource.body_sha256 = "different".to_owned();
    assert!(!service.finish_worker_start_identical_script_update(
        installing_version_id,
        installing_run.clone(),
        &different_resource
    ));
    {
        let mut state = service.inner.state.lock();
        state
            .versions
            .get_mut(&installing_version_id)
            .expect("installing version should exist")
            .allow_identical_script_update = false;
    }
    assert!(!service.finish_worker_start_identical_script_update(
        installing_version_id,
        installing_run.clone(),
        &test_script_resource(&script_url)
    ));
    {
        let mut state = service.inner.state.lock();
        state
            .versions
            .get_mut(&installing_version_id)
            .expect("installing version should exist")
            .allow_identical_script_update = true;
    }

    assert!(service.finish_worker_start_identical_script_update(
        installing_version_id,
        installing_run,
        &test_script_resource(&script_url)
    ));

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.starting_version_count, 0);
    assert_eq!(diagnostics.installing_version_count, 0);
    assert_eq!(diagnostics.activated_version_count, 1);

    let snapshot = snapshot_for_registration(&service, registration_id);
    assert!(snapshot.installing().is_none());
    assert!(snapshot.waiting().is_none());
    assert_eq!(
        snapshot
            .active()
            .expect("active version should remain")
            .version_id(),
        active_version_id
    );
    assert_eq!(
        service.matching_controller_for_document(&document_url),
        Some(active_state)
    );

    let completion = pop_register_completion(&mut completion_queue);
    assert_eq!(completion.request_id, 42);
    let completion_snapshot = completion
        .result
        .expect("identical main script update should resolve");
    assert!(completion_snapshot.installing().is_none());
    assert_eq!(
        completion_snapshot
            .active()
            .expect("completion should expose active version")
            .version_id(),
        active_version_id
    );
}

#[test]
fn script_resource_map_snapshot_preserves_aliases_and_requires_exact_run() {
    let service = new_service_worker_runtime_service();
    let (_, _, registration_id, version_id) = insert_starting_version(&service);
    let run = exact_version_run(&service, version_id);
    let owner = ServiceWorkerRunOwner::new(version_id, run.clone());
    for name in ["a.js", "b.js"] {
        let mut resource =
            test_worker_script_resource(&url(&format!("https://example.test/{name}")));
        resource.final_url = url("https://example.test/common.js");
        resource.classic_script = Some(crate::worker::WorkerStoredClassicScript {
            source: format!("self.name = '{name}';").into(),
            muted_errors: false,
            redirect_urls: vec![resource.final_url.clone()],
        });
        service.finish_imported_script_loaded(
            registration_id,
            version_id,
            run.clone(),
            resource,
        );
    }
    let snapshot = service.script_resource_map_snapshot(&owner).unwrap();
    assert!(snapshot.can_import_new_scripts);
    assert_eq!(snapshot.imported_scripts.len(), 2);
    assert_eq!(
        snapshot.imported_scripts[0].final_url,
        snapshot.imported_scripts[1].final_url
    );
    assert_ne!(
        snapshot.imported_scripts[0].request_url,
        snapshot.imported_scripts[1].request_url
    );
    assert_ne!(
        snapshot.imported_scripts[0].classic_script,
        snapshot.imported_scripts[1].classic_script
    );
    assert!(
        service
            .script_resource_map_snapshot(&ServiceWorkerRunOwner::new(
                version_id,
                RendererServiceWorkerRunIdentity::fresh(),
            ))
            .is_none()
    );
    service
        .inner
        .state
        .lock()
        .versions
        .get_mut(&version_id)
        .unwrap()
        .lifecycle_state = ServiceWorkerVersionLifecycleState::Activated;
    assert!(
        !service
            .script_resource_map_snapshot(&owner)
            .unwrap()
            .can_import_new_scripts
    );
}
