use super::*;

async fn wait_for_opfs_page_task(
    wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
) {
    let mut observed = Vec::new();
    let arrival = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let wake = wake_rx
                .recv()
                .await
                .expect("OPFS Page task route must remain attached");
            let source = wake.source_for_test();
            observed.push(source);
            if source == crate::page_task_queue::RendererOwnerWakeSource::OpfsTask {
                break;
            }
        }
    })
    .await;
    assert!(
        arrival.is_ok(),
        "OPFS storage completion should publish a typed Page task without an external retry; observed {observed:?}"
    );
}

fn take_opfs_page_task_for_test(
    page_vm: &mut PageVm,
) -> crate::page_task_queue::RendererPageOpfsTask {
    page_vm
        .page_task_executor_sources_for_test()
        .take_opfs_task_for_executor_test()
        .expect("one exact OPFS task should be ready")
}

fn publish_test_opfs_root_completion(
    producer: crate::page_task_queue::RendererPageOpfsTaskProducer,
) {
    producer
        .send(crate::opfs_task_result::OpfsTaskResult::GetRoot(Ok(Ok(
            moli_storage_service::OpfsPath::root(),
        ))))
        .expect("test OPFS completion should enter the stable Page source");
}

#[test]
fn opfs_task_rejects_a_real_page_vm_replacement_identity_collision() {
    run_page_vm_large_stack_async_test("opfs-real-page-vm-replacement-collision", || async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/replacement.html",
            "HTTP/1.1 200 OK",
            "<!doctype html><body>replacement</body>".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
        let (page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let retired_producer = page_vm
                    .vm_mut()
                    .register_pending_opfs_task_producer_for_test()?;
                let retired_owner = retired_producer.owner();
                assert_eq!(
                    retired_owner.root_document(),
                    page_vm.document_lifecycle.identity().document
                );
                publish_test_opfs_root_completion(retired_producer);

                let replacement_url = format!("{base_url}/replacement.html");
                page_vm
                    .vm_mut()
                    .eval(&format!("location.href = {replacement_url:?}; 'queued'"))?;
                let mut pending_document_lifecycle_turn = None;
                let navigation = page_vm
                    .follow_pending_location_navigation_one_turn_async(
                        &mut pending_document_lifecycle_turn,
                        PageVmInitStage::Load,
                    )
                    .await?;
                assert!(matches!(
                    navigation,
                    crate::runtime::PageVmFollowNavigationTurnOutcome::Completed
                        | crate::runtime::PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                            ..
                        }
                ));

                let current_producer = page_vm
                    .vm_mut()
                    .register_pending_opfs_task_producer_for_test()?;
                let current_owner = current_producer.owner();
                assert_eq!(
                    retired_owner.task(),
                    current_owner.task(),
                    "fresh PageVm counters should naturally reuse the first OPFS task id and transport generation"
                );
                assert_eq!(
                    retired_owner.execution_context(),
                    current_owner.execution_context(),
                    "fresh PageVm counters should naturally reuse the top Window/realm identity"
                );
                assert_ne!(
                    retired_owner.root_document(),
                    current_owner.root_document(),
                    "the stable Page queue must namespace identical local OPFS owners by root Document"
                );
                publish_test_opfs_root_completion(current_producer);

                assert!(
                    page_vm
                        .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::OpfsTask, &loader)
                        .await?,
                    "the retired OPFS completion should consume the first selected turn"
                );
                assert_eq!(
                    page_vm
                        .vm()
                        .current_pending_opfs_task_execution_context(current_owner.task()),
                    Some(current_owner.execution_context()),
                    "discarding the retired completion must not remove the colliding replacement Promise"
                );

                assert!(
                    page_vm
                        .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::OpfsTask, &loader)
                        .await?,
                    "the replacement OPFS completion should consume the following selected turn"
                );
                assert_eq!(
                    page_vm
                        .vm()
                        .current_pending_opfs_task_execution_context(current_owner.task()),
                    None,
                    "the current completion must settle exactly the replacement Promise"
                );
                Ok::<_, anyhow::Error>(())
            })
            .await
            .expect("OPFS replacement should run through the typed selected-task dispatcher");
        server
            .await
            .expect("OPFS PageVm replacement server should finish");
    });
}

#[test]
fn opfs_task_survives_document_open_within_the_same_window_realm() {
    run_page_vm_large_stack_async_test("opfs-document-open-owner", || async move {
        let result: anyhow::Result<()> = async {
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let document_url = Url::parse("https://opfs-document-open.test/").unwrap();
            let (mut page_vm, _resource_source, _owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

            let before_document_owner = page_vm
                .vm()
                .current_main_document_task_owner()
                .expect("initial main Document owner");
            let producer = page_vm
                .vm_mut()
                .register_pending_opfs_task_producer_for_test()?;
            let owner = producer.owner();

            page_vm.vm_mut().eval(
                r#"
document.open();
document.write("<!doctype html><title>replacement</title>");
document.close();
"replaced"
"#,
            )?;

            let after_document_owner = page_vm
                .vm()
                .current_main_document_task_owner()
                .expect("replacement main Document owner");
            assert_eq!(
                after_document_owner.local_window_id, before_document_owner.local_window_id,
                "document.open must preserve the Window execution context"
            );
            assert_ne!(
                after_document_owner.document_id, before_document_owner.document_id,
                "document.open must still rotate the Document owner"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .current_pending_opfs_task_execution_context(owner.task()),
                Some(owner.execution_context()),
                "Window-owned OPFS work must remain current across document.open"
            );

            publish_test_opfs_root_completion(producer);
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::OpfsTask,
                        &loader
                    )
                    .await?,
                "preserved OPFS work should consume one selected Page turn"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .current_pending_opfs_task_execution_context(owner.task()),
                None,
                "the selected task must settle the exact preserved OPFS Promise"
            );
            Ok(())
        }
        .await;
        result.expect("document.open OPFS task should run through the typed executor");
    });
}

#[tokio::test(flavor = "current_thread")]
async fn opfs_task_body_leaves_promise_reactions_for_selected_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/opfs-task-body-boundary").unwrap();
        let (mut page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
globalThis.__opfsTaskBodyBoundary = [];
navigator.storage.getDirectory().then(() => {
  __opfsTaskBodyBoundary.push("reaction");
  const script = document.createElement("script");
  script.textContent = "__opfsTaskBodyBoundary.push('runtime-script')";
  document.body.appendChild(script);
});
"queued"
"#,
        )?;

        wait_for_opfs_page_task(&mut owner_wake_rx).await;
        let task = take_opfs_page_task_for_test(&mut page_vm);
        let outcome = page_vm.apply_selected_page_opfs_task_turn(task)?;
        assert!(
            outcome.action.settled_current_owner(),
            "the exact OPFS task body should settle its pending Promise"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__opfsTaskBodyBoundary.join('|')")?,
            "",
            "the OPFS body must leave Promise reactions for selected-task completion"
        );
        page_vm
            .finish_selected_page_task_completion(
                outcome.action.into_page_task_completion(),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__opfsTaskBodyBoundary.join('|')")?,
            "reaction|runtime-script",
            "the selected OPFS task checkpoint must run its Promise reaction exactly once"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("OPFS body/completion boundary witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn opfs_selected_checkpoint_runs_a_reaction_that_creates_a_child() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/opfs-selected-completion-child").unwrap();
        let (mut page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
navigator.storage.getDirectory().then(() => {
  const frame = document.createElement("iframe");
  frame.id = "opfs-reaction-child";
  frame.srcdoc = "<!doctype html><body>child</body>";
  document.body.appendChild(frame);
});
"queued"
"#,
        )?;

        wait_for_opfs_page_task(&mut owner_wake_rx).await;
        let task = take_opfs_page_task_for_test(&mut page_vm);
        page_vm
            .apply_selected_page_scheduler_task_on_owner_lane_for_test(
                crate::page_task_queue::RendererPageSchedulerTask::OpfsTask(task),
                loader.clone(),
            )
            .await?;
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "the selected OPFS checkpoint must run the Promise reaction that publishes the child continuation before returning the Page"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("OPFS selected completion child witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn opfs_task_without_promise_handler_does_not_consume_unrelated_runtime_work() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/opfs-no-handler-completion").unwrap();
        let (mut page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm
            .vm_mut()
            .eval("void navigator.storage.getDirectory(); 'queued'")?;
        page_vm
            .vm_mut()
            .enqueue_test_pending_runtime_source_load();
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts.pending_source_load_count_for_test(),
            1,
            "the setup must retain unrelated runtime residence"
        );

        wait_for_opfs_page_task(&mut owner_wake_rx).await;
        let task = take_opfs_page_task_for_test(&mut page_vm);
        page_vm
            .apply_selected_page_scheduler_task_on_owner_lane_for_test(
                crate::page_task_queue::RendererPageSchedulerTask::OpfsTask(task),
                loader.clone(),
            )
            .await?;
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts.pending_source_load_count_for_test(),
            1,
            "an OPFS task with no queued Promise reaction owns a checkpoint, not unrelated runtime work"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("OPFS no-handler completion witness should run");
}

#[tokio::test]
async fn iframe_storage_manager_get_directory_is_owned_by_child_realm() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        const frame = document.createElement("iframe");
                        frame.id = "opfs-owner-child";
                        document.body.appendChild(frame);
                    })()
                    "#,
                )?;
                let child_handle = page_vm
                    .vm()
                    .element_handle_by_id_for_test("opfs-owner-child")
                    .expect("OPFS owner fixture should retain its iframe handle");
                materialize_child_realm_through_page_turn_for_test(
                    &mut page_vm,
                    "opfs-owner-child",
                )?;

                page_vm.vm_mut().eval(
                    r#"
                    globalThis.__iframeOpfsOwnerResult = "pending";
                    document.getElementById("opfs-owner-child")
                      .contentWindow.navigator.storage.getDirectory()
                      .then(
                        () => { globalThis.__iframeOpfsOwnerResult = "resolved"; },
                        error => {
                          globalThis.__iframeOpfsOwnerResult =
                            `rejected:${error && error.name}`;
                        }
                      );
                    "#,
                )?;
                assert!(
                    page_vm.vm().has_pending_opfs_tasks(),
                    "iframe getDirectory should remain pending on the storage owner"
                );

                page_vm
                    .vm_mut()
                    .retire_child_frame_realm_for_test(child_handle);

                assert!(
                    !page_vm.vm().has_pending_opfs_tasks(),
                    "retiring the iframe realm must retire its pending OPFS root task"
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("String(globalThis.__iframeOpfsOwnerResult)")?,
                    "pending",
                    "retiring the iframe must not settle its Promise in the top realm"
                );
                anyhow::Ok(())
            })
            .await
            .expect("iframe OPFS owner probe should run on the page owner lane");
    })
    .await;
}

#[tokio::test]
async fn opfs_directory_and_file_tasks_complete_through_storage_and_page_owner_queues() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__opfsOwnerReadDone = false;
                        globalThis.__opfsOwnerReadResult = "pending";

                        (async () => {
                            const root = await navigator.storage.getDirectory();
                            const directory = await root.getDirectoryHandle(
                                "owner-dir",
                                { create: true }
                            );
                            const handle = await directory.getFileHandle(
                                "owner-read.txt",
                                { create: true }
                            );
                            const writer = await handle.createWritable();
                            await writer.write("storage owner bytes");
                            await writer.close();
                            const snapshot = await handle.getFile();
                            const resolved = await root.resolve(handle);
                            await directory.getFileHandle(
                                "owner-second.txt",
                                { create: true }
                            );
                            const iterator = directory.keys();
                            const iteratorBatch = (await Promise.all([
                                iterator.next(),
                                iterator.next(),
                                iterator.next()
                            ])).map(result => result.done ? "done" : result.value);
                            await directory.removeEntry("owner-read.txt");
                            const removedFile = await directory.getFileHandle(
                                "owner-read.txt"
                            ).then(() => "present", error => error && error.name);
                            await directory.remove({ recursive: true });
                            const removedDirectory = await root.getDirectoryHandle(
                                "owner-dir"
                            ).then(() => "present", error => error && error.name);
                            globalThis.__opfsOwnerReadResult = JSON.stringify({
                                name: snapshot.name,
                                text: await snapshot.text(),
                                resolved,
                                iteratorBatch,
                                removedFile,
                                removedDirectory
                            });
                        })().catch(error => {
                            globalThis.__opfsOwnerReadResult =
                                `${error && error.name}:${error && error.message}`;
                        }).finally(() => {
                            globalThis.__opfsOwnerReadDone = true;
                        });
                    })()
                    "#,
                )?;
                assert!(
                    page_vm.vm().has_pending_opfs_tasks(),
                    "getFile should remain pending until its storage completion reaches the page owner"
                );
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__opfsOwnerReadDone === true)",
                    "OPFS storage-owner read should settle",
                )
                .await?;
                assert_eq!(
                    page_vm.vm_mut().eval("globalThis.__opfsOwnerReadResult")?,
                    r#"{"name":"owner-read.txt","text":"storage owner bytes","resolved":["owner-dir","owner-read.txt"],"iteratorBatch":["owner-read.txt","owner-second.txt","done"],"removedFile":"NotFoundError","removedDirectory":"NotFoundError"}"#
                );
                assert!(!page_vm.vm().has_pending_opfs_tasks());
                anyhow::Ok(())
            })
            .await
            .expect("OPFS owner queue probe should run on the page owner lane");
    })
    .await;
}

#[tokio::test]
async fn opfs_concurrent_file_moves_follow_storage_owner_order() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__opfsMoveDone = false;
                        globalThis.__opfsMoveResult = "pending";

                        (async () => {
                            const outcome = async promise => {
                                try {
                                    await promise;
                                    return "resolved";
                                } catch (error) {
                                    return error && error.name;
                                }
                            };
                            const root = await navigator.storage.getDirectory();
                            const source = await root.getDirectoryHandle(
                                "move-source",
                                { create: true }
                            );
                            const destination = await root.getDirectoryHandle(
                                "move-destination",
                                { create: true }
                            );
                            const file = await source.getFileHandle(
                                "before.txt",
                                { create: true }
                            );
                            const writer = await file.createWritable();
                            await writer.write("ordered move bytes");
                            await writer.close();

                            const firstMove = file.move("middle.txt");
                            const nameWhilePending = file.name;
                            const secondMove = file.move(destination, "after.txt");
                            const snapshot = file.getFile();
                            const resolved = root.resolve(file);
                            const sameSelf = file.isSameEntry(file);
                            const values = await Promise.all([
                                firstMove,
                                outcome(secondMove),
                                snapshot,
                                resolved,
                                sameSelf
                            ]);
                            const retryAfterSettlement = await outcome(
                                file.move(destination, "after.txt")
                            );
                            const sourceKeys = [];
                            for await (const key of source.keys()) sourceKeys.push(key);
                            const destinationKeys = [];
                            for await (const key of destination.keys()) destinationKeys.push(key);
                            globalThis.__opfsMoveResult = JSON.stringify({
                                nameWhilePending,
                                finalName: file.name,
                                secondMove: values[1],
                                retryAfterSettlement,
                                text: await values[2].text(),
                                resolved: values[3],
                                sameSelf: values[4],
                                sourceKeys,
                                destinationKeys
                            });
                        })().catch(error => {
                            globalThis.__opfsMoveResult =
                                `${error && error.name}:${error && error.message}`;
                        }).finally(() => {
                            globalThis.__opfsMoveDone = true;
                        });
                    })()
                    "#,
                )?;
                assert!(page_vm.vm().has_pending_opfs_tasks());
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__opfsMoveDone === true)",
                    "concurrent OPFS moves should settle",
                )
                .await?;
                assert_eq!(
                    page_vm.vm_mut().eval("globalThis.__opfsMoveResult")?,
                    r#"{"nameWhilePending":"before.txt","finalName":"after.txt","secondMove":"NoModificationAllowedError","retryAfterSettlement":"resolved","text":"ordered move bytes","resolved":["move-source","middle.txt"],"sameSelf":true,"sourceKeys":[],"destinationKeys":["after.txt"]}"#
                );
                assert!(!page_vm.vm().has_pending_opfs_tasks());
                anyhow::Ok(())
            })
            .await
            .expect("concurrent OPFS move probe should run on the page owner lane");
    })
    .await;
}

#[tokio::test]
async fn opfs_writable_acquisition_and_sink_follow_storage_owner_order() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__opfsWritableOwnerDone = false;
                        globalThis.__opfsWritableOwnerResult = "pending";

                        (async () => {
                            const outcome = async promise => {
                                try {
                                    await promise;
                                    return "resolved";
                                } catch (error) {
                                    return error && error.name;
                                }
                            };
                            const root = await navigator.storage.getDirectory();
                            const file = await root.getFileHandle(
                                "owner-writer.txt",
                                { create: true }
                            );

                            const acquisition = file.createWritable({ mode: "exclusive" });
                            const conflictingMove = file.move("blocked.txt");
                            const writer = await acquisition;
                            const conflict = await outcome(conflictingMove);

                            const first = writer.write("A");
                            const second = writer.write(new Uint8Array([66]));
                            const third = writer.write({
                                type: "write",
                                position: 2,
                                data: "C"
                            });
                            const close = writer.close();
                            const commandPromises = [first, second, third, close].every(
                                value => value && typeof value.then === "function"
                            );
                            await Promise.all([first, second, third, close]);

                            await file.move("after.txt");
                            const snapshot = await file.getFile();
                            globalThis.__opfsWritableOwnerResult = JSON.stringify({
                                conflict,
                                commandPromises,
                                finalName: file.name,
                                text: await snapshot.text()
                            });
                        })().catch(error => {
                            globalThis.__opfsWritableOwnerResult =
                                `${error && error.name}:${error && error.message}`;
                        }).finally(() => {
                            globalThis.__opfsWritableOwnerDone = true;
                        });
                    })()
                    "#,
                )?;
                assert!(page_vm.vm().has_pending_opfs_tasks());
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__opfsWritableOwnerDone === true)",
                    "OPFS writable owner operations should settle",
                )
                .await?;
                assert_eq!(
                    page_vm.vm_mut().eval("globalThis.__opfsWritableOwnerResult")?,
                    r#"{"conflict":"NoModificationAllowedError","commandPromises":true,"finalName":"after.txt","text":"ABC"}"#
                );
                assert!(!page_vm.vm().has_pending_opfs_tasks());
                anyhow::Ok(())
            })
            .await
            .expect("OPFS writable owner probe should run on the page owner lane");
    })
    .await;
}
