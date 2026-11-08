use super::*;

#[test]
fn window_focus_history_keeps_unfocused_windows_and_shared_ancestor_times() {
    let mut vm = new_storage_test_vm("https://focus-history.test/");
    vm.eval(r#"
        const root = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || root.appendChild(document.createElement('body'));
        body.innerHTML = '<iframe id=a></iframe><iframe id=b></iframe>';
        document.getElementById('a').focus();
    "#).unwrap();
    let (a, b) = {
        let host = vm._context_host.borrow();
        let frames = host.child_browsing_context_handles_in_document_order();
        (frames[0], frames[1])
    };
    let first = {
        let host = vm._context_host.borrow();
        let order = host
            .window_last_focus_order(crate::native_bridge::OwnerDispatchScope::Child(a))
            .unwrap();
        assert_eq!(
            host.window_last_focus_order(crate::native_bridge::OwnerDispatchScope::Top),
            Some(order)
        );
        assert_eq!(
            host.window_last_focus_order(crate::native_bridge::OwnerDispatchScope::Child(b)),
            None
        );
        order
    };
    vm.eval("document.getElementById('b').focus()").unwrap();
    let host = vm._context_host.borrow();
    let second = host
        .window_last_focus_order(crate::native_bridge::OwnerDispatchScope::Child(b))
        .unwrap();
    assert!(second > first);
    assert_eq!(
        host.window_last_focus_order(crate::native_bridge::OwnerDispatchScope::Top),
        Some(second)
    );
    assert_eq!(
        host.window_last_focus_order(crate::native_bridge::OwnerDispatchScope::Child(a)),
        Some(first)
    );
}

#[test]
fn document_has_focus_checks_every_live_ancestor_in_the_native_focus_chain() {
    let mut vm = new_storage_test_vm("https://document-focus-chain.test/");
    let result = vm.eval(r#"(() => {
        const root = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || root.appendChild(document.createElement('body'));
        body.innerHTML = '<input id=top><iframe id=outer></iframe>';
        const outer = document.getElementById('outer');
        const middle = outer.contentDocument;
        middle.body.innerHTML = '<input id=middle><iframe id=inner></iframe>';
        const inner = middle.getElementById('inner');
        const deep = inner.contentDocument;
        deep.body.innerHTML = '<input id=deep>';
        const state = () => [document.hasFocus(), middle.hasFocus(), deep.hasFocus()].join(',');
        const states = [state()];
        deep.getElementById('deep').focus();
        states.push(state());
        document.getElementById('top').focus();
        states.push(state());
        if (middle.activeElement !== inner) throw Error('inactive child lost its local focused area');
        middle.getElementById('middle').focus();
        states.push(state());
        deep.getElementById('deep').focus();
        states.push(state());
        outer.remove();
        states.push(state());
        if (document.implementation.createHTMLDocument('').hasFocus()) throw Error('detached Document focused');
        window.checkFocusState = state;
        return states.join('|');
    })()"#).unwrap();
    assert_eq!(
        result,
        "true,false,false|true,true,true|true,false,false|true,true,false|true,true,true|true,false,false"
    );
    vm.set_document_activity(moli_page_types::DocumentActivity::new(false, true))
        .unwrap();
    assert_eq!(vm.eval("checkFocusState()").unwrap(), "false,false,false");
}

#[test]
fn document_has_focus_uses_the_branded_receiver_without_reading_author_properties() {
    let mut vm = new_storage_test_vm("https://document-focus-receiver.test/");
    assert_eq!(vm.eval(r#"(() => {
        const hasFocus = Document.prototype.hasFocus;
        for (const receiver of [{defaultView: window}, Object.create(document), new Proxy(document, {}), document.createElement('div')]) {
            let error;
            try { hasFocus.call(receiver); } catch (caught) { error = caught; }
            if (!(error instanceof TypeError)) throw Error('invalid Document receiver accepted');
        }
        Object.defineProperty(document, 'defaultView', {get() { throw Error('author defaultView getter invoked'); }});
        return hasFocus.call(document);
    })()"#).unwrap(), "true");
}

#[test]
fn window_focus_and_blur_validate_the_receiver_in_the_callee_realm() {
    let mut vm = new_storage_test_vm("https://window-focus-native.test/");
    vm.eval(r#"(() => {
        const root = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || root.appendChild(document.createElement('body'));
        const frame = document.createElement('iframe');
        frame.id = 'focus-native-child';
        body.append(frame);
    })()"#).unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "Window focus child realm");
    assert_eq!(
        vm.eval(
            r#"(() => {
        const frame = document.getElementById('focus-native-child');
        const other = frame.contentWindow;
        const fake = Object.create(Object.getPrototypeOf(other));
        for (const name of ['focus', 'blur']) {
            for (const receiver of [fake, Object.create(other), new Proxy(other, {})]) {
                let error;
                try { Reflect.apply(other[name], receiver, []); } catch (caught) { error = caught; }
                if (!(error instanceof other.TypeError) || error instanceof TypeError)
                    throw Error(name + ' callee realm brand check');
            }
            if (Reflect.apply(other[name], window, []) !== undefined)
                throw Error(name + ' genuine cross-realm Window');
        }
        const focus = other.focus;
        if (focus() !== undefined || document.activeElement !== frame)
            throw Error('extracted focus must select its child browsing context');
        if (other.blur.call(null) !== undefined) throw Error('null global receiver');
        frame.remove();
        if (other.focus() !== undefined || other.blur() !== undefined)
            throw Error('retired genuine Window should be a safe no-op');
        return true;
    })()"#
        )
        .unwrap(),
        "true"
    );
}
