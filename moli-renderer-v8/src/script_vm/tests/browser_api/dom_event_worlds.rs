use super::*;

const WORLD_PROBE: &str = include_str!("dom_event_worlds.js");

fn assert_world_probe(result: &str, label: &str) {
    let facts: serde_json::Value = serde_json::from_str(result).unwrap();
    assert_eq!(facts["errors"], serde_json::json!([]), "{label}: {facts}");
    for group in ["rows", "dispatches"] {
        for row in facts[group].as_array().unwrap() {
            for (name, value) in row["checks"].as_object().unwrap() {
                assert_eq!(value, true, "{label} {name}: {row}");
            }
        }
    }
    for (name, value) in facts["checks"].as_object().unwrap() {
        assert_eq!(value, true, "{label} {name}: {facts}");
    }
}

#[test]
fn dom_event_worlds_project_interfaces_receivers_paths_and_ui_fields_in_both_directions() {
    let mut vm = new_storage_html_test_vm("https://event-worlds.test/");
    vm.eval(
        "document.body.innerHTML = '<button id=target></button><button id=peer></button>'; 'ready'",
    )
    .unwrap();
    vm.eval(&format!(
        "{WORLD_PROBE}\ninstallDomEventWorldProbe('main'); 'ready'"
    ))
    .unwrap();
    let isolated = vm.create_isolated_world("dom-events", false).unwrap();
    vm.eval_in_isolated_context(
        isolated,
        &format!("{WORLD_PROBE}\ninstallDomEventWorldProbe('isolated'); 'ready'"),
    )
    .unwrap();
    for isolated_dispatch in [false, true] {
        vm.eval("resetDomEventWorldProbe(); 'reset'").unwrap();
        vm.eval_in_isolated_context(isolated, "resetDomEventWorldProbe(); 'reset'")
            .unwrap();
        if isolated_dispatch {
            vm.eval_in_isolated_context(isolated, "dispatchDomEventWorldProbe(); 'dispatched'")
                .unwrap();
        } else {
            vm.eval("dispatchDomEventWorldProbe(); 'dispatched'")
                .unwrap();
        }
        assert_world_probe(
            &vm.eval("JSON.stringify(readDomEventWorldProbe())").unwrap(),
            "main",
        );
        assert_world_probe(
            &vm.eval_in_isolated_context(isolated, "JSON.stringify(readDomEventWorldProbe())")
                .unwrap(),
            "isolated",
        );
    }
}

#[tokio::test]
async fn dom_event_worlds_project_child_window_and_document_in_both_directions() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://event-worlds.test/", &loader);
    vm.eval(
        r#"
        if (!document.documentElement) document.appendChild(document.createElement('html'));
        if (!document.body) document.documentElement.appendChild(document.createElement('body'));
        const frame = document.body.appendChild(document.createElement('iframe'));
        frame.srcdoc = '<button id=target></button><button id=peer></button>';
        void frame.contentWindow;
        'ready'
    "#,
    )
    .unwrap();
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::RealmMaterialization,
            &loader
        )
        .await
        .unwrap()
    );
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
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
        &format!("{WORLD_PROBE}\ninstallDomEventWorldProbe('main'); 'ready'"),
    )
    .unwrap();
    let isolated = vm
        .create_isolated_world_for_frame(&frame_id, "child-dom-events", false)
        .unwrap();
    vm.eval_in_isolated_context(
        isolated,
        &format!("{WORLD_PROBE}\ninstallDomEventWorldProbe('isolated'); 'ready'"),
    )
    .unwrap();
    for isolated_dispatch in [false, true] {
        vm.eval_in_child_default_context(child, "resetDomEventWorldProbe(); 'reset'")
            .unwrap();
        vm.eval_in_isolated_context(isolated, "resetDomEventWorldProbe(); 'reset'")
            .unwrap();
        if isolated_dispatch {
            vm.eval_in_isolated_context(isolated, "dispatchDomEventWorldProbe(); 'dispatched'")
                .unwrap();
        } else {
            vm.eval_in_child_default_context(child, "dispatchDomEventWorldProbe(); 'dispatched'")
                .unwrap();
        }
        assert_world_probe(
            &vm.eval_in_child_default_context(child, "JSON.stringify(readDomEventWorldProbe())")
                .unwrap(),
            "child main",
        );
        assert_world_probe(
            &vm.eval_in_isolated_context(isolated, "JSON.stringify(readDomEventWorldProbe())")
                .unwrap(),
            "child isolated",
        );
    }
    assert_eq!(
        vm.eval("String(window.event === undefined && window.worldLabel === undefined)")
            .unwrap(),
        "true"
    );
}

#[tokio::test]
async fn dom_event_worlds_foreign_window_targets_keep_the_listener_world_and_original_identity() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://event-worlds.test/", &loader);
    vm.eval(
        r#"
        if (!document.documentElement) document.appendChild(document.createElement('html'));
        if (!document.body) document.documentElement.appendChild(document.createElement('body'));
        const frame = document.body.appendChild(document.createElement('iframe'));
        frame.srcdoc = '<p>Foreign Window target</p>';
        void frame.contentWindow;
        'ready'
    "#,
    )
    .unwrap();
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::RealmMaterialization,
            &loader
        )
        .await
        .unwrap()
    );
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    let child = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .next()
        .unwrap()
        .context_id;
    let isolated = vm
        .create_isolated_world("foreign-window-listener", false)
        .unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r#"
        globalThis.target = document.querySelector('iframe').contentWindow;
        const original = new UIEvent('foreign-probe', {view: window});
        const facts = [];
        target.addEventListener('foreign-probe', function(event) {
            facts.push(event instanceof UIEvent, event === original, this === target,
                event.target === target, event.currentTarget === target,
                window.event === event, event.view === window);
        }, {once: true});
        target.dispatchEvent(original);
        target.addEventListener('child-probe', function(event) {
            facts.push(event instanceof UIEvent, this === target, event.target === target,
                event.currentTarget === target, window.event === event, event.view === target);
        }, {once: true});
        globalThis.foreignFacts = facts;
        JSON.stringify(facts)
    "#
        )
        .unwrap(),
        "[true,true,true,true,true,true,true]"
    );
    vm.eval_in_child_default_context(
        child,
        "window.dispatchEvent(new UIEvent('child-probe', {view: window})); 'dispatched'",
    )
    .unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "JSON.stringify(foreignFacts)")
            .unwrap(),
        "[true,true,true,true,true,true,true,true,true,true,true,true,true]"
    );
}

#[test]
fn dom_event_worlds_share_propagation_once_passive_and_handler_cancellation() {
    let mut vm = new_storage_html_test_vm("https://event-worlds.test/");
    vm.eval(
        r#"
        document.body.innerHTML = '<button id=target></button>';
        globalThis.target = document.getElementById('target');
        globalThis.trace = [];
        target.addEventListener('cancel', event => trace.push(['first', event.defaultPrevented]));
        target.addEventListener('stop', () => trace.push('first'));
        target.addEventListener('passive', event => trace.push(event.defaultPrevented));
        target.addEventListener('nested', event => {
            trace.push(['outer', window.event === event]);
            target.dispatchEvent(new Event('inner'));
            trace.push(['restored', window.event === event]);
        });
        'ready'
    "#,
    )
    .unwrap();
    let isolated = vm.create_isolated_world("dom-control", false).unwrap();
    vm.eval_in_isolated_context(
        isolated,
        r#"
        globalThis.target = document.getElementById('target');
        globalThis.facts = [];
        target.addEventListener('cancel', function(event) {
            facts.push(this === target, event instanceof Event, window.event === event);
            event.preventDefault();
        }, {once: true});
        target.addEventListener('stop', event => event.stopImmediatePropagation(), {once: true});
        target.addEventListener('passive', event => {
            event.preventDefault();
            facts.push(event.defaultPrevented === false);
        }, {passive: true});
        target.onclick = function(event) {
            facts.push(this === target, event instanceof MouseEvent, window.event === event);
            return false;
        };
        target.addEventListener('inner', event => facts.push(window.event === event));
        'ready'
    "#,
    )
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"
        target.addEventListener('cancel', event => trace.push(['last', event.defaultPrevented]));
        target.addEventListener('stop', () => trace.push('last'));
        const first = new Event('cancel', {cancelable: true});
        const second = new Event('cancel', {cancelable: true});
        const stop = new Event('stop', {bubbles: true});
        const passive = new Event('passive', {cancelable: true});
        const click = new MouseEvent('click', {cancelable: true});
        const returned = [target.dispatchEvent(first), target.dispatchEvent(second),
            target.dispatchEvent(stop), target.dispatchEvent(passive), target.dispatchEvent(click)];
        target.dispatchEvent(new Event('nested'));
        JSON.stringify({returned, trace, canceled: [first.defaultPrevented, second.defaultPrevented,
            passive.defaultPrevented, click.defaultPrevented], cleared: window.event === undefined})
    "#
        )
        .unwrap(),
        r#"{"returned":[false,true,true,true,false],"trace":[["first",false],["last",true],["first",false],["last",false],"first",false,["outer",true],["restored",true]],"canceled":[true,false,false,true],"cleared":true}"#
    );
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "JSON.stringify(facts)")
            .unwrap(),
        "[true,true,true,true,true,true,true,true]"
    );
}

#[test]
fn dom_event_worlds_preserve_default_world_foreign_document_identity() {
    let mut vm = new_storage_html_test_vm("https://event-worlds.test/");
    assert_eq!(
        vm.eval(
            r#"
        const frame = document.body.appendChild(document.createElement('iframe'));
        const child = frame.contentWindow;
        const target = child.document.createElement('button');
        child.document.body.appendChild(target);
        const original = new child.MouseEvent('probe', {view: child, bubbles: true});
        const facts = [];
        target.addEventListener('probe', function(event) {
            facts.push(event === original, event instanceof child.MouseEvent,
                !(event instanceof MouseEvent), this === target, event.target === target,
                event.view === child, event.composedPath().at(-1) === child,
                window.event === event);
        });
        target.dispatchEvent(original);
        facts.push(window.event === undefined, child.event === undefined);
        JSON.stringify(facts)
    "#
        )
        .unwrap(),
        "[true,true,true,true,true,true,true,true,true,true]"
    );
}

#[test]
fn dom_event_worlds_use_native_slots_and_reuse_retained_views_on_redispatch() {
    let mut vm = new_storage_html_test_vm("https://event-worlds.test/");
    vm.eval(r#"
        document.body.innerHTML = '<button id=target></button>';
        globalThis.target = document.getElementById('target');
        globalThis.original = new UIEvent('probe', {view: window, detail: 3, cancelable: true});
        globalThis.reads = 0;
        for (const name of ['view', 'target', 'currentTarget', 'type', 'detail'])
            Object.defineProperty(original, name, {get() { reads++; throw new Error('author shadow'); }});
        globalThis.pageFacts = [];
        target.addEventListener('reused', event => {
            pageFacts.push(event === original,
                Object.getOwnPropertyDescriptor(Event.prototype, 'type').get.call(event) === 'reused');
            event.preventDefault();
        });
        'ready'
    "#).unwrap();
    let isolated = vm.create_isolated_world("dom-native-slots", false).unwrap();
    vm.eval_in_isolated_context(
        isolated,
        r#"
        globalThis.target = document.getElementById('target');
        globalThis.retained = null;
        globalThis.facts = [];
        const NativeUIEvent = UIEvent;
        target.addEventListener('probe', event => {
            retained = event;
            facts.push(event instanceof NativeUIEvent, event.view === window, event.detail === 3,
                event.target === target, event.currentTarget === target, event.type === 'probe');
        });
        // Rewrapping uses the intrinsic interface, even if authors replace a global constructor.
        globalThis.UIEvent = function() { throw new Error('author constructor'); };
        'ready'
    "#,
    )
    .unwrap();
    vm.eval("target.dispatchEvent(original); 'dispatched'")
        .unwrap();
    assert_eq!(vm.eval_in_isolated_context(isolated, r#"
        retained.initUIEvent('reused', false, true, window, 9);
        target.addEventListener('reused', event => facts.push(event === retained, event.defaultPrevented));
        facts.push(target.dispatchEvent(retained), retained.defaultPrevented,
            retained.currentTarget === null, retained.view === window, retained.detail === 9);
        JSON.stringify(facts)
    "#).unwrap(), "[true,true,true,true,true,true,true,true,false,true,true,true,true]");
    assert_eq!(
        vm.eval("JSON.stringify([reads, pageFacts, original.defaultPrevented])")
            .unwrap(),
        "[0,[true,true],true]"
    );
}

#[test]
fn dom_event_worlds_project_window_handlers_without_changing_legacy_error_arguments() {
    let mut vm = new_storage_html_test_vm("https://event-worlds.test/");
    vm.eval(
        r#"
        globalThis.unload = document.createEvent('BeforeUnloadEvent');
        unload.initEvent('beforeunload', false, true);
        globalThis.error = new ErrorEvent('error', {message: 'original', filename: 'source.js',
            lineno: 12, colno: 34, error: {marker: 7}, cancelable: true});
        'ready'
    "#,
    )
    .unwrap();
    let isolated = vm
        .create_isolated_world("dom-legacy-handlers", false)
        .unwrap();
    vm.eval_in_isolated_context(isolated, r#"
        globalThis.facts = [];
        window.onbeforeunload = function(event) {
            facts.push(this === window, event instanceof BeforeUnloadEvent, event.target === window,
                event.currentTarget === window, window.event === event);
            return 'leave?';
        };
        window.addEventListener('error', event => {
            facts.push(event instanceof ErrorEvent, event.target === window, window.event === event);
            globalThis.savedError = event;
        });
        window.onerror = function(message, source, line, column, error) {
            facts.push(this === window, window.event === savedError,
                arguments.length === 5, message === 'original', source === 'source.js',
                line === 12, column === 34, error === savedError.error, error.marker === 7);
            return true;
        };
        'ready'
    "#).unwrap();
    assert_eq!(vm.eval("JSON.stringify([window.dispatchEvent(unload), unload.defaultPrevented, unload.returnValue, window.dispatchEvent(error), error.defaultPrevented, window.event === undefined])").unwrap(),
        r#"[false,true,"leave?",false,true,true]"#);
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "JSON.stringify(facts)")
            .unwrap(),
        "[true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true]"
    );
}

#[test]
fn dom_event_worlds_keep_closed_shadow_retargeting_and_callback_object_semantics() {
    let mut vm = new_storage_html_test_vm("https://event-worlds.test/");
    vm.eval(r#"
        document.body.innerHTML = '<div id=host></div>';
        globalThis.host = document.getElementById('host');
        globalThis.root = host.attachShadow({mode: 'closed'});
        globalThis.inner = root.appendChild(document.createElement('button'));
        globalThis.original = new Event('probe', {bubbles: true, composed: true});
        globalThis.pageFacts = [];
        inner.addEventListener('probe', event => pageFacts.push(event === original, event.target === inner));
        document.addEventListener('probe', event => pageFacts.push(event === original,
            event.target === host, !event.composedPath().includes(inner)));
        'ready'
    "#).unwrap();
    let isolated = vm
        .create_isolated_world("dom-shadow-listener", false)
        .unwrap();
    vm.eval_in_isolated_context(
        isolated,
        r#"
        const host = document.getElementById('host');
        globalThis.facts = [];
        let reads = 0;
        const callback = {
            get handleEvent() {
                reads++;
                return function(event) {
                    const path = event.composedPath();
                    facts.push(this === callback, reads === 1, event instanceof Event,
                        event.target === host, event.currentTarget === document, path[0] === host,
                        path.includes(document), path.at(-1) === window,
                        path.every(value => value === window || value instanceof Node));
                };
            }
        };
        document.addEventListener('probe', callback, {once: true});
        'ready'
    "#,
    )
    .unwrap();
    vm.eval("inner.dispatchEvent(original); 'dispatched'")
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(pageFacts)").unwrap(),
        "[true,true,true,true,true]"
    );
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "JSON.stringify(facts)")
            .unwrap(),
        "[true,true,true,true,true,true,true,true,true]"
    );
}

#[test]
fn dom_event_worlds_do_not_root_unobserved_foreign_event_wrappers() {
    let mut vm = new_storage_html_test_vm("https://event-worlds.test/");
    vm.eval(
        r#"
        document.body.innerHTML = '<button id=target></button>';
        globalThis.keptEvent = new CustomEvent('probe', {detail: {value: 7}});
        'ready'
    "#,
    )
    .unwrap();
    let isolated = vm.create_isolated_world("dom-event-gc", false).unwrap();
    vm.eval_in_isolated_context(isolated, "document.getElementById('target').addEventListener('probe', event => { globalThis.view = event; }, {once: true}); 'ready'")
        .unwrap();
    vm.eval("document.getElementById('target').dispatchEvent(keptEvent); 'dispatched'")
        .unwrap();
    let context_ptr = &vm
        .page_isolated_world_contexts
        .context(isolated)
        .unwrap()
        .context as *const _;
    let view = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            let global = scope.get_current_context().global(scope);
            let value = global
                .get(scope, crate::util::v8str(scope, "view").into())
                .unwrap();
            Ok(v8::Weak::new(
                scope,
                v8::Local::<v8::Object>::try_from(value).unwrap(),
            ))
        })
        .unwrap();
    vm.eval_in_isolated_context(isolated, "view = null; 'released'")
        .unwrap();
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(|isolate| {
            isolate.low_memory_notification();
            Ok(())
        })
        .unwrap();
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        assert!(
            view.to_local(scope).is_none(),
            "a retained original event must not root its foreign wrapper"
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval("String(keptEvent.detail.value)").unwrap(), "7");
}
