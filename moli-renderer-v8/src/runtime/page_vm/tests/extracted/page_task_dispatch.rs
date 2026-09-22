use super::*;

#[test]
fn default_runtime_hooks_reject_direct_no_owner_page_vm_construction() {
    let _js_runtime = crate::JsRuntime::initialize();
    let loader =
        crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let local_executor = crate::local_executor::JsLocalExecutor::new();
    let error = match PageVm::new(
        PageId::new_for_testing(1),
        local_executor,
        &loader,
        &PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
            root_frame_id: None,
            main_document_commit: None,
            top_level_storage_key: None,
            document_start_scripts: vec![],
            runtime_bindings: vec![],
            runtime_inspector_session_restore_snapshots: vec![],
            runtime_isolated_worlds: vec![],
            permission_overrides: vec![],
            extra_http_headers: Default::default(),
            navigator_identity: loader.browser_identity().clone(),
            document_policy_container: Default::default(),
            document_default_language: None,
            document_last_modified: None,
            document_settings: Default::default(),
            network_offline: false,
            blocked_url_patterns: Vec::new(),
            indexed_db_manager: None,
            storage_bucket_store: None,
            fetch_subresource_interception_enabled: false,
            fetch_subresource_interception_resource_type: None,
            layout_configuration: moli_page_types::LayoutConfiguration {
                policy: crate::real_layout_test_policy(),
                scrollbars_hidden: false,
            },
            wpt_extensions_enabled: false,
            navigation_bootstrap_entry: None,
            navigation_history_source: None,
            reserved_service_worker_client_id: None,
        },
        PageVmRuntimeHooks::default(),
        DomHost::from_dom(HtmlParser::SCRIPTING_ENABLED.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        )),
        Instant::now(),
    ) {
        Ok(_) => panic!("default runtime hooks must not create standalone document isolates"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("PageVmRuntimeHooks::standalone_without_owner_reservation_for_test()"),
        "unexpected direct no-owner construction error: {error}"
    );
}
#[tokio::test]
async fn stream_declared_controller_and_writer_surface_ignores_reflection_and_spoofing() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        const methodShape = (object, name) => {
                            const descriptor = Object.getOwnPropertyDescriptor(
                                Object.getPrototypeOf(object),
                                name
                            );
                            return [
                                !!descriptor,
                                descriptor && descriptor.enumerable,
                                descriptor && descriptor.configurable,
                                descriptor && descriptor.writable,
                                descriptor && typeof descriptor.value,
                                descriptor && descriptor.value.length,
                                descriptor && descriptor.value.name
                            ].join(":");
                        };
                        const accessorShape = (object, name) => {
                            const descriptor = Object.getOwnPropertyDescriptor(
                                Object.getPrototypeOf(object),
                                name
                            );
                            return [
                                !!descriptor,
                                descriptor && descriptor.enumerable,
                                descriptor && descriptor.configurable,
                                descriptor && ("writable" in descriptor),
                                descriptor && typeof descriptor.get,
                                descriptor && descriptor.get.name,
                                descriptor && descriptor.get.length,
                                descriptor && typeof descriptor.set
                            ].join(":");
                        };
                        const internals = [
                            "__moliStreamControllerStream",
                            "__moliWritableStreamWriterStream"
                        ];
                        const reflectedInternals = object => Object.getOwnPropertyNames(object)
                            .filter(name => internals.includes(name))
                            .join(",");
                        const throwsTypeError = callback => {
                            try {
                                callback();
                                return "no-throw";
                            } catch (error) {
                                return `throw:${error.name}`;
                            }
                        };

                        let readableController;
                        const readable = new ReadableStream({
                            start(controller) {
                                readableController = controller;
                            }
                        }, { highWaterMark: 2 });
                        if (!readableController) {
                            throw new Error("ReadableStream start controller was not captured");
                        }
                        const readableReflectedBefore = reflectedInternals(readableController);
                        const readableDesiredGetter =
                            Object.getOwnPropertyDescriptor(
                                Object.getPrototypeOf(readableController),
                                "desiredSize"
                            ).get;
                        const readableBefore = readableController.desiredSize;
                        readableController.enqueue("one");
                        const readableAfter = readableController.desiredSize;
                        const readableFake = throwsTypeError(() => readableDesiredGetter.call({
                            __moliStreamControllerStream: readable
                        }));
                        readableController.__moliStreamControllerStream = new ReadableStream(
                            {},
                            { highWaterMark: 99 }
                        );
                        const readableSpoofed = readableController.desiredSize;

                        let transformController;
                        const transform = new TransformStream({
                            start(controller) {
                                transformController = controller;
                            }
                        }, undefined, { highWaterMark: 3 });
                        if (!transformController) {
                            throw new Error("TransformStream transform controller was not captured");
                        }
                        transformController.enqueue("chunk");
                        const transformReflectedBefore = reflectedInternals(transformController);
                        const transformBeforeSpoof = transformController.desiredSize;
                        transformController.__moliStreamControllerStream = new ReadableStream(
                            {},
                            { highWaterMark: 99 }
                        );
                        const transformSpoofed = transformController.desiredSize;

                        const writable = new WritableStream();
                        const writer = writable.getWriter();
                        const writerReflectedBefore = reflectedInternals(writer);
                        const writerReadyGetter =
                            Object.getOwnPropertyDescriptor(
                                Object.getPrototypeOf(writer),
                                "ready"
                            ).get;
                        const writerDesiredGetter =
                            Object.getOwnPropertyDescriptor(
                                Object.getPrototypeOf(writer),
                                "desiredSize"
                            ).get;
                        const writerDesiredBefore = writer.desiredSize;
                        const writerReadyIsPromise = writer.ready instanceof Promise;
                        const writerClosedIsPromise = writer.closed instanceof Promise;
                        const writerFakeReady = writerReadyGetter.call({
                            __moliWritableStreamWriterStream: writable
                        }) instanceof Promise;
                        const writerFakeDesired = throwsTypeError(() =>
                            writerDesiredGetter.call({
                                __moliWritableStreamWriterStream: writable
                            })
                        );
                        writer.__moliWritableStreamWriterStream = new WritableStream();
                        const writerDesiredSpoofed = writer.desiredSize;

                        return [
                            readableReflectedBefore,
                            methodShape(readableController, "enqueue"),
                            methodShape(readableController, "close"),
                            accessorShape(readableController, "desiredSize"),
                            `${readableBefore}:${readableAfter}:${readableFake}:${readableSpoofed}`,
                            transformReflectedBefore,
                            methodShape(transformController, "enqueue"),
                            String(Object.hasOwn(
                                Object.getPrototypeOf(transformController),
                                "close"
                            )),
                            accessorShape(transformController, "desiredSize"),
                            `${transformBeforeSpoof}:${transformSpoofed}`,
                            writerReflectedBefore,
                            accessorShape(writer, "ready"),
                            accessorShape(writer, "closed"),
                            accessorShape(writer, "desiredSize"),
                            `${writerDesiredBefore}:${writerReadyIsPromise}:${writerClosedIsPromise}:${writerFakeReady}:${writerFakeDesired}:${writerDesiredSpoofed}`
                        ].join("\n");
                    })()
                    "#,
                )
            })
            .await
            .expect("stream declared surface test should run on owner lane");

        assert_eq!(
            result,
            [
                "",
                "true:true:true:true:function:0:enqueue",
                "true:true:true:true:function:0:close",
                "true:true:true:false:function:get desiredSize:0:undefined",
                "2:1:throw:TypeError:1",
                "",
                "true:true:true:true:function:0:enqueue",
                "false",
                "true:true:true:false:function:get desiredSize:0:undefined",
                "2:2",
                "",
                "true:true:true:false:function:get ready:0:undefined",
                "true:true:true:false:function:get closed:0:undefined",
                "true:true:true:false:function:get desiredSize:0:undefined",
                "1:true:true:true:throw:TypeError:1",
            ]
            .join("\n")
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn direct_page_vm_wait_commands_fail_closed() {
    run_page_vm_async_test(async move {
        let loader_owner =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let loader = loader_owner.handle();

        let commands = [
            (
                RendererPageCommand::WaitForSelector {
                    selector: "#never".to_owned(),
                    timeout_ms: 1_000,
                    loader: loader.clone(),
                },
                "wait-for-selector must be routed through the renderer owner continuation",
            ),
            (
                RendererPageCommand::WaitForScriptTruthy {
                    expression: "false".to_owned(),
                    timeout_ms: 1_000,
                    loader: loader.clone(),
                },
                "wait-for-script-truthy must be routed through the renderer owner continuation",
            ),
            (
                RendererPageCommand::WaitForSubresourceResponse {
                    criteria: SubresourceResponseWaitCriteria::default(),
                    timeout_ms: 1_000,
                    loader,
                },
                "wait-for-subresource-response must be routed through the renderer owner continuation",
            ),
        ];

        for (command, expected_error) in commands {
            let mut page_vm = test_page_vm();
            let error = match page_vm
                .dispatch_renderer_page_command(command)
            {
                Ok(_) => panic!("direct wait command should fail closed"),
                Err(error) => error,
            };
            assert!(
                error.to_string().contains(expected_error),
                "unexpected error: {error}"
            );
        }
    })
    .await;
}
