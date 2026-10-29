use super::*;

#[test]
fn event_target_interfaces_inherit_canonical_operations() {
    let mut vm = new_storage_html_test_vm("https://event-target-inheritance.test/");
    vm.eval(include_str!(
        "../../../../tests/fixtures/event-target-inheritance.js"
    ))
    .expect("interface inheritance matrix should run");
    assert_eq!(vm.eval("String(!__eventTargetInheritance.pending && __eventTargetInheritance.rows.every(row => row[1]))").unwrap(), "true");
    assert_eq!(vm.eval("String(__eventTargetInheritance.rows.some(row => row[0] === 'battery: native inheritance'))").unwrap(), "true");
    vm.eval(
        r#"(() => {
        const popup = open('about:blank', 'inherited-event-target');
        try { __eventTargetInheritance.inspect(popup, 'lightweight popup'); }
        finally { popup.close(); }
    })()"#,
    )
    .expect("popup should inherit EventTarget operations");
}

#[test]
fn event_target_prototype_methods_preserve_native_state_and_realm_semantics() {
    let mut vm = new_parsed_test_vm(
        "https://event-target-prototype.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/event-target-prototype.js"
        ))
        .expect("EventTarget prototype matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "158");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn event_target_prototype_methods_accept_registered_native_proxies() {
    let mut vm = new_storage_html_test_vm("https://event-target-native-proxy.test/");
    vm.eval(
        r#"
        globalThis.nativeTarget = new EventTarget();
        globalThis.nativeTargetTraps = 0;
        globalThis.nativeTargetProxy = new Proxy(nativeTarget, {
          get() {nativeTargetTraps++; throw new Error('get trap');},
          getPrototypeOf() {nativeTargetTraps++; throw new Error('prototype trap');}
        });
        true;
        "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "nativeTargetProxy").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let proxy = v8::Local::<v8::Proxy>::try_from(value).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert!(crate::web_api_interfaces::EventTarget::is_instance(
            scope,
            proxy.into()
        ));
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
              const {addEventListener: add, removeEventListener: remove, dispatchEvent: dispatch} = EventTarget.prototype;
              let calls = 0;
              function listener(event) {
                if (this === nativeTargetProxy && event.target === nativeTargetProxy && event.currentTarget === nativeTargetProxy) calls++;
              }
              add.call(nativeTargetProxy, 'x', listener);
              if (!dispatch.call(nativeTargetProxy, new Event('x')) || calls !== 1) return false;
              remove.call(nativeTargetProxy, 'x', listener);
              dispatch.call(nativeTargetProxy, new Event('x'));
              if (calls !== 1 || nativeTargetTraps !== 0) return false;
              const author = new Proxy(nativeTargetProxy, {});
              let conversions = 0;
              const type = {toString() {conversions++; return 'x';}};
              for (const call of [() => add.call(author, type, null), () => remove.call(author, type, null), () => dispatch.call(author, new Event('x'))]) {
                try { call(); return false; } catch (error) { if (!(error instanceof TypeError)) return false; }
              }
              return conversions === 0 && nativeTargetTraps === 0;
            })()"#,
        )
        .unwrap(),
        "true",
    );
}

#[test]
fn event_target_prototype_methods_separate_listener_owner_from_dispatch_target() {
    let mut vm = new_storage_html_test_vm("https://event-target-dispatch-identity.test/");
    vm.eval(
        r#"
        globalThis.listenerOwner = new EventTarget();
        globalThis.dispatchTarget = new EventTarget();
        globalThis.identityTraps = 0;
        globalThis.nativeIdentityProxy = new Proxy(listenerOwner, {
            get() {identityTraps++; throw new Error('get trap');},
            getPrototypeOf() {identityTraps++; throw new Error('prototype trap');}
        });
        true;
    "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let owner = global
            .get(scope, crate::util::v8str(scope, "listenerOwner").into())
            .unwrap();
        let owner = v8::Local::<v8::Object>::try_from(owner).unwrap();
        let identity = global
            .get(scope, crate::util::v8str(scope, "dispatchTarget").into())
            .unwrap();
        let identity = v8::Local::<v8::Object>::try_from(identity).unwrap();
        let proxy = global
            .get(
                scope,
                crate::util::v8str(scope, "nativeIdentityProxy").into(),
            )
            .unwrap();
        let proxy = v8::Local::<v8::Proxy>::try_from(proxy).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        crate::context_bootstrap::install_simple_event_target_handlers(scope, owner, &["onprobe"]);
        crate::context_bootstrap::bind_dispatch_target(scope, owner, identity);
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const assert = (ok, label) => {if (!ok) throw Error(label);};
        const {addEventListener: add, removeEventListener: remove, dispatchEvent: dispatch} = EventTarget.prototype;
        const handler = Object.getOwnPropertyDescriptor(listenerOwner, 'onprobe');
        const calls = [];
        let otherCalls = 0;
        dispatchTarget.addEventListener('probe', () => otherCalls++);
        const observe = (label, receiver, event) => {
            calls.push([label, receiver === dispatchTarget, event.target === dispatchTarget,
                event.currentTarget === dispatchTarget, event.composedPath()[0] === dispatchTarget]);
            event.preventDefault();
        };
        function listener(event) {observe('listener', this, event);}
        const onprobe = function(event) {observe('handler', this, event);};
        const object = {handleEvent(event) {
            observe(this === object ? 'object' : 'wrong-this', event.currentTarget, event);
        }};
        add.call(nativeIdentityProxy, 'probe', listener);
        add.call(listenerOwner, 'probe', listener);
        handler.set.call(nativeIdentityProxy, onprobe);
        add.call(listenerOwner, 'probe', object, {once: true});
        assert(handler.get.call(listenerOwner) === onprobe &&
            handler.get.call(nativeIdentityProxy) === onprobe &&
            handler.get.call(dispatchTarget) === null, 'handler state stays on native receiver');
        const event = new Event('probe', {cancelable: true});
        assert(dispatch.call(nativeIdentityProxy, event) === false, 'cancellation');
        assert(JSON.stringify(calls) === JSON.stringify(['listener','handler','object']
            .map(label => [label,true,true,true,true])), 'listener owner and public identity');
        assert(otherCalls === 0 && event.target === dispatchTarget && event.currentTarget === null &&
            event.composedPath().length === 0, 'dispatch does not use public target registry');
        remove.call(nativeIdentityProxy, 'probe', listener);
        handler.set.call(listenerOwner, null);
        calls.length = 0;
        assert(dispatch.call(listenerOwner, new Event('probe')) && calls.length === 0,
            'once, remove and handler cleanup use listener owner');
        dispatchTarget.dispatchEvent(new Event('probe'));
        assert(otherCalls === 1, 'other registry is untouched');
        const controller = new AbortController();
        add.call(nativeIdentityProxy, 'probe', listener, {signal: controller.signal});
        controller.abort();
        dispatch.call(listenerOwner, new Event('probe'));
        assert(calls.length === 0, 'abort cleanup uses listener owner');
        globalThis.nativeIdentityCalls = 0;
        add.call(nativeIdentityProxy, 'native-probe', function(event) {
            if (this === dispatchTarget && event.target === dispatchTarget &&
                event.currentTarget === dispatchTarget && event.isTrusted) nativeIdentityCalls++;
        }, {once: true});
        return identityTraps === 0;
    })()"#).unwrap(), "true");
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let proxy = global
            .get(
                scope,
                crate::util::v8str(scope, "nativeIdentityProxy").into(),
            )
            .unwrap();
        let proxy = v8::Local::<v8::Object>::try_from(proxy).unwrap();
        let event =
            crate::context_bootstrap::construct_original_event(scope, "native-probe").unwrap();
        assert!(
            crate::context_bootstrap::dispatch_simple_event_target_event(
                scope,
                proxy,
                "native-probe",
                event,
            )
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([nativeIdentityCalls, identityTraps])")
            .unwrap(),
        "[1,0]"
    );
}

#[test]
fn event_target_native_aliases_preserve_shared_world_targets_and_handler_state() {
    let mut vm = new_storage_test_vm("https://event-target-native-worlds.test/");
    vm.eval(
        r#"
        navigation.addEventListener('navigate', event => {
            globalThis.mainSignal = event.signal;
        }, {once: true});
    "#,
    )
    .unwrap();
    let isolated = vm
        .create_isolated_world("native-target-identities", false)
        .unwrap();
    vm.eval_in_isolated_context(
        isolated,
        r#"
        navigation.addEventListener('navigate', event => {
            globalThis.worldSignal = event.signal;
        }, {once: true});
    "#,
    )
    .unwrap();
    vm.eval(
        r#"
        history.pushState({}, '', '#capture-signal');
        globalThis.aliasTraps = 0;
        const makeAlias = target => new Proxy(target, {
            get() {aliasTraps++; throw new Error('get trap');},
            getPrototypeOf() {aliasTraps++; throw new Error('prototype trap');}
        });
        globalThis.navigationAlias = makeAlias(navigation);
        globalThis.signalAlias = makeAlias(mainSignal);
        globalThis.mainEntry = navigation.currentEntry;
        globalThis.entryAlias = makeAlias(mainEntry);
        true;
    "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        for name in ["navigationAlias", "signalAlias", "entryAlias"] {
            let value = global
                .get(scope, crate::util::v8str(scope, name).into())
                .unwrap();
            let proxy = v8::Local::<v8::Proxy>::try_from(value).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        }
        Ok(())
    })
    .unwrap();
    vm.eval(r#"
        globalThis.mainFacts = [];
        globalThis.mainSpecs = [
            [navigation, navigationAlias, 'navigate', 'onnavigate',
                Object.getOwnPropertyDescriptor(navigation, 'onnavigate')],
            [mainSignal, signalAlias, 'abort', 'onabort',
                Object.getOwnPropertyDescriptor(AbortSignal.prototype, 'onabort')],
            [mainEntry, entryAlias, 'dispose', 'ondispose',
                Object.getOwnPropertyDescriptor(mainEntry, 'ondispose')],
        ];
        globalThis.mainHandlers = mainSpecs.map(([target, alias, type, slot, descriptor]) => {
            const handler = function(event) {
                mainFacts.push([this === target, event.target === target,
                    event.currentTarget === target, event instanceof CustomEvent, event.pageOnly === 42]);
            };
            descriptor.set.call(alias, handler);
            if (descriptor.get.call(alias) !== handler || descriptor.get.call(target) !== handler)
                throw Error(type + ': alias handler owner');
            return handler;
        });
        true;
    "#).unwrap();
    vm.eval_in_isolated_context(isolated, r#"
        globalThis.worldFacts = [];
        globalThis.worldEvents = [];
        globalThis.worldEntry = navigation.currentEntry;
        globalThis.worldSpecs = [[navigation, 'navigate', 'onnavigate'],
            [worldSignal, 'abort', 'onabort'], [worldEntry, 'dispose', 'ondispose']];
        globalThis.worldHandlers = worldSpecs.map(([target, type, slot]) => {
            const record = function(event) {
                worldEvents.push(event);
                worldFacts.push([this === target, event.target === target, event.currentTarget === target,
                    event.composedPath()[0] === target, event instanceof CustomEvent,
                    event.pageOnly === undefined, event.detail === 7]);
            };
            target[slot] = record;
            target.addEventListener(type, function(event) {
                record.call(this, event);
                event.preventDefault();
            }, {once: true});
            return record;
        });
        true;
    "#).unwrap();
    assert_eq!(vm.eval(r#"JSON.stringify(mainSpecs.map(([target, alias, type]) => {
        const event = new CustomEvent(type, {detail: 7, cancelable: true});
        event.pageOnly = 42;
        return [EventTarget.prototype.dispatchEvent.call(alias, event), event.defaultPrevented,
            event.target === target, event.currentTarget === null, event.composedPath().length === 0];
    }))"#).unwrap(), "[[false,true,true,true,true],[false,true,true,true,true],[false,true,true,true,true]]");
    assert_eq!(vm.eval_in_isolated_context(isolated, r#"String(
        worldFacts.length === 6 && worldFacts.every(row => row.every(Boolean)) &&
        worldEvents.every(event => event.defaultPrevented && event.currentTarget === null &&
            event.composedPath().length === 0) && worldSpecs.every(([target, type, slot], index) => {
                if (target[slot] !== worldHandlers[index]) return false;
                target[slot] = null;
                return target[slot] === null;
            })
    )"#).unwrap(), "true");
    assert_eq!(vm.eval(r#"String(mainSpecs.every(([target, alias, type, slot, descriptor], index) => {
        if (descriptor.get.call(alias) !== mainHandlers[index] || target[slot] !== mainHandlers[index])
            return false;
        const event = new CustomEvent(type, {detail: 7, cancelable: true});
        event.pageOnly = 42;
        return EventTarget.prototype.dispatchEvent.call(alias, event) && !event.defaultPrevented;
    }) && mainFacts.length === 6 && mainFacts.every(row => row.every(Boolean)) && aliasTraps === 0)"#).unwrap(), "true");
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "String(worldFacts.length === 6)")
            .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
        if (!document.documentElement) document.appendChild(document.createElement('html'));
        if (!document.body) document.documentElement.appendChild(document.createElement('body'));
        const frame = document.body.appendChild(document.createElement('iframe'));
        const facts = [];
        const listener = frame.contentWindow.Function('expected', 'facts', `return function(event) {
            facts.push(this === expected, event.target === expected,
                event.currentTarget === expected, event.composedPath()[0] === expected);
        }`)(navigation, facts);
        navigation.addEventListener('foreign-probe', listener, {once: true});
        EventTarget.prototype.dispatchEvent.call(navigationAlias, new Event('foreign-probe'));
        frame.remove();
        return JSON.stringify(facts);
    })()"#
        )
        .unwrap(),
        "[true,true,true,true]"
    );
    vm.eval(
        r#"
        globalThis.mainDisposeFacts = [];
        mainEntry.ondispose = function(event) {
            mainDisposeFacts.push(this === mainEntry, event.target === mainEntry,
                event.currentTarget === mainEntry, event.isTrusted);
        };
        true;
    "#,
    )
    .unwrap();
    vm.eval_in_isolated_context(
        isolated,
        r#"
        globalThis.worldDisposeFacts = [];
        worldEntry.ondispose = function(event) {
            worldDisposeFacts.push(this === worldEntry, event.target === worldEntry,
                event.currentTarget === worldEntry, event.isTrusted);
        };
        true;
    "#,
    )
    .unwrap();
    vm.eval("history.replaceState({}, '', '#disposed-entry')")
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(mainDisposeFacts)").unwrap(),
        "[true,true,true,true]"
    );
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "JSON.stringify(worldDisposeFacts)")
            .unwrap(),
        "[true,true,true,true]"
    );
}

#[test]
fn fixed_event_dispatch_targets_are_receiver_local_with_shared_listeners() {
    for fixed_first in [false, true] {
        let mut vm = new_storage_html_test_vm("https://shared-dispatch-policy.test/");
        vm.eval(
            r#"
            globalThis.owner = new EventTarget();
            globalThis.wrapper = new EventTarget();
            globalThis.ownerTarget = new EventTarget();
            globalThis.wrapperTarget = new EventTarget();
            globalThis.traps = 0;
            globalThis.alias = new Proxy(wrapper, {
                get() {traps++; throw Error('get trap');},
                getPrototypeOf() {traps++; throw Error('prototype trap');}
            });
            true;
        "#,
        )
        .unwrap();
        vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
            let global = scope.get_current_context().global(scope);
            let object = |name| {
                let value = global
                    .get(scope, crate::util::v8str(scope, name).into())
                    .unwrap();
                v8::Local::<v8::Object>::try_from(value).unwrap()
            };
            let owner = object("owner");
            let wrapper = object("wrapper");
            let target = object("ownerTarget");
            let proxy = v8::Local::<v8::Proxy>::try_from(object("alias")).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            if fixed_first {
                crate::context_bootstrap::bind_dispatch_target(scope, owner, target);
            }
            crate::context_bootstrap::bind_shared_target(scope, owner, owner);
            crate::context_bootstrap::bind_shared_target(scope, wrapper, owner);
            if !fixed_first {
                crate::context_bootstrap::bind_dispatch_target(scope, owner, target);
            }
            for target in [owner, wrapper] {
                crate::context_bootstrap::install_simple_event_target_handlers(
                    scope,
                    target,
                    &["onprobe"],
                );
            }
            Ok(())
        })
        .unwrap();
        vm.eval(r#"
            globalThis.expectedTarget = ownerTarget;
            globalThis.calls = [];
            globalThis.otherCalls = 0;
            for (const target of [ownerTarget, wrapperTarget])
                target.addEventListener('probe', () => otherCalls++);
            globalThis.observe = function(event) {
                calls.push(this === expectedTarget && event.target === expectedTarget &&
                    event.currentTarget === expectedTarget && event.composedPath()[0] === expectedTarget);
            };
            owner.addEventListener('probe', observe);
            EventTarget.prototype.addEventListener.call(alias, 'probe', observe);
            owner.onprobe = observe;
            Object.getOwnPropertyDescriptor(wrapper, 'onprobe').set.call(alias, observe);
            globalThis.checkDispatch = receiver => {
                calls.length = 0;
                const event = new Event('probe');
                if (!EventTarget.prototype.dispatchEvent.call(receiver, event) ||
                    calls.length !== 3 || !calls.every(Boolean) ||
                    event.target !== expectedTarget || event.currentTarget !== null ||
                    event.composedPath().length !== 0 || otherCalls !== 0)
                    throw Error('shared listener storage / receiver dispatch policy');
            };
            checkDispatch(owner);
            expectedTarget = wrapper;
            checkDispatch(wrapper);
            checkDispatch(alias);
            true;
        "#).unwrap();
        vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
            let global = scope.get_current_context().global(scope);
            let wrapper = global
                .get(scope, crate::util::v8str(scope, "alias").into())
                .unwrap();
            let target = global
                .get(scope, crate::util::v8str(scope, "wrapperTarget").into())
                .unwrap();
            crate::context_bootstrap::bind_dispatch_target(
                scope,
                v8::Local::<v8::Object>::try_from(wrapper).unwrap(),
                v8::Local::<v8::Object>::try_from(target).unwrap(),
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(
            vm.eval(
                r#"(() => {
            expectedTarget = wrapperTarget;
            checkDispatch(wrapper);
            checkDispatch(alias);
            expectedTarget = ownerTarget;
            checkDispatch(owner);
            const handler = Object.getOwnPropertyDescriptor(wrapper, 'onprobe');
            handler.set.call(alias, null);
            if (owner.onprobe !== observe || handler.get.call(wrapper) !== null) return false;
            EventTarget.prototype.removeEventListener.call(alias, 'probe', observe);
            calls.length = 0;
            owner.dispatchEvent(new Event('probe'));
            return calls.length === 1 && calls[0] && traps === 0;
        })()"#
            )
            .unwrap(),
            "true",
            "fixed_first={fixed_first}"
        );
    }
}

#[test]
fn model_context_native_aliases_project_shared_listener_targets_into_worlds() {
    let mut vm = new_storage_html_test_vm("https://model-context-native-worlds.test/");
    vm.eval(
        r#"
        globalThis.model = document.modelContext;
        globalThis.traps = 0;
        globalThis.alias = new Proxy(model, {
            get() {traps++; throw Error('get trap');},
            getPrototypeOf() {traps++; throw Error('prototype trap');}
        });
        true;
    "#,
    )
    .unwrap();
    let isolated = vm
        .create_isolated_world("native-model-context", false)
        .unwrap();
    vm.eval_in_isolated_context(
        isolated,
        r#"
        globalThis.model = document.modelContext;
        globalThis.facts = [];
        model.addEventListener('probe', function(event) {
            facts.push(this === model && event.target === model &&
                event.currentTarget === model && event.composedPath()[0] === model &&
                event instanceof CustomEvent && event.detail === 7 && event.pageOnly === undefined);
            event.preventDefault();
        }, {once: true});
        true;
    "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let value = global
            .get(scope, crate::util::v8str(scope, "alias").into())
            .unwrap();
        moli_webapi_declare::register_web_api_proxy(
            scope,
            v8::Local::<v8::Proxy>::try_from(value).unwrap(),
        )
        .unwrap();
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const facts = [];
        function listener(event) {
            facts.push(this === model && event.target === model && event.currentTarget === model &&
                event.composedPath()[0] === model && event.pageOnly === 42);
        }
        const {addEventListener: add, removeEventListener: remove, dispatchEvent: dispatch} = EventTarget.prototype;
        add.call(alias, 'probe', listener);
        model.addEventListener('probe', listener);
        const event = new CustomEvent('probe', {detail: 7, cancelable: true});
        event.pageOnly = 42;
        if (dispatch.call(alias, event) || !event.defaultPrevented || event.target !== model ||
            event.currentTarget !== null || event.composedPath().length !== 0) return false;
        remove.call(alias, 'probe', listener);
        return model.dispatchEvent(new Event('probe')) && facts.length === 1 && facts[0] && traps === 0;
    })()"#).unwrap(), "true");
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "JSON.stringify(facts)")
            .unwrap(),
        "[true]"
    );
}

#[test]
fn simple_event_target_registry_reuse_preserves_reentrant_listener_mutations() {
    let mut vm = new_storage_html_test_vm("https://event-listener-registry.test/");
    assert_eq!(vm.eval(r#"(() => {
        for (const make of [() => new EventTarget(), () => new FileReader()]) {
            const target = make();
            const trace = [];
            const controller = new AbortController();
            const victim = () => trace.push('victim');
            const late = () => trace.push('late');
            target.addEventListener('probe', () => {
                trace.push('capture');
            }, {capture: true, once: true});
            target.addEventListener('probe', () => {
                trace.push('first');
                target.removeEventListener('probe', victim);
                target.addEventListener('probe', victim);
                controller.abort();
            }, {once: true});
            target.addEventListener('probe', victim);
            target.addEventListener('probe', () => trace.push('aborted'), {signal: controller.signal});
            target.addEventListener('probe', () => {
                trace.push('once');
                target.dispatchEvent(new Event('probe'));
            }, {once: true});
            target.addEventListener('probe', late);
            target.dispatchEvent(new Event('probe'));
            if (trace.join(',') !== 'capture,first,once,late,victim,late') return false;
            trace.length = 0;
            target.dispatchEvent(new Event('probe'));
            if (trace.join(',') !== 'late,victim') return false;
            target.removeEventListener('probe', late);
            target.removeEventListener('probe', victim);
            trace.length = 0;
            target.dispatchEvent(new Event('probe'));
            if (trace.length !== 0) return false;
        }
        const reader = new FileReader();
        const trace = [];
        reader.addEventListener('load', () => {
            reader.onload = () => trace.push('new');
        }, {once: true});
        reader.onload = () => trace.push('old');
        reader.dispatchEvent(new Event('load'));
        reader.onload = null;
        reader.dispatchEvent(new Event('load'));
        return trace.join(',') === 'new';
    })()"#).unwrap(), "true");
}

#[test]
fn empty_native_event_targets_preserve_dispatch_lifecycle_with_or_without_registry() {
    fn checkpoint(scope: &mut v8::PinScope<'_, '_>) {
        let source = v8::String::new(scope, "emptyNativeCheckpoint = emptyNativeEvent.target === emptyNativeTarget && emptyNativeEvent.currentTarget === emptyNativeTarget && emptyNativeEvent.composedPath()[0] === emptyNativeTarget;").unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(
            crate::script_execution::execute_compiled_script(scope, script)
                .unwrap()
                .is_true()
        );
    }

    let mut vm = new_storage_html_test_vm("https://event-target-empty-native.test/");
    for initialized in [false, true] {
        for canceled in [false, true] {
            vm.eval(&format!(
                "globalThis.emptyNativeTarget = {}; globalThis.emptyNativeEvent = new Event('empty', {{cancelable: true}}); globalThis.emptyNativeCheckpoint = false; {}; true;",
                if initialized { "new EventTarget()" } else { "{}" },
                if canceled { "emptyNativeEvent.preventDefault(); emptyNativeEvent.stopImmediatePropagation();" } else { "" },
            )).unwrap();
            vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
                let global = scope.get_current_context().global(scope);
                let target = global
                    .get(scope, crate::util::v8str(scope, "emptyNativeTarget").into())
                    .unwrap();
                let target = v8::Local::<v8::Object>::try_from(target).unwrap();
                let event = global
                    .get(scope, crate::util::v8str(scope, "emptyNativeEvent").into())
                    .unwrap();
                let event = v8::Local::<v8::Object>::try_from(event).unwrap();
                crate::context_bootstrap::mark_event_trusted(scope, event);
                let result =
                    crate::context_bootstrap::dispatch_simple_event_target_event_with_checkpoint(
                        scope, target, "empty", event, checkpoint,
                    );
                assert!(!result.dispatched);
                assert_eq!(result.uncanceled, !canceled);
                Ok(())
            })
            .unwrap();
            assert_eq!(vm.eval("String(emptyNativeCheckpoint && emptyNativeEvent.target === emptyNativeTarget && emptyNativeEvent.currentTarget === null && emptyNativeEvent.eventPhase === 0 && emptyNativeEvent.composedPath().length === 0 && !emptyNativeEvent.cancelBubble && emptyNativeEvent.isTrusted)").unwrap(), "true");
        }
    }
    assert_eq!(vm.eval("(() => { const target = new EventTarget(); const event = new Event('empty'); return target.dispatchEvent(event) && event.target === target && event.currentTarget === null && event.composedPath().length === 0; })()").unwrap(), "true");
}
