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


use super::*;

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
    assert_eq!(
        child_request_origin,
        moli_url::WebOrigin::Opaque,
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

    assert_eq!(
        vm._context_host
            .borrow()
            .opfs_handle_registry()
            .expect("OPFS handle registry remains owned until host teardown")
            .len(),
        0,
        "page teardown must run handle finalizers before isolate teardown"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .opfs_directory_iterator_registry()
            .expect("OPFS iterator registry remains owned until host teardown")
            .len(),
        0,
        "page teardown must run iterator finalizers before isolate teardown"
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
fn context_wrapper_cache_is_cleared_on_script_vm_teardown() {
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
        retained_cache.wrapper_entry_count(),
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
fn page_context_teardown_releases_all_context_owned_v8_finalizers() {
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
    assert_eq!(
        vm._context_host.borrow().v8_finalizers.len(),
        0,
        "page context teardown must reset every weak handle before isolate teardown"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .resource_timing_buffer_count_for_test(),
        0,
        "Performance finalization must remove host-side buffer state"
    );

    vm.close_page_context_resources_for_context_teardown();
    drop(vm);
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
    let expected = vm.page_default_runtime_observable_context_token;

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

#[tokio::test]
async fn failed_object_attribute_navigation_enters_fallback_without_recreating_child_context() {
    let (object_url, request_rx, release_tx, server) =
        spawn_gated_child_document_resource_server(404).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &object_url.replace("/child.html", "/page"),
        &loader,
    );

    assert_eq!(
        vm.eval(&format!(
            r#"
(() => {{
  const root = document.body || document.documentElement || document;
  const object = document.createElement('object');
  object.type = 'text/html';
  object.data = {object_url:?};
  globalThis.__failedObjectEvents = [];
  object.addEventListener('load', () => __failedObjectEvents.push('load'));
  object.addEventListener('error', event => __failedObjectEvents.push(
    `error:${{event.isTrusted}}:${{object.contentWindow === null}}`
  ));
  const fallback = document.createElement('span');
  fallback.id = 'object-fallback';
  fallback.textContent = 'fallback';
  object.appendChild(fallback);
  root.appendChild(object);
  globalThis.__failedObject = object;
  return [object.contentWindow !== null, window.length].join('|');
}})()
"#
        ))
        .expect("failed object setup should evaluate"),
        "true|1",
        "the object should expose its initial child browsing context while loading"
    );
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "object attribute navigation should start from its frame-lane commit",
    )
    .await;
    request_rx
        .await
        .expect("failed object document request should arrive");
    release_tx
        .send(())
        .expect("release failed object document response");
    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "failed object document completion",
    )
    .await;

    assert_eq!(
        vm.eval(
            r#"
JSON.stringify({
  contentWindowIsNull: __failedObject.contentWindow === null,
  contentDocumentIsNull: __failedObject.contentDocument === null,
  childCount: window.length,
  fallbackConnected: document.getElementById('object-fallback').isConnected,
  events: __failedObjectEvents
})
"#,
        )
        .expect("failed object fallback state should evaluate"),
        r#"{"contentWindowIsNull":true,"contentDocumentIsNull":true,"childCount":0,"fallbackConnected":true,"events":["error:true:true"]}"#
    );
    assert_eq!(
        vm._context_host.borrow().child_browsing_context_count(),
        0,
        "contentWindow and contentDocument getters must not recreate a failed object context"
    );
    server.await.expect("failed object server should finish");
}

#[test]
fn frame_owner_content_accessors_live_on_exact_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://frame-owner-content-accessors.test/");

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
