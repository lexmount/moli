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
fn tab_navigation_enters_and_leaves_nested_frame_documents() {
    let mut vm = new_storage_html_test_vm("https://sequential-frame-focus.test/");
    vm.eval(
        r#"
        document.body.innerHTML = '<button id=before tabindex=1>Before</button>'
            + '<iframe id=outer tabindex=2></iframe><button id=after tabindex=3>After</button>'
            + '<iframe id=empty></iframe><iframe id=skipped tabindex=-1></iframe>';
        const child = document.getElementById('outer').contentDocument;
        child.body.innerHTML = '<iframe id=nested></iframe><button id=child tabindex=4>Child</button>';
        child.getElementById('nested').contentDocument.body.innerHTML = '<input id=deep>';
        document.getElementById('skipped').contentDocument.body.innerHTML = '<input id=excluded>';
        window.focusPath = () => {
            const path = [];
            let doc = document;
            for (let element; element = doc.activeElement;) {
                path.push(element.id || element.tagName);
                doc = element.contentDocument;
                if (!doc) break;
            }
            return path.join('/');
        };
        window.keyTarget = '';
        child.addEventListener('keydown', event => {
            keyTarget = event.target.tagName;
            event.preventDefault();
        }, {once: true});
        document.getElementById('outer').contentWindow.focus();
        "#,
    )
    .unwrap();
    for (modifiers, expected) in [
        (0, "outer/BODY"),
        (0, "outer/child"),
        (0, "outer/nested/deep"),
        (0, "after"),
        (0, "before"),
        (0, "outer/child"),
        (8, "before"),
        (8, "after"),
        (8, "outer/nested/deep"),
        (8, "outer/child"),
    ] {
        vm.dispatch_key_event("keydown", "Tab", "Tab", "", modifiers, false, false)
            .unwrap();
        assert_eq!(vm.eval("focusPath()").unwrap(), expected);
    }
    assert_eq!(vm.eval("keyTarget").unwrap(), "BODY");
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
        const root = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || root.appendChild(document.createElement('body'));
        const frame = body.appendChild(document.createElement('iframe'));
        const child = frame.contentWindow;
        const childDocument = frame.contentDocument;
        const input = childDocument.body.appendChild(childDocument.createElement('input'));
        input.focus();
        const poisoned = {get() { throw Error('author focus relationship getter invoked'); }};
        Object.defineProperty(document, 'defaultView', poisoned);
        Object.defineProperty(childDocument, 'defaultView', poisoned);
        Object.defineProperty(child, 'frameElement', poisoned);
        Object.defineProperty(frame, 'ownerDocument', poisoned);
        Object.defineProperty(document, 'activeElement', poisoned);
        return hasFocus.call(document) && hasFocus.call(childDocument);
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
        const transitions = [];
        window.addEventListener('blur', () => transitions.push('top blur'));
        other.addEventListener('focus', () => transitions.push('child focus'));
        const focus = other.focus;
        if (focus() !== undefined || document.activeElement !== frame)
            throw Error('extracted focus must select its child browsing context');
        if (transitions.join(',') !== 'top blur,child focus')
            throw Error('the initial focus transition starts at the top viewport: ' + transitions);
        const input = other.document.body.appendChild(other.document.createElement('input'));
        input.focus();
        other.focus();
        if (other.document.activeElement !== input)
            throw Error('focus on the current Window must preserve its focused control');
        window.focus();
        if (document.activeElement !== document.body || other.document.activeElement !== input)
            throw Error('focusing the outer Window must select its own viewport');
        other.focus();
        if (document.activeElement !== frame || other.document.activeElement !== input)
            throw Error('focusing a child Window must restore its local focus');
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

#[test]
fn window_focus_uses_the_target_documents_permissions_policy() {
    let mut vm = new_storage_test_vm("https://window-focus-policy.test/");
    vm.eval(r#"(() => {
        const root = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || root.appendChild(document.createElement('body'));
        body.innerHTML = `<input id=focused><iframe id=child allow="focus-without-user-activation 'none'"></iframe>`;
        body.querySelector('input').focus();
    })()"#).unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "Window focus permissions policy");
    assert_eq!(
        vm.eval(
            r#"(() => {
            const child = document.getElementById('child');
            child.contentWindow.focus();
            return document.activeElement.id;
        })()"#
        )
        .unwrap(),
        "focused"
    );
}
