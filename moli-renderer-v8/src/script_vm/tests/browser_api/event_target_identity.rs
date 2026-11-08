use super::*;

#[test]
fn native_event_target_aliases_keep_dom_and_window_handle_identity() {
    let mut vm = new_parsed_test_vm(
        "https://event-target-handle-identity.test/",
        "<!doctype html><iframe id=child></iframe><div id=host></div>",
    );
    vm.eval(
        r#"
        globalThis.aliasTraps = 0;
        const frameWindow = document.querySelector('#child').contentWindow;
        const host = document.querySelector('#host');
        const root = host.attachShadow({mode: 'closed'});
        const inner = root.appendChild(document.createElement('span'));
        globalThis.targets = [window, frameWindow, inner];
        globalThis.aliases = targets.map(target => new Proxy(target, {
            get() {aliasTraps++; throw Error('get trap');},
            getPrototypeOf() {aliasTraps++; throw Error('prototype trap');},
            getOwnPropertyDescriptor() {aliasTraps++; throw Error('descriptor trap');}
        }));
        true;
    "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let aliases = global
            .get(scope, crate::util::v8str(scope, "aliases").into())
            .unwrap();
        let aliases = v8::Local::<v8::Array>::try_from(aliases).unwrap();
        for index in 0..aliases.length() {
            let proxy = aliases.get_index(scope, index).unwrap();
            moli_webapi_declare::register_web_api_proxy(
                scope,
                v8::Local::<v8::Proxy>::try_from(proxy).unwrap(),
            )
            .unwrap();
        }
        Ok(())
    })
    .unwrap();
    vm.eval(
        r#"
        const assert = (ok, label) => {if (!ok) throw Error(label);};
        const {addEventListener: add, removeEventListener: remove, dispatchEvent: dispatch} = EventTarget.prototype;
        targets.forEach((target, index) => {
            let calls = 0;
            function listener(event) {
                assert(this === target && event.target === target && event.currentTarget === target,
                    'handle identity ' + index);
                calls++;
            }
            add.call(aliases[index], 'identity-probe', listener);
            add.call(target, 'identity-probe', listener);
            dispatch.call(aliases[index], new Event('identity-probe'));
            assert(calls === 1, 'shared native registry ' + index);
            remove.call(aliases[index], 'identity-probe', listener);
            dispatch.call(target, new Event('identity-probe'));
            assert(calls === 1, 'remove from native registry ' + index);
        });
        const observed = [];
        const object = {handleEvent(event) {
            observed.push(this === object && event.target === host && event.currentTarget === document &&
                event.composedPath()[0] === host);
        }};
        document.addEventListener('retarget-probe', object, {once: true});
        dispatch.call(aliases[2], new Event('retarget-probe', {bubbles: true, composed: true}));
        assert(observed.length === 1 && observed[0], 'Shadow DOM and handleEvent identity');
        assert(aliasTraps === 0, 'native identity resolution must not invoke traps');
    "#,
    )
    .expect("native aliases should reuse handle identity without changing DOM retargeting");
}

#[test]
fn simple_event_target_ancestors_resolve_listener_and_dispatch_identities_separately() {
    let mut vm = new_storage_html_test_vm("https://event-target-ancestor-identity.test/");
    vm.eval(
        r#"
        globalThis.leafOwner = new EventTarget();
        globalThis.parentOwner = new EventTarget();
        globalThis.leafTarget = new EventTarget();
        globalThis.parentTarget = new EventTarget();
        globalThis.aliasTraps = 0;
        globalThis.leafAlias = new Proxy(leafOwner, {
            get() {aliasTraps++; throw Error('get trap');},
            getPrototypeOf() {aliasTraps++; throw Error('prototype trap');}
        });
        globalThis.chainEvent = new Event('probe', {bubbles: true, cancelable: true});
        globalThis.facts = [];
        const observe = (label, receiver, current, phase, event) => facts.push([
            label, receiver === current, event.target === leafTarget,
            event.currentTarget === current, event.eventPhase === phase,
            event.composedPath().length === 2 && event.composedPath()[0] === leafTarget &&
                event.composedPath()[1] === parentTarget
        ]);
        parentOwner.addEventListener('probe', function(event) {
            observe('capture', this, parentTarget, 1, event);
        }, {capture: true, once: true});
        leafOwner.addEventListener('probe', function(event) {
            observe('target', this, leafTarget, 2, event);
        });
        const object = {handleEvent(event) {
            observe(this === object ? 'object' : 'wrong-this', event.currentTarget, parentTarget, 3, event);
            event.preventDefault();
        }};
        parentOwner.addEventListener('probe', object);
        leafTarget.addEventListener('probe', () => {throw Error('wrong listener registry');});
        parentTarget.addEventListener('probe', () => {throw Error('wrong ancestor registry');});
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
        let leaf = object("leafOwner");
        let parent = object("parentOwner");
        let leaf_target = object("leafTarget");
        let parent_target = object("parentTarget");
        let alias = object("leafAlias");
        let event = object("chainEvent");
        moli_webapi_declare::register_web_api_proxy(
            scope,
            v8::Local::<v8::Proxy>::try_from(alias).unwrap(),
        )
        .unwrap();
        crate::context_bootstrap::bind_dispatch_target(scope, leaf, leaf_target);
        crate::context_bootstrap::bind_dispatch_target(scope, parent, parent_target);
        let result = crate::context_bootstrap::dispatch_simple_event_target_event_with_ancestors(
            scope,
            alias,
            "probe",
            event,
            &[parent],
        );
        assert!(result.dispatched && !result.uncanceled && !result.did_listeners_throw);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(facts)").unwrap(),
        r#"[["capture",true,true,true,true,true],["target",true,true,true,true,true],["object",true,true,true,true,true]]"#
    );
    assert_eq!(
        vm.eval("String(chainEvent.target === leafTarget && chainEvent.currentTarget === null && chainEvent.eventPhase === 0 && chainEvent.composedPath().length === 0 && aliasTraps === 0)").unwrap(),
        "true"
    );
}
