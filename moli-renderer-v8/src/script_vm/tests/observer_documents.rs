use super::*;

#[tokio::test(flavor = "current_thread")]
async fn observers_sample_each_target_document_before_callbacks() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://observer-documents.test/");
    let count: usize = vm
        .eval(include_str!(
            "../../../tests/fixtures/observer-cross-document.js"
        ))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(count, 3);
    for index in 0..count {
        let name = vm
            .eval(&format!("__observerCrossDocument.start({index})"))
            .unwrap();
        vm.advance_timers_until_deadline_for_test(&loader)
            .await
            .unwrap();
        assert_eq!(
            vm.eval("__observerCrossDocument.result").unwrap(),
            vm.eval("__observerCrossDocument.expected").unwrap(),
            "{name}",
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn popup_layout_snapshots_belong_to_the_current_popup_document() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://popup-layout-owner.test/");
    vm.eval(
        r#"
globalThis.popup = window.open('about:blank', '', 'popup');
globalThis.mainTarget = document.body.appendChild(document.createElement('div'));
mainTarget.style.cssText = 'width:80px;height:20px';
globalThis.popupTarget = popup.document.body.appendChild(popup.document.createElement('div'));
popupTarget.style.cssText = 'width:40px;height:20px';
globalThis.observer = new ResizeObserver(() => {});
observer.observe(mainTarget);
observer.observe(popupTarget);
"#,
    )
    .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    let old_document = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let global = scope.get_current_context().global(scope);
            let key = v8::String::new(scope, "popupTarget").unwrap();
            let value = global.get(scope, key.into()).unwrap();
            let handle = crate::native_bridge::branded_node_handle(
                scope,
                value,
                crate::web_api_interfaces::Element::DESCRIPTOR,
            )
            .unwrap();
            let host = unsafe { &*host_ptr };
            let document = host.dom_host().owner_document_handle(handle).unwrap();
            assert!(
                host.with_latest_layout_tree_for_document(document, |_| ())
                    .is_some()
            );
            assert!(
                host.with_latest_layout_tree_for_document(host.document_handle(), |_| ())
                    .is_some()
            );
            Ok(document)
        })
        .unwrap();
    assert_eq!(vm.eval("mainTarget.getBoundingClientRect().width + ':' + popupTarget.getBoundingClientRect().width").unwrap(), "80:40");
    vm.eval(r#"
observer.disconnect();
popup.document.open();
popup.document.write('<!doctype html><body><div id="replacement" style="width:60px;height:20px"></div></body>');
popup.document.close();
"#).unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|_scope, host_ptr| {
        let host = unsafe { &*host_ptr };
        assert!(
            host.with_latest_layout_tree_for_document(old_document, |_| ())
                .is_none(),
            "document.open must release the old popup snapshot"
        );
        assert!(
            host.with_latest_layout_tree_for_document(host.document_handle(), |_| ())
                .is_some(),
            "popup replacement must preserve the opener snapshot"
        );
        Ok(())
    })
    .unwrap();
    vm.eval(
        r#"
globalThis.popupTarget = popup.document.getElementById('replacement');
observer.observe(mainTarget);
observer.observe(popupTarget);
"#,
    )
    .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("mainTarget.getBoundingClientRect().width + ':' + popupTarget.getBoundingClientRect().width").unwrap(), "80:60");
    let current_document = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let global = scope.get_current_context().global(scope);
            let key = v8::String::new(scope, "popupTarget").unwrap();
            let value = global.get(scope, key.into()).unwrap();
            let handle = crate::native_bridge::branded_node_handle(
                scope,
                value,
                crate::web_api_interfaces::Element::DESCRIPTOR,
            )
            .unwrap();
            Ok(unsafe { &*host_ptr }
                .dom_host()
                .owner_document_handle(handle)
                .unwrap())
        })
        .unwrap();
    vm.eval("observer.disconnect(); popup.close();").unwrap();
    // The timer/rendering-only driver cannot execute the DOM-manipulation
    // close task. Wait for that task through the production Page dispatcher.
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(popup.opener === null)",
        "true",
        "popup close must commit before inspecting the retired snapshot",
    )
    .await;
    vm.with_default_context_scope_and_checkpoint_for_test(|_scope, host_ptr| {
        let host = unsafe { &*host_ptr };
        assert!(
            host.with_latest_layout_tree_for_document(current_document, |_| ())
                .is_none(),
            "committed close must release the popup snapshot"
        );
        assert!(
            host.with_latest_layout_tree_for_document(host.document_handle(), |_| ())
                .is_some(),
            "popup close must preserve the opener snapshot"
        );
        Ok(())
    })
    .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn popup_iframe_resize_preserves_the_opener_layout() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://popup-frame-layout.test/");
    vm.eval(
        r#"
globalThis.popup = window.open('about:blank', '', 'popup');
globalThis.mainTarget = document.body.appendChild(document.createElement('div'));
mainTarget.style.cssText = 'width:80px;height:20px';
globalThis.popupTarget = popup.document.body.appendChild(popup.document.createElement('div'));
popupTarget.style.cssText = 'width:40px;height:20px';
globalThis.frame = popup.document.body.appendChild(popup.document.createElement('iframe'));
frame.style.border = '0';
frame.setAttribute('width', '300');
frame.contentWindow.document.body.innerHTML = '<div style="width:20px;height:10px">child</div>';
globalThis.observer = new ResizeObserver(() => {});
observer.observe(mainTarget);
observer.observe(popupTarget);
"#,
    )
    .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("mainTarget.getBoundingClientRect().width + ':' + popupTarget.getBoundingClientRect().width").unwrap(), "80:40");
    assert_eq!(
        vm.eval("frame.getBoundingClientRect().width").unwrap(),
        "300"
    );
    vm.eval("frame.setAttribute('width', '200');").unwrap();
    assert_eq!(
        vm.eval("frame.getBoundingClientRect().width").unwrap(),
        "300"
    );
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "popupTarget").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let handle = crate::native_bridge::branded_node_handle(
            scope,
            value,
            crate::web_api_interfaces::Element::DESCRIPTOR,
        )
        .unwrap();
        let host = unsafe { &*host_ptr };
        let document = host.dom_host().owner_document_handle(handle).unwrap();
        assert!(
            host.with_latest_layout_tree_for_document(document, |_| ())
                .is_some(),
            "changing iframe width must retain the published popup snapshot until rendering"
        );
        assert!(
            host.with_latest_layout_tree_for_document(host.document_handle(), |_| ())
                .is_some(),
            "changing popup iframe width must preserve the opener snapshot"
        );
        Ok(())
    })
    .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("mainTarget.getBoundingClientRect().width + ':' + popupTarget.getBoundingClientRect().width").unwrap(), "80:40");
    assert_eq!(
        vm.eval("frame.getBoundingClientRect().width").unwrap(),
        "200"
    );
    vm.eval("frame.remove();").unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|_scope, host_ptr| {
        let host = unsafe { &*host_ptr };
        assert!(
            host.with_latest_layout_tree_for_document(host.document_handle(), |_| ())
                .is_some(),
            "retiring a popup iframe must preserve the opener snapshot"
        );
        Ok(())
    })
    .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("mainTarget.getBoundingClientRect().width + ':' + popupTarget.getBoundingClientRect().width").unwrap(), "80:40");
    vm.eval("observer.disconnect(); popup.close();").unwrap();
}
