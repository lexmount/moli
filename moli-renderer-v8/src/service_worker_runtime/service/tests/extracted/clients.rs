use super::*;

#[test]
fn active_registration_does_not_control_existing_live_client_until_claimed() {
    let service = new_service_worker_runtime_service();
    let app_page = url("https://example.test/app/page.html");
    let app_client_id = register_client_for_test(&service, app_page.clone());
    let state = insert_registered_version(
        &service,
        ServiceWorkerRegistrationId(1),
        ServiceWorkerVersionId(1),
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );

    assert_eq!(service.matching_controller_for_document(&app_page), None);
    assert_eq!(
        service.matching_controller_for_fetch(&app_page, &url("https://other.test/api.json")),
        None
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.live_client_count, 1);
    assert_eq!(diagnostics.controlled_client_count, 0);

    service.finish_worker_clients_claim_requested(
        ServiceWorkerRegistrationId(1),
        ServiceWorkerVersionId(1),
    );
    assert_eq!(
        service.matching_controller_for_client(app_client_id),
        Some(state)
    );
}
#[test]
fn new_client_selects_longest_active_controller_on_registration() {
    let service = new_service_worker_runtime_service();
    let root = insert_registered_version(
        &service,
        ServiceWorkerRegistrationId(1),
        ServiceWorkerVersionId(1),
        url("https://example.test/root-worker.js"),
        url("https://example.test/"),
        [],
    );
    let app = insert_registered_version(
        &service,
        ServiceWorkerRegistrationId(2),
        ServiceWorkerVersionId(2),
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );

    let app_client_id =
        register_client_for_test(&service, url("https://example.test/app/page.html"));
    let root_client_id = register_client_for_test(&service, url("https://example.test/other.html"));
    let out_of_scope_client_id =
        register_client_for_test(&service, url("https://other.test/page.html"));

    assert_eq!(
        service.matching_controller_for_client(app_client_id),
        Some(app)
    );
    assert_eq!(
        service.matching_controller_for_client(root_client_id),
        Some(root)
    );
    assert_eq!(
        service.matching_controller_for_client(out_of_scope_client_id),
        None
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.live_client_count, 3);
    assert_eq!(diagnostics.controlled_client_count, 2);
}
#[test]
fn devtools_controlled_window_client_urls_use_current_document_urls() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [
            url("https://example.test/app/page.html#section"),
            url("https://example.test/app/other.html"),
        ],
    );

    assert_eq!(
        service.controlled_window_client_urls_for_version_for_devtools(registration_id, version_id),
        vec![
            "https://example.test/app/other.html".to_owned(),
            "https://example.test/app/page.html".to_owned(),
        ]
    );
    assert_eq!(
        service.controlled_window_client_ids_for_version_for_devtools(registration_id, version_id),
        vec![1, 2]
    );
    assert!(
        service
            .controlled_window_client_urls_for_version_for_devtools(
                registration_id,
                ServiceWorkerVersionId(99)
            )
            .is_empty()
    );
    assert!(
        service
            .controlled_window_client_ids_for_version_for_devtools(
                registration_id,
                ServiceWorkerVersionId(99)
            )
            .is_empty()
    );
}
#[test]
fn new_controlled_client_queues_service_worker_version_update_for_devtools() {
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
        [],
    );
    {
        let mut state = service.inner.state.lock();
        state.record_target_created(registration_id, version_id, script_url, scope_url);
        service.take_target_output_events_for_test();
    }

    register_client_for_test(&service, url("https://example.test/app/page.html"));

    let target_events = service.take_target_output_events_for_test();
    assert!(
        target_events.iter().any(|event| matches!(
            event,
            crate::runtime::RendererServiceWorkerTargetEvent::VersionUpdated {
                version_id: updated_version_id,
                status,
            } if *updated_version_id == version_id.as_u64()
                && *status == crate::runtime::RendererServiceWorkerVersionStatus::Activated
        )),
        "newly controlled client should refresh ServiceWorker version projection: {target_events:?}"
    );
}
#[test]
fn local_worker_client_inherits_parent_controller_and_keeps_creation_url() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let parent_url = url("https://example.test/app/page.html");
    let controller = insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/sw.js"),
        url("https://example.test/app/"),
        [parent_url.clone()],
    );
    let parent_client_id = client_id_for_document(&service, &parent_url);
    let storage_key = ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&parent_url);

    assert_eq!(
        service.register_reserved_worker_client_inheriting_controller_from_client(
            url("data:text/javascript,postMessage('ready')"),
            storage_key.clone(),
            ServiceWorkerClientType::DedicatedWorker,
            true,
            parent_client_id,
        ),
        None,
        "data worker scripts must not inherit a service worker controller"
    );

    let local_worker_url = url("blob:https://example.test/blob-worker");
    let client_id = service
        .register_reserved_worker_client_inheriting_controller_from_client(
            local_worker_url.clone(),
            storage_key,
            ServiceWorkerClientType::DedicatedWorker,
            true,
            parent_client_id,
        )
        .expect("parent blob worker client should inherit controller");
    assert_eq!(
        service.matching_controller_for_client(client_id),
        Some(controller.clone())
    );
    assert_eq!(
        service.matching_controller_for_client_fetch(
            client_id,
            &url("https://example.test/other/api.json")
        ),
        Some(controller.clone())
    );
    let (worker_tx, _worker_rx) = tokio::sync::mpsc::unbounded_channel();
    assert!(service.activate_reserved_worker_client(client_id, worker_tx));

    let query = ServiceWorkerClientQuery {
        request_id: 7,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: false,
                client_type: ServiceWorkerClientQueryType::Worker,
            },
        },
    };
    let snapshots = service.query_clients(&query);
    assert!(
        snapshots.clients.iter().any(|client| {
            client.id == client_id && client.url == local_worker_url && client.controlled
        }),
        "inherited blob worker client should stay controlled while exposing its creation URL: {:?}",
        snapshots.clients
    );
}
#[test]
fn clients_claim_controls_live_scope_clients_only_for_active_version() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let app_page = url("https://example.test/app/page.html");
    let other_page = url("https://example.test/other.html");
    let app_client_id = register_client_for_test(&service, app_page.clone());
    register_client_for_test(&service, other_page.clone());
    let state = insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );

    service.finish_worker_clients_claim_requested(registration_id, ServiceWorkerVersionId(99));
    assert_eq!(service.matching_controller_for_document(&app_page), None);

    service.finish_worker_clients_claim_requested(registration_id, version_id);

    assert_eq!(
        service.matching_controller_for_document(&app_page),
        Some(state.clone())
    );
    assert_eq!(
        service.matching_controller_for_fetch(&app_page, &url("https://other.test/api.json")),
        Some(state)
    );
    assert_eq!(service.matching_controller_for_document(&other_page), None);

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.live_client_count, 2);
    assert_eq!(diagnostics.controlled_client_count, 1);

    service.unregister_client(app_client_id);
    assert_eq!(service.matching_controller_for_document(&app_page), None);
    assert_eq!(service.diagnostics_snapshot().controlled_client_count, 0);
}
#[test]
fn clients_claim_does_not_claim_client_when_longer_registration_matches() {
    let service = new_service_worker_runtime_service();
    let root_registration_id = ServiceWorkerRegistrationId(1);
    let root_version_id = ServiceWorkerVersionId(1);
    let app_registration_id = ServiceWorkerRegistrationId(2);
    let app_version_id = ServiceWorkerVersionId(2);
    insert_registered_version(
        &service,
        root_registration_id,
        root_version_id,
        url("https://example.test/root-worker.js"),
        url("https://example.test/"),
        [],
    );
    let app_state = insert_registered_version(
        &service,
        app_registration_id,
        app_version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );
    let app_page = url("https://example.test/app/page.html");
    let app_client_id = register_client_for_test(&service, app_page.clone());

    assert_eq!(
        service.matching_controller_for_document(&app_page),
        Some(app_state.clone())
    );

    service.finish_worker_clients_claim_requested(root_registration_id, root_version_id);

    assert_eq!(
        service.matching_controller_for_document(&app_page),
        Some(app_state)
    );
    let state = service.inner.state.lock();
    let root_registration = state.registrations.get(&root_registration_id).unwrap();
    let app_registration = state.registrations.get(&app_registration_id).unwrap();
    assert!(
        !root_registration
            .controlled_client_ids
            .contains(&app_client_id)
    );
    assert!(
        app_registration
            .controlled_client_ids
            .contains(&app_client_id)
    );
}
#[test]
fn clients_claim_replaces_previous_controller_registration() {
    let service = new_service_worker_runtime_service();
    let root_registration_id = ServiceWorkerRegistrationId(1);
    let root_version_id = ServiceWorkerVersionId(1);
    let app_registration_id = ServiceWorkerRegistrationId(2);
    let app_version_id = ServiceWorkerVersionId(2);
    let root_state = insert_registered_version(
        &service,
        root_registration_id,
        root_version_id,
        url("https://example.test/root-worker.js"),
        url("https://example.test/"),
        [],
    );
    let app_page = url("https://example.test/app/page.html");
    let app_client_id = register_client_for_test(&service, app_page.clone());
    let app_state = insert_registered_version(
        &service,
        app_registration_id,
        app_version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );

    assert_eq!(
        service.matching_controller_for_document(&app_page),
        Some(root_state)
    );

    service.finish_worker_clients_claim_requested(app_registration_id, app_version_id);

    assert_eq!(
        service.matching_controller_for_document(&app_page),
        Some(app_state)
    );
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.live_client_count, 1);
    assert_eq!(diagnostics.controlled_client_count, 1);
    let state = service.inner.state.lock();
    let root_registration = state.registrations.get(&root_registration_id).unwrap();
    let app_registration = state.registrations.get(&app_registration_id).unwrap();
    assert!(
        !root_registration
            .controlled_client_ids
            .contains(&app_client_id)
    );
    assert!(
        app_registration
            .controlled_client_ids
            .contains(&app_client_id)
    );
}
#[test]
fn clients_claim_skips_ineligible_live_window_clients() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let eligible_client_id =
        register_client_for_test(&service, url("https://example.test/app/eligible.html"));
    let not_ready_client_id =
        register_client_for_test(&service, url("https://example.test/app/not-ready.html"));
    let discarded_client_id =
        register_client_for_test(&service, url("https://example.test/app/discarded.html"));
    let insecure_client_id =
        register_client_for_test(&service, url("https://example.test/app/insecure.html"));
    {
        let mut state = service.inner.state.lock();
        state
            .live_clients
            .get_mut(&not_ready_client_id)
            .unwrap()
            .execution_ready = false;
        state
            .live_clients
            .get_mut(&discarded_client_id)
            .unwrap()
            .discarded_or_frozen = true;
        state
            .live_clients
            .get_mut(&insecure_client_id)
            .unwrap()
            .secure_context = false;
    }
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );

    service.finish_worker_clients_claim_requested(registration_id, version_id);

    let state = service.inner.state.lock();
    let registration = state.registrations.get(&registration_id).unwrap();
    assert_eq!(
        registration.controlled_client_ids,
        HashSet::from([eligible_client_id])
    );
}
#[test]
fn reserved_window_client_is_hidden_from_clients_api_until_document_commit() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let control_state = insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );
    let reserved_creation_url = url("https://example.test/app/reserved.html#pending");
    let reserved_client_id = register_reserved_window_client_for_test(
        &service,
        reserved_creation_url.clone(),
        ServiceWorkerClientFrameType::TopLevel,
    );

    assert_eq!(
        service.matching_controller_for_client(reserved_client_id),
        Some(control_state)
    );

    let hidden_match_all = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 18,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: true,
                client_type: ServiceWorkerClientQueryType::Window,
            },
        },
    });
    assert!(hidden_match_all.clients.is_empty());

    let hidden_get = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 19,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::Get {
            exposed_client_id: service_worker_exposed_client_id(reserved_client_id),
        },
    });
    assert!(hidden_get.clients.is_empty());

    let committed_creation_url = url("https://example.test/app/committed.html#current");
    assert!(service.update_client_document_with_storage_key(
        reserved_client_id,
        committed_creation_url.clone(),
        ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&committed_creation_url,),
        ServiceWorkerClientFrameType::TopLevel,
        Some(crate::native_bridge::WindowDocumentOwner::for_test(3)),
    ));

    let visible = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 20,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: false,
                client_type: ServiceWorkerClientQueryType::Window,
            },
        },
    });
    assert_eq!(visible.clients.len(), 1);
    assert_eq!(visible.clients[0].id, reserved_client_id);
    assert_eq!(visible.clients[0].url, committed_creation_url);
    assert!(visible.clients[0].controlled);
}
#[test]
fn clients_claim_skips_reserved_window_clients_until_document_commit() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let ready_client_id =
        register_client_for_test(&service, url("https://example.test/app/ready.html"));
    let reserved_creation_url = url("https://example.test/app/reserved.html");
    let reserved_client_id = register_reserved_window_client_for_test(
        &service,
        reserved_creation_url.clone(),
        ServiceWorkerClientFrameType::TopLevel,
    );
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );

    service.finish_worker_clients_claim_requested(registration_id, version_id);

    {
        let state = service.inner.state.lock();
        let registration = state.registrations.get(&registration_id).unwrap();
        assert_eq!(
            registration.controlled_client_ids,
            HashSet::from([ready_client_id])
        );
        assert!(
            !state
                .live_clients
                .get(&reserved_client_id)
                .expect("reserved client should remain live")
                .execution_ready
        );
    }

    let hidden = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 21,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::Get {
            exposed_client_id: service_worker_exposed_client_id(reserved_client_id),
        },
    });
    assert!(hidden.clients.is_empty());

    assert!(service.update_client_document_with_storage_key(
        reserved_client_id,
        reserved_creation_url.clone(),
        ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&reserved_creation_url),
        ServiceWorkerClientFrameType::TopLevel,
        Some(crate::native_bridge::WindowDocumentOwner::for_test(4)),
    ));

    let visible = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 22,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: false,
                client_type: ServiceWorkerClientQueryType::Window,
            },
        },
    });
    assert_eq!(
        visible
            .clients
            .iter()
            .map(|client| client.id)
            .collect::<Vec<_>>(),
        vec![ready_client_id, reserved_client_id]
    );
}
#[test]
fn bypassed_navigation_client_stays_uncontrolled_until_claimed() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let document_url = url("https://example.test/app/bypassed.html");
    let storage_key = ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&document_url);
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );

    let client_id = service.register_reserved_client_with_storage_key_bypassing_service_worker(
        document_url.clone(),
        storage_key.clone(),
        ServiceWorkerClientFrameType::TopLevel,
        None,
    );
    assert!(service.matching_controller_for_client(client_id).is_none());

    assert!(
        service.update_client_document_with_storage_key_and_completion_sender(
            client_id,
            document_url,
            storage_key,
            ServiceWorkerClientFrameType::TopLevel,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(1)),
            test_completion_sender(),
        )
    );
    assert!(service.matching_controller_for_client(client_id).is_none());

    service.finish_worker_clients_claim_requested(registration_id, version_id);
    assert!(service.matching_controller_for_client(client_id).is_some());
}
#[test]
fn window_client_completion_target_rotates_with_exact_document_owner() {
    let service = new_service_worker_runtime_service();
    let document_url = url("https://client-epoch.test/page.html");
    let storage_key = ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&document_url);
    let client_id = service.register_client_with_storage_key(
        document_url.clone(),
        storage_key.clone(),
        ServiceWorkerClientFrameType::TopLevel,
        Some(crate::native_bridge::WindowDocumentOwner::for_test(10)),
        test_completion_sender(),
    );
    let initial_target = service
        .inner
        .state
        .lock()
        .live_clients
        .get(&client_id)
        .and_then(ServiceWorkerClient::window_completion_target)
        .expect("initial window client target");

    assert!(service.update_client_document_with_storage_key(
        client_id,
        document_url,
        storage_key,
        ServiceWorkerClientFrameType::TopLevel,
        Some(crate::native_bridge::WindowDocumentOwner::for_test(11)),
    ));
    let replacement_target = service
        .inner
        .state
        .lock()
        .live_clients
        .get(&client_id)
        .and_then(ServiceWorkerClient::window_completion_target)
        .expect("replacement window client target");

    assert_eq!(initial_target.client_id, replacement_target.client_id);
    assert_eq!(
        initial_target.document_owner,
        crate::native_bridge::WindowDocumentOwner::for_test(10)
    );
    assert_eq!(
        replacement_target.document_owner,
        crate::native_bridge::WindowDocumentOwner::for_test(11)
    );
    assert_ne!(initial_target, replacement_target);
}
#[test]
fn clients_claim_notifies_only_newly_controlled_live_window_clients() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let app_page = url("https://example.test/app/page.html");
    let outside_page = url("https://example.test/outside.html");
    let mut app_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let mut outside_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let app_client_id = service.register_client_with_storage_key(
        app_page.clone(),
        ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&app_page),
        ServiceWorkerClientFrameType::TopLevel,
        Some(crate::native_bridge::WindowDocumentOwner::for_test(7)),
        app_queue.sender(),
    );
    service.register_client_with_storage_key(
        outside_page.clone(),
        ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&outside_page),
        ServiceWorkerClientFrameType::TopLevel,
        Some(crate::native_bridge::WindowDocumentOwner::for_test(11)),
        outside_queue.sender(),
    );
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );

    service.finish_worker_clients_claim_requested(registration_id, version_id);

    let completion = match app_queue.pop_internal() {
        Some(crate::page_task_queue::RendererServiceWorkerInternalTask::ControllerChange(
            completion,
        )) => completion,
        other => panic!("expected controllerchange completion, got {other:?}"),
    };
    assert_eq!(completion.target.client_id, app_client_id);
    assert_eq!(
        completion.target.document_owner,
        crate::native_bridge::WindowDocumentOwner::for_test(7)
    );
    assert!(!app_queue.has_ready_task());
    assert!(!outside_queue.has_ready_task());

    service.finish_worker_clients_claim_requested(registration_id, version_id);

    assert!(!app_queue.has_ready_task());
    assert!(!outside_queue.has_ready_task());
}
#[test]
fn clients_claim_notifies_newly_controlled_live_worker_clients() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let app_worker = url("https://example.test/app/worker.js");
    let outside_worker = url("https://example.test/outside/worker.js");
    let (worker_tx, mut worker_rx) = tokio::sync::mpsc::unbounded_channel();
    service.register_worker_client_with_storage_key(
        app_worker.clone(),
        ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&app_worker),
        ServiceWorkerClientType::DedicatedWorker,
        true,
        worker_tx,
    );
    let (outside_tx, mut outside_rx) = tokio::sync::mpsc::unbounded_channel();
    service.register_worker_client_with_storage_key(
        outside_worker.clone(),
        ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&outside_worker),
        ServiceWorkerClientType::DedicatedWorker,
        true,
        outside_tx,
    );
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );

    service.finish_worker_clients_claim_requested(registration_id, version_id);

    assert!(matches!(
        worker_rx.try_recv(),
        Ok(crate::worker::WorkerMessage::ServiceWorkerControllerChange)
    ));
    assert!(matches!(
        worker_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    assert!(matches!(
        outside_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));

    service.finish_worker_clients_claim_requested(registration_id, version_id);

    assert!(matches!(
        worker_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    assert!(matches!(
        outside_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}
#[test]
fn client_query_matches_live_window_clients_for_active_version() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let controlled_page = url("https://example.test/app/controlled.html");
    let uncontrolled_page = url("https://example.test/app/uncontrolled.html");
    let not_ready_page = url("https://example.test/app/not-ready.html");
    let discarded_page = url("https://example.test/app/discarded.html");
    let out_of_scope_page = url("https://example.test/other.html");
    let cross_origin_page = url("https://other.test/app/page.html");
    let controlled_client_id = register_client_for_test(&service, controlled_page.clone());
    let uncontrolled_client_id = register_client_for_test(&service, uncontrolled_page.clone());
    let not_ready_client_id = register_client_for_test(&service, not_ready_page);
    let discarded_client_id = register_client_for_test(&service, discarded_page);
    let out_of_scope_client_id = register_client_for_test(&service, out_of_scope_page.clone());
    let cross_origin_client_id = register_client_for_test(&service, cross_origin_page);
    {
        let mut state = service.inner.state.lock();
        state
            .live_clients
            .get_mut(&not_ready_client_id)
            .expect("not-ready client should exist")
            .execution_ready = false;
        state
            .live_clients
            .get_mut(&discarded_client_id)
            .expect("discarded client should exist")
            .discarded_or_frozen = true;
    }
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );
    {
        let mut state = service.inner.state.lock();
        let registration = state.registrations.get_mut(&registration_id).unwrap();
        registration
            .controlled_client_ids
            .insert(controlled_client_id);
    }

    let controlled_only = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 10,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: false,
                client_type: ServiceWorkerClientQueryType::Window,
            },
        },
    });
    assert_eq!(controlled_only.request_id, 10);
    assert_eq!(controlled_only.clients.len(), 1);
    assert_eq!(controlled_only.clients[0].id, controlled_client_id);
    assert_eq!(
        controlled_only.clients[0].exposed_id,
        service_worker_exposed_client_id(controlled_client_id)
    );
    assert_ne!(
        controlled_only.clients[0].exposed_id,
        controlled_client_id.as_u64().to_string()
    );
    assert!(controlled_only.clients[0].controlled);
    assert_eq!(controlled_only.clients[0].url, controlled_page);
    assert_eq!(
        controlled_only.clients[0].client_type,
        ServiceWorkerClientType::Window
    );
    assert_eq!(
        controlled_only.clients[0].frame_type,
        ServiceWorkerClientFrameType::TopLevel
    );
    assert_eq!(
        controlled_only.clients[0].visibility_state,
        ServiceWorkerClientVisibilityState::Visible
    );

    let all_scope_windows = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 11,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: true,
                client_type: ServiceWorkerClientQueryType::All,
            },
        },
    });
    assert_eq!(
        all_scope_windows
            .clients
            .iter()
            .map(|client| client.id)
            .collect::<Vec<_>>(),
        vec![controlled_client_id, uncontrolled_client_id]
    );
    assert_eq!(all_scope_windows.clients[1].url, uncontrolled_page);
    assert!(!all_scope_windows.clients[1].controlled);

    let worker_clients = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 12,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: true,
                client_type: ServiceWorkerClientQueryType::Worker,
            },
        },
    });
    assert!(worker_clients.clients.is_empty());

    let shared_worker_clients = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 13,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: true,
                client_type: ServiceWorkerClientQueryType::SharedWorker,
            },
        },
    });
    assert!(shared_worker_clients.clients.is_empty());

    let found_uncontrolled = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 14,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::Get {
            exposed_client_id: service_worker_exposed_client_id(uncontrolled_client_id),
        },
    });
    assert_eq!(found_uncontrolled.clients.len(), 1);
    assert_eq!(found_uncontrolled.clients[0].id, uncontrolled_client_id);
    assert!(!found_uncontrolled.clients[0].controlled);

    let hidden_not_ready = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 19,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::Get {
            exposed_client_id: service_worker_exposed_client_id(not_ready_client_id),
        },
    });
    assert!(hidden_not_ready.clients.is_empty());

    let hidden_discarded = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 20,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::Get {
            exposed_client_id: service_worker_exposed_client_id(discarded_client_id),
        },
    });
    assert!(hidden_discarded.clients.is_empty());

    let hidden_legacy_internal_id = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 18,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::Get {
            exposed_client_id: uncontrolled_client_id.as_u64().to_string(),
        },
    });
    assert!(hidden_legacy_internal_id.clients.is_empty());

    let found_out_of_scope_same_origin = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 16,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::Get {
            exposed_client_id: service_worker_exposed_client_id(out_of_scope_client_id),
        },
    });
    assert_eq!(found_out_of_scope_same_origin.clients.len(), 1);
    assert_eq!(
        found_out_of_scope_same_origin.clients[0].id,
        out_of_scope_client_id
    );
    assert_eq!(
        found_out_of_scope_same_origin.clients[0].url,
        out_of_scope_page
    );
    assert!(!found_out_of_scope_same_origin.clients[0].controlled);

    let hidden_cross_origin = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 17,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::Get {
            exposed_client_id: service_worker_exposed_client_id(cross_origin_client_id),
        },
    });
    assert!(hidden_cross_origin.clients.is_empty());

    {
        let mut state = service.inner.state.lock();
        state.versions.get_mut(&version_id).unwrap().lifecycle_state =
            ServiceWorkerVersionLifecycleState::Installed;
    }
    let inactive_result = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 15,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::Get {
            exposed_client_id: service_worker_exposed_client_id(controlled_client_id),
        },
    });
    assert!(inactive_result.clients.is_empty());
}
#[test]
fn client_query_filters_existing_live_clients_by_type() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let window_client_id =
        register_client_for_test(&service, url("https://example.test/app/window.html"));
    let dedicated_worker_client_id = insert_live_client_record_for_test(
        &service,
        url("https://example.test/app/dedicated-worker.js"),
        ServiceWorkerClientType::DedicatedWorker,
    );
    let shared_worker_client_id = insert_live_client_record_for_test(
        &service,
        url("https://example.test/app/shared-worker.js"),
        ServiceWorkerClientType::SharedWorker,
    );
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );
    {
        let mut state = service.inner.state.lock();
        let registration = state.registrations.get_mut(&registration_id).unwrap();
        registration.controlled_client_ids.insert(window_client_id);
        registration
            .controlled_client_ids
            .insert(dedicated_worker_client_id);
    }

    let worker_clients = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 21,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: false,
                client_type: ServiceWorkerClientQueryType::Worker,
            },
        },
    });
    assert_eq!(
        worker_clients
            .clients
            .iter()
            .map(|client| client.id)
            .collect::<Vec<_>>(),
        vec![dedicated_worker_client_id]
    );
    assert_eq!(
        worker_clients.clients[0].client_type,
        ServiceWorkerClientType::DedicatedWorker
    );
    assert_eq!(
        worker_clients.clients[0].frame_type,
        ServiceWorkerClientFrameType::None
    );
    assert!(worker_clients.clients[0].controlled);

    let shared_worker_controlled_only = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 22,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: false,
                client_type: ServiceWorkerClientQueryType::SharedWorker,
            },
        },
    });
    assert!(shared_worker_controlled_only.clients.is_empty());

    let shared_worker_clients = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 23,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: true,
                client_type: ServiceWorkerClientQueryType::SharedWorker,
            },
        },
    });
    assert_eq!(shared_worker_clients.clients.len(), 1);
    assert_eq!(shared_worker_clients.clients[0].id, shared_worker_client_id);
    assert_eq!(
        shared_worker_clients.clients[0].client_type,
        ServiceWorkerClientType::SharedWorker
    );
    assert!(!shared_worker_clients.clients[0].controlled);

    let all_clients = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 24,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: true,
                client_type: ServiceWorkerClientQueryType::All,
            },
        },
    });
    assert_eq!(
        all_clients
            .clients
            .iter()
            .map(|client| client.id)
            .collect::<Vec<_>>(),
        vec![
            window_client_id,
            dedicated_worker_client_id,
            shared_worker_client_id
        ]
    );

    let window_clients = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 25,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: true,
                client_type: ServiceWorkerClientQueryType::Window,
            },
        },
    });
    assert_eq!(
        window_clients
            .clients
            .iter()
            .map(|client| client.id)
            .collect::<Vec<_>>(),
        vec![window_client_id]
    );
}
#[test]
fn client_query_preserves_window_client_frame_type() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let top_level_client_id = register_window_client_for_test(
        &service,
        url("https://example.test/app/top.html"),
        ServiceWorkerClientFrameType::TopLevel,
    );
    let nested_client_id = register_window_client_for_test(
        &service,
        url("https://example.test/app/frame.html"),
        ServiceWorkerClientFrameType::Nested,
    );
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );
    {
        let mut state = service.inner.state.lock();
        let registration = state.registrations.get_mut(&registration_id).unwrap();
        registration
            .controlled_client_ids
            .insert(top_level_client_id);
        registration.controlled_client_ids.insert(nested_client_id);
    }

    let clients = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 26,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: false,
                client_type: ServiceWorkerClientQueryType::Window,
            },
        },
    });
    assert_eq!(
        clients
            .clients
            .iter()
            .map(|client| (client.id, client.frame_type))
            .collect::<Vec<_>>(),
        vec![
            (top_level_client_id, ServiceWorkerClientFrameType::TopLevel),
            (nested_client_id, ServiceWorkerClientFrameType::Nested),
        ]
    );

    let nested = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 27,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::Get {
            exposed_client_id: service_worker_exposed_client_id(nested_client_id),
        },
    });
    assert_eq!(nested.clients.len(), 1);
    assert_eq!(
        nested.clients[0].frame_type,
        ServiceWorkerClientFrameType::Nested
    );
}
#[test]
fn client_query_requires_exact_active_controller_version() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let waiting_version_id = ServiceWorkerVersionId(2);
    let scope_url = url("https://example.test/app/");
    let active_script_url = url("https://example.test/app/worker.js");
    let waiting_script_url = url("https://example.test/app/worker-updated.js");
    let client_id = register_client_for_test(&service, url("https://example.test/app/page.html"));
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
        let registration = state.registrations.get_mut(&registration_id).unwrap();
        registration.script_url = waiting_script_url.clone();
        registration.waiting_version_id = Some(waiting_version_id);
        registration.controlled_client_ids.insert(client_id);
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

    let waiting_worker_clients = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 26,
        registration_id,
        version_id: waiting_version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: false,
                client_type: ServiceWorkerClientQueryType::Window,
            },
        },
    });
    assert!(waiting_worker_clients.clients.is_empty());

    let active_worker_clients = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 27,
        registration_id,
        version_id: active_version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: false,
                client_type: ServiceWorkerClientQueryType::Window,
            },
        },
    });
    assert_eq!(
        active_worker_clients
            .clients
            .iter()
            .map(|client| client.id)
            .collect::<Vec<_>>(),
        vec![client_id]
    );
}
#[test]
fn client_document_update_tracks_creation_url_and_regenerates_cross_origin_exposed_id() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let initial_creation_url = url("https://example.test/app/page.html#initial");
    let initial_current_url = url("https://example.test/app/page.html");
    let client_id = register_client_for_test(&service, initial_creation_url.clone());
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );

    let initial = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 30,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: true,
                client_type: ServiceWorkerClientQueryType::Window,
            },
        },
    });
    assert_eq!(initial.clients.len(), 1);
    assert_eq!(initial.clients[0].url, initial_creation_url);
    let initial_exposed_id = initial.clients[0].exposed_id.clone();
    {
        let state = service.inner.state.lock();
        let client = state.live_clients.get(&client_id).unwrap();
        assert_eq!(client.creation_url, initial_creation_url);
        assert_eq!(client.document_url, initial_current_url);
        assert_eq!(client.exposed_id, initial_exposed_id);
    }

    let same_origin_creation_url = url("https://example.test/app/next.html#same-origin");
    let same_origin_current_url = url("https://example.test/app/next.html");
    assert!(service.update_client_document_with_storage_key(
        client_id,
        same_origin_creation_url.clone(),
        ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&same_origin_creation_url,),
        ServiceWorkerClientFrameType::TopLevel,
        Some(crate::native_bridge::WindowDocumentOwner::for_test(2)),
    ));
    let same_origin_result = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 31,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::Get {
            exposed_client_id: initial_exposed_id.clone(),
        },
    });
    assert_eq!(same_origin_result.clients.len(), 1);
    assert_eq!(same_origin_result.clients[0].url, same_origin_creation_url);
    assert_eq!(same_origin_result.clients[0].exposed_id, initial_exposed_id);
    assert!(same_origin_result.clients[0].controlled);
    {
        let state = service.inner.state.lock();
        let client = state.live_clients.get(&client_id).unwrap();
        assert_eq!(client.creation_url, same_origin_creation_url);
        assert_eq!(client.document_url, same_origin_current_url);
        assert_eq!(client.exposed_id, initial_exposed_id);
    }

    let cross_origin_creation_url = url("https://other.test/app/page.html#cross-origin");
    let cross_origin_current_url = url("https://other.test/app/page.html");
    assert!(service.update_client_document_with_storage_key(
        client_id,
        cross_origin_creation_url.clone(),
        ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&cross_origin_creation_url,),
        ServiceWorkerClientFrameType::TopLevel,
        Some(crate::native_bridge::WindowDocumentOwner::for_test(3)),
    ));
    let hidden_old_exposed_id = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 32,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::Get {
            exposed_client_id: initial_exposed_id.clone(),
        },
    });
    assert!(hidden_old_exposed_id.clients.is_empty());
    {
        let state = service.inner.state.lock();
        let client = state.live_clients.get(&client_id).unwrap();
        assert_eq!(client.creation_url, cross_origin_creation_url);
        assert_eq!(client.document_url, cross_origin_current_url);
        assert_ne!(client.exposed_id, initial_exposed_id);
        let registration = state.registrations.get(&registration_id).unwrap();
        assert!(!registration.controlled_client_ids.contains(&client_id));
    }
}
#[test]
fn client_navigate_result_exposes_same_origin_window_client_only() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );

    let controlled_client_id =
        register_client_for_test(&service, url("https://example.test/app/next.html"));
    let out_of_scope_same_origin_client_id =
        register_client_for_test(&service, url("https://example.test/elsewhere.html"));
    let cross_origin_client_id =
        register_client_for_test(&service, url("https://other.test/app/next.html"));
    let about_blank_client_id = register_client_for_test(&service, url("about:blank"));

    let controlled_result = service
        .client_navigate_result_for_current_window_client(version_id, controlled_client_id)
        .expect("controlled same-origin client should produce a result")
        .expect("controlled same-origin client should be exposed");
    assert_eq!(controlled_result.id, controlled_client_id);
    assert_eq!(
        controlled_result.url,
        url("https://example.test/app/next.html")
    );
    assert!(controlled_result.controlled);
    assert_eq!(
        controlled_result.client_type,
        ServiceWorkerClientType::Window
    );

    let out_of_scope_same_origin_result = service
        .client_navigate_result_for_current_window_client(
            version_id,
            out_of_scope_same_origin_client_id,
        )
        .expect("same-origin client should produce a result")
        .expect("same-origin client should be exposed even if no longer controlled");
    assert_eq!(
        out_of_scope_same_origin_result.id,
        out_of_scope_same_origin_client_id
    );
    assert_eq!(
        out_of_scope_same_origin_result.url,
        url("https://example.test/elsewhere.html")
    );
    assert!(!out_of_scope_same_origin_result.controlled);

    let cross_origin_result = service
        .client_navigate_result_for_current_window_client(version_id, cross_origin_client_id)
        .expect("cross-origin client should resolve as a successful null result");
    assert_eq!(cross_origin_result, None);

    let about_blank_result = service
        .client_navigate_result_for_current_window_client(version_id, about_blank_client_id)
        .expect("about:blank client should resolve as a successful null result");
    assert_eq!(about_blank_result, None);
}
#[test]
fn client_focus_result_marks_current_window_client_focused() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );
    let first_client_id =
        register_client_for_test(&service, url("https://example.test/app/first.html"));
    let second_client_id =
        register_client_for_test(&service, url("https://example.test/app/second.html"));

    let focused = service
        .client_focus_result_for_current_window_client(version_id, second_client_id)
        .expect("focus should produce a window client snapshot");
    assert_eq!(focused.id, second_client_id);
    assert!(focused.focused);

    let all_scope_windows = service.query_clients(&ServiceWorkerClientQuery {
        request_id: 20,
        registration_id,
        version_id,
        kind: ServiceWorkerClientQueryKind::MatchAll {
            options: ServiceWorkerClientQueryOptions {
                include_uncontrolled: true,
                client_type: ServiceWorkerClientQueryType::Window,
            },
        },
    });
    assert_eq!(
        all_scope_windows
            .clients
            .iter()
            .map(|client| client.id)
            .collect::<Vec<_>>(),
        vec![second_client_id, first_client_id]
    );
    let first = all_scope_windows
        .clients
        .iter()
        .find(|client| client.id == first_client_id)
        .expect("first client should remain live");
    let second = all_scope_windows
        .clients
        .iter()
        .find(|client| client.id == second_client_id)
        .expect("second client should remain live");
    assert!(!first.focused);
    assert!(second.focused);
}
#[test]
fn client_focus_result_reports_not_found_for_missing_or_cross_origin_client() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );
    let cross_origin_client_id =
        register_client_for_test(&service, url("https://other.test/app/page.html"));

    let missing = service
        .client_focus_result_for_current_window_client(
            version_id,
            ServiceWorkerClientId::from_u64_for_test(999),
        )
        .expect_err("missing focus target should reject");
    assert_eq!(missing, ServiceWorkerClientFocusError::not_found());

    let cross_origin = service
        .client_focus_result_for_current_window_client(version_id, cross_origin_client_id)
        .expect_err("cross-origin focus target should be hidden as not found");
    assert_eq!(cross_origin, ServiceWorkerClientFocusError::not_found());
}
#[test]
fn client_focus_result_reports_inactive_for_discarded_window_client() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );
    let focused_client_id =
        register_client_for_test(&service, url("https://example.test/app/focused.html"));
    let discarded_client_id =
        register_client_for_test(&service, url("https://example.test/app/discarded.html"));
    {
        let mut state = service.inner.state.lock();
        state
            .live_clients
            .get_mut(&focused_client_id)
            .expect("focused client should exist")
            .focused = true;
        state
            .live_clients
            .get_mut(&discarded_client_id)
            .expect("discarded client should exist")
            .discarded_or_frozen = true;
    }

    let inactive = service
        .client_focus_result_for_current_window_client(version_id, discarded_client_id)
        .expect_err("discarded focus target should reject as inactive");
    assert_eq!(inactive, ServiceWorkerClientFocusError::inactive());

    let state = service.inner.state.lock();
    assert!(
        state
            .live_clients
            .get(&focused_client_id)
            .expect("focused client should remain live")
            .focused
    );
    assert!(
        !state
            .live_clients
            .get(&discarded_client_id)
            .expect("discarded client should remain live")
            .focused
    );
}
#[test]
fn client_focus_request_rejects_discarded_window_before_page_owner() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let document_url = url("https://example.test/app/page.html");
    let mut completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let client_id = service.register_client(document_url, 7, completion_queue.sender());
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );
    {
        let mut state = service.inner.state.lock();
        state
            .registrations
            .get_mut(&registration_id)
            .expect("registration should exist")
            .controlled_client_ids
            .insert(client_id);
        state
            .live_clients
            .get_mut(&client_id)
            .expect("client should exist")
            .discarded_or_frozen = true;
    }

    let run = exact_version_run(&service, version_id);
    service.finish_client_focus_requested(
        ServiceWorkerClientFocus {
            request_id: 40,
            source_version_id: version_id,
            target_client_id: client_id,
        },
        run,
    );

    assert!(!completion_queue.has_ready_task());
    let state = service.inner.state.lock();
    assert!(
        !state
            .live_clients
            .get(&client_id)
            .expect("client should remain live")
            .focused
    );
}
#[test]
fn clients_open_window_uses_ready_active_window_host() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let script_url = url("https://example.test/app/worker.js");
    let scope_url = url("https://example.test/app/");
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url,
        scope_url,
        [],
    );

    let mut not_ready_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let mut discarded_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let mut ready_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

    let not_ready_url = url("https://example.test/app/not-ready.html");
    let discarded_url = url("https://example.test/app/discarded.html");
    let ready_url = url("https://example.test/app/ready.html");
    let not_ready_client_id = service.register_client_with_storage_key(
        not_ready_url.clone(),
        ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&not_ready_url),
        ServiceWorkerClientFrameType::TopLevel,
        Some(crate::native_bridge::WindowDocumentOwner::for_test(21)),
        not_ready_queue.sender(),
    );
    let discarded_client_id = service.register_client_with_storage_key(
        discarded_url.clone(),
        ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&discarded_url),
        ServiceWorkerClientFrameType::TopLevel,
        Some(crate::native_bridge::WindowDocumentOwner::for_test(22)),
        discarded_queue.sender(),
    );
    let ready_client_id = service.register_client_with_storage_key(
        ready_url.clone(),
        ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&ready_url),
        ServiceWorkerClientFrameType::TopLevel,
        Some(crate::native_bridge::WindowDocumentOwner::for_test(23)),
        ready_queue.sender(),
    );
    {
        let mut state = service.inner.state.lock();
        let registration = state
            .registrations
            .get_mut(&registration_id)
            .expect("registration should exist");
        registration.controlled_client_ids.extend([
            not_ready_client_id,
            discarded_client_id,
            ready_client_id,
        ]);
        state
            .live_clients
            .get_mut(&not_ready_client_id)
            .expect("not-ready client should exist")
            .execution_ready = false;
        state
            .live_clients
            .get_mut(&discarded_client_id)
            .expect("discarded client should exist")
            .discarded_or_frozen = true;
    }

    let run = exact_version_run(&service, version_id);
    service.finish_clients_open_window_requested(
        ServiceWorkerClientsOpenWindow {
            request_id: 51,
            source_version_id: version_id,
            url: url("https://example.test/app/opened.html"),
        },
        run.clone(),
    );

    assert!(!not_ready_queue.has_ready_task());
    assert!(!discarded_queue.has_ready_task());
    let Some(crate::page_task_queue::RendererServiceWorkerInternalTask::ClientsOpenWindowRequest(
        completion,
    )) = ready_queue.pop_internal()
    else {
        panic!("ready active window should receive openWindow request");
    };
    assert_eq!(completion.request_id, 51);
    assert_eq!(completion.host.client_id, ready_client_id);
    assert_eq!(
        completion.host.document_owner,
        crate::native_bridge::WindowDocumentOwner::for_test(23)
    );
    assert_eq!(completion.source_version_id, version_id);
    assert_eq!(completion.source_run, run);
    assert_eq!(completion.url, url("https://example.test/app/opened.html"));
}
#[test]
fn unregister_client_removes_only_matching_client_id_for_same_url() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let state = insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [],
    );
    let page = url("https://example.test/app/page.html");
    let first_client_id = register_client_for_test(&service, page.clone());
    let second_client_id = register_client_for_test(&service, page);

    service.finish_worker_clients_claim_requested(registration_id, version_id);
    assert_eq!(service.diagnostics_snapshot().controlled_client_count, 2);
    assert_eq!(
        service.matching_controller_for_client(first_client_id),
        Some(state.clone())
    );
    assert_eq!(
        service.matching_controller_for_client(second_client_id),
        Some(state.clone())
    );

    service.unregister_client(first_client_id);

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.live_client_count, 1);
    assert_eq!(diagnostics.controlled_client_count, 1);
    assert_eq!(
        service.matching_controller_for_client(first_client_id),
        None
    );
    assert_eq!(
        service.matching_controller_for_client(second_client_id),
        Some(state)
    );
}
#[test]
fn unregister_last_controlled_client_queues_waiting_activation() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let active_version_id = ServiceWorkerVersionId(1);
    let waiting_version_id = ServiceWorkerVersionId(2);
    let client_id = ServiceWorkerClientId::from_u64_for_test(1);
    let scope_url = url("https://example.test/app/");
    let active_script_url = url("https://example.test/app/worker-v1.js");
    let waiting_script_url = url("https://example.test/app/worker-v2.js");
    let waiting_run = RendererServiceWorkerRunIdentity::fresh();
    let waiting_host = new_loading_test_host(waiting_version_id, &waiting_run);
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

    service.unregister_client(client_id);

    assert_eq!(service.pending_service_lane_event_count(), 0);
    let state = service.inner.state.lock();
    assert!(!state.live_clients.contains_key(&client_id));
    let registration = state.registrations.get(&registration_id).unwrap();
    assert!(registration.controlled_client_ids.is_empty());
    assert_eq!(registration.active_version_id, Some(active_version_id));
    assert_eq!(registration.waiting_version_id, Some(waiting_version_id));
    let waiting = state.versions.get(&waiting_version_id).unwrap();
    assert_eq!(
        waiting.lifecycle_state,
        ServiceWorkerVersionLifecycleState::Activating
    );
    assert_eq!(waiting.in_flight_event_count, 1);
    assert_eq!(waiting.pending_start_events.len(), 1);
    let ServiceWorkerPendingStartEvent::Lifecycle(event) =
        waiting.pending_start_events.front().unwrap()
    else {
        panic!("expected queued activate event");
    };
    assert_eq!(event.kind, ServiceWorkerLifecycleEventKind::Activate);
    assert_eq!(
        event.owner,
        test_run_owner(waiting_version_id, &waiting_run)
    );
}
#[test]
fn pending_unregistration_keeps_existing_controlled_fetch_until_client_closes() {
    let service = new_service_worker_runtime_service();
    let document_url = url("https://example.test/app/page.html");
    let state = insert_registered_version(
        &service,
        ServiceWorkerRegistrationId(1),
        ServiceWorkerVersionId(1),
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [document_url.clone()],
    );
    let client_id = client_id_for_document(&service, &document_url);

    assert!(service.mark_registration_unregistered(state.scope_url()));
    assert!(!service.mark_registration_unregistered(state.scope_url()));
    assert_eq!(
        service.matching_registration_for_client(&document_url),
        None
    );
    assert_eq!(
        service.matching_controller_for_fetch(
            &document_url,
            &url("https://example.test/app/api.json")
        ),
        Some(state.clone())
    );
    assert_eq!(
        service.matching_controller_for_fetch(&document_url, &url("https://other.test/api.json")),
        Some(state.clone())
    );
    assert_eq!(
        service.matching_controller_for_document(&document_url),
        Some(state.clone())
    );

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.pending_unregistration_count, 1);
    assert_eq!(diagnostics.controlled_client_count, 1);
    assert_eq!(
        diagnostics.registrations[0].pending_clear_phase,
        Some("waiting-for-controllees")
    );

    service.unregister_client(client_id);

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 0);
    assert_eq!(diagnostics.version_count, 0);
    assert_eq!(diagnostics.pending_unregistration_count, 0);
    assert_eq!(
        service.matching_controller_for_fetch(&document_url, &url("https://other.test/api")),
        None
    );
}
#[test]
fn registration_lookup_uses_client_url_and_omits_pending_unregistration() {
    let service = new_service_worker_runtime_service();
    insert_registered_version(
        &service,
        ServiceWorkerRegistrationId(1),
        ServiceWorkerVersionId(1),
        url("https://example.test/app/worker.js"),
        url("https://example.test/app/"),
        [url("https://example.test/app/page.html")],
    );
    let app = snapshot_for_registration(&service, ServiceWorkerRegistrationId(1));
    insert_registered_version(
        &service,
        ServiceWorkerRegistrationId(2),
        ServiceWorkerVersionId(2),
        url("https://example.test/other/worker.js"),
        url("https://example.test/other/"),
        [url("https://example.test/other/page.html")],
    );
    let other = snapshot_for_registration(&service, ServiceWorkerRegistrationId(2));

    assert_eq!(
        service.matching_registration_for_client(&url("https://example.test/app/page.html")),
        Some(app.clone())
    );
    assert_eq!(
        service.matching_registration_for_client(&url("https://example.test/other/page.html")),
        Some(other.clone())
    );
    let registrations = service.all_registrations(&url("https://example.test/app/page.html"));
    assert_eq!(registrations.len(), 2);
    assert!(registrations.contains(&app));
    assert!(registrations.contains(&other));

    assert!(service.mark_registration_unregistered(other.scope_url()));
    assert_eq!(
        service.matching_registration_for_client(&url("https://example.test/other/page.html")),
        None
    );
    assert_eq!(
        service.all_registrations(&url("https://example.test/app/page.html")),
        vec![app]
    );
}
#[test]
fn resource_store_restores_registration_at_service_startup_for_client_observation() {
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
            .versions
            .get_mut(&version_id)
            .expect("version should exist")
            .main_script_resource = Some(test_script_resource(&script_url));
    }
    assert!(
        first_service
            .store_registration_resources_for_test(registration_id, version_id)
            .expect("stored registration resources should persist")
    );

    let second_service = new_service_worker_runtime_service_with_resource_store(
        resource_store,
        test_worker_context_runtime(),
    );
    assert_eq!(second_service.diagnostics_snapshot().registration_count, 1);

    let restored = second_service
        .matching_registration_for_client(&document_url)
        .expect("matching client query should see startup-restored registration");
    assert_eq!(restored.scope_url(), &scope_url);
    assert_eq!(second_service.diagnostics_snapshot().registration_count, 1);

    let mut ready_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    assert!(second_service.watch_ready_registration(
        document_url.clone(),
        5,
        1,
        ready_queue.sender(),
    ));
    assert!(ready_queue.has_ready_task());

    let client_id = second_service.register_client(document_url, 1, test_completion_sender());
    let control = second_service
        .matching_controller_for_client(client_id)
        .expect("new client should select restored active registration as controller");
    assert_eq!(control.scope_url(), &scope_url);
    assert_eq!(
        second_service
            .all_registrations(&url("https://example.test/app/page.html"))
            .len(),
        1
    );
}
#[test]
fn startup_restored_registrations_filter_client_observation_by_storage_key() {
    let resource_store = new_shared_service_worker_resource_store();
    let scope_url = url("https://example.test/app/");
    let document_url = url("https://example.test/app/page.html");
    let script_url = url("https://example.test/app/worker.js");
    let wrong_script_url = url("https://example.test/app/wrong-worker.js");
    resource_store
        .lock()
        .store_registration(ServiceWorkerStoredRegistration {
            storage_key: "https://other-top-level.example".to_string(),
            scope_url: scope_url.clone(),
            script_url: wrong_script_url.clone(),
            script_kind: WorkerScriptKind::Classic,
            update_via_cache: ServiceWorkerUpdateViaCache::Imports,
            navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
            lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
            fetch_handler_existence: ServiceWorkerFetchHandlerExistence::DoesNotExist,
            fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
            last_update_check_time_ms: Some(2),
            main_script_resource: test_script_resource(&wrong_script_url),
            imported_script_resources: std::collections::BTreeMap::new(),
        })
        .expect("wrong-key registration should store");
    resource_store
        .lock()
        .store_registration(ServiceWorkerStoredRegistration {
            storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&document_url),
            scope_url: scope_url.clone(),
            script_url: script_url.clone(),
            script_kind: WorkerScriptKind::Classic,
            update_via_cache: ServiceWorkerUpdateViaCache::Imports,
            navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
            lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
            fetch_handler_existence: ServiceWorkerFetchHandlerExistence::DoesNotExist,
            fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
            last_update_check_time_ms: Some(3),
            main_script_resource: test_script_resource(&script_url),
            imported_script_resources: std::collections::BTreeMap::new(),
        })
        .expect("matching-key registration should store");

    let service = new_service_worker_runtime_service_with_resource_store(
        resource_store,
        test_worker_context_runtime(),
    );
    assert_eq!(service.diagnostics_snapshot().registration_count, 2);

    let registrations = service.all_registrations(&document_url);
    assert_eq!(registrations.len(), 1);
    assert_eq!(
        registrations[0]
            .active()
            .expect("matching-key registration should be active")
            .script_url(),
        &script_url
    );

    let matching = service
        .matching_registration_for_client(&document_url)
        .expect("matching registration should ignore different storage keys");
    assert_eq!(
        matching
            .active()
            .expect("matching registration should be active")
            .script_url(),
        &script_url
    );

    let client_id = service.register_client(document_url, 1, test_completion_sender());
    let control = service
        .matching_controller_for_client(client_id)
        .expect("controller selection should ignore different storage keys");
    assert_eq!(control.script_url(), &script_url);
}
#[test]
fn client_observation_uses_explicit_partitioned_storage_key() {
    let resource_store = new_shared_service_worker_resource_store();
    let scope_url = url("https://cdn.example.test/app/");
    let document_url = url("https://cdn.example.test/app/page.html");
    let first_script_url = url("https://cdn.example.test/app/first-worker.js");
    let second_script_url = url("https://cdn.example.test/app/second-worker.js");
    let first_storage_key = moli_storage_key::MoliStorageKey::from_url_and_top_level_site(
        &document_url,
        moli_storage_key::site_for_url(&url("https://first-top.test/page.html")),
        None,
    )
    .serialized_storage_key();
    let second_storage_key = moli_storage_key::MoliStorageKey::from_url_and_top_level_site(
        &document_url,
        moli_storage_key::site_for_url(&url("https://second-top.test/page.html")),
        None,
    )
    .serialized_storage_key();
    assert_ne!(first_storage_key, second_storage_key);

    for (storage_key, script_url, check_time) in [
        (first_storage_key.clone(), first_script_url.clone(), 11),
        (second_storage_key.clone(), second_script_url.clone(), 12),
    ] {
        resource_store
            .lock()
            .store_registration(ServiceWorkerStoredRegistration {
                storage_key,
                scope_url: scope_url.clone(),
                script_url: script_url.clone(),
                script_kind: WorkerScriptKind::Classic,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::DoesNotExist,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                last_update_check_time_ms: Some(check_time),
                main_script_resource: test_script_resource(&script_url),
                imported_script_resources: std::collections::BTreeMap::new(),
            })
            .expect("partitioned registration should store");
    }

    let service = new_service_worker_runtime_service_with_resource_store(
        resource_store,
        test_worker_context_runtime(),
    );
    let first_match = service
        .matching_registration_for_client_with_storage_key(&document_url, &first_storage_key)
        .expect("first storage key should match first registration");
    assert_eq!(
        first_match
            .active()
            .expect("first registration should be active")
            .script_url(),
        &first_script_url
    );
    let second_match = service
        .matching_registration_for_client_with_storage_key(&document_url, &second_storage_key)
        .expect("second storage key should match second registration");
    assert_eq!(
        second_match
            .active()
            .expect("second registration should be active")
            .script_url(),
        &second_script_url
    );

    assert_eq!(
        service
            .all_registrations_with_storage_key(&document_url, &first_storage_key)
            .len(),
        1
    );
    let client_id = service.register_client_with_storage_key(
        document_url,
        second_storage_key,
        ServiceWorkerClientFrameType::TopLevel,
        Some(crate::native_bridge::WindowDocumentOwner::for_test(1)),
        test_completion_sender(),
    );
    let control = service
        .matching_controller_for_client(client_id)
        .expect("partitioned client should select matching-key controller");
    assert_eq!(control.script_url(), &second_script_url);
}
#[tokio::test]
async fn controlled_fetch_from_worker_client_dispatches_to_active_worker() {
    ensure_v8_for_test();
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/sw.js");
    let worker_script_url = url("https://example.test/app/dedicated-worker.js");
    let request_url = url("https://example.test/app/data.txt");
    let (bootstrap_tx, mut bootstrap_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::worker::WorkerBootstrapCompletion>();
    let mut handle = crate::worker::spawn_worker_with_options(
        crate::worker::WorkerSpawnOptions::new_with_request_client(
            r#"
                self.addEventListener("fetch", event => {
                    event.respondWith(new Response(JSON.stringify({
                        url: event.request.url,
                        clientId: event.clientId,
                        destination: event.request.destination,
                        mode: event.request.mode,
                        credentials: event.request.credentials,
                        redirect: event.request.redirect,
                        header: event.request.headers.get("x-worker-fetch")
                    }), {
                        status: 209,
                        statusText: "Worker Controlled",
                        headers: {
                            "content-type": "application/json",
                            "x-service-worker": "worker-client"
                        }
                    }));
                });
                "#
            .to_owned(),
            script_url.to_string(),
            service.request_client(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id,
            version_id,
            scope_url: scope_url.clone(),
        })
        .with_bootstrap_completion_sender(bootstrap_tx),
    );
    let bootstrap = tokio::time::timeout(Duration::from_secs(5), bootstrap_rx.recv())
        .await
        .expect("timed out waiting for service worker bootstrap")
        .expect("service worker bootstrap channel closed");
    bootstrap
        .result
        .expect("service worker bootstrap should succeed");
    let mut parent_rx = handle
        .take_receiver()
        .expect("service worker should expose parent receiver");
    let host = new_running_test_host_with_handle(version_id, &run, handle);
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        script_url.clone(),
        scope_url.clone(),
        std::iter::empty::<Url>(),
    );
    {
        let mut state = service.inner.state.lock();
        let version = state.versions.get_mut(&version_id).unwrap();
        version.fetch_handler_existence = ServiceWorkerFetchHandlerExistence::Exists;
        version.fetch_handler_type = ServiceWorkerFetchHandlerType::NotSkippable;
        version.running_state = ServiceWorkerVersionRunningState::Running { host };
        version.run = run.clone();
    }
    let storage_key =
        ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&worker_script_url);
    let (worker_tx, _worker_rx) = tokio::sync::mpsc::unbounded_channel();
    let client_id = service.register_worker_client_with_storage_key(
        worker_script_url.clone(),
        storage_key,
        ServiceWorkerClientType::DedicatedWorker,
        true,
        worker_tx,
    );
    assert!(
        service
            .matching_controller_for_client_fetch(client_id, &request_url)
            .is_some(),
        "registered worker client should be controlled by the active registration"
    );

    let mut completion_queue = async_subresource_completion_queue();
    let (direct_completion_tx, direct_completion_rx) = tokio::sync::oneshot::channel();
    assert!(
        service.dispatch_controlled_fetch(ServiceWorkerFetchDispatch {
            internal_id: 90,
            request: ServiceWorkerFetchRequest {
                client_id,
                resulting_client_id: None,
                url: request_url.clone(),
                method: "GET".to_owned(),
                headers: vec![("x-worker-fetch".to_owned(), "dedicated".to_owned())],
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
                request_origin: moli_url::WebOrigin::from_url(&worker_script_url),
                document_url: worker_script_url,
                resource_type: crate::types::SubresourceResourceType::Fetch,
                policy_context: Default::default(),
            },
            completion_tx: completion_queue.sender(),
            request_client: test_request_client(&service),
            resource_task_runner: test_resource_task_runner(),
            cancel_handle: moli_fetch::FetchCancelHandle::new(),
            direct_completion_tx: Some(direct_completion_tx),
        }),
        "controlled worker client fetch should dispatch to the active worker"
    );

    let completion = loop {
        let message = tokio::time::timeout(Duration::from_secs(5), parent_rx.recv())
            .await
            .expect("timed out waiting for service worker fetch completion")
            .expect("service worker parent channel closed");
        match message {
            crate::worker::WorkerToParentMessage::ServiceWorkerFetchCompleted(completion) => {
                break completion;
            }
            crate::worker::WorkerToParentMessage::Console(_) => {}
            other => panic!("unexpected service worker parent message: {other:?}"),
        }
    };
    service.finish_fetch_event_completed(completion);

    let direct_result = tokio::time::timeout(Duration::from_secs(5), direct_completion_rx)
        .await
        .expect("timed out waiting for direct service worker fetch completion")
        .expect("direct service worker fetch channel closed");
    let ServiceWorkerDirectFetchResult::Response(response) = direct_result else {
        panic!("expected direct service worker response, got {direct_result:?}");
    };
    assert_eq!(response.response.status, 209);
    assert_eq!(
        response
            .response
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("x-service-worker"))
            .map(|(_, value)| value.as_slice()),
        Some(b"worker-client".as_slice())
    );
    assert_eq!(
        response.response.body_text(),
        format!(
            r#"{{"url":"https://example.test/app/data.txt","clientId":"client-{client_id:016x}","destination":"","mode":"cors","credentials":"same-origin","redirect":"follow","header":"dedicated"}}"#,
            client_id = client_id.as_u64()
        )
    );
    assert!(!completion_queue.has_ready_completion());
}
#[test]
fn stopped_active_message_queues_event_and_starts_worker() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/sw.js"),
        url("https://example.test/app/"),
        [],
    );
    let client_id = register_client_for_test(&service, url("https://example.test/app/page.html"));
    {
        let mut state = service.inner.state.lock();
        state.live_clients.get_mut(&client_id).unwrap().focused = true;
    }
    let previous_run = exact_version_run(&service, version_id);

    assert!(service.dispatch_message_to_version(
        version_id,
        client_id,
        Some("https://example.test".to_owned()),
        V8StructuredClonePayload::default()
    ));

    let state = service.inner.state.lock();
    let version = state.versions.get(&version_id).unwrap();
    assert!(matches!(
        version.running_state,
        ServiceWorkerVersionRunningState::Starting { .. }
    ));
    assert_ne!(version.run, previous_run);
    let restarted_run = version.run.clone();
    assert_eq!(version.in_flight_event_count, 1);
    assert_eq!(version.pending_start_events.len(), 1);
    let ServiceWorkerPendingStartEvent::Message(event) =
        version.pending_start_events.front().unwrap()
    else {
        panic!("expected pending message event");
    };
    assert_eq!(event.owner, test_run_owner(version_id, &restarted_run));
    assert_eq!(event.source_client_id, Some(client_id));
    assert_eq!(
        event.source_client_snapshot,
        Some(ServiceWorkerClientSnapshot {
            id: client_id,
            exposed_id: service_worker_exposed_client_id(client_id),
            url: url("https://example.test/app/page.html"),
            client_type: ServiceWorkerClientType::Window,
            frame_type: ServiceWorkerClientFrameType::TopLevel,
            visibility_state: ServiceWorkerClientVisibilityState::Visible,
            controlled: true,
            focused: true,
        })
    );
}
#[test]
fn message_to_redundant_or_missing_version_is_dropped_silently() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/sw.js"),
        url("https://example.test/app/"),
        [],
    );
    {
        let mut state = service.inner.state.lock();
        let version = state.versions.get_mut(&version_id).unwrap();
        version.lifecycle_state = ServiceWorkerVersionLifecycleState::Redundant;
    }
    let client_id = register_client_for_test(&service, url("https://example.test/app/page.html"));

    assert!(!service.dispatch_message_to_version(
        ServiceWorkerVersionId(404),
        client_id,
        Some("https://example.test".to_owned()),
        V8StructuredClonePayload::default()
    ));
    assert!(!service.dispatch_message_to_version(
        version_id,
        client_id,
        Some("https://example.test".to_owned()),
        V8StructuredClonePayload::default()
    ));

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.in_flight_event_count, 0);
    assert_eq!(diagnostics.pending_service_lane_event_count, 0);
}
#[test]
fn message_completion_releases_event_accounting_and_can_idle_stop() {
    let service = new_service_worker_runtime_service();
    service.set_idle_delay_for_test(Duration::ZERO);
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/sw.js");
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
                in_flight_event_count: 1,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    service.finish_message_event_completed(ServiceWorkerMessageCompletion {
        event_id: ServiceWorkerEventId(7),
        owner: test_run_owner(version_id, &run),
        result: Ok(()),
    });

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.in_flight_event_count, 0);
    assert_eq!(diagnostics.running_version_count, 1);
    assert_eq!(diagnostics.pending_service_lane_event_count, 1);
    assert_eq!(service.drain_service_lane(), 1);
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.running_version_count, 0);
    assert_eq!(diagnostics.stopped_version_count, 1);
}
#[test]
fn notification_action_navigate_routes_to_page_owner_without_click_event() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/sw.js");
    let action_url = url("https://example.test/app/reply.html");
    let host = new_running_test_host(version_id, &run);
    let mut completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let client_id = ServiceWorkerClientId(9);
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
                document_owner: Some(crate::native_bridge::WindowDocumentOwner::for_test(7)),
                endpoint: ServiceWorkerClientEndpoint::Page(completion_queue.sender()),
                focused: false,
            },
        );
    }

    assert!(service.show_notification_for_scope(
        &scope_url,
        "hello".to_owned(),
        String::new(),
        ServiceWorkerNotificationMetadata::default(),
        vec![ServiceWorkerNotificationAction {
            action: "reply".to_owned(),
            title: "Reply".to_owned(),
            icon: String::new(),
            navigate: Some(action_url.clone()),
        }],
        V8StructuredClonePayload::default(),
    ));
    assert!(service.dispatch_notification_click_for_scope(
        &scope_url,
        "hello".to_owned(),
        "reply".to_owned(),
    ));

    let Some(
            crate::page_task_queue::RendererServiceWorkerInternalTask::NotificationActionNavigateRequest(
                completion,
            ),
        ) = completion_queue.pop_internal()
        else {
            panic!("expected notification action navigate request");
        };
    assert_eq!(completion.host.client_id, client_id);
    assert_eq!(
        completion.host.document_owner,
        crate::native_bridge::WindowDocumentOwner::for_test(7)
    );
    assert_eq!(completion.url, action_url);
    let state = service.inner.state.lock();
    let version = state.versions.get(&version_id).unwrap();
    assert_eq!(version.in_flight_event_count, 0);
}
