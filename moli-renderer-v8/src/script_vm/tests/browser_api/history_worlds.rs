use super::*;

#[tokio::test]
async fn history_worlds_share_mutations_but_keep_wrappers_and_state_caches_separate() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://example.com/base", &loader);
    vm.eval(r##"
        globalThis.initialLength = history.length;
        globalThis.changes = [];
        navigation.addEventListener('currententrychange', () => changes.push(location.hash));
        history.replaceState({value: 1, map: new Map([['x', 2n]]), bytes: new Uint8Array([3])}, '', '#one');
        history.scrollRestoration = 'manual';
        globalThis.pageState = history.state;
        pageState.value = 99;
        history.expando = 'page';
        history.pushState = () => { throw new Error('page override'); };
        'ready'
    "##).expect("default world setup");
    let isolated = vm
        .create_isolated_world("history-shared", false)
        .expect("isolated world");
    const READ_STATE: &str = r#"JSON.stringify([
        history.state.value,
        history.state.map instanceof Map,
        history.state.map.get('x') === 2n,
        history.state.bytes instanceof Uint8Array,
        history.state.bytes[0],
        Object.getPrototypeOf(history.state) === Object.prototype,
        history.state === history.state,
        history.scrollRestoration,
        location.hash,
        history instanceof History,
        history.expando === undefined
    ])"#;
    assert_eq!(
        vm.eval_in_isolated_context(isolated, READ_STATE).unwrap(),
        r##"[1,true,true,true,3,true,true,"manual","#one",true,true]"##
    );
    assert_eq!(
        vm.eval("String(history.state === pageState && history.state.value === 99)")
            .unwrap(),
        "true"
    );
    vm.eval_in_isolated_context(
        isolated,
        r##"
        globalThis.isolatedState = history.state;
        isolatedState.value = 42;
        history.pushState({value: 2}, '', '#two');
        history.scrollRestoration = 'auto';
        'pushed'
    "##,
    )
    .expect("isolated mutation bypasses page override");
    const READ_CURRENT: &str = r#"JSON.stringify([
        history.state.value, location.hash, navigation.currentEntry.url,
        history.scrollRestoration, history.state === history.state,
        Object.getPrototypeOf(history.state) === Object.prototype
    ])"#;
    let expected = r##"[2,"#two","https://example.com/base#two","auto",true,true]"##;
    assert_eq!(vm.eval(READ_CURRENT).unwrap(), expected);
    assert_eq!(
        vm.eval_in_isolated_context(isolated, READ_CURRENT).unwrap(),
        expected
    );
    assert_eq!(
        vm.eval("String(history.length === initialLength + 1)")
            .unwrap(),
        "true"
    );
    let later = vm
        .create_isolated_world("history-later", false)
        .expect("later isolated world");
    assert_eq!(
        vm.eval_in_isolated_context(later, READ_CURRENT).unwrap(),
        expected
    );
    vm.eval("history.replaceState({value: 3}, '', '#three'); 'replaced'")
        .unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "String(history.state.value) + location.hash")
            .unwrap(),
        "3#three"
    );
    assert_eq!(
        vm.eval_in_isolated_context(later, "String(history.state.value) + location.hash")
            .unwrap(),
        "3#three"
    );
    assert_eq!(vm.eval("changes.join('|')").unwrap(), "#one|#two|#three");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    vm.eval_in_isolated_context(isolated, "history.back(); 'queued'")
        .unwrap();
    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(
        vm.eval_in_isolated_context(isolated, READ_STATE).unwrap(),
        r##"[1,true,true,true,3,true,true,"auto","#one",true,true]"##
    );
    assert_eq!(vm.eval("String(history.state.value)").unwrap(), "1");
    vm.eval_in_isolated_context(later, "history.forward(); 'queued'")
        .unwrap();
    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(
        vm.eval("String(history.state.value) + location.hash")
            .unwrap(),
        "3#three"
    );
}

#[tokio::test]
async fn history_worlds_child_mutations_share_only_the_child_window_history() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://example.com/base", &loader);
    vm.eval(
        r##"
        history.replaceState({owner: 'top'}, '', '#top');
        const frame = document.createElement('iframe');
        (document.body || document.documentElement || document).appendChild(frame);
        void frame.contentWindow;
        'created'
    "##,
    )
    .unwrap();
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::RealmMaterialization,
            &loader,
        )
        .await
        .unwrap()
    );
    let child = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .next()
        .unwrap()
        .context_id;
    let frame_id = vm
        .child_frame_realm_store
        .get(&child)
        .unwrap()
        .frame_id
        .clone();
    vm.eval_in_child_default_context(
        child,
        r##"
        history.replaceState({owner: 'child'}, '', '#child');
        history.scrollRestoration = 'manual';
        'ready'
    "##,
    )
    .unwrap();
    let isolated = vm
        .create_isolated_world_for_frame(&frame_id, "child-history", false)
        .unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            "history.state.owner + ':' + history.scrollRestoration + ':' + location.hash"
        )
        .unwrap(),
        "child:manual:#child"
    );
    vm.eval_in_isolated_context(
        isolated,
        r##"
        history.pushState({owner: 'isolated-child'}, '', '#next');
        history.scrollRestoration = 'auto';
        'pushed'
    "##,
    )
    .unwrap();
    const READ_CHILD: &str =
        "history.state.owner + ':' + history.scrollRestoration + ':' + location.hash";
    assert_eq!(
        vm.eval_in_child_default_context(child, READ_CHILD).unwrap(),
        "isolated-child:auto:#next"
    );
    assert_eq!(
        vm.eval("history.state.owner + ':' + location.hash")
            .unwrap(),
        "top:#top"
    );
    assert_eq!(
        vm.eval("document.querySelector('iframe').contentWindow.history.state.owner")
            .unwrap(),
        "isolated-child"
    );
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    vm.eval("globalThis.topPops = 0; addEventListener('popstate', () => topPops++); 'listening'")
        .unwrap();
    vm.eval_in_child_default_context(child,
        "globalThis.childPops = 0; addEventListener('popstate', () => childPops++); history.back(); 'queued'").unwrap();
    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(
        vm.eval_in_isolated_context(isolated, READ_CHILD).unwrap(),
        "child:auto:#child"
    );
    vm.eval_in_isolated_context(isolated, "history.forward(); 'queued'")
        .unwrap();
    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(
        vm.eval_in_child_default_context(child, READ_CHILD).unwrap(),
        "isolated-child:auto:#next"
    );
    assert_eq!(vm.eval("String(topPops)").unwrap(), "0");
    assert_eq!(
        vm.eval_in_child_default_context(child, "String(childPops)")
            .unwrap(),
        "2"
    );
    vm.eval_in_child_default_context(child, "location.hash = '#fragment'; 'navigated'")
        .unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "location.hash")
            .unwrap(),
        "#fragment"
    );
    vm.eval_in_isolated_context(
        isolated,
        "location.hash = '#isolated-fragment'; 'navigated'",
    )
    .unwrap();
    assert_eq!(
        vm.eval_in_child_default_context(child, "location.hash")
            .unwrap(),
        "#isolated-fragment"
    );
    assert_eq!(vm.eval("location.hash").unwrap(), "#top");
    vm.eval_in_isolated_context(isolated, "parent.detachedHistory = history; 'saved'")
        .unwrap();
    assert_eq!(
        vm.eval(
            r#"
            document.querySelector('iframe').remove();
            try { detachedHistory.pushState({}, '', '#detached'); 'accepted'; }
            catch (error) { error.name; }
        "#
        )
        .unwrap(),
        "SecurityError"
    );
}

#[tokio::test]
async fn history_worlds_preserve_the_shared_backing_when_initial_child_window_is_reused() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(
        r#"
        const frame = document.createElement('iframe');
        (document.body || document.documentElement || document).appendChild(frame);
        void frame.contentWindow;
    "#,
    )
    .unwrap();
    let child = materialize_single_child_default_realm_for_test(&mut vm, "History rebind");
    let frame_id = vm
        .child_frame_realm_store
        .get(&child)
        .unwrap()
        .frame_id
        .clone();
    let isolated = vm
        .create_isolated_world_for_frame(&frame_id, "rebound-history", false)
        .unwrap();
    vm.eval_in_isolated_context(isolated, "globalThis.originalHistory = history; 'saved'")
        .unwrap();
    vm.eval("frame.srcdoc = '<p>committed</p>'; 'navigating'")
        .unwrap();
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "History rebind").await;
    vm.eval_in_isolated_context(
        isolated,
        r##"
        originalHistory.replaceState({rebound: true}, '', '#rebound');
        'replaced'
    "##,
    )
    .unwrap();
    assert_eq!(
        vm.eval_in_child_default_context(
            child,
            "String(history.state.rebound) + ':' + location.hash"
        )
        .unwrap(),
        "true:#rebound"
    );
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "String(history === originalHistory)")
            .unwrap(),
        "true"
    );
}

#[test]
fn history_worlds_share_primitive_state_without_coercion() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    let isolated = vm
        .create_isolated_world("primitive-history", false)
        .unwrap();
    for state in ["undefined", "null", "-0", "0", "NaN", "1n", "false"] {
        let update = format!("history.replaceState({state}, ''); 'updated'");
        let check = format!("String(Object.is(history.state, {state}))");
        vm.eval(&update).unwrap();
        assert_eq!(
            vm.eval_in_isolated_context(isolated, &check).unwrap(),
            "true",
            "state: {state}"
        );
        vm.eval_in_isolated_context(isolated, &update).unwrap();
        assert_eq!(vm.eval(&check).unwrap(), "true", "state: {state}");
    }
}
