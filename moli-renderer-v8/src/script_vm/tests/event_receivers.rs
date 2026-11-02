use super::*;

#[test]
fn derived_event_dictionaries_convert_ancestors_before_own_members() {
    let mut vm = new_storage_page_task_executor_test_vm("https://event-dictionary-order.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
                const base = ['bubbles', 'cancelable', 'composed'];
                const cases = [
                    // Level 2 partial dictionaries add the nullable animation member.
                    ['AnimationEvent', ['animation', 'animationName', 'elapsedTime', 'pseudoElement']],
                    ['TransitionEvent', ['animation', 'elapsedTime', 'propertyName', 'pseudoElement']],
                    ['WebGLContextEvent', ['statusMessage']],
                    ['BlobEvent', ['data', 'timecode']],
                    ['MessageEvent', ['data', 'lastEventId', 'origin', 'ports', 'source']],
                    ['StorageEvent', ['key', 'newValue', 'oldValue', 'storageArea', 'url']],
                    ['ErrorEvent', ['colno', 'error', 'filename', 'lineno', 'message']],
                    ['DeviceOrientationEvent', ['absolute', 'alpha', 'beta', 'gamma']],
                    ['DeviceMotionEvent', ['acceleration', 'accelerationIncludingGravity', 'interval', 'rotationRate']],
                    ['MediaQueryListEvent', ['matches', 'media']],
                    ['IDBVersionChangeEvent', ['newVersion', 'oldVersion']],
                    ['SecurityPolicyViolationEvent', ['blockedURI', 'columnNumber', 'disposition', 'documentURI', 'effectiveDirective', 'lineNumber', 'originalPolicy', 'referrer', 'sample', 'sourceFile', 'statusCode', 'violatedDirective']],
                ];
                for (const realm of [window, document.querySelector('iframe').contentWindow]) {
                    for (const [name, own] of cases) {
                        const trace = [], expected = [...base, ...own];
                        const init = new Proxy({}, {get(_, key) {
                            trace.push(key);
                            if (base.includes(key)) return true;
                            if (name === 'BlobEvent' && key === 'data') return new realm.Blob([]);
                            return undefined;
                        }});
                        const event = new realm[name]('order', init);
                        if (trace.join() !== expected.join() || !event.bubbles ||
                            !event.cancelable || !event.composed)
                            throw Error(name + ' conversion order: ' + trace);
                        if ((name === 'AnimationEvent' || name === 'TransitionEvent') &&
                            event.animation !== null)
                            throw Error(name + ' nullable animation default');
                        for (const member of base) {
                            const sentinel = {}, before = [];
                            const failing = new Proxy({}, {get(_, key) {
                                before.push(key);
                                if (key === member) throw sentinel;
                                return false;
                            }});
                            let error;
                            try {new realm[name]('order', failing);} catch (caught) {error = caught;}
                            if (error !== sentinel || before.join() !==
                                base.slice(0, base.indexOf(member) + 1).join())
                                throw Error(name + ' ancestor exception: ' + before);
                        }
                        for (const member of own) {
                            const sentinel = {}, before = [];
                            const failing = new Proxy({}, {get(_, key) {
                                before.push(key);
                                if (key === member) throw sentinel;
                                if (name === 'BlobEvent' && key === 'data') return new realm.Blob([]);
                                return undefined;
                            }});
                            let error;
                            try {new realm[name]('order', failing);} catch (caught) {error = caught;}
                            if (error !== sentinel || before.join() !==
                                expected.slice(0, base.length + own.indexOf(member) + 1).join())
                                throw Error(name + ' own member exception: ' + before);
                        }
                    }
                }
                return true;
            })()"#,
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn event_bindings_validate_native_receivers_before_conversion_in_the_callee_realm() {
    let mut vm = new_parsed_test_vm(
        "https://event-receivers.test/",
        "<!doctype html><body><iframe id=child></iframe></body>",
    );
    vm.eval(include_str!("event_receivers.js")).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__eventReceiverFailures)")
            .unwrap(),
        "[]"
    );
}

#[test]
fn css_event_animation_members_preserve_native_identity_and_reject_forged_brands() {
    let mut vm = new_storage_html_test_vm("https://animation-event-members.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, descriptor) in [
            (
                "nativeAnimation",
                crate::web_api_interfaces::CSSAnimation::DESCRIPTOR,
            ),
            (
                "nativeTransition",
                crate::web_api_interfaces::CSSTransition::DESCRIPTOR,
            ),
        ] {
            let native = v8::Object::new(scope);
            descriptor.initialize(scope, native).unwrap();
            let key = crate::util::v8str(scope, name);
            assert_eq!(global.set(scope, key.into(), native.into()), Some(true));
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const child = document.querySelector('iframe').contentWindow;
        for (const realm of [window, child]) {
            for (const [name, native, wrong] of [
                ['AnimationEvent', nativeAnimation, nativeTransition],
                ['TransitionEvent', nativeTransition, nativeAnimation]
            ]) {
                const Ctor = realm[name];
                for (const animation of [undefined, null]) {
                    if (new Ctor('x', {animation}).animation !== null)
                        throw Error(name + ' nullable default');
                }
                const event = new Ctor('x', {animation:native});
                if (event.animation !== native || event.animation !== event.animation)
                    throw Error(name + ' native identity');
                const descriptor = Object.getOwnPropertyDescriptor(Ctor.prototype, 'animation');
                if (!descriptor.enumerable || !descriptor.configurable || descriptor.set ||
                    Object.hasOwn(event, 'animation')) throw Error(name + ' readonly surface');
                let traps = 0;
                const proxy = new Proxy(native, {get() { traps++; throw Error('trap'); }});
                const forged = Object.create(native);
                for (const animation of [wrong, {}, forged, proxy]) {
                    let converted = 0, error;
                    try { new Ctor('x', {animation, get elapsedTime() { converted++; return 1; }}); }
                    catch (caught) { error = caught; }
                    if (!(error instanceof realm.TypeError) || converted || traps)
                        throw Error(name + ' brand/conversion order');
                }
                let error;
                try { descriptor.get.call({}); } catch (caught) { error = caught; }
                if (!(error instanceof realm.TypeError)) throw Error(name + ' receiver');
            }
        }
        return true;
    })()"#).unwrap(), "true");
}
