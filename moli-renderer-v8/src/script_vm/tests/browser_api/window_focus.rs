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
fn changing_elements_inside_a_frame_preserves_document_focus_during_blur() {
    let mut vm = new_storage_html_test_vm("https://document-focus-transition.test/");
    let result = vm
        .eval(
            r#"(() => {
  const outer = document.body.appendChild(document.createElement('iframe'));
  const middle = outer.contentDocument;
  const inner = middle.body.appendChild(middle.createElement('iframe'));
  const deep = inner.contentDocument;
  deep.body.innerHTML = '<input id=a><input id=b>';
  const [a, b] = deep.body.children;
  const states = [];
  const read = () => states.push([
    document.hasFocus(), middle.hasFocus(), deep.hasFocus(),
    document.activeElement === outer, middle.activeElement === inner,
    deep.activeElement.id || deep.activeElement.tagName,
  ].join(','));

  outer.focus({preventScroll: true});
  if (!middle.hasFocus()) throw new Error('frame owner did not focus its Window');
  outer.blur();
  if (middle.hasFocus() || document.activeElement === outer) {
    throw new Error('blurring a frame owner left its Window focused');
  }

  a.focus({preventScroll: true});
  a.addEventListener('blur', read);
  a.addEventListener('focusout', read);
  b.focus({preventScroll: true});
  read();
  b.blur();
  read();
  outer.remove();
  read();
  return states.join('|');
})()"#,
        )
        .unwrap();
    assert_eq!(
        result,
        concat!(
            "true,true,true,true,true,BODY|true,true,true,true,true,BODY|",
            "true,true,true,true,true,b|true,true,true,true,true,BODY|",
            "true,false,false,false,false,BODY"
        )
    );
}

#[test]
fn reentrant_focus_dispatches_window_events_from_the_focused_frame() {
    let mut vm = new_storage_html_test_vm("https://window-focus-transition.test/");
    let result = vm
        .eval(
            r#"(() => {
  document.body.innerHTML = '<input id=top><iframe id=first></iframe><iframe id=second></iframe>';
  const [topInput, first, second] = document.body.children;
  const addInput = frame => {
    const child = frame.contentDocument;
    return child.body.appendChild(child.createElement('input'));
  };
  const a = addInput(first);
  const b = addInput(second);
  const states = [];
  const read = stage => states.push([
    stage, first.contentDocument.hasFocus(), second.contentDocument.hasFocus(),
    document.activeElement.id,
  ].join(','));

  a.focus({preventScroll: true});
  first.contentWindow.addEventListener('blur', () => read('first window blur'));
  second.contentWindow.addEventListener('focus', () => read('second window focus'));
  window.addEventListener('blur', () => read('top window blur'));
  window.addEventListener('focus', () => read('top window focus'));
  a.addEventListener('blur', () => {
    read('first input blur');
    b.focus({preventScroll: true});
  });
  topInput.focus({preventScroll: true});
  read('after redirect');
  return states.join('|');
})()"#,
        )
        .unwrap();
    assert_eq!(
        result,
        concat!(
            "first input blur,true,false,first|first window blur,false,true,second|",
            "second window focus,false,true,second|after redirect,false,true,second"
        )
    );
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
