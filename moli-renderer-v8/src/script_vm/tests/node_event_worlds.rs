use super::*;

#[tokio::test(flavor = "current_thread")]
async fn node_event_callbacks_project_nodes_and_event_state_in_both_layout_modes() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
    ] {
        let mut vm = new_storage_test_vm("https://node-event-worlds.test/");
        vm.set_layout_policy(policy);
        vm.eval(
            r#"
            globalThis.button = document.createElement('button');
            button.id = 'probe';
            button.textContent = 'text';
            (document.body || document.documentElement || document).appendChild(button);
            globalThis.nodes = [button, button.firstChild, document];
            globalThis.defaultFacts = [];
            globalThis.originals = [];
            globalThis.payload = {token: 'payload'};
            nodes.forEach((node, index) => {
                node.defaultNodeOnly = true;
                node.addEventListener('node-world-' + index, function(event) {
                    defaultFacts.push([
                        this === node, event === originals[index], event instanceof CustomEvent,
                        event.target === node, event.currentTarget === node,
                        event.srcElement === node, event.composedPath()[0] === node
                    ]);
                });
            });
        "#,
        )
        .unwrap();
        let isolated = vm
            .create_isolated_world("node-event-listeners", false)
            .unwrap();
        vm.eval_in_isolated_context(isolated, r#"
            globalThis.nodes = [document.getElementById('probe'), document.getElementById('probe').firstChild, document];
            globalThis.facts = [];
            globalThis.objectFacts = [];
            globalThis.savedEvents = [];
            nodes.forEach((node, index) => {
                node.isolatedNodeOnly = true;
                node.addEventListener('node-world-' + index, function(event) {
                    savedEvents[index] = event;
                    facts.push([
                        this === node, event instanceof CustomEvent,
                        event.target === node, event.currentTarget === node,
                        event.srcElement === node, event.composedPath()[0] === node,
                        window.event === event, event.defaultOnly === undefined,
                        event.target.defaultNodeOnly === undefined, event.detail.token === 'payload'
                    ]);
                    event.preventDefault();
                });
                const listener = {handleEvent(event) {
                    objectFacts.push([this === listener, event === savedEvents[index],
                        event.defaultPrevented, event.currentTarget === node]);
                }};
                node.addEventListener('node-world-' + index, listener);
            });
        "#).unwrap();
        let builds = vm.layout_pass_observability_for_test().1;
        let result = vm.eval(r#"
            JSON.stringify(nodes.map((node, index) => {
                const event = new CustomEvent('node-world-' + index, {cancelable: true, detail: payload});
                event.defaultOnly = true;
                originals[index] = event;
                return [node.dispatchEvent(event) === false, event.defaultPrevented,
                    event.detail === payload,
                    event.currentTarget === null, event.composedPath().length === 0,
                    node.isolatedNodeOnly === undefined];
            }));
        "#).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).unwrap(),
            serde_json::json!(vec![vec![true; 6]; 3]),
            "shared dispatch state and arbitrary JS payloads: {policy:?}"
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &vm.eval("JSON.stringify(defaultFacts)").unwrap()
            )
            .unwrap(),
            serde_json::json!(vec![vec![true; 7]; 3]),
            "default wrappers: {policy:?}"
        );
        for (expression, expected) in [
            (
                "JSON.stringify(facts)",
                serde_json::json!(vec![vec![true; 10]; 3]),
            ),
            (
                "JSON.stringify(objectFacts)",
                serde_json::json!(vec![vec![true; 4]; 3]),
            ),
            (
                "JSON.stringify(savedEvents.map(event => [event.currentTarget === null, event.composedPath().length === 0, event.defaultPrevented]))",
                serde_json::json!(vec![vec![true; 3]; 3]),
            ),
        ] {
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(
                    &vm.eval_in_isolated_context(isolated, expression).unwrap()
                )
                .unwrap(),
                expected,
                "{expression}: {policy:?}"
            );
        }
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            builds,
            "Node event projection must not obtain layout: {policy:?}"
        );
        vm.destroy_isolated_world_context(isolated);
        assert_eq!(vm.eval(r#"
            const later = new CustomEvent('node-world-0', {cancelable: true});
            originals[0] = later;
            JSON.stringify([button.dispatchEvent(later), later.defaultPrevented, defaultFacts.length]);
        "#).unwrap(), "[true,false,4]", "retired Node listeners: {policy:?}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn bubbling_node_events_project_the_whole_path_and_share_stop_flags() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
    ] {
        let mut vm = new_storage_test_vm("https://bubbling-node-worlds.test/");
        vm.set_layout_policy(policy);
        vm.eval(
            r#"
            globalThis.container = document.createElement('div');
            container.id = 'container';
            globalThis.button = document.createElement('button');
            button.id = 'probe';
            container.appendChild(button);
            (document.body || document.documentElement || document).appendChild(container);
            globalThis.sideEffects = 0;
        "#,
        )
        .unwrap();
        let isolated = vm
            .create_isolated_world("bubbling-node-listeners", false)
            .unwrap();
        vm.eval_in_isolated_context(
            isolated,
            r#"
            globalThis.button = document.getElementById('probe');
            globalThis.container = document.getElementById('container');
            globalThis.facts = [];
            globalThis.firstEvent = null;
            function observe(label, target, phase) {
                target.addEventListener('bubble-world', function(event) {
                    if (!firstEvent) firstEvent = event;
                    const path = event.composedPath();
                    facts.push([label, this === target, event === firstEvent,
                        event instanceof Event, event.target === button,
                        event.currentTarget === target, event.eventPhase === phase,
                        path[0] === button, path.includes(container), path.includes(document),
                        path[path.length - 1] === window, window.event === event]);
                    event.preventDefault();
                }, phase === 1);
            }
            observe('capture', document, 1);
            observe('node', button, 2);
            observe('ancestor', container, 3);
            observe('window', window, 3);
            button.addEventListener('stop-world', event => {
                event.preventDefault();
                event.stopImmediatePropagation();
            });
        "#,
        )
        .unwrap();
        vm.eval(
            r#"
            button.addEventListener('stop-world', () => sideEffects++);
            document.addEventListener('stop-world', () => sideEffects++);
        "#,
        )
        .unwrap();
        let builds = vm.layout_pass_observability_for_test().1;
        assert_eq!(
            vm.eval(
                r#"
            const first = new Event('bubble-world', {bubbles: true, cancelable: true});
            const second = new Event('stop-world', {bubbles: true, cancelable: true});
            JSON.stringify([button.dispatchEvent(first), first.defaultPrevented,
                button.dispatchEvent(second), second.defaultPrevented, sideEffects]);
        "#
            )
            .unwrap(),
            "[false,true,false,true,0]",
            "cancellation and propagation across worlds: {policy:?}"
        );
        let result = vm
            .eval_in_isolated_context(isolated, "JSON.stringify(facts)")
            .unwrap();
        let expected: Vec<_> = ["capture", "node", "ancestor", "window"]
            .into_iter()
            .map(|label| {
                let mut row = vec![serde_json::json!(label)];
                row.extend(std::iter::repeat_n(serde_json::json!(true), 11));
                row
            })
            .collect();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).unwrap(),
            serde_json::json!(expected),
            "Node-to-Window path projection: {policy:?}"
        );
        assert_eq!(vm.eval_in_isolated_context(isolated, "JSON.stringify([firstEvent.currentTarget === null, firstEvent.composedPath().length === 0, firstEvent.defaultPrevented])").unwrap(), "[true,true,true]");
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            builds,
            "bubbling must not obtain layout: {policy:?}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn borrowed_default_world_node_callback_keeps_the_target_documents_wrappers() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
    ] {
        let mut vm = new_storage_test_vm("https://borrowed-node-world.test/");
        vm.set_layout_policy(policy);
        vm.eval(
            r#"
            globalThis.frame = document.createElement('iframe');
            globalThis.button = document.createElement('button');
            const container = document.createElement('div');
            (document.body || document.documentElement || document).appendChild(container);
            container.append(button, frame);
        "#,
        )
        .unwrap();
        materialize_single_child_default_realm_for_test(&mut vm, "borrowed Node callback");
        vm.eval(r#"
            globalThis.facts = [];
            globalThis.original = null;
            button.addEventListener('borrowed-node', frame.contentWindow.Function('event', `
                parent.facts.push([
                    this === parent.button, this instanceof parent.Node, !(this instanceof Node),
                    event === parent.original, event instanceof parent.Event, !(event instanceof Event),
                    event.target === parent.button, event.currentTarget === parent.button,
                    event.composedPath()[0] === parent.button, window.event === event
                ]);
            `));
        "#).unwrap();
        let builds = vm.layout_pass_observability_for_test().1;
        let result = vm
            .eval(
                r#"
            original = new Event('borrowed-node');
            button.dispatchEvent(original);
            JSON.stringify(facts);
        "#,
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).unwrap(),
            serde_json::json!([vec![true; 10]]),
            "borrowed callback must retain the real target Window: {policy:?}"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    }
}
