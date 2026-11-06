use super::*;

#[tokio::test]
async fn image_input_replacements_discard_obsolete_requests_and_queued_events() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://image-input.test/page.html",
        &loader,
    );
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Image));
    vm.eval(
        r#"
        globalThis.input = document.createElement('input');
        globalThis.events = [];
        input.type = 'image';
        input.onload = () => events.push('load');
        input.onerror = () => events.push('error');
        input.src = '/aborted.gif';
    "#,
    )
    .unwrap();
    let aborted = vm.take_pending_subresource_fetch_infos();
    assert_eq!(aborted.len(), 1);
    vm.eval("input.src = '/obsolete.gif'").unwrap();
    let obsolete = vm.take_pending_subresource_fetch_infos();
    assert_eq!(obsolete.len(), 1);
    assert!(
        vm.fulfill_pending_subresource_fetch(
            aborted[0].internal_id,
            200,
            vec![("Content-Type".into(), b"image/gif".to_vec())],
            one_by_one_gif_response_body(),
        )
        .is_err(),
        "replacing src must retire the previous intercepted request"
    );
    assert!(
        !vm.run_one_user_interaction_executor_turn(&loader)
            .await
            .unwrap()
    );
    vm.fulfill_pending_subresource_fetch(
        obsolete[0].internal_id,
        200,
        vec![("Content-Type".into(), b"image/gif".to_vec())],
        one_by_one_gif_response_body(),
    )
    .unwrap();
    vm.eval("input.src = '/current.gif'").unwrap();
    let current = vm.take_pending_subresource_fetch_infos();
    assert_eq!(current.len(), 1);
    // Consume the old event's task without allowing its callback to run.
    vm.run_one_user_interaction_executor_turn(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("events.join(':')").unwrap(), "");
    vm.fulfill_pending_subresource_fetch(
        current[0].internal_id,
        200,
        vec![("Content-Type".into(), b"image/gif".to_vec())],
        one_by_one_gif_response_body(),
    )
    .unwrap();
    run_input_image_event(&mut vm, &loader).await;
    assert_eq!(vm.eval("events.join(':')").unwrap(), "load");
    assert_eq!(
        vm.eval("performance.getEntriesByType('resource').length")
            .unwrap(),
        "3"
    );
}

#[tokio::test]
async fn image_input_network_completion_releases_document_load_before_its_event() {
    assert_image_input_network_load_delay(false).await;
}

#[tokio::test]
async fn adopted_image_input_completion_releases_original_document_load_delay() {
    assert_image_input_network_load_delay(true).await;
}

async fn assert_image_input_network_load_delay(adopted: bool) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_image_fetch_enabled(true);
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://image-input.test/page.html",
        "<!doctype html><html><head></head><body><input id='button' type='image' src='/button.gif'></body></html>",
        &loader,
    );
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Image));
    vm.eval(
        r#"
        globalThis.inputEvent = 'pending';
        document.getElementById('button').onload = () => inputEvent = 'load';
    "#,
    )
    .unwrap();
    let owner = vm.current_main_document_task_owner().unwrap();
    let interactive = vm.finish_current_main_document_parsing(owner).unwrap();
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .unwrap();
    let requests = vm.take_pending_subresource_fetch_infos();
    assert_eq!(
        requests.len(),
        1,
        "parsed image button must start one request"
    );
    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false)
    );
    if adopted {
        vm.eval("document.implementation.createHTMLDocument('').adoptNode(document.getElementById('button'))")
            .unwrap();
    }
    vm.fulfill_pending_subresource_fetch(
        requests[0].internal_id,
        200,
        vec![("Content-Type".into(), b"image/gif".to_vec())],
        one_by_one_gif_response_body(),
    )
    .unwrap();
    assert_eq!(vm.eval("inputEvent").unwrap(), "pending");
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(true),
        "input image delays document load until resource completion, independently of its user-interaction event"
    );
    if adopted {
        assert!(
            !vm.run_one_user_interaction_executor_turn(&loader)
                .await
                .unwrap()
        );
        assert_eq!(vm.eval("inputEvent").unwrap(), "pending");
    } else {
        run_input_image_event(&mut vm, &loader).await;
        assert_eq!(vm.eval("inputEvent").unwrap(), "load");
    }
}

#[tokio::test]
async fn image_input_crossorigin_and_img_only_attributes_do_not_enable_cors() {
    let (base_url, request_rx, server) =
        spawn_image_response_server("200 OK", "image/gif", ONE_BY_ONE_GIF).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_image_fetch_enabled(true);
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("http://127.0.0.1:9/page.html", &loader);
    vm.eval(&format!(
        r#"
        globalThis.__crossInputImage = 'pending';
        const input = document.createElement('input');
        input.type = 'image';
        input.setAttribute('crossorigin', 'anonymous');
        input.setAttribute('loading', 'lazy');
        input.setAttribute('srcset', '/wrong.gif 1x');
        input.onload = () => {{
          const entries = performance.getEntriesByName(input.src);
          __crossInputImage = ['load', entries.length, entries[0]?.initiatorType,
            entries[0]?.responseStatus].join(':');
        }};
        input.onerror = () => __crossInputImage = 'error';
        input.src = '{base_url}/button.gif';
        document.body.appendChild(input);
        "#,
    ))
    .unwrap();
    wait_for_and_apply_image_completions(&mut vm).await;
    assert_eq!(vm.eval("__crossInputImage").unwrap(), "pending");
    run_input_image_event(&mut vm, &loader).await;
    assert_eq!(vm.eval("__crossInputImage").unwrap(), "load:1:input:0");
    let request = request_rx.await.unwrap();
    assert!(request.starts_with("GET /button.gif HTTP/1.1\r\n"));
    server.await.unwrap();
}

async fn run_input_image_event(
    vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
) {
    assert!(!vm.has_ready_timeout());
    assert!(
        !vm.has_ready_dom_manipulation_family_for_test(
            PageDomManipulationTestFamily::ImageLoadEvent,
        )
    );
    assert!(
        vm.run_one_user_interaction_executor_turn(loader)
            .await
            .unwrap(),
        "image-button load/error must run on the user-interaction task source"
    );
}

#[tokio::test]
async fn image_input_load_records_timing_before_its_user_interaction_event() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
    ] {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        loader.set_image_fetch_enabled(true);
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(
            "https://image-input.test/page.html",
            &loader,
        );
        vm.set_layout_policy(policy);
        vm.set_fetch_subresource_interception(
            true,
            Some(crate::types::SubresourceResourceType::Image),
        );
        vm.eval(
            r#"
            globalThis.__inputImageResult = 'pending';
            const input = document.createElement('input');
            input.type = 'image';
            input.onload = event => {
              const entries = performance.getEntriesByName(input.src);
              __inputImageResult = ['load', entries.length, entries[0]?.initiatorType,
                entries[0]?.responseStatus, event.bubbles, event.cancelable].join(':');
            };
            input.onerror = () => __inputImageResult = 'error';
            input.src = '/button.gif';
            document.body.appendChild(input);
            "#,
        )
        .unwrap();
        let requests = vm.take_pending_subresource_fetch_infos();
        assert_eq!(requests.len(), 1, "{policy:?}: input image fetch");
        assert_eq!(
            requests[0].url.as_str(),
            "https://image-input.test/button.gif"
        );
        vm.fulfill_pending_subresource_fetch(
            requests[0].internal_id,
            200,
            vec![("Content-Type".into(), b"image/gif".to_vec())],
            one_by_one_gif_response_body(),
        )
        .unwrap();
        assert_eq!(vm.eval("__inputImageResult").unwrap(), "pending");
        run_input_image_event(&mut vm, &loader).await;
        assert_eq!(
            vm.eval("__inputImageResult").unwrap(),
            "load:1:input:200:false:false",
            "{policy:?}"
        );
    }
}

#[tokio::test]
async fn image_input_failures_queue_error_after_resource_timing() {
    for (status, body) in [
        (404, b"missing".as_slice()),
        (200, b"not an image".as_slice()),
    ] {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        loader.set_image_fetch_enabled(true);
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(
            "https://image-input.test/page.html",
            &loader,
        );
        vm.set_fetch_subresource_interception(
            true,
            Some(crate::types::SubresourceResourceType::Image),
        );
        vm.eval(
            r#"
            globalThis.__inputImageError = 'pending';
            const input = document.createElement('input');
            input.type = 'image';
            input.onload = () => __inputImageError = 'load';
            input.onerror = () => {
              const entries = performance.getEntriesByName(input.src);
              __inputImageError = ['error', entries.length, entries[0]?.initiatorType,
                entries[0]?.responseStatus].join(':');
            };
            input.src = '/broken.gif';
            document.body.appendChild(input);
            "#,
        )
        .unwrap();
        let requests = vm.take_pending_subresource_fetch_infos();
        assert_eq!(requests.len(), 1, "status={status}: input image fetch");
        vm.fulfill_pending_subresource_fetch(
            requests[0].internal_id,
            status,
            vec![("Content-Type".into(), b"image/gif".to_vec())],
            crate::runtime::RendererSyntheticResponseBody::from_bytes(body.to_vec()),
        )
        .unwrap();
        assert_eq!(vm.eval("__inputImageError").unwrap(), "pending");
        run_input_image_event(&mut vm, &loader).await;
        assert_eq!(
            vm.eval("__inputImageError").unwrap(),
            format!("error:1:input:{status}")
        );
    }
}

#[tokio::test]
async fn image_input_type_transitions_and_src_changes_use_image_button_rules() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://image-input.test/page.html",
        &loader,
    );
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Image));
    vm.eval(
        r#"
        globalThis.__inputImageLoads = 0;
        globalThis.__inputImageErrors = 0;
        globalThis.input = document.createElement('input');
        input.onload = () => ++__inputImageLoads;
        input.onerror = () => ++__inputImageErrors;
        input.src = '/first.gif';
        input.setAttribute('srcset', '/wrong.gif 1x');
        input.setAttribute('sizes', '1px');
        input.setAttribute('loading', 'lazy');
        input.setAttribute('crossorigin', 'anonymous');
        document.body.appendChild(input);
        "#,
    )
    .unwrap();
    assert!(vm.take_pending_subresource_fetch_infos().is_empty());
    for (assignment, path) in [
        ("input.type = 'IMAGE'", "/first.gif"),
        (
            "input.type = 'text'; input.src = '/second.gif'; input.setAttributeNS(null, 'type', 'image')",
            "/second.gif",
        ),
        ("input.setAttribute('src', '/second.gif')", "/second.gif"),
    ] {
        vm.eval(assignment).unwrap();
        let requests = vm.take_pending_subresource_fetch_infos();
        assert_eq!(requests.len(), 1, "{assignment}");
        assert_eq!(requests[0].url.path(), path);
        vm.fulfill_pending_subresource_fetch(
            requests[0].internal_id,
            200,
            vec![("Content-Type".into(), b"image/gif".to_vec())],
            one_by_one_gif_response_body(),
        )
        .unwrap();
        run_input_image_event(&mut vm, &loader).await;
    }
    vm.eval("input.type = 'text'; input.type = 'image'; input.type = 'ImAgE'")
        .unwrap();
    assert!(vm.take_pending_subresource_fetch_infos().is_empty());
    assert!(
        !vm.run_one_user_interaction_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(
        vm.eval("[__inputImageLoads, __inputImageErrors, performance.getEntriesByType('resource').length].join(':')")
            .unwrap(),
        "3:0:3"
    );
}
