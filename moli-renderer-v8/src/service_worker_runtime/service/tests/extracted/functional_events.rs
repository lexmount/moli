use super::*;

#[test]
fn pending_unregistration_prunes_sync_and_periodic_sync_records() {
    let service = new_service_worker_runtime_service();
    let first_registration_id = ServiceWorkerRegistrationId(1);
    let second_registration_id = ServiceWorkerRegistrationId(2);
    let first_scope_url = url("https://example.test/app/");
    let second_scope_url = url("https://example.test/other/");
    insert_registered_version(
        &service,
        first_registration_id,
        ServiceWorkerVersionId(1),
        url("https://example.test/app/worker.js"),
        first_scope_url.clone(),
        [],
    );
    insert_registered_version(
        &service,
        second_registration_id,
        ServiceWorkerVersionId(2),
        url("https://example.test/other/worker.js"),
        second_scope_url,
        [],
    );
    {
        let mut state = service.inner.state.lock();
        state.sync_registrations.insert(
            (first_registration_id, "first-sync".to_owned()),
            ServiceWorkerSyncRegistrationRecord {
                failed_attempts: 1,
                ..Default::default()
            },
        );
        state.sync_registrations.insert(
            (second_registration_id, "second-sync".to_owned()),
            ServiceWorkerSyncRegistrationRecord {
                failed_attempts: 0,
                ..Default::default()
            },
        );
        state.periodic_sync_registrations.insert(
            (first_registration_id, "first-periodic".to_owned()),
            ServiceWorkerPeriodicSyncRegistrationRecord::new(10),
        );
        state.periodic_sync_registrations.insert(
            (second_registration_id, "second-periodic".to_owned()),
            ServiceWorkerPeriodicSyncRegistrationRecord::new(20),
        );
    }

    assert!(service.mark_registration_unregistered(&first_scope_url));

    let state = service.inner.state.lock();
    assert!(
        state
            .sync_registrations
            .keys()
            .all(|(registration_id, _)| *registration_id != first_registration_id)
    );
    assert!(
        state
            .periodic_sync_registrations
            .keys()
            .all(|(registration_id, _)| *registration_id != first_registration_id)
    );
    assert!(
        state
            .sync_registrations
            .contains_key(&(second_registration_id, "second-sync".to_owned()))
    );
    assert!(
        state
            .periodic_sync_registrations
            .contains_key(&(second_registration_id, "second-periodic".to_owned()))
    );
}
#[tokio::test]
async fn stale_notification_sync_periodic_and_push_owner_requests_reject_worker_promises() {
    ensure_v8_for_test();
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let scope_url = url("https://example.test/app/");
    let script_url = url("https://example.test/app/sw.js");
    let mut handle = crate::worker::spawn_worker_with_options(
        crate::worker::WorkerSpawnOptions::new_with_request_client(
            r#"
self.addEventListener("message", event => {
  event.waitUntil((async () => {
    async function expectTypeError(label, promise) {
      let caught = null;
      try {
        await promise;
      } catch (error) {
        caught = {
          name: error && error.name,
          message: error && error.message
        };
      }
      if (!caught || caught.name !== "TypeError" ||
          !String(caught.message).includes("stale")) {
        throw new Error(label + " did not reject with stale TypeError: " + JSON.stringify(caught));
      }
    }

    await expectTypeError("sync.register", self.registration.sync.register("stale-sync"));
    await expectTypeError("sync.getTags", self.registration.sync.getTags());
    await expectTypeError("showNotification", self.registration.showNotification("stale"));
    await expectTypeError("getNotifications", self.registration.getNotifications());
    await expectTypeError("periodicSync.register",
      self.registration.periodicSync.register("stale-periodic", { minInterval: 60000 }));
    await expectTypeError("periodicSync.getTags", self.registration.periodicSync.getTags());
    await expectTypeError("periodicSync.unregister",
      self.registration.periodicSync.unregister("stale-periodic"));
    await expectTypeError("pushManager.getSubscription",
      self.registration.pushManager.getSubscription());
    await expectTypeError("pushManager.subscribe",
      self.registration.pushManager.subscribe({ userVisibleOnly: true }));

    const subscription = await self.registration.pushManager.subscribe({ userVisibleOnly: true });
    if (!subscription || typeof subscription.unsubscribe !== "function") {
      throw new Error("pushManager.subscribe did not return a PushSubscription");
    }
    await expectTypeError("PushSubscription.unsubscribe", subscription.unsubscribe());
  })());
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
        .with_network_policy(WorkerNetworkPolicy {
            permission_overrides: vec![
                crate::protocol_types::PermissionOverrideRegistration {
                    permission: serde_json::Value::String("background-sync".to_owned()),
                    setting: "granted".to_owned(),
                    origin: None,
                    embedded_origin: None,
                },
                crate::protocol_types::PermissionOverrideRegistration {
                    permission: serde_json::Value::String("periodic-background-sync".to_owned()),
                    setting: "granted".to_owned(),
                    origin: None,
                    embedded_origin: None,
                },
                crate::protocol_types::PermissionOverrideRegistration {
                    permission: serde_json::Value::String("notifications".to_owned()),
                    setting: "granted".to_owned(),
                    origin: None,
                    embedded_origin: None,
                },
            ],
            ..WorkerNetworkPolicy::default()
        }),
    );
    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(61),
        owner: test_run_owner(version_id, &run),
        source_client_id: None,
        source_client_url: None,
        source_client_snapshot: None,
        source_worker: None,
        source_origin: String::new(),
        payload: serialize_test_string("stale-requests"),
        window_interaction_allowed: false,
    });
    let first_message = tokio::time::timeout(Duration::from_secs(5), handle.recv())
        .await
        .expect("timed out waiting for first service worker owner request")
        .expect("service worker parent channel closed before first request");
    let mut parent_rx = handle
        .take_receiver()
        .expect("service worker should expose parent receiver");
    let host = new_running_test_host_with_handle(version_id, &run, handle);
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
                running_state: ServiceWorkerVersionRunningState::Running { host: host.clone() },
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

    let stale_run = RendererServiceWorkerRunIdentity::fresh();
    let mut stale_rejection_count = 0;
    let mut push_subscribe_count = 0;
    let mut first_message = Some(first_message);
    loop {
        let message = match first_message.take() {
            Some(message) => message,
            None => tokio::time::timeout(Duration::from_secs(5), parent_rx.recv())
                .await
                .expect("timed out waiting for stale owner request settlement")
                .expect("service worker parent channel closed"),
        };
        match message {
            crate::worker::WorkerToParentMessage::ServiceWorkerSyncRegistration(request) => {
                service.finish_sync_registration_requested(
                    request,
                    stale_run.clone(),
                    host.clone(),
                );
                stale_rejection_count += 1;
            }
            crate::worker::WorkerToParentMessage::ServiceWorkerSyncGetTags(request) => {
                service.finish_sync_get_tags_requested(request, stale_run.clone(), host.clone());
                stale_rejection_count += 1;
            }
            crate::worker::WorkerToParentMessage::ServiceWorkerGetNotifications(request) => {
                service.finish_get_notifications_requested(
                    request,
                    stale_run.clone(),
                    host.clone(),
                );
                stale_rejection_count += 1;
            }
            crate::worker::WorkerToParentMessage::ServiceWorkerShowNotification(request) => {
                service.finish_show_notification_requested(
                    request,
                    stale_run.clone(),
                    host.clone(),
                );
                stale_rejection_count += 1;
            }
            crate::worker::WorkerToParentMessage::ServiceWorkerPeriodicSyncRegistration(
                request,
            ) => {
                service.finish_periodic_sync_registration_requested(
                    request,
                    stale_run.clone(),
                    host.clone(),
                );
                stale_rejection_count += 1;
            }
            crate::worker::WorkerToParentMessage::ServiceWorkerPeriodicSyncGetTags(request) => {
                service.finish_periodic_sync_get_tags_requested(
                    request,
                    stale_run.clone(),
                    host.clone(),
                );
                stale_rejection_count += 1;
            }
            crate::worker::WorkerToParentMessage::ServiceWorkerPeriodicSyncUnregistration(
                request,
            ) => {
                service.finish_periodic_sync_unregistration_requested(
                    request,
                    stale_run.clone(),
                    host.clone(),
                );
                stale_rejection_count += 1;
            }
            crate::worker::WorkerToParentMessage::ServiceWorkerPushGetSubscription(request) => {
                service.finish_push_get_subscription_requested(
                    request,
                    stale_run.clone(),
                    host.clone(),
                );
                stale_rejection_count += 1;
            }
            crate::worker::WorkerToParentMessage::ServiceWorkerPushSubscribe(request) => {
                push_subscribe_count += 1;
                if push_subscribe_count == 1 {
                    service.finish_push_subscribe_requested(
                        request,
                        stale_run.clone(),
                        host.clone(),
                    );
                    stale_rejection_count += 1;
                } else {
                    service.finish_push_subscribe_requested(request, run.clone(), host.clone());
                }
            }
            crate::worker::WorkerToParentMessage::ServiceWorkerPushUnsubscribe(request) => {
                service.finish_push_unsubscribe_requested(request, stale_run.clone(), host.clone());
                stale_rejection_count += 1;
            }
            crate::worker::WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(61)
                );
                assert_eq!(completion.result, Ok(()));
                assert_eq!(stale_rejection_count, 10);
                assert_eq!(push_subscribe_count, 2);
                break;
            }
            crate::worker::WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            crate::worker::WorkerToParentMessage::RuntimeInspectorMessages(_) => {}
            other => panic!("unexpected worker message: {other:?}"),
        }
    }
    host.terminate_without_join();
}
#[test]
fn show_notification_record_enables_scope_click_dispatch() {
    let service = new_service_worker_runtime_service();
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
                in_flight_event_count: 0,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    assert!(!service.dispatch_notification_click_for_scope(
        &scope_url,
        "hello".to_owned(),
        "open".to_owned(),
    ));

    assert!(service.show_notification_for_scope(
        &scope_url,
        "hello".to_owned(),
        String::new(),
        ServiceWorkerNotificationMetadata::default(),
        Vec::new(),
        V8StructuredClonePayload::default(),
    ));
    {
        let state = service.inner.state.lock();
        assert_eq!(state.notification_records.len(), 1);
        assert_eq!(
            state.notification_records[0].registration_id,
            registration_id
        );
        assert_eq!(state.notification_records[0].title, "hello");
    }

    assert!(service.dispatch_notification_click_for_scope(
        &scope_url,
        "hello".to_owned(),
        "open".to_owned(),
    ));
    let state = service.inner.state.lock();
    let version = state.versions.get(&version_id).unwrap();
    assert_eq!(version.in_flight_event_count, 1);
}
#[test]
fn push_dispatch_for_active_scope_uses_service_lane_completion() {
    let service = new_service_worker_runtime_service();
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
                in_flight_event_count: 0,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    assert!(service.dispatch_push_for_scope(&scope_url, Some(b"payload".to_vec())));
    {
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 1);
    }

    assert_eq!(service.drain_service_lane(), 1);
    let state = service.inner.state.lock();
    let version = state.versions.get(&version_id).unwrap();
    assert_eq!(version.in_flight_event_count, 0);
    assert_eq!(
        version.last_start_error.as_deref(),
        Some("service worker push dispatch failed: worker is not running")
    );
}
#[test]
fn push_subscription_store_tracks_active_scope_subscription() {
    let service = new_service_worker_runtime_service();
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
                in_flight_event_count: 0,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    assert!(service.push_subscription_for_scope(&scope_url).is_none());
    let subscription = service
        .subscribe_push_for_scope(&scope_url, true)
        .expect("active registration should accept push subscription");
    assert_eq!(
        subscription.endpoint,
        "https://moli.invalid/service-worker/push/1"
    );
    assert!(subscription.user_visible_only);
    assert_eq!(
        service.push_subscription_for_scope(&scope_url),
        Some(subscription)
    );
    assert!(service.unsubscribe_push_for_scope(&scope_url));
    assert!(service.push_subscription_for_scope(&scope_url).is_none());
    assert!(!service.unsubscribe_push_for_scope(&scope_url));
    assert!(
        service
            .subscribe_push_for_scope(&url("https://example.test/other/"), true)
            .is_none()
    );
}
#[test]
fn sync_register_for_active_scope_retains_tag_until_success_or_last_chance_failure() {
    let service = new_service_worker_runtime_service();
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
                in_flight_event_count: 0,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    assert!(service.register_sync_for_scope(&scope_url, "sync-tag".to_owned()));
    assert_eq!(service.sync_tags_for_scope(&scope_url), vec!["sync-tag"]);
    {
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 1);
    }

    assert_eq!(service.drain_service_lane(), 1);
    assert_eq!(
        service.sync_tags_for_scope(&scope_url),
        vec!["sync-tag"],
        "a failed first sync attempt should keep the registration for retry"
    );
    let state = service.inner.state.lock();
    let version = state.versions.get(&version_id).unwrap();
    assert_eq!(
        version.in_flight_event_count, 1,
        "first failure should immediately schedule a lastChance retry"
    );
    assert_eq!(
        version.last_start_error.as_deref(),
        Some("service worker sync dispatch failed: worker is not running")
    );
    assert_eq!(
        state
            .sync_registrations
            .get(&(registration_id, "sync-tag".to_owned()))
            .map(|record| record.failed_attempts),
        Some(1)
    );
    drop(state);
    assert_eq!(service.pending_service_lane_event_count(), 1);

    assert_eq!(service.drain_service_lane(), 1);
    assert!(service.sync_tags_for_scope(&scope_url).is_empty());
    let state = service.inner.state.lock();
    let version = state.versions.get(&version_id).unwrap();
    assert_eq!(version.in_flight_event_count, 0);
    assert_eq!(
        version.last_start_error.as_deref(),
        Some("service worker sync dispatch failed: worker is not running")
    );
}
#[test]
fn sync_registration_request_deduplicates_active_tag_and_refires_after_finish() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let sync_key = (registration_id, "sync-tag".to_owned());
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
                running_state: ServiceWorkerVersionRunningState::Running { host: host.clone() },
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

    for request_id in [1, 2] {
        service.finish_sync_registration_requested(
            ServiceWorkerSyncRegistration {
                request_id,
                registration_id,
                version_id,
                tag: "sync-tag".to_owned(),
            },
            run.clone(),
            host.clone(),
        );
    }
    {
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 1);
        let record = state.sync_registrations.get(&sync_key).unwrap();
        assert_eq!(record.failed_attempts, 0);
        assert!(matches!(
            record.dispatch_state,
            ServiceWorkerTagDispatchState::Active {
                refire_after_finish: true,
                ..
            }
        ));
    }
    assert_eq!(
        service.pending_service_lane_event_count(),
        1,
        "duplicate register must not dispatch a second active sync event"
    );

    assert_eq!(service.drain_service_lane(), 1);
    {
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 1);
        let record = state.sync_registrations.get(&sync_key).unwrap();
        assert_eq!(record.failed_attempts, 0);
        assert!(matches!(
            record.dispatch_state,
            ServiceWorkerTagDispatchState::Active {
                refire_after_finish: false,
                ..
            }
        ));
    }
    assert_eq!(
        service.pending_service_lane_event_count(),
        1,
        "refire should schedule one normal follow-up sync event"
    );

    assert_eq!(service.drain_service_lane(), 1);
    {
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 1);
        let record = state.sync_registrations.get(&sync_key).unwrap();
        assert_eq!(record.failed_attempts, 1);
        assert!(matches!(
            record.dispatch_state,
            ServiceWorkerTagDispatchState::Active {
                refire_after_finish: false,
                ..
            }
        ));
    }
    assert_eq!(
        service.pending_service_lane_event_count(),
        1,
        "the refired sync failure should still get one lastChance retry"
    );

    assert_eq!(service.drain_service_lane(), 1);
    let state = service.inner.state.lock();
    let version = state.versions.get(&version_id).unwrap();
    assert_eq!(version.in_flight_event_count, 0);
    assert!(!state.sync_registrations.contains_key(&sync_key));
}
#[test]
fn sync_retry_marks_failed_registration_as_last_chance_event() {
    let service = new_service_worker_runtime_service();
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
                running_state: ServiceWorkerVersionRunningState::Starting { host },
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
        state.sync_registrations.insert(
            (registration_id, "sync-tag".to_owned()),
            ServiceWorkerSyncRegistrationRecord {
                failed_attempts: 1,
                ..Default::default()
            },
        );
    }

    assert!(service.retry_sync_for_scope(&scope_url, "sync-tag"));
    let state = service.inner.state.lock();
    let version = state.versions.get(&version_id).unwrap();
    assert_eq!(version.in_flight_event_count, 1);
    let Some(ServiceWorkerPendingStartEvent::Sync(event)) = version.pending_start_events.back()
    else {
        panic!("expected queued sync retry event");
    };
    assert_eq!(event.tag, "sync-tag");
    assert!(event.last_chance);
    assert_eq!(event.owner, test_run_owner(version_id, &run));
}
#[test]
fn periodic_sync_register_get_tags_and_unregister_use_owner_store() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let scope_url = url("https://example.test/app/");
    insert_registered_version(
        &service,
        registration_id,
        version_id,
        url("https://example.test/app/sw.js"),
        scope_url.clone(),
        [],
    );

    assert!(service.register_periodic_sync_for_scope(&scope_url, "daily".to_owned(), 86_400_000,));
    assert!(service.register_periodic_sync_for_scope(&scope_url, "hourly".to_owned(), 3_600_000,));
    assert_eq!(
        service.periodic_sync_tags_for_scope(&scope_url),
        vec!["daily", "hourly"]
    );

    assert!(service.register_periodic_sync_for_scope(&scope_url, "daily".to_owned(), 172_800_000,));
    {
        let state = service.inner.state.lock();
        assert_eq!(
            state
                .periodic_sync_registrations
                .get(&(registration_id, "daily".to_owned()))
                .map(|record| record.min_interval_ms),
            Some(172_800_000)
        );
    }

    assert!(service.unregister_periodic_sync_for_scope(&scope_url, "daily"));
    assert_eq!(
        service.periodic_sync_tags_for_scope(&scope_url),
        vec!["hourly"]
    );

    let inactive_scope = url("https://example.test/inactive/");
    insert_inactive_registration(
        &service,
        ServiceWorkerRegistrationId(2),
        ServiceWorkerVersionId(2),
        url("https://example.test/inactive/sw.js"),
        inactive_scope.clone(),
    );
    assert!(!service.register_periodic_sync_for_scope(&inactive_scope, "inactive".to_owned(), 1,));
    assert!(
        service
            .periodic_sync_tags_for_scope(&inactive_scope)
            .is_empty()
    );
}
#[test]
fn periodic_sync_dispatch_for_registered_tag_queues_functional_event() {
    let service = new_service_worker_runtime_service();
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
                running_state: ServiceWorkerVersionRunningState::Starting { host },
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

    assert!(service.register_periodic_sync_for_scope(&scope_url, "daily".to_owned(), 86_400_000,));
    assert!(!service.dispatch_periodic_sync_for_scope(&scope_url, "missing"));
    assert!(service.dispatch_periodic_sync_for_scope(&scope_url, "daily"));
    {
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 1);
        let Some(ServiceWorkerPendingStartEvent::PeriodicSync(event)) =
            version.pending_start_events.back()
        else {
            panic!("expected queued periodic sync event");
        };
        assert_eq!(event.registration_id, registration_id);
        assert_eq!(event.owner, test_run_owner(version_id, &run));
        assert_eq!(event.tag, "daily");
    }

    service.finish_worker_start_failed(
        version_id,
        run,
        ServiceWorkerVersionStartFailure::ScriptLoad {
            message: "worker start failed".to_owned(),
        },
    );
    let state = service.inner.state.lock();
    let version = state.versions.get(&version_id).unwrap();
    assert_eq!(version.in_flight_event_count, 0);
    assert_eq!(
        version.last_start_error.as_deref(),
        Some("periodicsync `daily` failed: worker start failed")
    );
    assert!(
        state
            .periodic_sync_registrations
            .contains_key(&(registration_id, "daily".to_owned())),
        "functional dispatch should not remove the periodic sync registration"
    );
}
#[test]
fn periodic_sync_dispatch_deduplicates_active_tag_and_refires_after_finish() {
    let service = new_service_worker_runtime_service();
    let registration_id = ServiceWorkerRegistrationId(1);
    let version_id = ServiceWorkerVersionId(1);
    let run = RendererServiceWorkerRunIdentity::fresh();
    let periodic_key = (registration_id, "daily".to_owned());
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
                in_flight_event_count: 0,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    assert!(service.register_periodic_sync_for_scope(&scope_url, "daily".to_owned(), 86_400_000,));
    assert!(service.dispatch_periodic_sync_for_scope(&scope_url, "daily"));
    assert!(service.dispatch_periodic_sync_for_scope(&scope_url, "daily"));
    {
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 1);
        let record = state
            .periodic_sync_registrations
            .get(&periodic_key)
            .unwrap();
        assert!(matches!(
            record.dispatch_state,
            ServiceWorkerTagDispatchState::Active {
                refire_after_finish: true,
                ..
            }
        ));
    }
    assert_eq!(
        service.pending_service_lane_event_count(),
        1,
        "duplicate periodic sync dispatch must not enqueue a concurrent event"
    );

    assert_eq!(service.drain_service_lane(), 1);
    {
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 1);
        let record = state
            .periodic_sync_registrations
            .get(&periodic_key)
            .unwrap();
        assert_eq!(record.min_interval_ms, 86_400_000);
        assert!(matches!(
            record.dispatch_state,
            ServiceWorkerTagDispatchState::Active {
                refire_after_finish: false,
                ..
            }
        ));
    }
    assert_eq!(
        service.pending_service_lane_event_count(),
        1,
        "refire should schedule one follow-up periodic sync event"
    );

    assert_eq!(service.drain_service_lane(), 1);
    let state = service.inner.state.lock();
    let version = state.versions.get(&version_id).unwrap();
    assert_eq!(version.in_flight_event_count, 0);
    let record = state
        .periodic_sync_registrations
        .get(&periodic_key)
        .unwrap();
    assert!(matches!(
        record.dispatch_state,
        ServiceWorkerTagDispatchState::Idle
    ));
}
#[test]
fn notification_store_filters_replaces_tagged_records_and_closes() {
    let service = new_service_worker_runtime_service();
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
                in_flight_event_count: 0,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    assert!(service.show_notification_for_scope(
        &scope_url,
        "first".to_owned(),
        "same".to_owned(),
        ServiceWorkerNotificationMetadata {
            body: "old body".to_owned(),
            timestamp: Some(111),
            ..ServiceWorkerNotificationMetadata::default()
        },
        Vec::new(),
        V8StructuredClonePayload::default(),
    ));
    assert!(service.show_notification_for_scope(
        &scope_url,
        "second".to_owned(),
        "same".to_owned(),
        ServiceWorkerNotificationMetadata {
            dir: "rtl".to_owned(),
            lang: "fr".to_owned(),
            body: "new body".to_owned(),
            icon: "/icon.png".to_owned(),
            image: "/image.png".to_owned(),
            badge: "/badge.png".to_owned(),
            vibrate: vec![10, 20],
            timestamp: Some(222),
            renotify: true,
            silent: Some(true),
            require_interaction: true,
        },
        vec![ServiceWorkerNotificationAction {
            action: "reply".to_owned(),
            title: "Reply".to_owned(),
            icon: "/reply.png".to_owned(),
            navigate: None,
        }],
        V8StructuredClonePayload::default(),
    ));
    assert!(service.show_notification_for_scope(
        &scope_url,
        "loose".to_owned(),
        String::new(),
        ServiceWorkerNotificationMetadata::default(),
        Vec::new(),
        V8StructuredClonePayload::default(),
    ));

    let all = service.notifications_for_scope(&scope_url, None);
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].title, "second");
    assert_eq!(all[0].tag, "same");
    assert_eq!(all[0].actions.len(), 1);
    assert_eq!(all[0].actions[0].action, "reply");
    assert_eq!(all[0].actions[0].title, "Reply");
    assert_eq!(all[0].actions[0].icon, "/reply.png");
    assert_eq!(all[0].metadata.dir, "rtl");
    assert_eq!(all[0].metadata.lang, "fr");
    assert_eq!(all[0].metadata.body, "new body");
    assert_eq!(all[0].metadata.icon, "/icon.png");
    assert_eq!(all[0].metadata.image, "/image.png");
    assert_eq!(all[0].metadata.badge, "/badge.png");
    assert_eq!(all[0].metadata.vibrate, vec![10, 20]);
    assert_eq!(all[0].metadata.timestamp, Some(222));
    assert!(all[0].metadata.renotify);
    assert_eq!(all[0].metadata.silent, Some(true));
    assert!(all[0].metadata.require_interaction);
    assert_eq!(all[1].title, "loose");

    let tagged = service.notifications_for_scope(&scope_url, Some("same"));
    assert_eq!(tagged.len(), 1);
    assert_eq!(tagged[0].title, "second");
    assert!(service.close_notification(registration_id, tagged[0].id));

    let tagged_after_close = service.notifications_for_scope(&scope_url, Some("same"));
    assert!(tagged_after_close.is_empty());
    let all_after_close = service.notifications_for_scope(&scope_url, None);
    assert_eq!(all_after_close.len(), 1);
    assert_eq!(all_after_close[0].title, "loose");
}
#[test]
fn notification_close_dispatch_removes_record_and_starts_event() {
    let service = new_service_worker_runtime_service();
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
                in_flight_event_count: 0,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    assert!(service.show_notification_for_scope(
        &scope_url,
        "closing".to_owned(),
        "tag".to_owned(),
        ServiceWorkerNotificationMetadata::default(),
        Vec::new(),
        V8StructuredClonePayload::default(),
    ));
    assert_eq!(
        service
            .notifications_for_scope(&scope_url, Some("tag"))
            .len(),
        1
    );
    assert!(service.dispatch_notification_close_for_scope(&scope_url, "closing".to_owned(),));

    assert!(
        service
            .notifications_for_scope(&scope_url, Some("tag"))
            .is_empty()
    );
    let state = service.inner.state.lock();
    let version = state.versions.get(&version_id).unwrap();
    assert_eq!(version.in_flight_event_count, 1);
}
#[test]
fn notification_click_dispatch_enters_active_version_accounting() {
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
                in_flight_event_count: 0,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    assert!(service.dispatch_notification_click_event(
        registration_id,
        1,
        "hello".to_owned(),
        String::new(),
        ServiceWorkerNotificationMetadata::default(),
        Vec::new(),
        "open".to_owned(),
        V8StructuredClonePayload::default(),
    ));

    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.in_flight_event_count, 1);
    assert_eq!(diagnostics.pending_service_lane_event_count, 1);

    assert_eq!(service.drain_service_lane(), 1);
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.in_flight_event_count, 0);
    assert_eq!(diagnostics.running_version_count, 1);
    assert_eq!(diagnostics.pending_service_lane_event_count, 1);

    assert_eq!(service.drain_service_lane(), 1);
    let diagnostics = service.diagnostics_snapshot();
    assert_eq!(diagnostics.running_version_count, 0);
    assert_eq!(diagnostics.stopped_version_count, 1);
}
