use super::*;

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
