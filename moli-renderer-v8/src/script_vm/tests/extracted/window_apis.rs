use super::*;

#[test]
fn opfs_completion_rechecks_named_bucket_liveness_before_settlement() {
    let origin = "https://stale-opfs-completion.test/";
    let mut vm = new_storage_test_vm(origin);
    let storage_key = moli_storage_key::MoliStorageKey::first_party_from_url(
        &url::Url::parse(origin).unwrap(),
        None,
    )
    .serialized_storage_key();
    let locator = {
        let mut store = vm.storage_bucket_store.lock();
        store.open_bucket(&storage_key, "stale-read").unwrap();
        store
            .bucket_locator(&storage_key, "stale-read")
            .expect("named bucket locator")
    };
    let completion = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let resolver = v8::PromiseResolver::new(scope).expect("OPFS resolver");
            let promise = resolver.get_promise(scope);
            assert_eq!(
                scope.get_current_context().global(scope).set(
                    scope,
                    crate::util::v8str(scope, "__staleOpfsPromise").into(),
                    promise.into(),
                ),
                Some(true)
            );
            let (_, completion) = unsafe { &mut *host_ptr }
                .register_pending_opfs_task(scope, resolver, locator.clone(), None)
                .expect("OPFS task should capture its Window execution context");
            Ok(completion)
        })
        .expect("OPFS task should register");
    vm.eval(
        r#"
        globalThis.__staleOpfsResult = "pending";
        globalThis.__staleOpfsPromise.then(
          () => { globalThis.__staleOpfsResult = "resolved"; },
          error => { globalThis.__staleOpfsResult = error && error.name; }
        );
        "attached"
        "#,
    )
    .expect("OPFS rejection reaction should attach");

    vm.storage_bucket_store
        .lock()
        .delete_bucket(&storage_key, "stale-read")
        .expect("bucket delete should persist its tombstone")
        .expect("named bucket should exist");
    completion
        .send(crate::opfs_task_result::OpfsTaskResult::GetFile(
            crate::opfs_task_result::OpfsGetFileTaskResult {
                result: Ok(Ok(crate::opfs_task_result::OpfsReadFileResult {
                    path: moli_storage_service::OpfsPath::from_components(vec![
                        "stale.txt".to_owned(),
                    ])
                    .expect("valid OPFS path"),
                    snapshot: moli_storage_service::FileSnapshot {
                        name: "stale.txt".to_owned(),
                        modified_ms: 1,
                        bytes: b"must not escape".to_vec(),
                        identity: moli_storage_service::FileSnapshotIdentity::from_raw(1, 1)
                            .expect("test snapshot identity should be non-zero"),
                    },
                })),
            },
        ))
        .expect("stale OPFS completion should enter its production Page source");
    assert!(
        vm.run_opfs_task_body_for_authorization_test()
            .expect("stale OPFS completion body should settle"),
        "the typed OPFS source should contain the exact pending completion"
    );
    assert_eq!(
        vm.eval_without_microtask_checkpoint_for_test("globalThis.__staleOpfsResult")
            .expect("OPFS body-only reaction state"),
        "pending",
        "the body-only OPFS support must leave Promise reactions to the selected-task checkpoint"
    );
    vm.with_default_context_scope_and_checkpoint_for_test(|_scope, _host_ptr| Ok(()))
        .expect("OPFS rejection checkpoint should run");

    assert_eq!(
        vm.eval("globalThis.__staleOpfsResult")
            .expect("stale OPFS outcome"),
        "NotFoundError"
    );
    assert_eq!(vm._context_host.borrow().pending_opfs_task_count(), 0);
}
#[test]
fn window_opfs_owner_state_materializes_only_after_first_opfs_operation() {
    let mut vm = new_storage_page_task_executor_test_vm("https://opfs-owner-state-lazy.test/");

    let navigator_diagnostics = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            crate::context_bootstrap::navigator_storage_wrapper_diagnostics(scope)
                .ok_or_else(|| anyhow::anyhow!("Navigator diagnostics are unavailable"))
        })
        .expect("initial Navigator diagnostics");
    assert!(!navigator_diagnostics.storage_manager_materialized);
    assert!(!navigator_diagnostics.storage_bucket_manager_materialized);
    assert!(
        !vm._context_host.borrow().has_opfs_owner_state(),
        "creating a Window realm must not eagerly allocate OPFS owner state"
    );
    assert_eq!(
        vm.eval("navigator.storage === navigator.storage")
            .expect("StorageManager SameObject probe"),
        "true"
    );
    let navigator_diagnostics = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            crate::context_bootstrap::navigator_storage_wrapper_diagnostics(scope)
                .ok_or_else(|| anyhow::anyhow!("Navigator diagnostics are unavailable"))
        })
        .expect("post-storage Navigator diagnostics");
    assert!(navigator_diagnostics.storage_manager_materialized);
    assert!(!navigator_diagnostics.storage_bucket_manager_materialized);
    assert_eq!(
        vm.eval("navigator.storageBuckets === navigator.storageBuckets")
            .expect("StorageBucketManager SameObject probe"),
        "true"
    );
    let navigator_diagnostics = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            crate::context_bootstrap::navigator_storage_wrapper_diagnostics(scope)
                .ok_or_else(|| anyhow::anyhow!("Navigator diagnostics are unavailable"))
        })
        .expect("post-storageBuckets Navigator diagnostics");
    assert!(navigator_diagnostics.storage_manager_materialized);
    assert!(navigator_diagnostics.storage_bucket_manager_materialized);
    assert!(
        !vm._context_host.borrow().has_opfs_owner_state(),
        "reading navigator storage wrappers must not allocate OPFS owner state"
    );

    vm.exec(
        r#"
        globalThis.__lazyOpfsRoot = "pending";
        navigator.storage.getDirectory().then(root => {
          globalThis.__lazyOpfsRoot = root.kind;
        });
        "#,
        None,
    )
    .expect("OPFS root probe should schedule");
    assert_eq!(
        vm.eval_after_selected_page_tasks("String(globalThis.__lazyOpfsRoot)")
            .expect("OPFS root probe should settle"),
        "directory"
    );
    assert!(
        vm._context_host.borrow().has_opfs_owner_state(),
        "the first OPFS operation must allocate its owner state"
    );
}
#[test]
fn window_storage_constructor_globals_materialize_on_first_value_read() {
    let mut vm = new_storage_test_vm("https://storage-constructors-lazy.test/");
    let materialization_count = |vm: &mut ScriptVm, name| {
        vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            Ok(crate::context_bootstrap::lazy_constructor_materialization_count(scope, name))
        })
        .expect("lazy constructor diagnostics")
    };

    for name in [
        "StorageManager",
        "StorageEstimate",
        "StorageBucketManager",
        "StorageBucket",
        "FileSystemHandle",
        "FileSystemFileHandle",
        "FileSystemDirectoryHandle",
        "FileSystemWritableFileStream",
    ] {
        assert_eq!(
            materialization_count(&mut vm, name),
            0,
            "{name} must remain lazy during Window bootstrap"
        );
    }

    assert_eq!(
        vm.eval(
            r#"
            [
              "StorageManager" in globalThis,
              Object.hasOwn(globalThis, "StorageManager"),
              Object.keys(globalThis).includes("StorageManager")
            ].join("|")
            "#
        )
        .expect("non-value constructor probes"),
        "true|true|false"
    );
    assert_eq!(
        materialization_count(&mut vm, "StorageManager"),
        0,
        "existence and enumeration probes must not invoke the lazy getter"
    );

    assert_eq!(
        vm.eval(
            r#"
            const firstStorageManager = StorageManager;
            const descriptor =
              Object.getOwnPropertyDescriptor(globalThis, "StorageManager");
            JSON.stringify({
              same: firstStorageManager === StorageManager,
              name: firstStorageManager.name,
              descriptorValue: descriptor.value === firstStorageManager,
              writable: descriptor.writable,
              enumerable: descriptor.enumerable,
              configurable: descriptor.configurable,
              getterType: typeof descriptor.get
            })
            "#
        )
        .expect("StorageManager first value read"),
        r#"{"same":true,"name":"StorageManager","descriptorValue":true,"writable":true,"enumerable":false,"configurable":true,"getterType":"undefined"}"#
    );
    assert_eq!(materialization_count(&mut vm, "StorageManager"), 1);

    assert_eq!(
        vm.eval(
            r#"
            const directoryConstructor = FileSystemDirectoryHandle;
            const basePrototype =
              Object.getPrototypeOf(directoryConstructor.prototype);
            [
              directoryConstructor === FileSystemDirectoryHandle,
              directoryConstructor.name,
              Object.getPrototypeOf(directoryConstructor) === FileSystemHandle,
              basePrototype[Symbol.toStringTag],
              typeof directoryConstructor.prototype.getDirectoryHandle,
              typeof Object.getOwnPropertyDescriptor(basePrototype, "kind").get,
              Object.getOwnPropertyDescriptor(basePrototype, "kind").get.name
            ].join("|")
            "#
        )
        .expect("derived OPFS constructor first value read"),
        "true|FileSystemDirectoryHandle|true|FileSystemHandle|function|function|get kind"
    );
    assert_eq!(
        materialization_count(&mut vm, "FileSystemDirectoryHandle"),
        1
    );
    assert_eq!(
        materialization_count(&mut vm, "FileSystemHandle"),
        1,
        "materializing a derived constructor must bind it to the same-realm public base constructor"
    );

    assert_eq!(
        vm.eval(
            r#"
            Object.getPrototypeOf(FileSystemWritableFileStream) === WritableStream &&
              Object.getPrototypeOf(
                FileSystemWritableFileStream.prototype
              ) === WritableStream.prototype
            "#
        )
        .expect("writable stream constructor inheritance"),
        "true"
    );
    assert_eq!(
        materialization_count(&mut vm, "FileSystemWritableFileStream"),
        1
    );
}
#[test]
fn assigning_lazy_storage_constructor_before_read_skips_materialization() {
    let mut vm = new_storage_test_vm("https://storage-constructor-override.test/");
    assert_eq!(
        vm.eval(
            r#"
            globalThis.StorageEstimate = 17;
            [
              globalThis.StorageEstimate,
              Object.getOwnPropertyDescriptor(globalThis, "StorageEstimate").value
            ].join("|")
            "#
        )
        .expect("lazy constructor override"),
        "17|17"
    );
    let count = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            Ok(
                crate::context_bootstrap::lazy_constructor_materialization_count(
                    scope,
                    "StorageEstimate",
                ),
            )
        })
        .expect("lazy constructor diagnostics");
    assert_eq!(
        count, 0,
        "assignment before first read must replace the lazy property without invoking it"
    );
}
#[test]
fn window_fetch_retirement_covers_every_host_stage() {
    let mut vm = new_storage_test_vm("https://fetch-stage-retirement.test/");
    let stages = [
        PendingWindowFetchTestStage::Pending,
        PendingWindowFetchTestStage::Running,
        PendingWindowFetchTestStage::Streaming,
        PendingWindowFetchTestStage::Auth,
        PendingWindowFetchTestStage::Response,
        PendingWindowFetchTestStage::ServiceWorkerInFlight,
    ];
    let mut ordinary = Vec::new();
    let mut keepalive = Vec::new();
    let mut expected_owner = None;
    let mut expected_realm = None;
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        let host = unsafe { &mut *host_ptr };
        for stage in stages {
            let ordinary_fetch = register_pending_window_fetch_for_test(scope, host, false, stage);
            let keepalive_fetch = register_pending_window_fetch_for_test(scope, host, true, stage);
            expected_owner = Some(ordinary_fetch.1);
            expected_realm = Some(ordinary_fetch.2);
            ordinary.push((ordinary_fetch.0, ordinary_fetch.3));
            keepalive.push((keepalive_fetch.0, keepalive_fetch.3));
        }
        Ok(())
    })
    .expect("all Window Fetch host stages should be registered");
    let expected_owner = expected_owner.expect("Window Fetch owner");
    let expected_realm = expected_realm.expect("Window Fetch realm");

    assert_eq!(
        vm._context_host
            .borrow_mut()
            .retire_window_fetches_for_execution_context_owner(expected_owner),
        (stages.len(), stages.len()),
        "one owner retirement must abort ordinary Fetch and detach keepalive in every host stage"
    );
    assert!(
        ordinary
            .iter()
            .all(|(_, cancel_handle)| cancel_handle.is_cancelled()),
        "ordinary Fetch transport must be cancelled in every host stage"
    );
    assert!(
        keepalive
            .iter()
            .all(|(_, cancel_handle)| !cancel_handle.is_cancelled()),
        "execution-context destruction must not cancel keepalive transport"
    );
    let detached = vm
        ._context_host
        .borrow()
        .pending_window_fetch_execution_contexts_for_test();
    assert_eq!(detached.len(), stages.len());
    assert!(detached.iter().all(|(_, is_detached, owner, realm)| {
        *is_detached && *owner == Some(expected_owner) && *realm == Some(expected_realm)
    }));

    for (internal_id, _) in keepalive {
        assert!(
            vm._context_host
                .borrow_mut()
                .abort_subresource_fetch(internal_id),
            "detached keepalive stage should remain explicitly cancellable"
        );
    }
    assert!(
        vm._context_host
            .borrow()
            .pending_window_fetch_execution_contexts_for_test()
            .is_empty()
    );
}
#[test]
fn window_fetch_request_start_records_keepalive_disposition() {
    let mut vm = new_storage_test_vm("https://fetch-keepalive-output.test/");
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        let host = unsafe { &mut *host_ptr };
        let _ordinary = register_pending_window_fetch_for_test(
            scope,
            host,
            false,
            PendingWindowFetchTestStage::Pending,
        );
        let _keepalive = register_pending_window_fetch_for_test(
            scope,
            host,
            true,
            PendingWindowFetchTestStage::Pending,
        );
        Ok(())
    })
    .expect("ordinary and keepalive Fetches should register");

    let starts = vm
        .take_network_output()
        .into_items()
        .filter_map(|item| match item {
            crate::types::ScriptNetworkOutputItem::SubresourceRequestStarted(request) => {
                Some(request.keepalive())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        starts,
        vec![false, true],
        "renderer network start facts must preserve the resource teardown disposition"
    );
}
#[test]
fn cancel_pending_window_fetch_auth_preserves_401_for_response_stage() {
    let mut vm = new_storage_test_vm("https://fetch-auth-cancel.test/");
    let internal_id = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            Ok(register_pending_window_fetch_for_test(
                scope,
                unsafe { &mut *host_ptr },
                false,
                PendingWindowFetchTestStage::Auth,
            )
            .0)
        })
        .expect("pending Window Fetch auth should register");
    {
        let mut host = vm._context_host.borrow_mut();
        let mut pending = host
            .take_pending_subresource_auth(internal_id)
            .expect("pending auth state");
        pending.intercept_response = true;
        host.record_pending_subresource_auth(pending);
    }

    let _ = vm
        .cancel_pending_subresource_auth_body(internal_id)
        .expect("CancelAuth should expose the challenged response");

    let events = vm.take_pending_subresource_continue_events();
    let [crate::types::PendingSubresourceContinueEvent::ResponsePaused(info)] = events.as_slice()
    else {
        panic!("CancelAuth should emit one response-stage pause, got {events:?}");
    };
    assert_eq!(info.internal_id, internal_id);
    assert_eq!(info.response_status, 401);
    assert_eq!(
        info.response_body.try_bytes().unwrap().as_ref(),
        b"auth required"
    );

    let pending = vm
        ._context_host
        .borrow_mut()
        .take_pending_subresource_response(internal_id)
        .expect("challenged response should remain pending for Fetch.continueResponse");
    assert_eq!(pending.response.status, 401);
    assert_eq!(pending.response.body_text(), "auth required");
}
#[tokio::test]
async fn default_video_fetch_policy_synthesizes_lifecycle_without_a_network_request() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://media-policy.test/page.html",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__defaultVideoPolicyEvents = [];
  const video = document.createElement("video");
  video.id = "default-video-policy";
  for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay", "error"]) {
    video.addEventListener(type, () => __defaultVideoPolicyEvents.push(type));
  }
  video.src = "https://media-policy.test/large-video.mp4";
  (document.body || document.documentElement || document).appendChild(video);
})()
"#,
    )
    .expect("default-policy video should initialize");

    let media = vm
        .document_runtime
        .get_element_by_id("default-video-policy")
        .expect("video handle");
    let pending = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("policy-skipped media keeps its event owner until terminal dispatch");
    assert!(
        pending.network_request_id().is_none(),
        "the default policy must reject video before binding a network request"
    );
    assert!(
        !vm.take_network_output().into_items().any(|item| {
            matches!(
                item,
                crate::types::ScriptNetworkOutputItem::SubresourceRequestStarted(request)
                    if request.resource_type()
                        == crate::types::SubresourceResourceType::Video
            )
        }),
        "policy-skipped video must not emit a requestWillBeSent source record"
    );

    for phase in ["loadstart", "loadedmetadata", "loadeddata", "canplay"] {
        run_next_page_media_element_event_for_test(
            &mut vm,
            &loader,
            &format!("default-policy video {phase} turn"),
        )
        .await;
    }
    assert_eq!(
        vm.eval(
            "JSON.stringify({events: __defaultVideoPolicyEvents, ready: document.getElementById('default-video-policy').readyState, network: document.getElementById('default-video-policy').networkState})"
        )
        .expect("default-policy video terminal state"),
        r#"{"events":["loadstart","loadedmetadata","loadeddata","canplay"],"ready":4,"network":1}"#
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_none(),
        "the synthetic terminal must retire the exact media owner"
    );
}
