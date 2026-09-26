use super::*;

#[test]
fn history_worlds_slot_backed_platform_fields_keep_local_wrappers() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(r#"
        if (!document.documentElement) document.appendChild(document.createElement('html'));
        if (!document.body) document.documentElement.appendChild(document.createElement('body'));
        document.body.innerHTML = '<button id="submitter"></button>';
        globalThis.data = new FormData();
        data.set('value', 'original');
        data.pageOnly = true;
        data.get = () => 'page override';
        globalThis.formEvent = new FormDataEvent('form-probe', {formData: data});
        globalThis.submitEvent = new SubmitEvent('submit-probe', {submitter: document.getElementById('submitter')});
        submitEvent.submitter.pageOnly = true;
        globalThis.getterCalls = 0;
        Object.defineProperty(formEvent, 'formData', {get() { getterCalls++; throw new Error('shadow'); }});
        Object.defineProperty(submitEvent, 'submitter', {get() { getterCalls++; throw new Error('shadow'); }});
        'ready'
    "#).unwrap();
    let isolated = vm.create_isolated_world("slot-fields", false).unwrap();
    vm.eval_in_isolated_context(isolated, r#"
        globalThis.facts = [];
        navigation.addEventListener('form-probe', event => {
            facts.push(event instanceof FormDataEvent, event.formData instanceof FormData,
                event.formData.pageOnly === undefined, event.formData.get('value'));
            event.formData.set('value', 'updated');
        }, {once: true});
        navigation.addEventListener('submit-probe', event => {
            facts.push(event instanceof SubmitEvent, event.submitter instanceof HTMLButtonElement,
                event.submitter === document.getElementById('submitter'), event.submitter.pageOnly === undefined);
        }, {once: true});
        'ready'
    "#).unwrap();
    vm.eval("navigation.dispatchEvent(formEvent); navigation.dispatchEvent(submitEvent); 'sent'")
        .unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "JSON.stringify(facts)")
            .unwrap(),
        r#"[true,true,true,"original",true,true,true,true]"#
    );
    assert_eq!(
        vm.eval("JSON.stringify([getterCalls, FormData.prototype.get.call(data, 'value')])")
            .unwrap(),
        r#"[0,"updated"]"#
    );
}

#[test]
fn history_worlds_event_backing_collects_cycles_and_releases_replaced_values() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(
        r#"
        globalThis.payloadForGc = {value: 7};
        globalThis.eventForGc = new CustomEvent('gc-probe', {detail: payloadForGc});
        payloadForGc.event = eventForGc;
        'ready'
    "#,
    )
    .unwrap();
    let isolated = vm.create_isolated_world("event-backing-gc", false).unwrap();
    vm.eval_in_isolated_context(isolated, r#"
        globalThis.foreignEventForGc = null;
        navigation.addEventListener('gc-probe', event => { foreignEventForGc = event; }, {once: true});
        'ready'
    "#).unwrap();
    vm.eval("navigation.dispatchEvent(eventForGc); 'dispatched'")
        .unwrap();
    let context_ptr = &vm
        .page_isolated_world_contexts
        .context(isolated)
        .unwrap()
        .context as *const _;
    let weak_values = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            let global = scope.get_current_context().global(scope);
            let event = global
                .get(scope, crate::util::v8str(scope, "foreignEventForGc").into())
                .unwrap();
            let event = v8::Local::<v8::Object>::try_from(event).unwrap();
            let payload = event
                .get(scope, crate::util::v8str(scope, "detail").into())
                .unwrap();
            let payload = v8::Local::<v8::Object>::try_from(payload).unwrap();
            Ok([v8::Weak::new(scope, event), v8::Weak::new(scope, payload)])
        })
        .unwrap();
    // The foreign view owns the same private state after the source globals go away.
    vm.eval("eventForGc = payloadForGc = null; 'released'")
        .unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "String(foreignEventForGc.detail.value)")
            .unwrap(),
        "7"
    );
    vm.eval_in_isolated_context(
        isolated,
        "foreignEventForGc.initCustomEvent('reused', false, false, '\\ud800'); 'replaced'",
    )
    .unwrap();
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(|isolate| {
            isolate.low_memory_notification();
            Ok(())
        })
        .unwrap();
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        assert!(weak_values[0].to_local(scope).is_some());
        assert!(
            weak_values[1].to_local(scope).is_none(),
            "replacing a backing field must release the previous cyclic payload"
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "String(foreignEventForGc.detail.charCodeAt(0))")
            .unwrap(),
        "55296"
    );
    // Create a cycle again, then drop its final root. A native strong Global
    // would keep this graph alive; the private V8 backing must not.
    vm.eval_in_isolated_context(
        isolated,
        r#"
        foreignEventForGc.initCustomEvent('cycle', false, false, {event: foreignEventForGc});
        foreignEventForGc = null;
        'released'
    "#,
    )
    .unwrap();
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(|isolate| {
            isolate.low_memory_notification();
            Ok(())
        })
        .unwrap();
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        assert!(
            weak_values[0].to_local(scope).is_none(),
            "event/backing/payload cycles must be collectible"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn history_worlds_toggle_events_share_fields_without_reconstructing_the_event() {
    for isolated_source in [false, true] {
        let mut vm = new_storage_test_vm("https://example.com/base");
        let isolated = vm.create_isolated_world("toggle-fields", false).unwrap();
        let listener = r#"
            globalThis.facts = [];
            navigation.addEventListener('toggle-probe', event => {
                facts.push(event instanceof ToggleEvent, event instanceof Event,
                    event.oldState, event.newState, event.source === null,
                    event.target === navigation, event.currentTarget === navigation,
                    event.expando === undefined);
            }, {once: true});
            'ready'
        "#;
        if isolated_source {
            vm.eval(listener).unwrap();
        } else {
            vm.eval_in_isolated_context(isolated, listener).unwrap();
        }
        let dispatch = r#"
            const original = new ToggleEvent('toggle-probe', {oldState: 'closed', newState: 'open'});
            original.expando = 'source-only';
            let sameObject = false;
            navigation.addEventListener('toggle-probe', event => { sameObject = event === original; }, {once: true});
            JSON.stringify([navigation.dispatchEvent(original), sameObject, original.oldState, original.newState])
        "#;
        let source = if isolated_source {
            vm.eval_in_isolated_context(isolated, dispatch)
        } else {
            vm.eval(dispatch)
        }
        .unwrap();
        assert_eq!(source, r#"[true,true,"closed","open"]"#);
        let foreign = if isolated_source {
            vm.eval("JSON.stringify(facts)")
        } else {
            vm.eval_in_isolated_context(isolated, "JSON.stringify(facts)")
        }
        .unwrap();
        assert_eq!(
            foreign,
            r#"[true,true,"closed","open",true,true,true,true]"#
        );
    }
}

#[test]
fn history_worlds_event_fields_ignore_public_shadows_and_getters() {
    for isolated_source in [false, true] {
        let mut vm = new_storage_test_vm("https://example.com/base");
        let isolated = vm
            .create_isolated_world("internal-event-fields", false)
            .unwrap();
        let listener = r#"
            globalThis.facts = [];
            globalThis.previousEvent = null;
            navigation.addEventListener('field-probe', event => {
                facts.push([event instanceof CustomEvent, event.detail, event.type,
                    event.cancelable, event.bubbles, event.target === navigation,
                    event.composedPath()[0] === navigation, event.expando === undefined]);
                previousEvent = event;
                event.preventDefault();
            });
            navigation.addEventListener('reinitialized', event => {
                facts.push([event === previousEvent, event.detail, event.type,
                    event.target === navigation]);
            });
            'ready'
        "#;
        if isolated_source {
            vm.eval(listener).unwrap();
        } else {
            vm.eval_in_isolated_context(isolated, listener).unwrap();
        }
        let dispatch = r#"
            globalThis.getterCalls = 0;
            globalThis.sourceFacts = [];
            for (const getter of [false, true]) {
                const original = new CustomEvent('field-probe', {detail: 7, cancelable: true, bubbles: true});
                original.expando = true;
                Object.defineProperty(original, 'detail', getter
                    ? {get() { getterCalls++; return 99; }, configurable: true}
                    : {value: 99, configurable: true});
                for (const property of ['type', 'cancelable', 'bubbles', 'target']) {
                    Object.defineProperty(original, property, {get() { getterCalls++; return 'shadow'; }, configurable: true});
                }
                let sameObject = false;
                navigation.addEventListener('field-probe', event => { sameObject = event === original; }, {once: true});
                sourceFacts.push(navigation.dispatchEvent(original), sameObject, original.defaultPrevented);
                original.initCustomEvent('reinitialized', false, false, 9);
                sourceFacts.push(navigation.dispatchEvent(original), original.defaultPrevented, getterCalls);
                if (!getter) sourceFacts.push(original.detail);
            }
            JSON.stringify(sourceFacts)
        "#;
        let source = if isolated_source {
            vm.eval_in_isolated_context(isolated, dispatch)
        } else {
            vm.eval(dispatch)
        }
        .unwrap();
        assert_eq!(
            source,
            "[false,true,true,true,false,0,99,false,true,true,true,false,0]"
        );
        let foreign = if isolated_source {
            vm.eval("JSON.stringify(facts)")
        } else {
            vm.eval_in_isolated_context(isolated, "JSON.stringify(facts)")
        }
        .unwrap();
        assert_eq!(
            foreign,
            r#"[[true,7,"field-probe",true,true,true,true,true],[true,9,"reinitialized",true],[true,7,"field-probe",true,true,true,true,true],[true,9,"reinitialized",true]]"#
        );
    }
}

#[test]
fn history_worlds_custom_event_detail_keeps_the_original_js_value() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(
        r#"
        globalThis.payload = {value: 7};
        globalThis.original = new CustomEvent('object-detail', {detail: payload});
        Object.defineProperty(original, 'detail', {value: {value: 99}});
        'ready'
    "#,
    )
    .unwrap();
    let isolated = vm.create_isolated_world("object-detail", false).unwrap();
    vm.eval_in_isolated_context(isolated, r#"
        navigation.addEventListener('object-detail', event => { event.detail.value = 8; }, {once: true});
        'ready'
    "#).unwrap();
    assert_eq!(vm.eval("navigation.dispatchEvent(original); JSON.stringify([payload.value, original.detail.value])").unwrap(), "[8,99]");
}

#[test]
fn history_worlds_event_prototype_getters_are_not_used_as_internal_state() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(r#"
        globalThis.getterCalls = 0;
        Object.defineProperty(CustomEvent.prototype, 'detail', {get() { getterCalls++; return 99; }, configurable: true});
        globalThis.original = new CustomEvent('prototype-probe', {detail: 7});
        delete original.detail;
        'ready'
    "#).unwrap();
    let isolated = vm.create_isolated_world("prototype-fields", false).unwrap();
    vm.eval_in_isolated_context(isolated, r#"
        globalThis.detail = null;
        navigation.addEventListener('prototype-probe', event => { detail = event.detail; }, {once: true});
        'ready'
    "#).unwrap();
    assert_eq!(
        vm.eval("navigation.dispatchEvent(original); String(getterCalls)")
            .unwrap(),
        "0"
    );
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "String(detail)")
            .unwrap(),
        "7"
    );
}

#[test]
fn history_worlds_retained_page_event_does_not_retain_unobserved_foreign_views() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval("navigation.addEventListener('navigate', event => { globalThis.keptEvent = event; }, {once: true}); 'ready'")
        .unwrap();
    let isolated = vm
        .create_isolated_world("collectible-views", false)
        .unwrap();
    vm.eval_in_isolated_context(isolated, "navigation.addEventListener('navigate', event => { globalThis.views = [event, event.destination, event.signal]; }, {once: true}); 'ready'")
        .unwrap();
    vm.eval("history.pushState({}, '', '#retained'); 'pushed'")
        .unwrap();
    let context_ptr = &vm
        .page_isolated_world_contexts
        .context(isolated)
        .unwrap()
        .context as *const _;
    let views = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            let global = scope.get_current_context().global(scope);
            let value = global
                .get(scope, crate::util::v8str(scope, "views").into())
                .unwrap();
            let array = v8::Local::<v8::Array>::try_from(value).unwrap();
            Ok((0..array.length())
                .map(|index| {
                    let value = array.get_index(scope, index).unwrap();
                    let object = v8::Local::<v8::Object>::try_from(value).unwrap();
                    v8::Weak::new(scope, object)
                })
                .collect::<Vec<_>>())
        })
        .unwrap();
    vm.eval_in_isolated_context(isolated, "views = null; 'released'")
        .unwrap();
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(|isolate| {
            isolate.low_memory_notification();
            Ok(())
        })
        .unwrap();
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        assert!(
            views.iter().all(|view| view.to_local(scope).is_none()),
            "a retained page event must not root its foreign Event, destination, or signal wrappers"
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval("String(keptEvent.destination.url.endsWith('#retained') && keptEvent.signal instanceof AbortSignal)").unwrap(), "true");
}

#[test]
fn history_worlds_navigate_signal_has_a_local_wrapper() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(
        r#"
        navigation.addEventListener('navigate', event => {
            event.signal.pageOnly = 'page';
            event.signal.addEventListener = () => { throw new Error('page override'); };
        }, {once: true});
        'ready'
    "#,
    )
    .unwrap();
    let isolated = vm.create_isolated_world("signal-view", false).unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r#"
        globalThis.facts = [];
        navigation.addEventListener('navigate', event => {
            facts.push(event instanceof NavigateEvent,
                event.signal instanceof AbortSignal,
                Object.getPrototypeOf(event.signal) === AbortSignal.prototype,
                event.signal.pageOnly === undefined);
            try { event.signal.addEventListener('abort', () => {}); facts.push(true); }
            catch (_) { facts.push(false); }
        }, {once: true});
        history.pushState({}, '', '#signal');
        JSON.stringify(facts)
    "#
        )
        .unwrap(),
        "[true,true,true,true,true]"
    );
}

#[test]
fn history_worlds_synthetic_events_keep_interface_identity_and_shared_dispatch() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(
        r#"
        globalThis.pageFacts = [];
        globalThis.pageEvents = [];
        for (const type of ['probe', 'navigate']) {
            navigation.addEventListener(type, function (event) {
                pageEvents.push(event);
                pageFacts.push([event instanceof Event, event.expando === undefined,
                    event.target === navigation, event.currentTarget === navigation,
                    this === navigation, event instanceof CustomEvent,
                    event instanceof NavigateEvent, event.detail ?? null]);
                event.preventDefault();
            });
        }
        'ready'
    "#,
    )
    .unwrap();
    let isolated = vm.create_isolated_world("synthetic-views", false).unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r#"
        const facts = [];
        for (const original of [new CustomEvent('probe', {detail: 7, cancelable: true}),
                                new Event('navigate', {cancelable: true})]) {
            original.expando = 'isolated';
            navigation.addEventListener(original.type, event => {
                facts.push(event === original, event.defaultPrevented,
                    event.target === navigation, event.composedPath()[0] === navigation);
            }, {once: true});
            facts.push(navigation.dispatchEvent(original), original.defaultPrevented,
                original.currentTarget === null, original.composedPath().length === 0);
        }
        JSON.stringify(facts)
    "#
        )
        .unwrap(),
        "[true,true,true,true,false,true,true,true,true,true,true,true,false,true,true,true]"
    );
    assert_eq!(
        vm.eval("JSON.stringify(pageFacts)").unwrap(),
        "[[true,true,true,true,true,true,false,7],[true,true,true,true,true,false,false,null]]"
    );
    assert_eq!(vm.eval("String(pageEvents.every(event => event.defaultPrevented && event.currentTarget === null))").unwrap(), "true");
}

#[test]
fn history_worlds_nested_platform_objects_share_state_without_sharing_wrappers() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(r#"
        globalThis.destination = null;
        navigation.addEventListener('navigate', event => { destination = event.destination; }, {once: true});
        history.pushState({}, '', '#destination');
        globalThis.controller = new AbortController();
        globalThis.data = new FormData();
        data.set('value', 'page');
        const file = new File(['bytes'], 'probe.txt', {type: 'text/plain', lastModified: 7});
        file.pageOnly = true;
        data.set('file', file);
        globalThis.source = document.createElement('button');
        source.id = 'source';
        if (!document.documentElement) document.appendChild(document.createElement('html'));
        if (!document.body) document.documentElement.appendChild(document.createElement('body'));
        document.body.appendChild(source);
        globalThis.original = new NavigateEvent('probe', {
            destination, signal: controller.signal, formData: data, sourceElement: source,
        });
        navigation.addEventListener('probe', event => {
            event.signal.pageOnly = true;
            event.signal.addEventListener = () => { throw new Error('page signal override'); };
            event.formData.pageOnly = true;
            event.formData.get = () => 'page override';
            event.sourceElement.pageOnly = true;
            event.sourceElement.getAttribute = () => 'page override';
        });
        'ready'
    "#).unwrap();
    let isolated = vm.create_isolated_world("platform-views", false).unwrap();
    vm.eval_in_isolated_context(
        isolated,
        r#"
        globalThis.facts = [];
        globalThis.abortFacts = [];
        navigation.addEventListener('probe', event => {
            globalThis.savedSignal = event.signal;
            globalThis.savedData = event.formData;
            const file = event.formData.get('file');
            facts.push(event instanceof NavigateEvent,
                event.signal instanceof AbortSignal, event.signal.pageOnly === undefined,
                event.formData instanceof FormData, event.formData.pageOnly === undefined,
                event.formData.get('value') === 'page',
                event.sourceElement instanceof HTMLButtonElement,
                event.sourceElement === document.getElementById('source'),
                event.sourceElement.pageOnly === undefined,
                event.sourceElement.getAttribute('id') === 'source',
                file instanceof File, file.pageOnly === undefined,
                file === event.formData.get('file'), file.name === 'probe.txt',
                file.size === 5, file.type === 'text/plain', file.lastModified === 7);
            event.formData.set('value', 'isolated');
            event.signal.addEventListener('abort', function (abort) {
                abortFacts.push(this === savedSignal, abort.target === savedSignal,
                    abort instanceof Event, savedSignal.aborted, savedSignal.reason === 'stop');
            }, {once: true});
        });
        'ready'
    "#,
    )
    .unwrap();
    vm.eval("navigation.dispatchEvent(original); controller.abort('stop'); 'dispatched'")
        .unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "JSON.stringify(facts)")
            .unwrap(),
        "[true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true]"
    );
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "JSON.stringify(abortFacts)")
            .unwrap(),
        "[true,true,true,true,true]"
    );
    assert_eq!(
        vm.eval("FormData.prototype.get.call(data, 'value')")
            .unwrap(),
        "isolated"
    );
    vm.eval("FormData.prototype.set.call(data, 'value', 'updated'); 'updated'")
        .unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "savedData.get('value')")
            .unwrap(),
        "updated"
    );
}

#[test]
fn history_worlds_synthetic_event_views_can_be_reinitialized_and_redispatched() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(
        r#"
        globalThis.original = new CustomEvent('probe', {detail: 1, cancelable: true});
        original.expando = 'page';
        globalThis.pageFacts = [];
        navigation.addEventListener('probe', event => {
            pageFacts.push(event === original, event.target === navigation);
        });
        navigation.addEventListener('reused', event => {
            pageFacts.push(event === original, event.detail === 9, event.expando === 'page',
                event.target === navigation, event.composedPath()[0] === navigation);
            event.preventDefault();
        });
        'ready'
    "#,
    )
    .unwrap();
    let isolated = vm.create_isolated_world("redispatch-views", false).unwrap();
    vm.eval_in_isolated_context(
        isolated,
        r#"
        globalThis.view = null;
        globalThis.facts = [];
        navigation.addEventListener('probe', event => {
            view = event;
            facts.push(event instanceof CustomEvent, event.expando === undefined,
                event.target === navigation, event.detail === 1);
        });
        'ready'
    "#,
    )
    .unwrap();
    vm.eval("navigation.dispatchEvent(original); 'dispatched'")
        .unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r#"
        view.initCustomEvent('reused', false, true, 9);
        view.expando = 'isolated';
        navigation.addEventListener('reused', event => {
            facts.push(event === view, event.defaultPrevented,
                event.target === navigation, event.detail === 9);
        }, {once: true});
        facts.push(navigation.dispatchEvent(view));
        JSON.stringify(facts)
    "#
        )
        .unwrap(),
        "[true,true,true,true,true,true,true,true,false]"
    );
    assert_eq!(
        vm.eval("JSON.stringify(pageFacts)").unwrap(),
        "[true,true,true,true,true,true,true]"
    );
}

#[test]
fn history_worlds_navigation_entries_keep_their_own_wrappers() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(
        r#"
        navigation.updateCurrentEntry({state: {value: 7, map: new Map([['x', 2n]])}});
        navigation.currentEntry.expando = 'page';
        navigation.currentEntry.getState = () => 'page-override';
        'ready'
    "#,
    )
    .unwrap();
    let isolated = vm.create_isolated_world("entries", false).unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r#"JSON.stringify([
        navigation.currentEntry instanceof NavigationHistoryEntry,
        navigation.currentEntry.expando === undefined,
        navigation.currentEntry.getState().value,
        navigation.currentEntry.getState().map instanceof Map,
        navigation.currentEntry.getState().map.get('x') === 2n,
        navigation.entries().includes(navigation.currentEntry),
        navigation.currentEntry.index === navigation.entries().length - 1,
        navigation.currentEntry === navigation.currentEntry
    ])"#
        )
        .unwrap(),
        "[true,true,7,true,true,true,true,true]"
    );
}

#[test]
fn history_worlds_navigation_listeners_share_notifications_and_cancellation() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(
        r#"
        globalThis.changes = 0;
        globalThis.navigates = 0;
        navigation.addEventListener('currententrychange', () => changes++);
        navigation.addEventListener('navigate', () => navigates++);
        'ready'
    "#,
    )
    .unwrap();
    let isolated = vm.create_isolated_world("events", false).unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r##"
        globalThis.changes = 0;
        navigation.addEventListener('currententrychange', () => changes++);
        history.pushState({value: 1}, '', '#next');
        String(changes)
    "##
        )
        .unwrap(),
        "1"
    );
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r##"
        const before = location.href;
        const length = history.length;
        let seen = 0;
        navigation.addEventListener('navigate', event => {
            seen++;
            event.preventDefault();
        }, {once: true});
        history.pushState({}, '', '#should-not-commit');
        JSON.stringify([seen, location.href === before, history.length === length, changes])
    "##
        )
        .unwrap(),
        "[1,true,true,1]"
    );
    assert_eq!(
        vm.eval("JSON.stringify([changes, navigates])").unwrap(),
        "[1,2]"
    );
    vm.eval("history.pushState({}, '', '#page'); 'pushed'")
        .unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "String(changes)")
            .unwrap(),
        "2"
    );
}

#[test]
fn history_worlds_security_errors_belong_to_the_binding_realm() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    let isolated = vm.create_isolated_world("errors", false).unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r#"JSON.stringify(
        ['pushState', 'replaceState'].flatMap(method => {
            try { history[method]({}, '', 'https://other.test/'); }
            catch (error) { return [error.name, error instanceof DOMException,
                Object.getPrototypeOf(error) === DOMException.prototype]; }
            return ['did not throw'];
        })
    )"#
        )
        .unwrap(),
        r#"["SecurityError",true,true,"SecurityError",true,true]"#
    );
}

#[test]
fn history_worlds_navigation_events_use_local_views_and_one_dispatch_state() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(
        r#"
        globalThis.pageEvents = [];
        globalThis.pageEntry = navigation.currentEntry;
        navigation.addEventListener('navigate', function (event) {
            pageEvents.push([this === navigation, event.target === navigation,
                event instanceof NavigateEvent, event.defaultPrevented]);
            event.expando = 'page';
            event.preventDefault = () => { throw new Error('page override'); };
            event.destination.getState = () => 'page override';
        });
        'ready'
    "#,
    )
    .unwrap();
    let isolated = vm.create_isolated_world("event-views", false).unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r##"
        globalThis.initialEntry = navigation.currentEntry;
        globalThis.savedEvent = null;
        globalThis.seen = [];
        navigation.addEventListener('navigate', function (event) {
            savedEvent = event;
            try {
            seen.push([this === navigation, event.target === navigation,
                event.currentTarget === navigation, event instanceof NavigateEvent,
                event.composedPath()[0] === navigation, event.expando === undefined,
                Object.getPrototypeOf(event.destination.getState) === Function.prototype,
                Object.getPrototypeOf(event.destination.getState()) === Object.prototype]);
            } catch (error) { seen.push(String(error)); }
            event.preventDefault();
        }, {once: true});
        history.pushState({value: 1}, '', '#cancelled');
        JSON.stringify([seen, savedEvent.defaultPrevented, savedEvent.currentTarget,
            savedEvent.eventPhase, savedEvent.composedPath().length,
            location.hash, navigation.currentEntry === initialEntry])
    "##
        )
        .unwrap(),
        r#"[[[true,true,true,true,true,true,true,true]],true,null,0,0,"",true]"#
    );
    assert_eq!(
        vm.eval("JSON.stringify(pageEvents)").unwrap(),
        "[[true,true,true,false]]"
    );
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r##"
        globalThis.from = null;
        navigation.oncurrententrychange = function (event) {
            from = event.from;
            seen.push([this === navigation, event.currentTarget === navigation,
                event instanceof NavigationCurrentEntryChangeEvent,
                event.from instanceof NavigationHistoryEntry,
                event.from === initialEntry]);
        };
        history.pushState({}, '', '#committed');
        JSON.stringify(seen[1])
    "##
        )
        .unwrap(),
        "[true,true,true,true,true]"
    );
}

#[test]
fn history_worlds_navigation_handlers_and_dispose_listeners_share_target_identity() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(
        r#"
        globalThis.pageCalls = 0;
        globalThis.pageDisposed = 0;
        navigation.oncurrententrychange = () => pageCalls++;
        navigation.currentEntry.ondispose = () => pageDisposed++;
        'ready'
    "#,
    )
    .unwrap();
    let isolated = vm.create_isolated_world("handlers", false).unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r##"
        const entry = navigation.currentEntry;
        let disposed = 0;
        let calls = 0;
        let handler = () => calls++;
        navigation.oncurrententrychange = handler;
        entry.addEventListener('dispose', function (event) {
            if (this === entry && event.target === entry && event instanceof Event) disposed++;
        }, {once: true});
        entry.ondispose = () => disposed++;
        history.replaceState({}, '', '#replaced');
        navigation.oncurrententrychange = null;
        history.pushState({}, '', '#pushed');
        JSON.stringify([calls, disposed, navigation.oncurrententrychange, entry.index])
    "##
        )
        .unwrap(),
        "[1,2,null,-1]"
    );
    assert_eq!(
        vm.eval("JSON.stringify([pageCalls, pageDisposed])")
            .unwrap(),
        "[2,1]"
    );
}

#[test]
fn history_worlds_navigation_results_and_transition_use_the_callers_wrappers() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    let isolated = vm.create_isolated_world("results", false).unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r##"
        globalThis.facts = [];
        const from = navigation.currentEntry;
        navigation.addEventListener('navigate', event => {
            event.intercept({handler() {
                const transition = navigation.transition;
                facts.push([transition instanceof NavigationTransition,
                    transition.from === from, transition.to === event.destination,
                    transition.committed instanceof Promise,
                    transition.finished instanceof Promise, this === event]);
            }});
        }, {once: true});
        const result = navigation.navigate('#next');
        result.committed.then(entry => facts.push(['committed',
            entry instanceof NavigationHistoryEntry, entry === navigation.currentEntry]));
        result.finished.then(entry => facts.push(['finished',
            entry instanceof NavigationHistoryEntry, entry === navigation.currentEntry]));
        String(result.committed instanceof Promise && result.finished instanceof Promise)
    "##
        )
        .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "JSON.stringify(facts)")
            .unwrap(),
        r#"[[true,true,true,true,true,true],["committed",true,true],["finished",true,true]]"#
    );
}

#[test]
fn history_worlds_navigation_listener_order_cancellation_and_event_reuse_are_shared() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(
        r#"
        globalThis.order = [];
        navigation.addEventListener('navigate', event => {
            order.push('first:' + event.defaultPrevented);
        });
        'ready'
    "#,
    )
    .unwrap();
    let isolated = vm.create_isolated_world("ordered-dispatch", false).unwrap();
    vm.eval_in_isolated_context(
        isolated,
        r#"
        globalThis.saved = null;
        navigation.addEventListener('navigate', event => {
            saved = event;
            event.preventDefault();
            event.stopImmediatePropagation();
        }, {once: true});
        'ready'
    "#,
    )
    .unwrap();
    vm.eval(r#"
        navigation.addEventListener('navigate', event => order.push('last:' + event.defaultPrevented));
        history.pushState({}, '', '#blocked');
        'pushed'
    "#).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([order, location.hash])").unwrap(),
        r#"[["first:false"],""]"#
    );
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r#"
        const trusted = saved.isTrusted;
        saved.initEvent('probe', false, true);
        let calls = 0;
        navigation.addEventListener('probe', event => {
            if (event === saved && event.target === navigation && !event.isTrusted) calls++;
            event.preventDefault();
        });
        const result = navigation.dispatchEvent(saved);
        JSON.stringify([trusted, calls, result, saved.defaultPrevented,
            saved.currentTarget, saved.eventPhase, saved.composedPath().length])
    "#
        )
        .unwrap(),
        "[true,1,false,true,null,0,0]"
    );
    vm.eval("history.pushState({}, '', '#allowed'); 'pushed'")
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([order, location.hash])").unwrap(),
        r##"[["first:false","first:false","last:false"],"#allowed"]"##
    );
}

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
        r##"[1,true,true,true,3,true,true,"manual","#one",true,true]"##
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
        frame.srcdoc = '<p>History owner</p>';
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
    // Traversal needs a committed Document: pushState replaces the entry of
    // the initial empty Document rather than appending one.
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
        r##"
        history.replaceState({owner: 'child'}, '', 'about:srcdoc#child');
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
        history.pushState({owner: 'isolated-child'}, '', 'about:srcdoc#next');
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
        "child:manual:#child"
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
        // Keep the initial empty Window pending until the srcdoc commit.
        frame.srcdoc = '<p>pending initial load</p>';
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
        originalHistory.replaceState({rebound: true}, '', 'about:srcdoc#rebound');
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

#[test]
fn history_native_snapshot_survives_isolate_replacement_with_structured_values() {
    let seed = {
        let mut vm = new_storage_test_vm("https://example.com/base");
        vm.eval(r##"
            globalThis.jsonHooks = 0;
            Object.defineProperty(Object.prototype, 'toJSON', {configurable: true, get() {
                jsonHooks++;
                throw new Error('history must not call toJSON');
            }});
            const state = {map: new Map([['answer', 42n]]), bytes: new Uint8Array([3, 4]), missing: undefined, zero: -0, blob: new Blob(['native'], {type: 'text/plain'})};
            state.self = state;
            history.replaceState(state, '', '#saved');
            history.scrollRestoration = 'manual';
            navigation.updateCurrentEntry({state: new Set([7n])});
            history.state.map.set('answer', 99n);
            navigation.navigate('/next', {history: 'push'});
            'queued'
        "##).unwrap();
        let pending = vm.take_pending_location_navigation_with_seed().unwrap();
        assert_eq!(vm.eval("String(jsonHooks)").unwrap(), "0");
        let mut seed = pending.entry_seed.unwrap();
        seed.current_index = seed
            .entries
            .iter()
            .find(|entry| entry.url.ends_with("#saved"))
            .unwrap()
            .history_index;
        seed.activation = None;
        seed
    };
    // The source VM (including its isolate and all JS objects) is gone.
    let mut restored = new_storage_test_vm("https://example.com/base#saved");
    restored.install_navigation_bootstrap_entry(Some(seed));
    let isolated = restored
        .create_isolated_world("restored-history", false)
        .unwrap();
    const READ: &str = r#"JSON.stringify([
        history.state.map instanceof Map,
        history.state.map.get('answer') === 42n,
        history.state.bytes instanceof Uint8Array,
        history.state.bytes[1] === 4,
        'missing' in history.state && history.state.missing === undefined,
        Object.is(history.state.zero, -0),
        history.state.self === history.state,
        history.scrollRestoration,
        navigation.currentEntry.getState().has(7n),
        history.state.blob instanceof Blob && history.state.blob.size === 6 && history.state.blob.type === 'text/plain'
    ])"#;
    assert_eq!(
        restored.eval(READ).unwrap(),
        r#"[true,true,true,true,true,true,true,"manual",true,true]"#
    );
    assert_eq!(
        restored.eval_in_isolated_context(isolated, READ).unwrap(),
        r#"[true,true,true,true,true,true,true,"manual",true,true]"#
    );
}

#[test]
fn history_restored_sparse_entries_keep_world_identity_and_refresh_reused_ids() {
    const ENTRIES: &str = r#"JSON.stringify(navigation.entries()
        .filter(entry => entry.url.includes('#'))
        .map(entry => [entry.url, entry.id, entry.key, entry.index, entry.sameDocument,
            entry instanceof NavigationHistoryEntry, entry.getState() instanceof Map,
            String(entry.getState().get('value'))]))"#;
    let (mut seed, expected) = {
        let mut vm = new_storage_test_vm("https://example.com/base");
        vm.eval(
            r##"
            history.replaceState({value: 1n}, '', '#one');
            history.scrollRestoration = 'manual';
            navigation.updateCurrentEntry({state: new Map([['value', 1n]])});
            history.pushState({value: 2n}, '', '#two');
            navigation.updateCurrentEntry({state: new Map([['value', 2n]])});
            'ready'
        "##,
        )
        .unwrap();
        let expected = vm.eval(ENTRIES).unwrap();
        vm.eval("navigation.navigate('/next', {history: 'push'}); 'queued'")
            .unwrap();
        let mut seed = vm
            .take_pending_location_navigation_with_seed()
            .unwrap()
            .entry_seed
            .unwrap();
        seed.current_index = seed
            .entries
            .iter()
            .find(|entry| entry.url.ends_with("#one"))
            .unwrap()
            .history_index;
        seed.activation = None;
        // Snapshot positions may be sparse and do not follow serialization order.
        // The current serialized position still fits in the dense vector, where
        // using it as an offset would silently select the wrong entry.
        seed.current_index *= 3;
        for entry in &mut seed.entries {
            entry.history_index *= 3;
        }
        seed.entries.reverse();
        (seed, expected)
    };
    let mut restored = new_storage_test_vm("https://example.com/base#one");
    restored.install_navigation_bootstrap_entry(Some(seed.clone()));
    let isolated = restored
        .create_isolated_world("lazy-history-entries", false)
        .unwrap();
    // The first observation of non-current entries is in the isolated world.
    assert_eq!(
        restored
            .eval_in_isolated_context(isolated, ENTRIES)
            .unwrap(),
        expected
    );
    restored
        .eval_in_isolated_context(
            isolated,
            r#"
        navigation.entries().find(entry => entry.url.endsWith('#two')).worldOnly = 'isolated';
        globalThis.retained = navigation.currentEntry;
        'observed'
    "#,
        )
        .unwrap();
    assert_eq!(restored.eval(ENTRIES).unwrap(), expected);
    assert_eq!(
        restored
            .eval(r#"String(navigation.entries().every(entry => entry.worldOnly === undefined))"#)
            .unwrap(),
        "true"
    );
    restored
        .eval("globalThis.retained = navigation.currentEntry; 'retained'")
        .unwrap();
    let replacement = seed
        .entries
        .iter()
        .find(|entry| entry.url.ends_with("#two"))
        .unwrap()
        .clone();
    let current = seed
        .entries
        .iter_mut()
        .find(|entry| entry.url.ends_with("#one"))
        .unwrap();
    current.history_state = replacement.history_state;
    current.navigation_state = replacement.navigation_state;
    current.scroll_restoration = moli_history::ScrollRestoration::Auto;
    // Reusing serialized IDs must not return wrappers for the previous native records.
    restored.install_navigation_bootstrap_entry(Some(seed));
    const REFRESHED: &str = r#"JSON.stringify([
        navigation.currentEntry !== retained,
        navigation.currentEntry.id === retained.id,
        navigation.currentEntry instanceof NavigationHistoryEntry,
        navigation.entries().includes(navigation.currentEntry),
        history.state.value === 2n,
        navigation.currentEntry.getState().get('value') === 2n,
        retained.getState().get('value') === 1n,
        history.scrollRestoration
    ])"#;
    for result in [
        restored.eval(REFRESHED),
        restored.eval_in_isolated_context(isolated, REFRESHED),
    ] {
        assert_eq!(
            result.unwrap(),
            r#"[true,true,true,true,true,true,true,"auto"]"#
        );
    }
}

#[test]
fn history_native_fragment_navigation_preserves_structured_navigation_state() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(
        r##"
        history.replaceState({value: 5n}, '');
        navigation.updateCurrentEntry({state: new Map([['value', 9n]])});
        globalThis.retained = navigation.currentEntry;
        location.hash = '#next';
        'changed'
    "##,
    )
    .unwrap();
    assert_eq!(vm.eval("String(history.state === null && navigation.currentEntry.getState().get('value') === 9n && retained.getState().get('value') === 9n)").unwrap(), "true");
}

#[test]
fn event_worlds_beforeunload_return_value_uses_shared_backing() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(
        r#"
        globalThis.event = document.createEvent('BeforeUnloadEvent');
        event.initEvent('beforeunload', false, true);
        event.returnValue = 'original';
        event.pageOnly = true;
        'ready'
    "#,
    )
    .unwrap();
    let isolated = vm
        .create_isolated_world("beforeunload-state", false)
        .unwrap();
    vm.eval_in_isolated_context(
        isolated,
        r#"
        globalThis.facts = [];
        navigation.addEventListener('beforeunload', event => {
            facts.push(event instanceof BeforeUnloadEvent, event.returnValue,
                event.pageOnly === undefined);
            event.returnValue = 'updated';
            event.preventDefault();
        }, {once: true});
        'ready'
    "#,
    )
    .unwrap();
    assert_eq!(vm.eval("JSON.stringify([navigation.dispatchEvent(event), event.returnValue, event.defaultPrevented])").unwrap(),
        r#"[false,"updated",true]"#);
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "JSON.stringify(facts)")
            .unwrap(),
        r#"[true,"original",true]"#
    );
}

#[tokio::test]
async fn event_worlds_precommit_controllers_expire_after_navigation_settles() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);
    let isolated = vm.create_isolated_world("precommit-state", false).unwrap();
    vm.eval_in_isolated_context(
        isolated,
        r##"
        globalThis.handlers = 0;
        navigation.addEventListener('navigate', event => {
            event.intercept({precommitHandler(controller) {
                globalThis.savedController = controller;
                controller.addHandler(() => { handlers++; });
            }});
        }, {once: true});
        'ready'
    "##,
    )
    .unwrap();
    vm.eval("navigation.navigate('#next'); 'started'").unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r#"
        (() => {
            let errorName = '';
            try { savedController.addHandler(() => {}); }
            catch (error) { errorName = error.name; }
            return JSON.stringify([location.hash, handlers, errorName]);
        })()
    "#
        )
        .unwrap(),
        r##"["#next",1,"InvalidStateError"]"##
    );
}
