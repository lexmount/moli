use super::*;

#[test]
fn beacon_receiver_owner_follows_the_navigator_when_methods_are_borrowed() {
    let mut vm = new_storage_html_test_vm("https://beacon-receiver.test/parent/page");
    vm.set_fetch_subresource_interception(
        crate::types::SubresourceInterceptionClasses::for_resource_type(
            crate::types::SubresourceResourceType::Ping,
        ),
    );
    vm.eval(r#"
        globalThis.frame = document.createElement('iframe');
        document.body.appendChild(frame);
        frame.contentDocument.head.appendChild(Object.assign(frame.contentDocument.createElement('base'), {href: 'https://beacon-receiver.test/child/'}));
        globalThis.childNavigator = frame.contentWindow.navigator;
        globalThis.topSend = Navigator.prototype.sendBeacon;
        globalThis.childSend = frame.contentWindow.Navigator.prototype.sendBeacon;
    "#).unwrap();
    for (call, owner_url, target_url) in [
        (
            "topSend.call(childNavigator, 'from-top', 'body')",
            "frame.contentDocument.URL",
            "new URL('from-top', frame.contentDocument.baseURI).href",
        ),
        (
            "childSend.call(navigator, 'from-child', 'body')",
            "document.URL",
            "new URL('from-child', document.baseURI).href",
        ),
    ] {
        assert_eq!(vm.eval(&format!("String({call})")).unwrap(), "true");
        let expected_owner = vm.eval(owner_url).unwrap();
        let expected_target = vm.eval(target_url).unwrap();
        let requests = vm.take_pending_subresource_fetch_infos();
        assert_eq!(requests.len(), 1, "{call}");
        assert_eq!(requests[0].document_url.as_str(), expected_owner, "{call}");
        assert_eq!(requests[0].url.as_str(), expected_target, "{call}");
        assert_eq!(requests[0].request_body.as_deref(), Some("body"));
    }
}

#[test]
fn beacon_receiver_owner_is_retained_through_reentry_and_rechecked_after_retirement() {
    let mut vm = new_storage_html_test_vm("https://beacon-reentry.test/parent/page");
    vm.set_fetch_subresource_interception(
        crate::types::SubresourceInterceptionClasses::for_resource_type(
            crate::types::SubresourceResourceType::Ping,
        ),
    );
    vm.eval(r#"
        globalThis.frame = document.body.appendChild(document.createElement('iframe'));
        globalThis.other = frame.contentWindow.navigator;
        globalThis.childUrl = frame.contentDocument.URL;
        globalThis.send = Navigator.prototype.sendBeacon;
        globalThis.childSend = frame.contentWindow.Navigator.prototype.sendBeacon;
        globalThis.converted = 0;
        send.call(other, {toString() { ++converted; childSend.call(navigator, '/inner'); return '/outer'; }});
    "#).unwrap();
    let requests = vm.take_pending_subresource_fetch_infos();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].url.path(), "/inner");
    assert_eq!(
        requests[0].document_url.as_str(),
        "https://beacon-reentry.test/parent/page"
    );
    assert_eq!(requests[1].url.path(), "/outer");
    assert_eq!(
        requests[1].document_url.as_str(),
        vm.eval("childUrl").unwrap()
    );
    assert_eq!(vm.eval("String(converted)").unwrap(), "1");
    assert_eq!(vm.eval("String(send.call(other, {toString() {frame.remove(); ++converted; return '/retired'; }}))").unwrap(), "false");
    assert_eq!(vm.eval("String(converted)").unwrap(), "2");
    assert!(vm.take_pending_subresource_fetch_infos().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn beacon_receiver_owner_uses_popup_markers_and_rechecks_after_native_close() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://beacon-popup.test/parent/page",
        &loader,
    );
    vm.set_fetch_subresource_interception(
        crate::types::SubresourceInterceptionClasses::for_resource_type(
            crate::types::SubresourceResourceType::Ping,
        ),
    );
    vm.eval(r#"
        globalThis.topNavigator = navigator;
        globalThis.popup = open('about:blank', 'beacon-popup');
        popup.document.head.appendChild(Object.assign(popup.document.createElement('base'), {href: 'https://beacon-popup.test/popup/'}));
        globalThis.popupNavigator = popup.navigator;
        globalThis.popupDocument = popup.document;
        globalThis.topSend = Navigator.prototype.sendBeacon;
        globalThis.popupSend = popupNavigator.sendBeacon;
    "#).unwrap();

    for (call, owner_url, target_url) in [
        (
            "topSend.call(popupNavigator, 'from-top', 'popup-body')",
            "popup.document.URL",
            "'https://beacon-popup.test/popup/from-top'",
        ),
        (
            "popupSend.call(topNavigator, 'from-popup', 'popup-body')",
            "document.URL",
            "new URL('from-popup', document.baseURI).href",
        ),
    ] {
        assert_eq!(vm.eval(&format!("String({call})")).unwrap(), "true");
        let expected_owner = vm.eval(owner_url).unwrap();
        let expected_target = vm.eval(target_url).unwrap();
        let requests = vm.take_pending_subresource_fetch_infos();
        assert_eq!(requests.len(), 1, "{call}");
        assert_eq!(requests[0].document_url.as_str(), expected_owner, "{call}");
        assert_eq!(requests[0].url.as_str(), expected_target, "{call}");
        assert_eq!(requests[0].request_body.as_deref(), Some("popup-body"));
    }

    let popup_id = vm._context_host.borrow().open_lightweight_popup_ids()[0];
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let previous = crate::native_bridge::enter_active_lightweight_popup_scope(scope, popup_id);
        let code = v8::String::new(
            scope,
            "topSend.call(topNavigator, 'ambient-popup', 'main-body')",
        )
        .unwrap();
        let accepted = v8::Script::compile(scope, code, None)
            .and_then(|script| crate::script_execution::execute_compiled_script(scope, script))
            .map(|value| value.boolean_value(scope));
        crate::native_bridge::restore_active_lightweight_popup_scope(scope, previous);
        assert_eq!(accepted, Some(true));
        Ok(())
    })
    .unwrap();
    let requests = vm.take_pending_subresource_fetch_infos();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].document_url.as_str(),
        "https://beacon-popup.test/parent/page"
    );
    assert_eq!(
        requests[0].url.as_str(),
        "https://beacon-popup.test/parent/ambient-popup"
    );

    assert_eq!(
        vm.eval(
            r#"
        globalThis.bodyConversions = 0;
        String(topSend.call(popupNavigator, 'closed', {
            toString() { ++bodyConversions; popup.close(); return 'closed-body'; }
        }))
    "#
        )
        .unwrap(),
        "true"
    );
    assert_eq!(vm.eval("String(bodyConversions)").unwrap(), "1");
    assert_eq!(
        vm.eval("String(popup.closed && popupDocument.defaultView !== null)")
            .unwrap(),
        "true"
    );
    let requests = vm.take_pending_subresource_fetch_infos();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].document_url.as_str(),
        vm.eval("popupDocument.URL").unwrap()
    );
    assert_eq!(
        requests[0].url.as_str(),
        "https://beacon-popup.test/popup/closed"
    );
    assert_eq!(requests[0].request_body.as_deref(), Some("closed-body"));
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(popup.closed && popupDocument.defaultView === null)",
        "true",
        "the native close task must retire the popup before its retained Navigator is rejected",
    )
    .await;
    assert_eq!(
        vm.eval("String(topSend.call(popupNavigator, 'retired', {toString(){++bodyConversions;return 'retired-body'}}))")
            .unwrap(),
        "false"
    );
    assert_eq!(vm.eval("String(bodyConversions)").unwrap(), "2");
    assert!(vm.take_pending_subresource_fetch_infos().is_empty());
}
