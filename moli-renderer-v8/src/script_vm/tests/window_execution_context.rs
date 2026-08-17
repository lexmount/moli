use super::*;

#[test]
fn strict_window_binding_resolves_registry_policy_and_rejects_retired_realm() {
    let mut vm = new_storage_test_vm("https://strict-window-binding.test/");
    let runtime_enable_messages = vm
        .dispatch_inspector_protocol_message(
            &serde_json::json!({
                "id": 1,
                "method": "Runtime.enable",
            })
            .to_string(),
        )
        .expect("Runtime.enable should attach the primary Inspector session");
    assert!(
        runtime_enable_messages
            .iter()
            .any(|message| message["id"] == serde_json::json!(1)),
        "Runtime.enable should return its protocol response"
    );
    assert_eq!(
        vm.page_inspector
            .context_registration_count_for_diagnostics(),
        1,
        "the document should initially own only its default context registration"
    );
    let context_id = vm
        .create_isolated_world("strict-window-binding", true)
        .expect("universal isolated world should be created");
    assert_eq!(
        vm.page_inspector
            .context_registration_count_for_diagnostics(),
        2,
        "creating an isolated world should add one document-owned registration"
    );
    let created_context_ids = vm
        .page_inspector
        .take_outbound_messages_for_session(None)
        .into_iter()
        .filter(|message| message["method"] == serde_json::json!("Runtime.executionContextCreated"))
        .filter_map(|message| message["params"]["context"]["id"].as_i64())
        .collect::<Vec<_>>();
    assert_eq!(
        created_context_ids,
        vec![context_id],
        "the isolated registration should report exactly one context creation"
    );
    let context_ptr = {
        let world = vm
            .page_isolated_world_contexts
            .context(context_id)
            .expect("isolated world should be tracked");
        &world.context as *const _
    };
    let binding = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            let binding = host
                .current_runtime_window_execution_context_binding(scope)
                .expect("isolated world should expose an exact Window realm binding");
            let identity = binding
                .resolve_identity(host)
                .expect("binding should resolve through the realm registry");
            assert!(
                identity.grants_universal_access(),
                "access policy must come from the authoritative registry registration"
            );
            Ok(binding)
        })
        .expect("isolated binding capture should succeed");

    assert!(binding.is_current(&vm._context_host.borrow()));
    vm.destroy_isolated_world_context(context_id);
    assert_eq!(
        vm.page_inspector
            .context_registration_count_for_diagnostics(),
        1,
        "destroying the isolated world should release exactly its registration"
    );
    let destroyed_context_ids = vm
        .page_inspector
        .take_outbound_messages_for_session(None)
        .into_iter()
        .filter(|message| {
            message["method"] == serde_json::json!("Runtime.executionContextDestroyed")
        })
        .filter_map(|message| message["params"]["executionContextId"].as_i64())
        .collect::<Vec<_>>();
    assert_eq!(
        destroyed_context_ids,
        vec![context_id],
        "the registration guard should report exactly one context destruction"
    );
    vm.destroy_isolated_world_context(context_id);
    assert_eq!(
        vm.page_inspector
            .context_registration_count_for_diagnostics(),
        1,
        "repeated isolated-world teardown must not release the default registration"
    );
    assert!(
        vm.page_inspector
            .take_outbound_messages_for_session(None)
            .is_empty(),
        "repeated isolated-world teardown must not emit another Inspector event"
    );
    assert!(
        !binding.is_current(&vm._context_host.borrow()),
        "a retained V8 Global must not make a retired realm current again"
    );
}

#[test]
fn borrowed_fetch_enforces_cross_origin_receiver_before_entering_its_realm() {
    let mut vm = new_storage_test_vm("https://fetch-accessing-origin.test/");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Fetch));
    vm.exec(
        r#"
        const frame = document.createElement("iframe");
        frame.src = "data:text/html,<title>opaque</title>";
        (document.body || document.documentElement || document).appendChild(frame);
        globalThis.__crossOriginFetchFrame = frame;
        "#,
        None,
    )
    .expect("cross-origin Fetch receiver should be created");
    vm.drain_pending_child_frame_work_for_test();

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              try {
                fetch.call(__crossOriginFetchFrame.contentWindow, "/blocked");
                return "no-throw";
              } catch (error) {
                return `${error && error.name}:${error instanceof DOMException}`;
              }
            })()
            "#,
        )
        .expect("cross-origin Fetch receiver probe should evaluate"),
        "SecurityError:true"
    );
    assert!(
        vm.take_pending_subresource_fetch_infos().is_empty(),
        "cross-origin authorization must run before a request or target-realm Promise is created"
    );
}

#[test]
fn chromium_fetch_promise_realm_matrix_uses_current_realm_for_pre_idl_errors() {
    let mut vm = new_storage_test_vm("https://fetch-promise-realm.test/");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Fetch));
    vm.eval(
        r#"
        const frame = document.createElement("iframe");
        globalThis.__fetchPromiseRealmFrame = frame;
        (document.body || document.documentElement || document).appendChild(frame);
        void frame.contentWindow;
        "frame-ready"
        "#,
    )
    .expect("same-origin Fetch receiver should be created");
    materialize_single_child_default_realm_for_test(&mut vm, "Fetch Promise realm receiver");

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const same = __fetchPromiseRealmFrame.contentWindow;
              const regular = fetch.call(same, "/regular");
              const currentMissing = fetch.call();
              const childFunctionMissing = same.fetch.call();
              const borrowedMissing = fetch.call(same);
              const illegal = fetch.call({});
              for (const promise of [
                currentMissing,
                childFunctionMissing,
                borrowedMissing,
                illegal
              ]) {
                promise.catch(() => {});
              }
              return JSON.stringify([
                regular.constructor.constructor === same.Function,
                currentMissing.constructor.constructor === Function,
                childFunctionMissing.constructor.constructor === same.Function,
                borrowedMissing.constructor.constructor === Function,
                illegal.constructor.constructor === Function
              ]);
            })()
            "#,
        )
        .expect("Chromium Fetch Promise realm matrix should evaluate"),
        "[true,true,true,true,true]",
        "normal promises use the receiver relevant realm, while binding-time TypeErrors use the current function realm"
    );
}

#[test]
fn fetch_receiver_generation_is_frozen_before_request_init_getters_run() {
    let mut vm = new_storage_test_vm("https://fetch-receiver-generation.test/");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Fetch));
    vm.eval(
        r#"
        const frame = document.createElement("iframe");
        globalThis.__fetchGenerationFrame = frame;
        (document.body || document.documentElement || document).appendChild(frame);
        void frame.contentWindow;
        "frame-ready"
        "#,
    )
    .expect("Fetch generation child should be exposed");
    materialize_single_child_default_realm_for_test(&mut vm, "Fetch generation receiver");

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const frame = __fetchGenerationFrame;
              const staleWindow = frame.contentWindow;
              let replaced = false;
              const init = {
                get method() {
                  if (!replaced) {
                    replaced = true;
                    frame.remove();
                    (document.body || document.documentElement || document)
                      .appendChild(frame);
                    globalThis.__fetchGenerationReplacement = frame.contentWindow;
                  }
                  return "GET";
                }
              };
              globalThis.__fetchGenerationResult = "pending";
              const promise = fetch.call(staleWindow, "/must-not-start", init);
              promise.then(
                () => { __fetchGenerationResult = "resolved"; },
                error => {
                  __fetchGenerationResult = JSON.stringify([
                    error instanceof TypeError,
                    error.message.includes("shutting down")
                  ]);
                }
              );
              return JSON.stringify([
                staleWindow !== __fetchGenerationReplacement,
                Object.getPrototypeOf(promise) === Promise.prototype
              ]);
            })()
            "#,
        )
        .expect("RequestInit getter should be allowed to replace the receiver Window"),
        "[true,true]",
        "reattaching the same iframe element should create a new Window generation while the failure Promise remains in the calling realm"
    );
    vm.eval("0")
        .expect("stale Fetch receiver rejection checkpoint should evaluate");
    assert_eq!(
        vm.eval("__fetchGenerationResult")
            .expect("stale Fetch receiver rejection should be observable"),
        "[true,true]"
    );
    assert!(
        vm.take_pending_subresource_fetch_infos().is_empty(),
        "Fetch must reject against the captured stale LocalWindow instead of rebinding through the reused iframe handle"
    );
}

#[test]
fn detached_fetch_receiver_returns_a_rejected_current_realm_promise() {
    let mut vm = new_storage_test_vm("https://detached-fetch-receiver.test/");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Fetch));

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              globalThis.__detachedFetchWindow = frame.contentWindow;
              frame.remove();
              globalThis.__detachedFetchResult = "pending";
              let promise;
              try {
                promise = fetch.call(__detachedFetchWindow, "/never-started");
              } catch (error) {
                return "sync:" + (error && error.name);
              }
              promise.then(
                () => { __detachedFetchResult = "resolved"; },
                error => {
                  __detachedFetchResult =
                    `${error && error.name}:${String(error && error.message).includes("shutting down")}`;
                }
              );
              return [
                "promise",
                Object.getPrototypeOf(promise) === Promise.prototype
              ].join(":");
            })()
            "#,
        )
        .expect("detached Fetch receiver probe should evaluate"),
        "promise:true",
        "Promise-returning WebIDL operations convert shutdown failure into a current-realm rejection"
    );
    vm.eval("0")
        .expect("detached Fetch rejection checkpoint should evaluate");
    assert_eq!(
        vm.eval("__detachedFetchResult")
            .expect("detached Fetch rejection should be observable"),
        "TypeError:true"
    );
    assert!(
        vm.take_pending_subresource_fetch_infos().is_empty(),
        "detached receivers must not start a request"
    );
}

#[test]
fn chromium_discarded_window_fetch_rejects_in_the_detached_function_realm() {
    let mut vm = new_storage_test_vm("https://discarded-window-fetch.test/");

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const discarded = frame.contentWindow;
              frame.remove();
              globalThis.__discardedWindowFetchResult = "pending";
              const promise = discarded.fetch("/never-started");
              promise.then(
                () => { __discardedWindowFetchResult = "resolved"; },
                error => {
                  __discardedWindowFetchResult = JSON.stringify([
                    error instanceof discarded.TypeError,
                    error.message ===
                      "Failed to execute 'fetch' on 'Window': The global scope is shutting down."
                  ]);
                }
              );
              return JSON.stringify([
                typeof discarded.fetch === "function",
                Object.getPrototypeOf(promise) === discarded.Promise.prototype
              ]);
            })()
            "#,
        )
        .expect("discarded Window Fetch should return its realm's Promise"),
        "[true,true]"
    );
    vm.eval("0")
        .expect("discarded Window Fetch rejection checkpoint should evaluate");
    assert_eq!(
        vm.eval("__discardedWindowFetchResult")
            .expect("discarded Window Fetch rejection should be observable"),
        "[true,true]"
    );
}

#[test]
fn borrowed_fetch_uses_receiver_realm_and_keeps_reaction_realm_independent() {
    let mut vm = new_storage_test_vm("https://borrowed-fetch-context.test/top/");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Fetch));
    vm.eval(
        r#"
        const frame = document.createElement("iframe");
        globalThis.__borrowedFetchFrame = frame;
        (document.body || document.documentElement || document).appendChild(frame);
        void frame.contentWindow;
        "frame-ready"
        "#,
    )
    .expect("borrowed Fetch child should be exposed");
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "borrowed Fetch target");
    let child_handle = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("borrowed Fetch child realm")
        .child_handle;
    let child_owner = vm
        ._context_host
        .borrow()
        .current_window_execution_context_owner(crate::native_bridge::OwnerDispatchScope::Child(
            child_handle,
        ))
        .expect("borrowed Fetch child LocalWindow");
    vm.eval(
        r#"
        const childBase = __borrowedFetchFrame.contentDocument.createElement("base");
        childBase.href = "https://borrowed-fetch-context.test/child-base/";
        (__borrowedFetchFrame.contentDocument.head ||
          __borrowedFetchFrame.contentDocument.documentElement
        ).appendChild(childBase);
        "#,
    )
    .expect("borrowed Fetch child base should be installed without replacing its realm");
    assert_eq!(
        vm.eval(
            r#"
            const borrowedFetch = fetch;
            globalThis.__borrowedFetchResult = "pending";
            globalThis.__borrowedFetchRejections = [];
            addEventListener("unhandledrejection", event => {
              __borrowedFetchRejections.push(
                "parent:" + String(event.reason && event.reason.message)
              );
              event.preventDefault();
            });
            __borrowedFetchFrame.contentWindow.addEventListener(
              "unhandledrejection",
              event => {
                __borrowedFetchRejections.push(
                  "child:" + String(event.reason && event.reason.message)
                );
                event.preventDefault();
              }
            );
            globalThis.__borrowedFetchCompletion = borrowedFetch.call(
              __borrowedFetchFrame.contentWindow,
              "completion"
            );
            globalThis.__borrowedFetchOrdinary = borrowedFetch.call(
              __borrowedFetchFrame.contentWindow,
              "ordinary"
            );
            globalThis.__borrowedFetchKeepalive = borrowedFetch.call(
              __borrowedFetchFrame.contentWindow,
              "keepalive",
              { keepalive: true }
            );
            __borrowedFetchCompletion.then(response => {
              __borrowedFetchResult =
                "child-response:" +
                String(
                  Object.getPrototypeOf(response) ===
                    __borrowedFetchFrame.contentWindow.Response.prototype
                );
            });
            globalThis.__borrowedFetchReaction =
              __borrowedFetchCompletion.then(() => {
                throw new Error("borrowed-reaction");
              });
            globalThis.__borrowedFetchChildReaction =
              __borrowedFetchCompletion.then(
                __borrowedFetchFrame.contentWindow.Function(
                  'throw new Error("child-reaction")'
                )
              );
            JSON.stringify([
              Object.getPrototypeOf(__borrowedFetchCompletion) ===
                __borrowedFetchFrame.contentWindow.Promise.prototype,
              Object.getPrototypeOf(__borrowedFetchOrdinary) ===
                __borrowedFetchFrame.contentWindow.Promise.prototype,
              Object.getPrototypeOf(__borrowedFetchKeepalive) ===
                __borrowedFetchFrame.contentWindow.Promise.prototype
            ])
            "#,
        )
        .expect("borrowed Fetch calls should evaluate"),
        "[true,true,true]",
        "Blink selects the receiver Window's relevant realm, not the borrowed function object's realm"
    );

    let contexts = vm
        ._context_host
        .borrow()
        .active_window_fetch_contexts_for_test();
    assert_eq!(contexts.len(), 3);
    let child_realm_token = contexts[0].3;
    assert!(contexts.iter().all(
        |(_, promise_owner, promise_scope, realm_token, request_target)| {
            *promise_owner == child_owner
                && *promise_scope == crate::native_bridge::OwnerDispatchScope::Child(child_handle)
                && *realm_token == child_realm_token
                && *request_target
                    == crate::native_bridge::WindowTaskTarget::new(
                        crate::native_bridge::OwnerDispatchScope::Child(child_handle),
                        child_owner,
                    )
        }
    ));

    let mut infos = vm.take_pending_subresource_fetch_infos();
    infos.sort_by(|left, right| left.url.as_str().cmp(right.url.as_str()));
    assert_eq!(infos.len(), 3);
    assert!(
        infos.iter().all(|info| info
            .url
            .as_str()
            .starts_with("https://borrowed-fetch-context.test/child-base/")),
        "relative Fetch URLs must use the receiver Window's document base"
    );
    let completion_id = infos
        .iter()
        .find(|info| info.url.as_str().ends_with("/completion"))
        .expect("borrowed completion request")
        .internal_id;
    let keepalive_id = infos
        .iter()
        .find(|info| info.url.as_str().ends_with("/keepalive"))
        .expect("borrowed keepalive request")
        .internal_id;

    let completion_url = Url::parse("https://borrowed-fetch-context.test/child-base/completion")
        .expect("completion URL");
    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: completion_id,
        request_url: completion_url.clone(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new(),
        request_body: None,
        response_status_text: Some("OK".to_owned()),
        skip_fetch_security_validation: true,
        response_filter: None,
        network_error_text: None,
        parkable_image: None,
        result: Ok(crate::types::NavigationResponse::from_text_body(
            completion_url,
            200,
            vec![("content-type".to_owned(), "text/plain".to_owned())],
            "borrowed completion".to_owned(),
        )),
    })
    .expect("borrowed Fetch completion should enter the receiver Promise realm");
    vm.eval("0")
        .expect("borrowed Fetch rejection checkpoint should evaluate");
    assert_eq!(
        vm.eval("__borrowedFetchResult")
            .expect("borrowed Fetch result should evaluate"),
        "child-response:true",
        "Response construction and Promise settlement must use the receiver relevant realm"
    );
    assert_eq!(
        vm.eval("JSON.stringify(__borrowedFetchRejections)")
            .expect("borrowed Fetch rejection routing should evaluate"),
        r#"["parent:borrowed-reaction","child:child-reaction"]"#,
        "each later reaction is reported to its callback's current realm, independently of the Fetch Promise and request owner"
    );

    assert_eq!(
        vm._context_host
            .borrow_mut()
            .retire_window_fetches_for_execution_context_owner(child_owner),
        (1, 1),
        "receiver LocalWindow retirement must follow the request target"
    );
    assert!(
        vm._context_host
            .borrow()
            .active_window_fetch_contexts_for_test()
            .is_empty()
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_fetch_execution_contexts_for_test(),
        vec![(
            keepalive_id,
            true,
            Some(child_owner),
            Some(child_realm_token)
        )],
        "detached keepalive must preserve its receiver Window owner and receiver realm token"
    );
    assert!(
        vm._context_host
            .borrow_mut()
            .abort_subresource_fetch(keepalive_id)
    );
}
