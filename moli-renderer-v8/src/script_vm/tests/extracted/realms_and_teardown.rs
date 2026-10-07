use super::*;

fn new_vm_with_pending_response_for_teardown_test()
-> (StandaloneScriptVmHarness, crate::types::NetworkBodySourceId) {
    let mut vm = new_storage_test_vm("https://body-teardown.test/");
    let body_source_id = crate::network_host::new_network_body_source_id();
    let document_url = vm.document_runtime.document_url().clone();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let response =
            crate::network_host::build_fetch_response_object_from_stream_for_request_mode(
                scope,
                &document_url,
                crate::network_host::FetchResponseRequest {
                    method: "GET",
                    mode: moli_fetch::RequestMode::Cors,
                },
                moli_fetch::ResponseHead {
                    final_url: document_url.join("data.json").unwrap(),
                    status: 200,
                    headers: vec![("content-type".to_owned(), b"application/json".to_vec())],
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                },
                body_source_id,
            );
        let global = scope.get_current_context().global(scope);
        let _ = global.set(
            scope,
            crate::util::v8str(scope, "response").into(),
            response.into(),
        );
        Ok(())
    })
    .expect("pending Response should be installed");
    (vm, body_source_id)
}

fn error_response_for_teardown_test(
    vm: &mut StandaloneScriptVmHarness,
    body_source_id: crate::types::NetworkBodySourceId,
) {
    vm.eval("globalThis.bodyFailure = new Error('body failed')")
        .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let reason = global
            .get(scope, crate::util::v8str(scope, "bodyFailure").into())
            .unwrap();
        crate::network_host::error_pending_network_body_stream_with_reason(
            scope,
            body_source_id,
            "body failed".to_owned(),
            reason,
        );
        Ok(())
    })
    .expect("pending Response should retain its rejection reason");
}

#[test]
fn pending_response_body_releases_native_host_on_context_teardown() {
    let (mut vm, _) = new_vm_with_pending_response_for_teardown_test();
    vm.eval("globalThis.bodyPromise = response.json(); 1")
        .expect("body materialization should wait for network completion");
    let weak_host = vm.context_host_weak_for_test();
    let isolate = vm.renderer_document_isolate.clone();
    drop(vm);
    for _ in 0..2 {
        isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    assert!(
        weak_host.upgrade().is_none(),
        "a pending body resolver must not root its retired native Document"
    );
}

#[test]
fn errored_response_body_releases_native_host_on_context_teardown() {
    let (mut vm, body_source_id) = new_vm_with_pending_response_for_teardown_test();
    error_response_for_teardown_test(&mut vm, body_source_id);
    let weak_host = vm.context_host_weak_for_test();
    let isolate = vm.renderer_document_isolate.clone();
    drop(vm);
    for _ in 0..2 {
        isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    assert!(
        weak_host.upgrade().is_none(),
        "a stored body error must not root its retired native Document"
    );
}

#[test]
fn retained_response_body_keeps_bytes_until_the_last_v8_reference() {
    for pending in [false, true] {
        let (mut vm, body_source_id) = new_vm_with_pending_response_for_teardown_test();
        vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
            crate::network_host::enqueue_pending_network_body_chunk(
                scope,
                body_source_id,
                br#"{"ok":true}"#.to_vec(),
            );
            if !pending {
                crate::network_host::close_pending_network_body_stream(scope, body_source_id);
            }
            Ok(())
        })
        .unwrap();
        vm.eval(if pending {
            "globalThis.bodyPromise = response.text(); globalThis.readBody = () => bodyPromise"
        } else {
            "globalThis.readBody = () => response.text()"
        })
        .unwrap();
        let isolate = vm.renderer_document_isolate.clone();
        let callback = isolate.with_renderer_document_isolate_mut(|isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = v8::Local::new(scope, &vm.page_default_runtime.context);
            let scope = &mut v8::ContextScope::new(scope, context);
            let value = context
                .global(scope)
                .get(scope, crate::util::v8str(scope, "readBody").into())
                .unwrap();
            let function = v8::Local::<v8::Function>::try_from(value).unwrap();
            v8::Global::new(scope, function)
        });
        let weak_host = vm.context_host_weak_for_test();
        drop(vm);
        for _ in 0..2 {
            isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
        }
        assert!(weak_host.upgrade().is_some());
        let value = isolate.with_renderer_document_isolate_mut(|isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let caller_context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, caller_context);
            let function = v8::Local::new(scope, &callback);
            let context = function.get_creation_context(scope).unwrap();
            let scope = &mut v8::ContextScope::new(scope, context);
            if pending {
                crate::network_host::close_pending_network_body_stream(scope, body_source_id);
            }
            let receiver = v8::undefined(scope).into();
            let value = crate::script_execution::call_function(scope, function, receiver, &[])
                .expect("retained Response body should remain readable");
            let promise = v8::Local::<v8::Promise>::try_from(value).unwrap();
            assert_eq!(promise.state(), v8::PromiseState::Fulfilled);
            promise.result(scope).to_rust_string_lossy(scope)
        });
        assert_eq!(value, r#"{"ok":true}"#);
        isolate.with_renderer_document_isolate_mut(|isolate| {
            drop(callback);
            isolate.low_memory_notification();
        });
        isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
        assert!(
            weak_host.upgrade().is_none(),
            "releasing the body reader must release its retired native Document"
        );
    }
}

#[test]
fn retained_response_body_keeps_error_reason_until_the_last_v8_reference() {
    let (mut vm, body_source_id) = new_vm_with_pending_response_for_teardown_test();
    error_response_for_teardown_test(&mut vm, body_source_id);
    vm.eval("globalThis.readBody = () => response.text()")
        .unwrap();
    let isolate = vm.renderer_document_isolate.clone();
    let callback = isolate.with_renderer_document_isolate_mut(|isolate| {
        let scope = std::pin::pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        let context = v8::Local::new(scope, &vm.page_default_runtime.context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let value = context
            .global(scope)
            .get(scope, crate::util::v8str(scope, "readBody").into())
            .unwrap();
        let function = v8::Local::<v8::Function>::try_from(value).unwrap();
        v8::Global::new(scope, function)
    });
    let weak_host = vm.context_host_weak_for_test();
    drop(vm);
    for _ in 0..2 {
        isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    assert!(weak_host.upgrade().is_some());
    isolate.with_renderer_document_isolate_mut(|isolate| {
        let scope = std::pin::pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        let caller_context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, caller_context);
        let function = v8::Local::new(scope, &callback);
        let context = function.get_creation_context(scope).unwrap();
        let scope = &mut v8::ContextScope::new(scope, context);
        let receiver = v8::undefined(scope).into();
        let value = crate::script_execution::call_function(scope, function, receiver, &[])
            .expect("retained Response should preserve its body error");
        let promise = v8::Local::<v8::Promise>::try_from(value).unwrap();
        assert_eq!(promise.state(), v8::PromiseState::Rejected);
        let reason = context
            .global(scope)
            .get(scope, crate::util::v8str(scope, "bodyFailure").into())
            .unwrap();
        assert!(promise.result(scope).strict_equals(reason));
    });
    isolate.with_renderer_document_isolate_mut(|isolate| {
        drop(callback);
        isolate.low_memory_notification();
    });
    isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    assert!(
        weak_host.upgrade().is_none(),
        "releasing the body reader must release its retired native Document"
    );
}

fn error_response_with_foreign_reason_for_test(
    vm: &mut StandaloneScriptVmHarness,
    body_source_id: crate::types::NetworkBodySourceId,
) -> (v8::Weak<v8::Object>, v8::Weak<v8::Context>) {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let foreign_context = v8::Context::new(scope, Default::default());
        let reason = {
            let scope = &mut v8::ContextScope::new(scope, foreign_context);
            let message = crate::util::v8str(scope, "foreign body failure");
            v8::Exception::error(scope, message)
        };
        crate::network_host::error_pending_network_body_stream_with_reason(
            scope,
            body_source_id,
            "foreign body failure".to_owned(),
            reason,
        );
        Ok((
            v8::Weak::new(scope, v8::Local::<v8::Object>::try_from(reason).unwrap()),
            v8::Weak::new(scope, foreign_context),
        ))
    })
    .unwrap()
}

#[test]
fn unread_response_body_error_is_collectible_while_document_is_alive() {
    let (mut vm, body_source_id) = new_vm_with_pending_response_for_teardown_test();
    let (reason, foreign_context) =
        error_response_with_foreign_reason_for_test(&mut vm, body_source_id);
    let isolate = vm.renderer_document_isolate.clone();
    for _ in 0..2 {
        isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        assert!(
            reason.to_local(scope).is_some(),
            "Response must retain its error"
        );
        assert!(foreign_context.to_local(scope).is_some());
        Ok(())
    })
    .unwrap();

    vm.eval("globalThis.response = null").unwrap();
    for _ in 0..2 {
        isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    assert!(
        host.borrow()
            .pending_network_body_sources
            .contains_key(&body_source_id)
    );
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        assert!(
            reason.to_local(scope).is_none(),
            "the native body table must not root an unreachable error"
        );
        assert!(
            foreign_context.to_local(scope).is_none(),
            "the native body table must not retain the error's foreign realm"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn response_body_and_clones_preserve_foreign_error_after_gc() {
    let (mut vm, body_source_id) = new_vm_with_pending_response_for_teardown_test();
    vm.eval("globalThis.before = response.clone()").unwrap();
    let (reason, _) = error_response_with_foreign_reason_for_test(&mut vm, body_source_id);
    vm.eval("globalThis.after = response.clone()").unwrap();
    let isolate = vm.renderer_document_isolate.clone();
    for _ in 0..2 {
        isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    vm.eval(
        "globalThis.rejections = [];\n\
         for (const item of [response, before, after]) {\n\
           item.text().catch(error => rejections.push(error));\n\
         }",
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let reason = reason
            .to_local(scope)
            .expect("observable body errors must survive GC");
        let global = scope.get_current_context().global(scope);
        let rejections = global
            .get(scope, crate::util::v8str(scope, "rejections").into())
            .unwrap();
        let rejections = v8::Local::<v8::Array>::try_from(rejections).unwrap();
        assert_eq!(rejections.length(), 3);
        for index in 0..3 {
            assert!(
                rejections
                    .get_index(scope, index)
                    .unwrap()
                    .strict_equals(reason.into())
            );
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn response_body_and_clones_preserve_primitive_errors_after_gc() {
    for expression in ["undefined", "null", "42", "'body failure'"] {
        let (mut vm, body_source_id) = new_vm_with_pending_response_for_teardown_test();
        vm.eval(&format!(
            "globalThis.bodyFailure = {expression}; globalThis.before = response.clone()"
        ))
        .unwrap();
        vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
            let global = scope.get_current_context().global(scope);
            let reason = global
                .get(scope, crate::util::v8str(scope, "bodyFailure").into())
                .unwrap();
            crate::network_host::error_pending_network_body_stream_with_reason(
                scope,
                body_source_id,
                "fallback must not replace the original value".to_owned(),
                reason,
            );
            Ok(())
        })
        .unwrap();
        vm.eval("globalThis.after = response.clone()").unwrap();
        for _ in 0..2 {
            vm.renderer_document_isolate
                .with_renderer_document_isolate_mut(|isolate| {
                    isolate.low_memory_notification();
                });
        }
        vm.eval(
            "globalThis.matches = [];\n\
             for (const item of [response, before, after]) {\n\
               item.text().then(() => matches.push(false), error => matches.push(error === bodyFailure));\n\
             }",
        )
        .unwrap();
        assert_eq!(
            vm.eval("matches.join(',')").unwrap(),
            "true,true,true",
            "{expression}"
        );
    }
}

#[test]
fn response_body_preserves_error_with_an_inherited_read_only_array_index() {
    let (mut vm, body_source_id) = new_vm_with_pending_response_for_teardown_test();
    vm.eval(
        "Object.defineProperty(Array.prototype, '0', {\n\
           value: 'inherited value', writable: false, configurable: true\n\
         });",
    )
    .unwrap();
    error_response_for_teardown_test(&mut vm, body_source_id);
    vm.eval(
        "globalThis.sameError = false;\n\
         response.clone().text().catch(error => { sameError = error === bodyFailure; });",
    )
    .unwrap();
    assert_eq!(vm.eval("sameError").unwrap(), "true");
}

fn new_vm_with_evaluated_module_for_teardown_test() -> StandaloneScriptVmHarness {
    let mut vm = new_storage_test_vm("https://module-teardown.test/");
    let url = vm.document_runtime.document_url().clone();
    vm.document_runtime
        .register_import_map_source(
            r#"{"imports":{"retained":"https://module-teardown.test/retained.mjs"}}"#,
        )
        .expect("module import map should register");
    let source = crate::module_runtime::ModuleSource::text(
        "export const node = document.createElement('p');\n\
         node.textContent = 'module';\n\
         export function readNative() { return node.textContent + ':' + import.meta.resolve('retained'); }\n\
         globalThis.readNative = readNative;"
            .to_owned(),
    );
    let mut job = crate::module_runtime::runtime_owned_loaded_module_script_graph_job(
        &mut vm,
        source,
        &url,
        &url,
        &crate::planning::ScriptFetchMetadata::default(),
        false,
    )
    .expect("module graph should be accepted");
    let crate::module_runtime::NativeModuleGraphJobAdvance::Complete(graph) = job
        .advance_module_script_owner_lane(&mut vm)
        .expect("import-free module graph should complete")
    else {
        panic!("an import-free module must not fetch");
    };
    vm.instantiate_native_module_graph(&graph)
        .expect("module graph should instantiate");
    vm.evaluate_native_module_graph(graph.root_entry)
        .expect("module graph should evaluate");
    assert_eq!(
        vm.eval("readNative()").unwrap(),
        "module:https://module-teardown.test/retained.mjs"
    );
    vm
}

#[test]
fn compiled_module_releases_native_host_on_context_teardown() {
    let vm = new_vm_with_evaluated_module_for_teardown_test();
    let weak_host = vm.context_host_weak_for_test();
    let isolate = vm.renderer_document_isolate.clone();
    drop(vm);
    for _ in 0..2 {
        isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    assert!(
        weak_host.upgrade().is_none(),
        "compiled module records must not root their retired native Document"
    );
}

#[test]
fn pending_dynamic_import_releases_native_host_on_context_teardown() {
    let mut vm = new_storage_test_vm("https://pending-module-teardown.test/");
    vm.eval("void import('./never-ready.mjs')")
        .expect("dynamic import should queue");
    assert!(vm.document_runtime.has_ready_native_dynamic_module_import());
    let weak_host = vm.context_host_weak_for_test();
    let isolate = vm.renderer_document_isolate.clone();
    drop(vm);
    for _ in 0..2 {
        isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    assert!(
        weak_host.upgrade().is_none(),
        "a pending dynamic import must not root its retired native Document"
    );
}

#[test]
fn retained_module_function_keeps_native_values_until_the_last_v8_reference() {
    let vm = new_vm_with_evaluated_module_for_teardown_test();
    let isolate = vm.renderer_document_isolate.clone();
    let callback = isolate.with_renderer_document_isolate_mut(|isolate| {
        let scope = std::pin::pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        let context = v8::Local::new(scope, &vm.page_default_runtime.context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let value = context
            .global(scope)
            .get(scope, crate::util::v8str(scope, "readNative").into())
            .unwrap();
        let function = v8::Local::<v8::Function>::try_from(value).unwrap();
        v8::Global::new(scope, function)
    });
    let weak_host = vm.context_host_weak_for_test();
    drop(vm);
    for _ in 0..2 {
        isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    assert!(weak_host.upgrade().is_some());
    let value = isolate.with_renderer_document_isolate_mut(|isolate| {
        let scope = std::pin::pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        let caller_context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, caller_context);
        let function = v8::Local::new(scope, &callback);
        let context = function.get_creation_context(scope).unwrap();
        let scope = &mut v8::ContextScope::new(scope, context);
        let receiver = v8::undefined(scope).into();
        crate::script_execution::call_function(scope, function, receiver, &[])
            .expect("retained module function should read its native DOM value")
            .to_rust_string_lossy(scope)
    });
    assert_eq!(value, "module:https://module-teardown.test/retained.mjs");
    isolate.with_renderer_document_isolate_mut(|isolate| {
        drop(callback);
        isolate.low_memory_notification();
    });
    isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    assert!(
        weak_host.upgrade().is_none(),
        "releasing the exported function must release its retired native Document"
    );
}

#[test]
fn unpromoted_child_realm_releases_native_host_on_document_teardown() {
    let vm = new_vm_with_unpromoted_child_realm_for_teardown_test();
    let weak_host = vm.context_host_weak_for_test();
    let isolate = vm.renderer_document_isolate.clone();
    drop(vm);
    for _ in 0..2 {
        isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    assert!(
        weak_host.upgrade().is_none(),
        "an iframe awaiting realm promotion must not root its retired native Document"
    );
}

fn new_vm_with_unpromoted_child_realm_for_teardown_test() -> StandaloneScriptVmHarness {
    let mut vm = new_storage_test_vm("https://unpromoted-child-teardown.test/");
    vm.eval(
        r#"
        const root = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || root.appendChild(document.createElement('body'));
        const frame = document.createElement('iframe');
        body.appendChild(frame);
        typeof frame.contentWindow.Function
        "#,
    )
    .expect("exposing an iframe should prebootstrap its realm");
    assert_eq!(vm.prebootstrapped_child_default_contexts.borrow().len(), 1);
    assert_eq!(vm.child_frame_realm_store.len(), 0);
    vm
}

#[test]
fn isolated_realm_retirement_releases_broadcast_channels_without_closing_parent() {
    let mut vm = new_storage_test_vm("https://isolated-channel-retirement.test/");
    vm.eval("globalThis.parentChannel = new BroadcastChannel('realm-retirement')")
        .unwrap();
    let context_id = vm.create_isolated_world("channel-owner", false).unwrap();
    vm.eval_in_isolated_context(
        context_id,
        "globalThis.isolatedChannel = new BroadcastChannel('realm-retirement'); 'created'",
    )
    .unwrap();
    let weak_context = vm
        .renderer_document_isolate
        .with_renderer_document_isolate_mut(|isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = v8::Local::new(
                scope,
                &vm.page_isolated_world_contexts
                    .context(context_id)
                    .unwrap()
                    .context,
            );
            v8::Weak::new(scope, context)
        });
    vm.destroy_isolated_world_context(context_id);
    for _ in 0..2 {
        vm.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    assert!(
        weak_context.is_empty(),
        "a retired BroadcastChannel must not root its isolated realm"
    );
    assert_eq!(
        vm.eval("parentChannel.postMessage('still open'); 'sent'")
            .unwrap(),
        "sent"
    );
}

#[test]
fn unpromoted_child_retirement_closes_only_its_execution_resources() {
    let mut vm = new_vm_with_unpromoted_child_realm_for_teardown_test();
    vm.eval("setTimeout(() => {}, 60000); frame.contentWindow.setTimeout(() => {}, 60000)")
        .unwrap();
    let (child_handle, context_ptr) = {
        let contexts = vm.prebootstrapped_child_default_contexts.borrow();
        let (handle, child) = contexts.iter().next().unwrap();
        (*handle, &child.context as *const v8::Global<v8::Context>)
    };
    let (ordinary, keepalive) = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            Ok((
                register_pending_window_fetch_for_test(
                    scope,
                    host,
                    false,
                    PendingWindowFetchTestStage::Pending,
                ),
                register_pending_window_fetch_for_test(
                    scope,
                    host,
                    true,
                    PendingWindowFetchTestStage::Pending,
                ),
            ))
        })
        .unwrap();
    let parent = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            Ok(register_pending_window_fetch_for_test(
                scope,
                unsafe { &mut *host_ptr },
                false,
                PendingWindowFetchTestStage::Pending,
            ))
        })
        .unwrap();
    vm.eval("frame.remove()").unwrap();
    vm.prune_stale_child_default_execution_contexts();

    assert!(ordinary.3.is_cancelled());
    assert!(!keepalive.3.is_cancelled());
    assert!(!parent.3.is_cancelled());
    assert!(
        !vm.prebootstrapped_child_default_contexts
            .borrow()
            .contains_key(&child_handle)
    );
    assert_eq!(
        vm.document_runtime
            .cancel_timers_for_context_token(ordinary.2),
        0
    );
    assert_eq!(
        vm.document_runtime
            .cancel_timers_for_context_token(parent.2),
        1
    );
    let pending = vm
        ._context_host
        .borrow()
        .pending_window_fetch_execution_contexts_for_test();
    assert!(pending.iter().any(|fetch| fetch.0 == parent.0));
    assert!(!pending.iter().any(|fetch| fetch.0 == ordinary.0));
}

#[test]
fn dropped_unpromoted_runtime_defers_close_and_releases_its_context() {
    let mut vm = new_vm_with_unpromoted_child_realm_for_teardown_test();
    let (child_handle, context_ptr) = {
        let contexts = vm.prebootstrapped_child_default_contexts.borrow();
        let (handle, child) = contexts.iter().next().unwrap();
        (*handle, &child.context as *const v8::Global<v8::Context>)
    };
    let fetch = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, host_ptr| {
            Ok(register_pending_window_fetch_for_test(
                scope,
                unsafe { &mut *host_ptr },
                false,
                PendingWindowFetchTestStage::Pending,
            ))
        })
        .unwrap();
    let weak_context = vm
        .renderer_document_isolate
        .with_renderer_document_isolate_mut(|isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            v8::Weak::new(scope, context)
        });
    let child = vm
        .prebootstrapped_child_default_contexts
        .borrow_mut()
        .remove(&child_handle)
        .unwrap();
    {
        let _host_borrow = vm._context_host.borrow_mut();
        drop(child);
        assert!(
            !fetch.3.is_cancelled(),
            "Drop must not reenter a borrowed host"
        );
    }
    vm.renderer_document_isolate
        .with_renderer_document_isolate_mut(|_| {});
    assert!(fetch.3.is_cancelled());
    for _ in 0..2 {
        vm.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    assert!(
        weak_context.is_empty(),
        "failed publication must release the old Context while its parent lives"
    );
    assert_eq!(
        vm.eval("document.URL").unwrap(),
        "https://unpromoted-child-teardown.test/"
    );
    assert_eq!(
        vm.eval("typeof frame.contentWindow.Function").unwrap(),
        "function"
    );
}

#[test]
fn child_realm_promotion_moves_the_runtime_without_closing_its_work() {
    let mut vm = new_vm_with_unpromoted_child_realm_for_teardown_test();
    let (child_handle, token, context_ptr) = {
        let contexts = vm.prebootstrapped_child_default_contexts.borrow();
        let (handle, child) = contexts.iter().next().unwrap();
        (
            *handle,
            child.runtime_observable_context_token,
            &child.context as *const v8::Global<v8::Context>,
        )
    };
    let fetch = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, host_ptr| {
            Ok(register_pending_window_fetch_for_test(
                scope,
                unsafe { &mut *host_ptr },
                false,
                PendingWindowFetchTestStage::Pending,
            ))
        })
        .unwrap();
    assert!(vm.run_child_realm_materialization_body_for_test().unwrap());
    assert!(
        vm.prebootstrapped_child_default_contexts
            .borrow()
            .is_empty()
    );
    let realm = vm
        .child_frame_realm_store
        .values()
        .find(|child| child.child_handle == child_handle)
        .unwrap();
    assert_eq!(realm.runtime_observable_context_token, token);
    assert!(
        !fetch.3.is_cancelled(),
        "promotion transfers execution ownership"
    );
    let context_id = realm.inspector_execution_context_id;
    vm.destroy_child_default_context(context_id);
    assert!(fetch.3.is_cancelled());
}

#[test]
fn failed_child_realm_bootstrap_releases_context_and_registration() {
    let mut vm = new_storage_test_vm("https://failed-realm-bootstrap.test/");
    vm.eval("const frame = document.createElement('iframe'); document.appendChild(frame)")
        .unwrap();
    let child = vm
        .live_child_default_context_entries()
        .into_iter()
        .next()
        .unwrap();
    let mut owner = vm
        ._context_host
        .borrow()
        .current_child_document_task_owner(child.handle)
        .unwrap();
    owner.document_id = crate::frame_owner_model::DocumentId(u64::MAX);
    let baseline = vm
        .renderer_document_isolate
        .with_renderer_document_isolate_mut(|isolate| {
            isolate.low_memory_notification();
            isolate.get_heap_statistics().number_of_native_contexts()
        });
    let host = vm._context_host.clone();
    for _ in 0..8 {
        let result = vm
            .renderer_document_isolate
            .with_entered_renderer_document_isolate_and_bootstrap(|isolate, cache| {
                ScriptVmContextBootstrap::new_child_default(
                    isolate,
                    cache,
                    host.clone(),
                    vm.resource_owner_id,
                    &vm.promise_reject_dispatch,
                    None,
                    Some(vm.storage_bucket_store.clone()),
                    child.handle,
                    owner,
                )
            });
        assert!(
            result.is_err(),
            "a stale Document must fail after allocating its Context"
        );
    }
    for _ in 0..2 {
        vm.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    let remaining = vm
        .renderer_document_isolate
        .with_renderer_document_isolate_mut(|isolate| {
            isolate.get_heap_statistics().number_of_native_contexts()
        });
    assert_eq!(remaining, baseline, "failed contexts must not accumulate");
    assert_eq!(
        vm._context_host
            .borrow()
            .window_execution_context_registry_counts_for_test(),
        (1, 1)
    );
    assert_eq!(
        vm.eval("typeof frame.contentWindow.Function").unwrap(),
        "function"
    );
}

#[test]
fn retained_unpromoted_child_realm_keeps_native_values_until_the_last_v8_reference() {
    let mut vm = new_vm_with_unpromoted_child_realm_for_teardown_test();
    vm.eval(
        r#"
        frame.contentWindow.Function(`
            globalThis.savedNode = document.createElement('p');
            savedNode.textContent = 'child';
            (document.body || document.documentElement || document).appendChild(savedNode);
        `)()
        "#,
    )
    .expect("an unpromoted child realm should expose native DOM values");
    let weak_host = vm.context_host_weak_for_test();
    let isolate = vm.renderer_document_isolate.clone();
    let context = vm
        .prebootstrapped_child_default_contexts
        .borrow()
        .values()
        .next()
        .unwrap()
        .context
        .clone();
    drop(vm);
    for _ in 0..2 {
        isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    assert!(weak_host.upgrade().is_some());
    let value = isolate.with_renderer_document_isolate_mut(|isolate| {
        let scope = std::pin::pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        let context = v8::Local::new(scope, &context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let source = crate::util::v8str(scope, "savedNode.textContent = 'retained-child'");
        let script =
            v8::Script::compile(scope, source, None).expect("retained child realm compiles");
        crate::script_execution::execute_compiled_script(scope, script)
            .expect("retained child native values remain usable")
            .to_rust_string_lossy(scope)
    });
    assert_eq!(value, "retained-child");
    isolate.with_renderer_document_isolate_mut(|isolate| {
        drop(context);
        isolate.low_memory_notification();
    });
    isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    assert!(
        weak_host.upgrade().is_none(),
        "the last child realm reference must release its retired native Document"
    );
}

fn capture_initial_environment_for_gc_test(
    vm: &StandaloneScriptVmHarness,
) -> crate::script_vm::ScriptVmCapturedDocumentEnvironment {
    let isolate = vm.renderer_document_isolate.clone();
    let url = vm.document_runtime.document_url().clone();
    let environment = isolate.with_renderer_document_isolate_mut(|isolate| {
        let scope = std::pin::pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        let context = v8::Local::new(scope, &vm.page_default_runtime.context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let opener = context.global(scope);
        crate::script_vm::ScriptVmInitialDocumentEnvironment::inherited_in_scope(
            scope,
            opener,
            moli_url::origin_ascii_serialization(&url),
            moli_storage_key::MoliStorageKey::first_party_from_url(
                &url,
                moli_storage_key::url_needs_opaque_nonce(&url)
                    .then(|| moli_storage_key::OpaqueOriginNonce::new(1)),
            ),
            url.clone(),
            Default::default(),
        )
        .expect("initiator environment captures its actual V8 security token")
    });
    crate::script_vm::ScriptVmCapturedDocumentEnvironment::new(environment, isolate)
}

#[test]
fn captured_tuple_environment_releases_source_document_before_consumption() {
    let vm = new_parsed_test_vm(
        "https://captured-tuple.test/source",
        "<body>source document</body>",
    );
    let weak_host = vm.context_host_weak_for_test();
    let isolate = vm.renderer_document_isolate.clone();
    let captured = capture_initial_environment_for_gc_test(&vm);
    drop(vm);
    for _ in 0..2 {
        isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    assert!(
        weak_host.upgrade().is_none(),
        "a tuple-origin token must not keep the source DOM alive"
    );
    let identity = isolate.identity_key();
    isolate.with_renderer_document_isolate_mut(|isolate| {
        let environment = captured
            .take(identity)
            .expect("same-isolate token consumption");
        let scope = std::pin::pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        let token = v8::Local::new(scope, environment.security_token.as_ref().unwrap());
        let token =
            v8::Local::<v8::String>::try_from(token).expect("tuple-origin token is a string");
        assert_eq!(
            token.to_rust_string_lossy(scope),
            "moli-window-origin-v1:https://captured-tuple.test",
            "the accepted token remains usable after the source Document is collected"
        );
    });
}

fn assert_captured_context_token_releases_realm_on_cancellation(url: &str, relax_domain: bool) {
    let mut vm = new_parsed_test_vm(url, "<body>source document</body>");
    if relax_domain {
        vm.eval("document.domain=location.hostname").unwrap();
    }
    let weak_host = vm.context_host_weak_for_test();
    let isolate = vm.renderer_document_isolate.clone();
    let captured = capture_initial_environment_for_gc_test(&vm);
    drop(vm);
    for _ in 0..2 {
        isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    assert!(
        weak_host.upgrade().is_some(),
        "a context token retains its creation realm and native backing"
    );
    // Cancellation may run outside an entered isolate. The established release
    // queue must drop the token on entry before the next GC can retire its realm.
    drop(captured);
    for _ in 0..2 {
        isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    assert!(
        weak_host.upgrade().is_none(),
        "cancelled navigation releases the last token and the source DOM"
    );
}

#[test]
fn captured_opaque_environment_keeps_realm_until_cancellation() {
    assert_captured_context_token_releases_realm_on_cancellation("about:blank", false);
}

#[test]
fn captured_document_domain_environment_keeps_realm_until_cancellation() {
    assert_captured_context_token_releases_realm_on_cancellation(
        "https://captured-domain.test/source",
        true,
    );
}

#[test]
fn retained_document_realm_keeps_native_values_until_the_last_v8_reference() {
    let mut vm = new_parsed_test_vm(
        "https://retained-document.test/old",
        "<!doctype html><p>old document</p>",
    );
    vm.eval(
        r#"
        globalThis.savedNode = document.querySelector('p');
        globalThis.savedDecoder = new TextDecoder();
        globalThis.savedBlob = new Blob(['retained blob']);
        globalThis.savedController = new AbortController();
        globalThis.savedSignal = savedController.signal;
        globalThis.savedComposite = AbortSignal.any([savedSignal]);
        globalThis.abortEventRan = false;
        savedSignal.onabort = () => { abortEventRan = true; };
        globalThis.savedReason = {kind: 'old realm'};
        globalThis.savedFunction = () => [savedNode.textContent, document.URL];
        'ready'
    "#,
    )
    .unwrap();
    let weak_host = vm.context_host_weak_for_test();
    let isolate = vm.renderer_document_isolate.clone();
    let context = vm.page_default_runtime.context.clone();
    drop(vm);
    assert!(
        weak_host.upgrade().is_some(),
        "retained realm owns its native DOM"
    );

    let value = isolate.with_renderer_document_isolate_mut(|isolate| {
        let scope = std::pin::pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        let context = v8::Local::new(scope, &context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let source = crate::util::v8str(scope, r#"
            savedNode.textContent = 'retained';
            savedController.abort(savedReason);
            JSON.stringify([
                savedFunction(),
                savedDecoder.decode(new Uint8Array([65])),
                savedBlob.size,
                savedSignal.aborted,
                savedSignal.reason === savedReason,
                savedComposite.aborted && savedComposite.reason === savedReason,
                abortEventRan,
                savedNode === document.querySelector('p'),
                (() => { try { savedSignal.throwIfAborted(); } catch (e) { return e === savedReason; } })()
            ])
        "#);
        let script = v8::Script::compile(scope, source, None).expect("retained realm compiles");
        crate::script_execution::execute_compiled_script(scope, script)
            .expect("retained native values remain usable")
            .to_rust_string_lossy(scope)
    });
    assert_eq!(
        value,
        r#"[["retained","https://retained-document.test/old"],"A",13,true,true,true,false,true,true]"#
    );

    isolate.with_renderer_document_isolate_mut(|isolate| {
        drop(context);
        isolate.low_memory_notification();
    });
    isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    assert!(
        weak_host.upgrade().is_none(),
        "last V8 reference releases native DOM outside GC"
    );
}

#[test]
fn runtime_binding_calls_freeze_the_invoking_realm_generation() {
    let mut vm = new_storage_test_vm("https://runtime-binding-source-realm.test/");
    vm.install_runtime_binding("mainRealmBinding", None, None)
        .expect("main Runtime binding should install");
    let isolated_context_id = vm
        .create_isolated_world("binding-source-realm", false)
        .expect("isolated Runtime binding world");
    vm.install_runtime_binding("isolatedRealmBinding", None, Some(isolated_context_id))
        .expect("isolated Runtime binding should install");

    vm.eval(r#"mainRealmBinding("main")"#)
        .expect("main binding call");
    vm.exec_in_execution_context(isolated_context_id, r#"isolatedRealmBinding("isolated")"#)
        .expect("isolated binding call");

    let calls = vm.take_runtime_binding_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0].source.local_window_id(),
        calls[1].source.local_window_id(),
        "main and isolated worlds belong to the same Window"
    );
    assert_ne!(
        calls[0].source.realm_generation(),
        calls[1].source.realm_generation(),
        "binding calls must retain the exact invoking realm instead of relying on a reusable public execution-context id"
    );
    assert_ne!(calls[0].execution_context_id, calls[1].execution_context_id);
}
#[tokio::test]
async fn opaque_child_isolated_world_projects_only_its_own_document() {
    let mut vm = new_storage_test_vm("https://opaque-child-isolated-world.test/");
    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "opaque-isolated-frame";
  frame.name = "opaque-isolated-child";
  frame.sandbox = "allow-scripts";
  frame.srcdoc = "<p id='opaque-marker'>opaque child document</p>";
  body.appendChild(frame);
  void frame.contentWindow;
})()
"#,
    )
    .expect("opaque child isolated-world setup should evaluate");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "opaque child isolated-world setup",
    )
    .await;

    let child_realm = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .next()
        .expect("opaque child default realm should exist");
    let child_handle = vm
        .child_frame_realm_store
        .get(&child_realm.context_id)
        .expect("opaque child realm record should exist")
        .child_handle;
    assert!(
        vm._context_host
            .borrow()
            .child_browsing_context_has_opaque_origin(child_handle),
        "sandbox without allow-same-origin must create an opaque child origin"
    );
    let child_request_origin = {
        let host = vm._context_host.borrow();
        let owner = crate::native_bridge::OwnerDispatchScope::Child(child_handle);
        let loader = host
            .document_resource_loader_for_dispatch_scope(owner)
            .expect("opaque child resource loader should exist");
        host.subresource_request_environment(&loader, owner)
            .expect("opaque child request environment should exist")
            .request_origin
    };
    assert!(
        matches!(child_request_origin, moli_url::WebOrigin::Opaque),
        "sandboxed child subresource requests must use an opaque client origin"
    );
    assert_eq!(
        vm.eval("document.getElementById('opaque-isolated-frame').contentDocument === null")
            .expect("top opaque contentDocument visibility should evaluate"),
        "true",
        "top must not gain DOM access to the opaque child"
    );

    let frame_id = vm
        ._context_host
        .borrow()
        .frame_owner_frame_id_for_child_handle(child_handle)
        .expect("opaque child frame id should exist")
        .0;
    let isolated_context_id = vm
        .create_isolated_world_for_frame(&frame_id, "opaque-child-utility", false)
        .expect("opaque child isolated world should be created");
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated_context_id,
            "document.getElementById('opaque-marker').textContent",
        )
        .expect("opaque child isolated world should access its own document"),
        "opaque child document"
    );
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated_context_id,
            r#"
(() => {
  class IsolatedChildElement extends HTMLElement {}
  customElements.define("isolated-child-element", IsolatedChildElement);
  const element = document.createElement("isolated-child-element");
  globalThis.__opaqueChildOpfsResult = "pending";
  navigator.storage.getDirectory().then(
    () => { globalThis.__opaqueChildOpfsResult = "resolved"; },
    error => { globalThis.__opaqueChildOpfsResult = error.name; }
  );
  return JSON.stringify({
    parentIsSelf: parent === self,
    topIsParent: top === parent,
    name,
    origin,
    navigationName: performance.getEntriesByType("navigation")[0].name,
    customElementUsesIsolatedDefinition:
      Object.getPrototypeOf(element) === IsolatedChildElement.prototype,
    webAssemblyConstructorUsesIsolatedFunctionPrototype:
      Object.getPrototypeOf(WebAssembly.Module) === Function.prototype
  });
})()
"#,
        )
        .expect("opaque child isolated-world state should evaluate"),
        r#"{"parentIsSelf":false,"topIsParent":true,"name":"opaque-isolated-child","origin":"null","navigationName":"about:srcdoc","customElementUsesIsolatedDefinition":true,"webAssemblyConstructorUsesIsolatedFunctionPrototype":true}"#
    );
    assert_eq!(
        vm.eval_in_isolated_context(isolated_context_id, "__opaqueChildOpfsResult")
            .expect("opaque child isolated-world OPFS result should evaluate"),
        "SecurityError",
        "isolated child navigator.storage must use the child opaque storage owner"
    );
}
#[tokio::test]
async fn initial_empty_child_isolated_world_rebinds_committed_document() {
    let mut vm = new_storage_test_vm("https://initial-empty-isolated-world.test/");
    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "initial-empty-isolated-frame";
  body.appendChild(frame);
  void frame.contentWindow;
})()
"#,
    )
    .expect("initial-empty child isolated-world setup should evaluate");

    assert!(
        vm.run_child_realm_materialization_body_for_test()
            .expect("initial-empty child realm turn should succeed"),
        "Window exposure should enqueue the initial-empty child realm"
    );

    let child_realm = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .next()
        .expect("initial-empty child default realm should exist");
    let child_handle = vm
        .child_frame_realm_store
        .get(&child_realm.context_id)
        .expect("initial-empty child realm record should exist")
        .child_handle;
    let frame_id = vm
        ._context_host
        .borrow()
        .frame_owner_frame_id_for_child_handle(child_handle)
        .expect("initial-empty child frame id should exist")
        .0;
    let isolated_context_id = vm
        .create_isolated_world_for_frame(&frame_id, "initial-empty-utility", false)
        .expect("initial-empty child isolated world should be created");
    let registration_id = vm
        .page_isolated_world_contexts
        .context(isolated_context_id)
        .unwrap()
        .inspector_context_registration_id;
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated_context_id,
            "globalThis.__initialEmptyUtilityExpando = 'preserved'",
        )
        .expect("initial-empty isolated document should evaluate"),
        "preserved"
    );

    vm.eval(
        r#"
document.getElementById("initial-empty-isolated-frame").srcdoc =
  "<!doctype html><body><p id='committed-marker'>committed child document</p></body>";
"navigating"
"#,
    )
    .expect("initial-empty child srcdoc navigation should evaluate");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "initial-empty child isolated-world commit",
    )
    .await;

    assert_eq!(
        vm.eval_in_isolated_context(
            isolated_context_id,
            "__initialEmptyUtilityExpando + '|' + document.getElementById('committed-marker').textContent",
        )
        .expect("rebound child isolated world should project the committed document"),
        "preserved|committed child document",
        "secure initial-empty reuse must preserve the isolated context while rotating its Document owner"
    );
    let committed_world = vm
        .page_isolated_world_contexts
        .context(isolated_context_id)
        .unwrap();
    assert_eq!(
        committed_world.inspector_context_registration_id,
        registration_id
    );
    assert_eq!(
        Some(committed_world.document_owner),
        vm._context_host
            .borrow()
            .current_child_document_task_owner(child_handle)
    );
    assert_eq!(
        vm.create_isolated_world_for_frame(&frame_id, "initial-empty-utility", false)
            .unwrap(),
        isolated_context_id,
        "Window reuse must keep the cached world identity"
    );

    // A subsequent navigation replaces the Window and must retire this world.
    vm.eval("document.getElementById('initial-empty-isolated-frame').srcdoc = '<main>replacement</main>'")
        .expect("second child navigation should evaluate");
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "replaced child Window").await;
    assert!(
        vm.page_isolated_world_contexts
            .context(isolated_context_id)
            .is_none()
    );
    let replacement_context_id = vm
        .create_isolated_world_for_frame(&frame_id, "initial-empty-utility", false)
        .unwrap();
    assert_ne!(replacement_context_id, isolated_context_id);
    assert_eq!(
        vm.eval_in_isolated_context(
            replacement_context_id,
            "typeof __initialEmptyUtilityExpando + '|' + document.querySelector('main').textContent"
        )
        .unwrap(),
        "undefined|replacement"
    );
}
#[test]
fn isolated_realm_destruction_retires_pending_opfs_task() {
    let origin = "https://isolated-opfs-owner.test/";
    let mut vm = new_storage_test_vm(origin);
    let isolated_context_id = vm
        .create_isolated_world("opfs-owner", false)
        .expect("isolated world should be created");
    let isolated_context_ptr = {
        let world = vm
            .page_isolated_world_contexts
            .context(isolated_context_id)
            .expect("isolated world should be tracked");
        &world.context as *const _
    };
    let locator = moli_storage_service::StorageBucketLocator::default_bucket(
        moli_storage_key::MoliStorageKey::first_party_from_url(
            &url::Url::parse(origin).unwrap(),
            None,
        )
        .serialized_storage_key(),
    );
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(
        isolated_context_ptr,
        |scope, host_ptr| {
            let resolver = v8::PromiseResolver::new(scope).expect("isolated OPFS resolver");
            assert!(
                unsafe { &mut *host_ptr }
                    .register_pending_opfs_task(scope, resolver, locator, None)
                    .is_some()
            );
            Ok(())
        },
    )
    .expect("isolated OPFS task should register");
    assert_eq!(vm._context_host.borrow().pending_opfs_task_count(), 1);

    vm.destroy_isolated_world_context(isolated_context_id);

    assert_eq!(
        vm._context_host.borrow().pending_opfs_task_count(),
        0,
        "destroying the Promise relevant realm must release its OPFS resolver"
    );
}

#[test]
fn isolated_realm_destruction_releases_indexed_db_without_closing_default_world() {
    let mut page = new_storage_page_task_executor_test_vm("https://isolated-idb-retirement.test/");
    page.eval(
        "window.defaultOpen=false;const request=indexedDB.open('default-idb',1);request.onupgradeneeded=()=>request.result.createObjectStore('store');request.onsuccess=()=>{window.defaultDb=request.result;defaultOpen=true}",
    ).unwrap();
    assert_eq!(
        page.eval_after_selected_page_tasks("defaultOpen").unwrap(),
        "true"
    );
    let isolated_id = page.create_isolated_world("idb-retirement", false).unwrap();
    page.eval_in_isolated_context(
        isolated_id,
        "window.opened=false;window.payload=new Array(1024*1024).fill(17);const request=indexedDB.open('isolated-idb',1);request.onsuccess=()=>{window.db=request.result;opened=true}",
    ).unwrap();
    page.eval_after_selected_page_tasks("true").unwrap();
    assert_eq!(
        page.eval_in_isolated_context(isolated_id, "opened")
            .unwrap(),
        "true"
    );
    let context_ptr = &page
        .page_isolated_world_contexts
        .context(isolated_id)
        .unwrap()
        .context as *const _;
    let weak = page
        .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, host_ptr| {
            let context = scope.get_current_context();
            let window = context.global(scope);
            let db_key = crate::util::v8str(scope, "db");
            let db = window.get(scope, db_key.into()).unwrap();
            let default = unsafe { &*host_ptr }.page_default_context(scope).unwrap();
            let retained_key = crate::util::v8str(scope, "retainedIsolatedDb");
            default.global(scope).set(scope, retained_key.into(), db);
            Ok(v8::Weak::new(scope, context))
        })
        .unwrap();
    page.destroy_isolated_world_context(isolated_id);
    for _ in 0..5 {
        page.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    assert_eq!(
        page.eval("retainedIsolatedDb.name").unwrap(),
        "isolated-idb",
        "a retained IndexedDB wrapper must preserve its native data after realm retirement"
    );
    assert!(
        page.with_default_context_scope_and_checkpoint_for_test(|scope, _| Ok(weak
            .to_local(scope)
            .is_some()))
            .unwrap()
    );
    page.eval("delete window.retainedIsolatedDb").unwrap();
    for _ in 0..5 {
        page.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    assert!(
        page.with_default_context_scope_and_checkpoint_for_test(|scope, _| Ok(weak
            .to_local(scope)
            .is_none()))
            .unwrap(),
        "no IndexedDB host handle may keep a retired isolated realm alive"
    );
    assert_eq!(
        page.eval("defaultDb.transaction('store').objectStore('store').name")
            .unwrap(),
        "store",
        "retiring one token must leave the default world's connection open"
    );
    page.eval("defaultDb.close();window.upgraded=false;const upgrade=indexedDB.open('isolated-idb',2);upgrade.onsuccess=()=>{upgraded=true;upgrade.result.close()}").unwrap();
    assert_eq!(
        page.eval_after_selected_page_tasks("upgraded").unwrap(),
        "true"
    );
}
#[test]
fn page_context_teardown_releases_opfs_handle_and_directory_iterator_registrations() {
    let mut vm = new_storage_page_task_executor_test_vm("https://opfs-iterator-teardown.test/");
    vm.exec(
        r#"
        globalThis.__opfsIteratorSetup = "pending";
        navigator.storage.getDirectory().then(root => {
          globalThis.__opfsRoot = root;
          globalThis.__opfsIterators = [];
          for (let index = 0; index < 16; index += 1) {
            globalThis.__opfsIterators.push(root.keys());
          }
          globalThis.__opfsIteratorSetup = String(globalThis.__opfsIterators.length);
        });
        "#,
        None,
    )
    .expect("OPFS iterator teardown probe should schedule");
    assert_eq!(
        vm.eval_after_selected_page_tasks("String(globalThis.__opfsIteratorSetup)")
            .expect("OPFS iterator setup should settle"),
        "16"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .opfs_handle_registry()
            .expect("OPFS handle registry should be materialized")
            .len(),
        1
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .opfs_directory_iterator_registry()
            .expect("OPFS iterator registry should be materialized")
            .len(),
        16
    );

    vm.close_page_context_resources_for_context_teardown();

    let handles = vm._context_host.borrow().opfs_handle_registry().unwrap();
    let iterators = vm
        ._context_host
        .borrow()
        .opfs_directory_iterator_registry()
        .unwrap();
    assert_eq!(
        handles.len(),
        1,
        "retained handle remains backed while its realm is alive"
    );
    assert_eq!(
        iterators.len(),
        16,
        "retained iterators remain backed while their realm is alive"
    );
    assert_eq!(vm.eval("__opfsRoot.kind").unwrap(), "directory");
    drop(vm);
    assert_eq!(
        handles.len(),
        0,
        "final Context release retires native handle registrations"
    );
    assert_eq!(
        iterators.len(),
        0,
        "final Context release retires native iterator registrations"
    );
}
#[test]
fn isolated_realm_destruction_retires_webcrypto_task_without_retiring_local_window() {
    let mut vm = new_storage_test_vm("https://isolated-webcrypto-owner.test/");
    let main_owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let isolated_context_id = vm
        .create_isolated_world("webcrypto-owner", false)
        .expect("isolated world should be created");
    let isolated_context_ptr = {
        let world = vm
            .page_isolated_world_contexts
            .context(isolated_context_id)
            .expect("isolated world should be tracked");
        &world.context as *const _
    };
    let producer = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(
            isolated_context_ptr,
            |scope, host_ptr| {
                let resolver = v8::PromiseResolver::new(scope)
                    .expect("isolated WebCrypto resolver should exist");
                unsafe { &mut *host_ptr }
                    .register_pending_webcrypto_task(scope, resolver)
                    .ok_or_else(|| {
                        anyhow::anyhow!("isolated WebCrypto task should capture its realm")
                    })
            },
        )
        .expect("isolated WebCrypto task should register");
    let pending = vm
        ._context_host
        .borrow()
        .pending_webcrypto_execution_contexts_for_test();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].0,
        crate::native_bridge::WindowExecutionContextOwner::Frame(main_owner.local_window_id)
    );

    vm.destroy_isolated_world_context(isolated_context_id);

    assert_eq!(
        vm._context_host.borrow().pending_webcrypto_task_count(),
        0,
        "destroying the Promise relevant realm must release its resolver"
    );
    assert_eq!(
        vm.current_main_document_task_owner()
            .map(|owner| owner.local_window_id),
        Some(main_owner.local_window_id),
        "realm retirement must not retire the owning LocalWindow"
    );

    producer
        .send(Ok(crate::context_bootstrap::WebCryptoTaskResult::Bool(
            true,
        )))
        .expect("retired-realm completion should still enter the stable Page source");
    assert!(
        vm.run_webcrypto_task_body_for_authorization_test()
            .expect("retired-realm WebCrypto task should consume one stale turn")
    );
    assert_eq!(
        vm._context_host.borrow().pending_webcrypto_task_count(),
        0,
        "a queued completion for the retired realm must not recreate or settle a pending Promise"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn popup_replacement_retires_local_window_owned_webcrypto_tasks() {
    let mut vm = new_storage_test_vm("https://popup-owner-webcrypto.test/");
    assert_eq!(
        vm.eval(
            r#"
            globalThis.__ownerBoundCryptoPopup = open("about:blank", "crypto-owner-popup");
            String(globalThis.__ownerBoundCryptoPopup !== null)
            "#,
        )
        .expect("popup WebCrypto owner window should open"),
        "true"
    );
    let popup_id = vm
        .take_pending_popup_activations()
        .into_iter()
        .next()
        .and_then(|activation| activation.popup_id())
        .expect("popup WebCrypto owner id");
    let initial_local_window_id = vm
        ._context_host
        .borrow()
        .current_lightweight_popup_local_window_id(popup_id)
        .expect("initial popup LocalWindow owner");

    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        let previous_popup =
            crate::native_bridge::enter_active_lightweight_popup_scope(scope, popup_id);
        let resolver = v8::PromiseResolver::new(scope).expect("popup WebCrypto test resolver");
        let registered = unsafe { &mut *host_ptr }
            .register_pending_webcrypto_task(scope, resolver)
            .is_some();
        crate::native_bridge::restore_active_lightweight_popup_scope(scope, previous_popup);
        assert!(
            registered,
            "popup WebCrypto task should bind a Window execution context"
        );
        Ok(())
    })
    .expect("popup WebCrypto task should register");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_webcrypto_execution_contexts_for_test()
            .into_iter()
            .map(|(owner, _)| owner)
            .collect::<Vec<_>>(),
        vec![
            crate::native_bridge::WindowExecutionContextOwner::LightweightPopup {
                popup_id,
                local_window_id: initial_local_window_id,
            }
        ],
        "popup WebCrypto work must capture the preparation-time popup LocalWindow"
    );

    assert_eq!(
        vm.eval(
            r#"
            open("about:blank", "crypto-owner-popup");
            "replacement-committed"
            "#,
        )
        .expect("named popup replacement should commit"),
        "replacement-committed"
    );
    let replacement_local_window_id = vm
        ._context_host
        .borrow()
        .current_lightweight_popup_local_window_id(popup_id)
        .expect("replacement popup LocalWindow owner");
    assert_ne!(replacement_local_window_id, initial_local_window_id);
    assert_eq!(
        vm._context_host.borrow().pending_webcrypto_task_count(),
        0,
        "popup replacement must retire old-LocalWindow WebCrypto resolvers"
    );
}
#[test]
fn isolated_realm_destruction_retires_xhr_without_retiring_local_window() {
    let mut vm = new_storage_test_vm("https://isolated-xhr-owner.test/");
    let main_owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let isolated_context_id = vm
        .create_isolated_world("xhr-owner", false)
        .expect("isolated world should be created");
    let isolated_context_ptr = {
        let world = vm
            .page_isolated_world_contexts
            .context(isolated_context_id)
            .expect("isolated world should be tracked");
        &world.context as *const _
    };
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(
        isolated_context_ptr,
        |scope, host_ptr| {
            let (_, owner, _) = register_pending_window_xhr_for_test(
                scope,
                unsafe { &mut *host_ptr },
                cancel_handle.clone(),
            );
            assert_eq!(
                owner,
                crate::native_bridge::WindowExecutionContextOwner::Frame(
                    main_owner.local_window_id
                )
            );
            Ok(())
        },
    )
    .expect("isolated XHR should register");

    vm.destroy_isolated_world_context(isolated_context_id);

    assert!(
        vm._context_host
            .borrow()
            .pending_window_xhr_execution_contexts_for_test()
            .is_empty(),
        "destroying the XHR relevant realm must release its wrapper and request"
    );
    assert!(cancel_handle.is_cancelled());
    assert_eq!(
        vm.current_main_document_task_owner()
            .map(|owner| owner.local_window_id),
        Some(main_owner.local_window_id),
        "realm retirement must not retire the owning LocalWindow"
    );
}
#[test]
fn popup_replacement_retires_local_window_owned_xhr() {
    let mut vm = new_storage_test_vm("https://popup-owner-xhr.test/");
    assert_eq!(
        vm.eval(
            r#"
            globalThis.__ownerBoundXhrPopup = open("about:blank", "xhr-owner-popup");
            String(globalThis.__ownerBoundXhrPopup !== null)
            "#,
        )
        .expect("popup XHR owner window should open"),
        "true"
    );
    let popup_id = vm
        .take_pending_popup_activations()
        .into_iter()
        .next()
        .and_then(|activation| activation.popup_id())
        .expect("popup XHR owner id");
    let initial_local_window_id = vm
        ._context_host
        .borrow()
        .current_lightweight_popup_local_window_id(popup_id)
        .expect("initial popup LocalWindow owner");
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let (_, owner, _) = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let previous_popup =
                crate::native_bridge::enter_active_lightweight_popup_scope(scope, popup_id);
            let registered = register_pending_window_xhr_for_test(
                scope,
                unsafe { &mut *host_ptr },
                cancel_handle.clone(),
            );
            crate::native_bridge::restore_active_lightweight_popup_scope(scope, previous_popup);
            Ok(registered)
        })
        .expect("popup XHR should register");
    assert_eq!(
        owner,
        crate::native_bridge::WindowExecutionContextOwner::LightweightPopup {
            popup_id,
            local_window_id: initial_local_window_id,
        }
    );

    vm.eval(r#"open("about:blank", "xhr-owner-popup"); "replacement-committed""#)
        .expect("named popup replacement should commit");

    assert!(
        vm._context_host
            .borrow()
            .pending_window_xhr_execution_contexts_for_test()
            .is_empty(),
        "popup replacement must remove old-LocalWindow XHR state"
    );
    assert!(
        cancel_handle.is_cancelled(),
        "popup replacement must abort old-LocalWindow XHR transport"
    );
}
#[test]
fn isolated_realm_destruction_aborts_fetch_and_detaches_keepalive() {
    let mut vm = new_storage_test_vm("https://isolated-fetch-owner.test/");
    let main_owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let isolated_context_id = vm
        .create_isolated_world("fetch-owner", false)
        .expect("isolated world should be created");
    let isolated_context_ptr = {
        let world = vm
            .page_isolated_world_contexts
            .context(isolated_context_id)
            .expect("isolated world should be tracked");
        &world.context as *const _
    };
    let (ordinary, keepalive) = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(
            isolated_context_ptr,
            |scope, host_ptr| {
                let host = unsafe { &mut *host_ptr };
                Ok((
                    register_pending_window_fetch_for_test(
                        scope,
                        host,
                        false,
                        PendingWindowFetchTestStage::Pending,
                    ),
                    register_pending_window_fetch_for_test(
                        scope,
                        host,
                        true,
                        PendingWindowFetchTestStage::Pending,
                    ),
                ))
            },
        )
        .expect("isolated Fetches should register");

    vm.destroy_isolated_world_context(isolated_context_id);

    assert!(ordinary.3.is_cancelled());
    assert!(!keepalive.3.is_cancelled());
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_fetch_execution_contexts_for_test(),
        vec![(keepalive.0, true, Some(keepalive.1), Some(keepalive.2))]
    );
    assert_eq!(
        vm.current_main_document_task_owner()
            .map(|owner| owner.local_window_id),
        Some(main_owner.local_window_id),
        "realm retirement must not retire the owning LocalWindow"
    );
    let request_url = Url::parse("https://fetch-execution-context.test/pending").unwrap();
    let body_source_id = 50_000 + keepalive.0;
    vm.start_streaming_async_subresource_fetch(crate::types::AsyncSubresourceStreamingStarted {
        skip_fetch_security_validation: false,
        response_filter: None,
        internal_id: keepalive.0,
        request_url: request_url.clone(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new().into(),
        request_body: None,
        body_source_id,
        network_request_headers: None,
        head: moli_fetch::ResponseHead {
            final_url: request_url,
            status: 200,
            headers: vec![("content-type".to_owned(), b"text/plain".to_vec())],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        },
    })
    .expect("detached keepalive should accept streaming headers without V8");
    vm.append_streaming_async_subresource_fetch_chunk(
        body_source_id,
        b"detached streaming body".to_vec(),
    );
    vm.finish_streaming_async_subresource_fetch(keepalive.0, body_source_id, Ok(()))
        .expect("detached keepalive stream should finish without V8");
    assert!(
        vm._context_host
            .borrow()
            .pending_window_fetch_execution_contexts_for_test()
            .is_empty(),
        "detached streaming terminal must release its host state"
    );
    assert!(!keepalive.3.is_cancelled());
    assert_eq!(
        vm.take_network_output()
            .into_items()
            .filter(|item| matches!(
                item,
                crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
                    if record.url().as_str()
                        == "https://fetch-execution-context.test/pending"
            ))
            .count(),
        1,
        "detached streaming keepalive must preserve terminal network observation"
    );
}
#[test]
fn popup_replacement_aborts_fetch_and_detaches_keepalive() {
    let mut vm = new_storage_test_vm("https://popup-owner-fetch.test/");
    assert_eq!(
        vm.eval(
            r#"
            globalThis.__ownerBoundFetchPopup = open("about:blank", "fetch-owner-popup");
            String(globalThis.__ownerBoundFetchPopup !== null)
            "#,
        )
        .expect("popup Fetch owner window should open"),
        "true"
    );
    let popup_id = vm
        .take_pending_popup_activations()
        .into_iter()
        .next()
        .and_then(|activation| activation.popup_id())
        .expect("popup Fetch owner id");
    let (ordinary, keepalive) = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let previous_popup =
                crate::native_bridge::enter_active_lightweight_popup_scope(scope, popup_id);
            let host = unsafe { &mut *host_ptr };
            let registered = (
                register_pending_window_fetch_for_test(
                    scope,
                    host,
                    false,
                    PendingWindowFetchTestStage::Pending,
                ),
                register_pending_window_fetch_for_test(
                    scope,
                    host,
                    true,
                    PendingWindowFetchTestStage::Pending,
                ),
            );
            crate::native_bridge::restore_active_lightweight_popup_scope(scope, previous_popup);
            Ok(registered)
        })
        .expect("popup Fetches should register");

    vm.eval(r#"open("about:blank", "fetch-owner-popup"); "replacement-committed""#)
        .expect("named popup replacement should commit");

    assert!(ordinary.3.is_cancelled());
    assert!(!keepalive.3.is_cancelled());
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_fetch_execution_contexts_for_test(),
        vec![(keepalive.0, true, Some(keepalive.1), Some(keepalive.2))]
    );
    assert!(
        vm._context_host
            .borrow_mut()
            .abort_subresource_fetch(keepalive.0)
    );
}
#[test]
fn isolated_world_bridge_ref_is_released_with_script_vm() {
    let mut vm = new_storage_test_vm("https://example.test/");
    let context_host = vm.context_host_weak_for_test();

    vm.ensure_isolated_world_for_owner(None, "bridge-ref-regression", false)
        .expect("isolated world should be created");
    assert!(
        context_host.upgrade().is_some(),
        "context host should stay alive while the ScriptVm owns its contexts"
    );

    drop(vm);
    assert!(
        context_host.upgrade().is_none(),
        "dropping ScriptVm should release every V8 bridge Rc ref-count"
    );
}
#[test]
fn promise_reject_context_slot_does_not_retain_context_host_after_script_vm_drop() {
    let mut vm = new_parsed_test_vm("https://example.test/", "<!doctype html><p>promise</p>");
    let context_host = vm.context_host_weak_for_test();

    let retained_slot = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
            scope
                .get_current_context()
                .get_slot::<super::runtime_bindings::PromiseRejectDispatchSlot>()
                .ok_or_else(|| anyhow::anyhow!("promise reject dispatch slot missing"))
        })
        .expect("promise reject dispatch slot should be installed");
    assert!(
        context_host.upgrade().is_some(),
        "context host should stay alive while ScriptVm owns the page context"
    );

    drop(vm);
    assert!(
        context_host.upgrade().is_none(),
        "retaining the V8 context slot must not keep the page context host alive"
    );
    assert!(
        retained_slot.host_weak.upgrade().is_none(),
        "promise rejection slot should only keep a weak host reference"
    );
}
#[test]
fn context_wrapper_cache_releases_native_roots_on_script_vm_teardown() {
    let mut vm = new_parsed_test_vm(
        "https://wrapper-cache-retention.test/",
        "<!doctype html><main></main>",
    );

    let retained_cache = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
            Ok(crate::native_bridge::identity::retain_context_wrapper_cache_for_test(scope))
        })
        .expect("wrapper cache should be retainable for regression testing");

    let baseline = vm
        .eval(
            r#"
(() => {
  void document.body;
  return "baseline";
})()
"#,
        )
        .expect("wrapper cache baseline should evaluate");
    assert_eq!(baseline, "baseline");

    let created = vm
        .eval(
            r#"
(() => {
  for (let index = 0; index < 64; index += 1) {
    document.createElement("span");
  }
  return "created";
})()
"#,
        )
        .expect("wrapper cache setup should evaluate");
    assert_eq!(created, "created");
    assert!(
        retained_cache.wrapper_entry_count() >= 64,
        "transient DOM wrappers should populate the per-context wrapper cache"
    );

    drop(vm);
    assert_eq!(
        retained_cache.strong_wrapper_entry_count(),
        0,
        "page context teardown must clear strong wrapper cache entries before contexts are dropped"
    );
}
#[test]
fn script_vm_page_context_teardown_is_idempotent() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            "https://teardown-idempotent.test/page.html",
            &loader,
        );
    assert_eq!(
        browser_context_runtime
            .service_worker_runtime()
            .diagnostics_snapshot()
            .live_client_count,
        1,
        "new ScriptVm should register its top-level service worker window client"
    );

    vm.close_page_context_resources_for_context_teardown();
    assert_eq!(
        browser_context_runtime
            .service_worker_runtime()
            .diagnostics_snapshot()
            .live_client_count,
        0,
        "first context teardown should unregister the top-level window client"
    );

    vm.close_page_context_resources_for_context_teardown();
    drop(vm);
    assert_eq!(
        browser_context_runtime
            .service_worker_runtime()
            .diagnostics_snapshot()
            .live_client_count,
        0,
        "repeated teardown and ScriptVm drop should not touch already closed page resources"
    );
}
#[test]
fn page_context_teardown_preserves_finalizers_for_retained_native_objects() {
    let mut vm = new_parsed_test_vm(
        "https://v8-finalizer-teardown.test/",
        "<!doctype html><body></body>",
    );

    let created = vm
        .eval(
            r#"
(() => {
  globalThis.__finalizerObjects = [];
  for (let index = 0; index < 32; index += 1) {
    const element = document.createElement("div");
    element.style.color = "red";

    const sheet = new CSSStyleSheet();
    sheet.replaceSync(`.item-${index} { color: red; }`);
    sheet.cssRules[0].style.setProperty("color", "blue");

    const blob = new Blob([`payload-${index}`], { type: "text/plain" });
    globalThis.__finalizerObjects.push(element, sheet, blob);
  }
  globalThis.__finalizerPerformance = performance;
  performance.setResourceTimingBufferSize(150);
  return globalThis.__finalizerObjects.length;
})()
"#,
        )
        .expect("context-owned finalizer objects should evaluate");
    assert_eq!(created, "96");
    assert!(
        vm._context_host.borrow().v8_finalizers.len() >= 128,
        "CSS declaration/rule-tree and Blob objects should be tracked by the page context owner"
    );
    assert!(
        vm._context_host
            .borrow()
            .resource_timing_buffer_count_for_test()
            >= 1,
        "the top-level Performance buffer should be owned by the host registry"
    );

    vm.close_page_context_resources_for_context_teardown();
    assert!(
        vm._context_host.borrow().v8_finalizers.len() >= 128,
        "retiring active execution must preserve native objects retained by author code"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .resource_timing_buffer_count_for_test(),
        0,
        "Performance finalization must remove host-side buffer state"
    );

    vm.close_page_context_resources_for_context_teardown();
    assert_eq!(vm.eval("__finalizerObjects[2].size").unwrap(), "9");
    let weak_host = vm.context_host_weak_for_test();
    drop(vm);
    assert!(
        weak_host.upgrade().is_none(),
        "final isolate release must drop the native host"
    );
}
#[test]
fn embedded_frame_owners_create_child_contexts_only_for_document_content() {
    let mut vm = new_storage_html_test_vm("https://embedded-frame-owner-selection.test/");

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const root = document.body || document.documentElement || document;
  const append = (tag, id, attribute, value, type = "") => {
    const element = document.createElement(tag);
    element.id = id;
    if (id === "svg-embed") element.name = "svg_child";
    if (type) element.type = type;
    element[attribute] = value;
    root.appendChild(element);
  };
  append("iframe", "accepted-iframe", "src", "/child.html?iframe");
  append("frame", "accepted-frame", "src", "/child.html?frame");
  append("embed", "accepted-embed", "src", "/child.html?embed");
  append("object", "accepted-object", "data", "/child.html?object");
  append("embed", "svg-embed", "src", "/graphic.svg", "image/svg+xml");
  append("embed", "inferred-svg-embed", "src", "/graphic.svg");
  append("object", "svg-object", "data", "/graphic.svg", "image/svg+xml");
  append("embed", "image-embed", "src", "/image.png");
  append("object", "image-object", "data", "/image.png", "image/png");
  append("object", "plugin-object", "data", "/child.html", "application/x-test-plugin");

  for (const tag of ["audio", "video"]) {
    const media = document.createElement(tag);
    const embed = document.createElement("embed");
    embed.id = `${tag}-embed`;
    embed.type = "text/html";
    embed.src = `/${tag}-embed.html`;
    media.appendChild(embed);
    const object = document.createElement("object");
    object.id = `${tag}-object`;
    object.type = "text/html";
    object.data = `/${tag}-object.html`;
    media.appendChild(object);
    root.appendChild(media);
  }
  return "created";
})()
"#,
        )
        .expect("embedded frame-owner selection should evaluate"),
        "created"
    );

    let host = vm._context_host.borrow();
    assert_eq!(host.child_browsing_context_count(), 7);
    for id in [
        "accepted-iframe",
        "accepted-frame",
        "accepted-embed",
        "accepted-object",
        "svg-embed",
        "inferred-svg-embed",
        "svg-object",
    ] {
        let handle = host
            .dom_host()
            .element_handle_by_id(id)
            .expect("accepted frame owner should exist");
        assert!(
            host.child_browsing_context_document_handle(handle)
                .is_some(),
            "{id} should own an initial-empty child document"
        );
    }
    for id in [
        "image-embed",
        "image-object",
        "plugin-object",
        "audio-embed",
        "audio-object",
        "video-embed",
        "video-object",
    ] {
        let handle = host
            .dom_host()
            .element_handle_by_id(id)
            .expect("rejected embedded element should exist");
        assert!(
            host.child_browsing_context_document_handle(handle)
                .is_none(),
            "{id} must not be projected as a child browsing context"
        );
    }
    drop(host);

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const embed = document.getElementById("svg-embed");
  return [
    window.length,
    window.svg_child.frameElement === embed
  ].join("|");
})()
"#,
        )
        .expect("SVG embedded document should expose a named child window"),
        "7|true"
    );

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const object = document.getElementById("accepted-object");
  const contentDocument = object.contentDocument;
  const contentWindow = object.contentWindow;
  return [
    contentDocument !== null,
    contentWindow !== null,
    contentDocument === contentWindow.document
  ].join("|");
})()
"#,
        )
        .expect("object child browsing context accessors should evaluate"),
        "true|true|true"
    );

    assert_eq!(
        vm.eval(
            r#"
(() => {
  document.getElementById("accepted-embed").type = "image/png";
  document.getElementById("accepted-object").data = "/image.png";
  return "reclassified";
})()
"#,
        )
        .expect("connected embedded frame owners should reclassify"),
        "reclassified"
    );
    assert_eq!(
        vm._context_host.borrow().child_browsing_context_count(),
        5,
        "switching accepted embedded content to image content must retire both child contexts"
    );

    assert_eq!(
        vm.eval(
            r#"
(() => {
  document.getElementById("accepted-embed").type = "text/html";
  document.getElementById("accepted-object").data = "/replacement.html";
  return "restored";
})()
"#,
        )
        .expect("connected embedded document owners should restore"),
        "restored"
    );
    assert_eq!(
        vm._context_host.borrow().child_browsing_context_count(),
        7,
        "switching back to document content must create fresh child contexts"
    );
}
#[tokio::test]
async fn empty_named_preload_materializes_child_isolated_world() {
    let mut vm = new_storage_test_vm("https://child-empty-named-preload.test/");
    vm.set_stored_document_start_scripts(&[crate::DocumentStartScript {
        registry_key: Some("empty-utility-world".to_owned()),
        devtools_session: None,
        source: String::new(),
        world_name: Some("playwright-utility".to_owned()),
        has_bidi_channel_argument: false,
        bidi_channel_handoffs: Vec::new(),
    }]);

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.srcdoc = "<!doctype html><body>child</body>";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("empty named preload child setup should evaluate");

    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "empty named preload child document",
    )
    .await;
    let frame_id = {
        let host = vm._context_host.borrow();
        let handles = host.child_browsing_context_handles_in_document_order();
        assert_eq!(handles.len(), 1, "expected one child browsing context");
        host.child_browsing_context_frame_id_by_owner_node_id(handles[0])
            .expect("child browsing context should have a frame id")
    };
    assert!(
        vm.has_isolated_world_named_for_frame(&frame_id, "playwright-utility"),
        "an empty world-scoped preload must still declare the child isolated world"
    );
}
#[tokio::test]
async fn child_body_onload_materializes_default_context_at_host_load() {
    let mut vm = new_storage_test_vm("https://child-body-onload-lazy.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__childBodyOnloadEvents = [];
  const frame = document.createElement("iframe");
  frame.srcdoc = `<body onload="parent.__childBodyOnloadEvents.push(globalThis === self)">`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child body onload setup should evaluate");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child body onload srcdoc should commit before lifecycle",
    )
    .await;
    assert_eq!(
        vm.child_frame_realm_store.len(),
        0,
        "a native body onload attribute should not materialize its realm before load dispatch"
    );
    for transition in ["interactive", "DOMContentLoaded", "complete"] {
        run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &format!("child body onload should run its {transition} lifecycle turn"),
        )
        .await;
    }
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "child body onload should dispatch from HostLoad",
    )
    .await;

    assert_eq!(
        vm.eval("__childBodyOnloadEvents.join('|')")
            .expect("child body onload trace should evaluate"),
        "true"
    );
    assert_eq!(
        vm.child_frame_realm_store.len(),
        1,
        "observable child window load work should materialize exactly one default realm"
    );
}
#[tokio::test]
async fn child_execution_context_exec_runs_as_frame_script_job() {
    let mut vm = new_storage_test_vm("https://child-context-exec-driver.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child exec frame setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child execution-context setup",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child execution-context setup");
    let owner_realm_id = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist")
        .owner_realm_id;

    vm.exec_in_execution_context(
        child_context_id,
        "globalThis.__childContextExecFrameJob = globalThis === self ? 37 : -1;",
    )
    .expect("child execution context source should execute through frame script job");

    let observed = vm
        .eval_in_frame_realm(
            owner_realm_id,
            "String(globalThis.__childContextExecFrameJob)",
        )
        .expect("child execution context side effect should be visible in child realm");
    assert_eq!(observed, "37");
    let parent_observed = vm
        .eval("String(globalThis.__childContextExecFrameJob)")
        .expect("parent realm should evaluate");
    assert_eq!(parent_observed, "undefined");
}
#[tokio::test]
async fn pre_realm_modulepreload_rejects_the_first_established_realm_after_replacement() {
    let (mut vm, modulepreload_source) = new_child_modulepreload_page_test_vm(
        "https://child-modulepreload-pre-realm-replaced.test/",
    );

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "replace-first-modulepreload-realm";
  frame.srcdoc = `<link rel="modulepreload" href="/must-not-start.mjs">`;
  body.appendChild(frame);
})()
"#,
    )
    .expect("first-realm replacement fixture should evaluate");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit)
    );
    let child_handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handles_in_document_order()
        .into_iter()
        .next()
        .expect("first-realm replacement fixture should retain one child");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_modulepreload_work_awaiting_realm_for_test(),
        1
    );

    vm.eval(
        "void document.getElementById('replace-first-modulepreload-realm').contentWindow.Function",
    )
    .expect("first child Window exposure should establish semantic realm identity");
    let first_realm = vm
        ._context_host
        .borrow()
        .frame_owner_current_child_snapshot(child_handle)
        .and_then(|snapshot| snapshot.realm_id)
        .expect("first Window exposure should establish a realm id");
    vm._context_host
        .borrow_mut()
        .clear_child_default_execution_context_id(child_handle);
    vm.eval(
        "void document.getElementById('replace-first-modulepreload-realm').contentWindow.Function",
    )
    .expect("second child Window exposure should establish replacement realm identity");
    let replacement_realm = vm
        ._context_host
        .borrow()
        .frame_owner_current_child_snapshot(child_handle)
        .and_then(|snapshot| snapshot.realm_id)
        .expect("second Window exposure should establish a replacement realm id");
    assert_ne!(first_realm, replacement_realm);

    assert!(
        vm.run_one_child_realm_materialization_body_for_test()
            .expect("child realm materialization body should succeed")
            .is_some(),
        "the replacement realm still owns one materialization turn"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_modulepreload_work_awaiting_realm_for_test(),
        0,
        "the stale first-realm task should be consumed as a discard"
    );
    assert!(
        !modulepreload_source.has_ready_task(),
        "work stamped by the first established realm must not rebind to its replacement"
    );
}
#[test]
fn resource_owner_id_is_available_from_current_context_slot() {
    let mut vm = new_parsed_test_vm("https://example.test/", "<!doctype html><p>owner</p>");
    let expected = vm.resource_owner_id;

    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
        assert_eq!(
            scope
                .get_current_context()
                .get_slot::<crate::resource_owner::ResourceOwnerId>()
                .as_deref()
                .copied(),
            Some(expected)
        );
        assert_eq!(
            crate::resource_owner::current_resource_owner_id(scope),
            Some(expected)
        );
        assert!(
            scope
                .get_slot::<crate::resource_owner::ResourceOwnerId>()
                .is_none()
        );
        Ok(())
    })
    .expect("resource owner id should be visible from current context");
}
#[test]
fn runtime_observable_context_token_is_available_from_current_context_slot() {
    let mut vm = new_parsed_test_vm("https://example.test/", "<!doctype html><p>runtime</p>");
    let expected = vm.page_default_runtime.runtime_observable_context_token;

    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
        assert_eq!(
            scope
                .get_current_context()
                .get_slot::<crate::native_bridge::RuntimeObservableContextToken>()
                .as_deref()
                .copied(),
            Some(expected)
        );
        assert_eq!(
            crate::native_bridge::current_runtime_observable_context_token(scope),
            Some(expected)
        );
        assert!(
            scope
                .get_slot::<crate::native_bridge::RuntimeObservableContextToken>()
                .is_none()
        );
        Ok(())
    })
    .expect("runtime observable context token should be visible from current context");
}
#[test]
fn promise_reject_dispatch_is_available_from_current_context_slot() {
    let mut vm = new_parsed_test_vm("https://example.test/", "<!doctype html><p>promise</p>");

    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
        assert!(
            scope
                .get_current_context()
                .get_slot::<super::runtime_bindings::PromiseRejectDispatchSlot>()
                .is_some()
        );
        assert!(super::runtime_bindings::promise_reject_dispatch_is_available_for_test(scope));
        assert!(
            scope
                .get_slot::<super::runtime_bindings::PromiseRejectDispatchSlot>()
                .is_none()
        );
        Ok(())
    })
    .expect("promise reject dispatch should be visible from current context");
}
#[test]
fn indexed_db_manager_is_available_from_context_slots() {
    let mut vm = new_storage_test_vm("https://indexeddb-context-slot.test/");

    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
        assert!(crate::context_bootstrap::indexed_db_manager_context_slot_present_for_test(scope));
        assert!(!crate::context_bootstrap::indexed_db_manager_isolate_slot_present_for_test(scope));
        Ok(())
    })
    .expect("indexedDB manager should be visible from default context");

    let isolated_context_id = vm
        .create_isolated_world("indexeddb-context-slot", false)
        .expect("isolated world should be created");
    let isolated_context_ptr = {
        let world = vm
            .page_isolated_world_contexts
            .context(isolated_context_id)
            .expect("isolated world context should be tracked");
        &world.context as *const _
    };
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(
        isolated_context_ptr,
        |scope, _runtime_ptr| {
            assert!(
                crate::context_bootstrap::indexed_db_manager_context_slot_present_for_test(scope)
            );
            assert!(
                !crate::context_bootstrap::indexed_db_manager_isolate_slot_present_for_test(scope)
            );
            Ok(())
        },
    )
    .expect("indexedDB manager should be visible from isolated context");
}
#[test]
fn inspector_context_created_matches_same_name_child_isolated_world_by_frame_id() {
    let mut vm = new_storage_test_vm("https://isolated-world-frame-match.test/");
    vm.root_frame_id = Some("root-frame".to_owned());

    let root_context_id = vm
        .create_isolated_world("shared-utility", false)
        .expect("root isolated world should be created");
    let child_context_id = vm
        .create_new_isolated_world(
            None,
            "shared-utility",
            false,
            Some("child-frame".to_owned()),
            None,
        )
        .expect("child-frame isolated world should be created");
    assert_ne!(root_context_id, child_context_id);
    assert_eq!(vm.page_isolated_world_contexts.len(), 2);

    let root_frame_id = vm.root_frame_id.clone();
    vm.page_isolated_world_contexts
        .record_inspector_context_state(
            &[serde_json::json!({
                "method": "Runtime.executionContextCreated",
                "params": {
                    "context": {
                        "id": child_context_id,
                        "uniqueId": "child-frame-replayed-realm",
                        "name": "shared-utility",
                        "auxData": {
                            "type": "isolated",
                            "frameId": "child-frame"
                        }
                    }
                }
            })],
            root_frame_id.as_deref(),
        );

    assert!(
        vm.page_isolated_world_contexts
            .has_execution_context_id(root_context_id),
        "child-frame inspector event must not re-key the root isolated world"
    );
    let child_world = vm
        .page_isolated_world_contexts
        .context(child_context_id)
        .expect("child isolated world should remain keyed by its execution context id");
    assert_eq!(child_world.frame_id.as_deref(), Some("child-frame"));
    assert_eq!(
        child_world.inspector_execution_context_realm_id.as_deref(),
        Some("child-frame-replayed-realm")
    );
    assert_eq!(vm.page_isolated_world_contexts.len(), 2);
}
#[test]
fn same_name_isolated_worlds_are_scoped_to_devtools_session_and_detach() {
    let mut vm = new_storage_test_vm("https://isolated-world-session.test/");
    let session_a = moli_page_types::DevToolsSessionKey::from_wire_session_id(Some("session-a"));
    let session_b = moli_page_types::DevToolsSessionKey::from_wire_session_id(Some("session-b"));

    let context_a = vm
        .ensure_isolated_world_for_owner(Some(&session_a), "utility", false)
        .expect("session A isolated world should be created");
    let context_b = vm
        .ensure_isolated_world_for_owner(Some(&session_b), "utility", false)
        .expect("session B same-name isolated world should be distinct");
    assert_ne!(context_a, context_b);
    vm.eval_in_isolated_context(context_a, "globalThis.owner = 'session-a'")
        .expect("session A isolated world should evaluate");
    vm.eval_in_isolated_context(context_b, "globalThis.owner = 'session-b'")
        .expect("session B isolated world should evaluate");
    assert_eq!(
        vm.eval_in_isolated_context(context_a, "owner")
            .expect("session A isolated world should retain its state"),
        "session-a"
    );
    assert_eq!(
        vm.eval_in_isolated_context(context_b, "owner")
            .expect("session B isolated world should retain its state"),
        "session-b"
    );

    assert!(vm.detach_runtime_inspector_session(Some("session-a")));
    assert!(
        vm.page_isolated_world_contexts.context(context_a).is_none(),
        "detaching session A must retire only its isolated world"
    );
    assert_eq!(
        vm.eval_in_isolated_context(context_b, "owner")
            .expect("session B isolated world should survive peer detach"),
        "session-b"
    );

    let replacement_session =
        moli_page_types::DevToolsSessionKey::from_wire_session_id(Some("session-c"));
    let replacement_context = vm
        .ensure_isolated_world_for_owner(Some(&replacement_session), "utility", false)
        .expect("replacement session isolated world should be created");
    assert_eq!(
        vm.eval_in_isolated_context(replacement_context, "typeof owner")
            .expect("replacement session isolated world should evaluate"),
        "undefined"
    );
}

#[test]
fn frame_owner_get_svg_document_methods_reject_html_documents_and_enforce_brands() {
    let mut vm = new_storage_test_vm("https://frame-owner-get-svg-document.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    const html = document.createElement('html');
    html.appendChild(document.createElement('body'));
    document.appendChild(html);
  }
  const root = document.body || document.documentElement;
  const iframe = document.createElement("iframe");
  iframe.srcdoc = "<body>iframe child</body>";
  const embed = document.createElement("embed");
  embed.type = "text/html";
  embed.src = "about:blank";
  const object = document.createElement("object");
  object.type = "text/html";
  object.data = "about:blank";
  root.appendChild(iframe);
  root.appendChild(embed);
  root.appendChild(object);

  const interfaces = [
    [HTMLIFrameElement.prototype, iframe, "HTMLIFrameElement"],
    [HTMLEmbedElement.prototype, embed, "HTMLEmbedElement"],
    [HTMLObjectElement.prototype, object, "HTMLObjectElement"]
  ];
  const descriptors = interfaces.map(([prototype]) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, "getSVGDocument");
    return {
      type: typeof descriptor.value,
      name: descriptor.value.name,
      length: descriptor.value.length,
      writable: descriptor.writable,
      enumerable: descriptor.enumerable,
      configurable: descriptor.configurable
    };
  });
  const brandErrors = interfaces.map(([prototype], index) => {
    try {
      prototype.getSVGDocument.call(interfaces[(index + 1) % interfaces.length][1]);
      return "accepted";
    } catch (error) {
      return error.name;
    }
  });

  return JSON.stringify({
    descriptors,
    iframeIsNull: iframe.getSVGDocument() === null,
    embedIsNull: embed.getSVGDocument() === null,
    objectIsNull: object.getSVGDocument() === null,
    brandErrors,
    absentFromBase: !("getSVGDocument" in HTMLElement.prototype)
  });
})()
"#,
        )
        .expect("frame owner getSVGDocument methods should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":[{"type":"function","name":"getSVGDocument","length":0,"writable":true,"enumerable":true,"configurable":true},{"type":"function","name":"getSVGDocument","length":0,"writable":true,"enumerable":true,"configurable":true},{"type":"function","name":"getSVGDocument","length":0,"writable":true,"enumerable":true,"configurable":true}],"iframeIsNull":true,"embedIsNull":true,"objectIsNull":true,"brandErrors":["TypeError","TypeError","TypeError"],"absentFromBase":true}"#
    );
}

#[test]
fn frame_owner_get_svg_document_uses_native_document_content_type() {
    let mut vm = new_storage_html_test_vm("https://frame-owner-svg-type.test/");
    vm.eval(
        r#"
for (const tag of ['iframe', 'embed', 'object']) {
  const owner = document.createElement(tag);
  owner.id = tag;
  if (tag === 'object') owner.data = 'about:blank';
  else owner.src = 'about:blank';
  document.body.appendChild(owner);
  if (owner.getSVGDocument() !== null) throw new Error('HTML document accepted');
}
"#,
    )
    .expect("frame owner setup should evaluate");

    // Simulate the response MIME metadata at a child document commit. Parsing
    // and frame navigation are covered separately; this checks the public
    // method against native metadata rather than author-visible properties.
    {
        let mut host = vm._context_host.borrow_mut();
        for id in ["iframe", "embed", "object"] {
            let owner = host.dom_host().element_handle_by_id(id).unwrap();
            let document = host.child_browsing_context_document_handle(owner).unwrap();
            host.set_dom_document_content_type_for_handle(document, "image/svg+xml");
        }
    }
    assert_eq!(
        vm.eval(
            r#"
['iframe', 'embed', 'object'].every(id => {
  const owner = document.getElementById(id);
  const svg = owner.getSVGDocument();
  if (svg === null || svg.contentType !== 'image/svg+xml') return false;
  if (id !== 'embed' && svg !== owner.contentDocument) return false;
  Object.defineProperty(svg, 'contentType', {value: 'text/html', configurable: true});
  return owner.getSVGDocument() === svg;
})
"#
        )
        .expect("native SVG document metadata should determine the result"),
        "true"
    );
}

async fn commit_opaque_child_for_test(vm: &mut StandaloneScriptVmHarness, element_id: &str) -> i64 {
    let created = vm
        .eval(&format!(
            r#"
(() => {{
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = {element_id:?};
  frame.sandbox = "allow-scripts";
  frame.srcdoc = "<p id='opaque-marker'>opaque child</p>";
  body.appendChild(frame);
  void frame.contentWindow;
  return "created";
}})()
"#
        ))
        .expect("opaque child setup should evaluate");
    assert_eq!(created, "created");
    run_child_navigation_commit_and_host_load_for_test(vm, element_id).await;
    vm.live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .unwrap_or_else(|| panic!("{element_id}: opaque child realm should exist"))
}

fn child_context_weak_for_test(
    vm: &mut StandaloneScriptVmHarness,
    context_id: i64,
) -> v8::Weak<v8::Context> {
    vm.with_child_frame_realm_context_scope(context_id, |scope, _| {
        let context = scope.get_current_context();
        Ok(v8::Weak::new(scope, context))
    })
    .expect("child context should still be reachable")
}

fn collect_isolate_garbage_for_test(vm: &StandaloneScriptVmHarness) {
    let isolate = vm.renderer_document_isolate.clone();
    for _ in 0..5 {
        isolate.with_renderer_document_isolate_mut(|isolate| {
            isolate.low_memory_notification();
        });
    }
}

fn context_weak_was_collected_for_test(
    vm: &mut StandaloneScriptVmHarness,
    weak: &v8::Weak<v8::Context>,
) -> bool {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        Ok(weak.to_local(scope).is_none())
    })
    .expect("parent context should remain usable")
}

fn expose_real_top_window_for_test(vm: &mut StandaloneScriptVmHarness, context_id: i64) {
    // `parent` on an opaque child is a caller-local projection. The cache and
    // borrowed-receiver bugs live on the real top Window.
    vm.with_child_frame_realm_context_scope(context_id, |scope, host_ptr| {
        let parent_context = unsafe { &*host_ptr }
            .page_default_context(scope)
            .expect("parent context");
        let parent_window = parent_context.global(scope);
        let key = v8::String::new(scope, "__realTopWindow").expect("property name");
        let installed = scope
            .get_current_context()
            .global(scope)
            .set(scope, key.into(), parent_window.into())
            .unwrap_or(false);
        assert!(installed, "opaque child should retain the real top Window");
        Ok(())
    })
    .expect("real top Window should be reachable from the opaque child");
}

#[tokio::test]
async fn borrowed_cross_origin_window_accessors_follow_the_receiver() {
    let mut vm = new_storage_test_vm("https://cross-origin-window-receiver.test/");
    let child_context_id = commit_opaque_child_for_test(&mut vm, "receiver-frame").await;
    expose_real_top_window_for_test(&mut vm, child_context_id);
    let parent_hash_before = vm
        .eval("location.hash")
        .expect("parent hash should be readable");

    let result = vm
        .eval_in_child_default_context(
            child_context_id,
            r##"
(() => {
  const w = __realTopWindow;
  const getParent = Object.getOwnPropertyDescriptor(w, "parent").get;
  const setLocation = Object.getOwnPropertyDescriptor(w, "location").set;
  let plain = "no-throw";
  try {
    getParent.call({});
  } catch (error) {
    plain = error instanceof TypeError ? "type-error" : String(error && error.name);
  }
  setLocation.call(window, "#pr957-receiver");
  return JSON.stringify({
    plain,
    normalParentIsSelf: w.parent === w,
    borrowedParent: getParent.call(window) === parent,
    borrowedState: ['closed', 'length', 'opener', 'parent', 'top', 'location'].every(
      name => Object.getOwnPropertyDescriptor(w, name).get.call(window) === window[name]),
    childHash: location.hash,
    postMessagePrototype: Object.getPrototypeOf(w.postMessage) === Function.prototype
  });
})()
"##,
        )
        .expect("borrowed cross-origin accessors should evaluate");
    assert_eq!(
        result,
        r##"{"plain":"type-error","normalParentIsSelf":true,"borrowedParent":true,"borrowedState":true,"childHash":"#pr957-receiver","postMessagePrototype":true}"##
    );
    assert_eq!(
        vm.eval("location.hash")
            .expect("parent hash should stay readable"),
        parent_hash_before,
        "borrowing the cross-origin location setter must not navigate the captured window"
    );
}

#[tokio::test]
async fn cross_origin_surface_cache_releases_removed_accessor_realm() {
    let mut vm = new_storage_test_vm("https://cross-origin-surface-cache.test/");

    let control_id = commit_opaque_child_for_test(&mut vm, "control-frame").await;
    let control_weak = child_context_weak_for_test(&mut vm, control_id);
    assert_eq!(
        vm.eval(
            r#"
document.getElementById("control-frame").remove();
"removed"
"#
        )
        .expect("control frame removal should evaluate"),
        "removed"
    );
    assert!(
        vm.live_child_default_runtime_realm_inventory().is_empty(),
        "removing the control frame should drop its realm record"
    );
    collect_isolate_garbage_for_test(&vm);
    assert!(
        context_weak_was_collected_for_test(&mut vm, &control_weak),
        "an accessor that never read a cross-origin property must be collectable"
    );

    let accessor_id = commit_opaque_child_for_test(&mut vm, "accessor-frame").await;
    expose_real_top_window_for_test(&mut vm, accessor_id);
    assert_eq!(
        vm.eval_in_child_default_context(accessor_id, "String(__realTopWindow.closed)")
            .expect("cross-origin closed read should evaluate"),
        "false"
    );
    let accessor_weak = child_context_weak_for_test(&mut vm, accessor_id);
    assert_eq!(
        vm.eval(
            r#"
document.getElementById("accessor-frame").remove();
"removed"
"#
        )
        .expect("accessor frame removal should evaluate"),
        "removed"
    );
    assert!(
        vm.live_child_default_runtime_realm_inventory().is_empty(),
        "removing the accessor frame should drop its realm record"
    );
    collect_isolate_garbage_for_test(&vm);
    assert!(
        context_weak_was_collected_for_test(&mut vm, &accessor_weak),
        "the surviving target must not retain an accessor realm after that page is gone"
    );
}

#[tokio::test]
async fn cross_origin_window_cache_ignores_author_weak_map_overrides() {
    let mut vm = new_storage_test_vm("https://cross-origin-cache-intrinsics.test/");
    let child_id = commit_opaque_child_for_test(&mut vm, "observer").await;
    expose_real_top_window_for_test(&mut vm, child_id);
    vm.eval(
        r#"
window.cacheHooks = 0;
const OriginalWeakMap = WeakMap;
for (const name of ['get', 'set']) {
  const original = OriginalWeakMap.prototype[name];
  OriginalWeakMap.prototype[name] = function(...args) {
    cacheHooks++;
    return Reflect.apply(original, this, args);
  };
}
window.WeakMap = function(...args) {
  cacheHooks++;
  return new OriginalWeakMap(...args);
};
"#,
    )
    .expect("author may replace its WeakMap constructor and methods");
    assert_eq!(
        vm.eval_in_child_default_context(
            child_id,
            r#"
window.savedPostMessage = __realTopWindow.postMessage;
window.savedParentGetter = Object.getOwnPropertyDescriptor(__realTopWindow, 'parent').get;
window.savedLocation = __realTopWindow.location;
String(__realTopWindow.closed)
"#,
        )
        .expect("cross-origin access should not invoke author cache hooks"),
        "false"
    );
    collect_isolate_garbage_for_test(&vm);
    assert_eq!(
        vm.eval_in_child_default_context(
            child_id,
            r#"
savedPostMessage === __realTopWindow.postMessage &&
savedParentGetter === Object.getOwnPropertyDescriptor(__realTopWindow, 'parent').get &&
savedLocation === __realTopWindow.location &&
Object.getPrototypeOf(savedPostMessage) === Function.prototype
"#,
        )
        .expect("live observer descriptor identity should survive collection"),
        "true"
    );
    assert_eq!(
        vm.eval("cacheHooks")
            .expect("cache hooks should be readable"),
        "0",
        "native cross-origin caching must not execute author code"
    );
}

#[tokio::test]
async fn cross_origin_window_keys_follow_frame_removal() {
    let mut vm = new_storage_test_vm("https://cross-origin-window-keys.test/");
    let child_id = commit_opaque_child_for_test(&mut vm, "observer").await;
    expose_real_top_window_for_test(&mut vm, child_id);
    vm.eval(
        r#"
const sibling = document.createElement('iframe');
sibling.id = 'sibling';
document.body.appendChild(sibling);
void sibling.contentWindow;
"#,
    )
    .expect("second frame should be created");
    let probe = r#"
(() => {
  const w = __realTopWindow;
  const indices = Reflect.ownKeys(w).filter(key => typeof key === 'string' && /^\d+$/.test(key));
  return JSON.stringify([w.length, indices, Object.keys(w), indices.every(key => w[key] !== undefined)]);
})()
"#;
    assert_eq!(
        vm.eval_in_child_default_context(child_id, probe)
            .expect("cross-origin Window keys should be enumerable"),
        r#"[2,["0","1"],["0","1"],true]"#
    );
    vm.eval("document.getElementById('sibling').remove()")
        .expect("sibling should be removable");
    assert_eq!(
        vm.eval_in_child_default_context(child_id, probe)
            .expect("cross-origin Window keys should reflect removed frames"),
        r#"[1,["0"],["0"],true]"#
    );
}

#[test]
fn frame_owner_content_accessors_live_on_exact_owner_prototypes() {
    let mut vm = new_storage_html_test_vm("https://frame-owner-content-accessors.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.body || document.documentElement || document;
  const frame = document.createElement("frame");
  frame.src = "about:blank";
  const iframe = document.createElement("iframe");
  iframe.src = "about:blank";
  const object = document.createElement("object");
  object.type = "text/html";
  object.data = "about:blank";

  const owners = [
    [HTMLFrameElement.prototype, frame],
    [HTMLIFrameElement.prototype, iframe],
    [HTMLObjectElement.prototype, object]
  ];
  for (const [, element] of owners) {
    root.appendChild(element);
  }

  const properties = ["contentDocument", "contentWindow"];
  const descriptors = owners.map(([prototype]) =>
    Object.fromEntries(properties.map(property => {
      const descriptor = Object.getOwnPropertyDescriptor(prototype, property);
      return [property, {
        get: typeof descriptor.get,
        set: typeof descriptor.set,
        enumerable: descriptor.enumerable,
        configurable: descriptor.configurable
      }];
    }))
  );
  const sameOriginValues = owners.map(([, element]) =>
    element.contentDocument !== null &&
      element.contentWindow !== null &&
      element.contentDocument === element.contentWindow.document
  );
  const brandErrors = owners.map(([prototype], index) =>
    properties.map(property => {
      const getter = Object.getOwnPropertyDescriptor(prototype, property).get;
      try {
        getter.call(owners[(index + 1) % owners.length][1]);
        return "accepted";
      } catch (error) {
        return error.name;
      }
    })
  );

  return JSON.stringify({
    descriptors,
    sameOriginValues,
    brandErrors,
    absentFromBase: properties.every(property =>
      !Object.hasOwn(HTMLElement.prototype, property))
  });
})()
"#,
        )
        .expect("frame owner content accessors should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":[{"contentDocument":{"get":"function","set":"undefined","enumerable":true,"configurable":true},"contentWindow":{"get":"function","set":"undefined","enumerable":true,"configurable":true}},{"contentDocument":{"get":"function","set":"undefined","enumerable":true,"configurable":true},"contentWindow":{"get":"function","set":"undefined","enumerable":true,"configurable":true}},{"contentDocument":{"get":"function","set":"undefined","enumerable":true,"configurable":true},"contentWindow":{"get":"function","set":"undefined","enumerable":true,"configurable":true}}],"sameOriginValues":[true,true,true],"brandErrors":[["TypeError","TypeError"],["TypeError","TypeError"],["TypeError","TypeError"]],"absentFromBase":true}"#
    );
}

#[test]
fn unreferenced_traversal_filters_release_retired_document() {
    for method in ["createTreeWalker", "createNodeIterator"] {
        let mut vm = new_parsed_test_vm("https://traversal.test/old", "<body>old document</body>");
        vm.eval(&format!(
            "document.{method}(document, NodeFilter.SHOW_ALL, () => NodeFilter.FILTER_ACCEPT); 1"
        ))
        .expect("unreferenced traversal setup");
        let host = vm.context_host_weak_for_test();
        let isolate = vm.renderer_document_isolate.clone();
        drop(vm);
        for _ in 0..3 {
            isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
        }
        assert!(
            host.upgrade().is_none(),
            "{method}: an unreferenced filter must not retain its retired Document host"
        );
    }
}

#[test]
fn retained_traversal_filters_release_host_after_last_reference() {
    for method in ["createTreeWalker", "createNodeIterator"] {
        let mut vm = new_parsed_test_vm(
            "https://retained-traversal.test/old",
            "<body>old document</body>",
        );
        vm.eval(&format!(
            r#"
            window.retainedFilter = () => NodeFilter.FILTER_ACCEPT;
            window.retainedTraversal = document.{method}(
                document.body, NodeFilter.SHOW_ALL, retainedFilter
            );
            1
            "#
        ))
        .expect("retained traversal setup");
        let walker = vm
            .with_default_context_scope_and_checkpoint_for_test(|scope, _| {
                let context = scope.get_current_context();
                let value = context
                    .global(scope)
                    .get(scope, crate::util::v8str(scope, "retainedTraversal").into())
                    .unwrap();
                let walker = v8::Local::<v8::Object>::try_from(value).unwrap();
                Ok(v8::Global::new(scope, walker))
            })
            .unwrap();
        let host = vm.context_host_weak_for_test();
        let isolate = vm.renderer_document_isolate.clone();
        drop(vm);
        for _ in 0..3 {
            isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
        }
        assert!(
            host.upgrade().is_some(),
            "a retained walker keeps native backing alive"
        );
        let result = isolate.with_renderer_document_isolate_mut(|isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let caller = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, caller);
            let walker = v8::Local::new(scope, &walker);
            let context = walker.get_creation_context(scope).unwrap();
            let scope = &mut v8::ContextScope::new(scope, context);
            let source = crate::util::v8str(
                scope,
                r#"JSON.stringify([
                    retainedTraversal.filter === retainedFilter,
                    retainedTraversal.root.textContent
                ])"#,
            );
            let script = v8::Script::compile(scope, source, None).unwrap();
            crate::script_execution::execute_compiled_script(scope, script)
                .unwrap()
                .to_rust_string_lossy(scope)
        });
        assert_eq!(result, r#"[true,"old document"]"#);
        isolate.with_renderer_document_isolate_mut(|isolate| {
            drop(walker);
            isolate.low_memory_notification();
        });
        for _ in 0..3 {
            isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
        }
        assert!(
            host.upgrade().is_none(),
            "the last walker reference releases its retired host"
        );
    }
}
