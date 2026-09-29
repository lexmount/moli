use super::*;

#[test]
fn form_wrappers_preserve_own_properties_and_identity_across_adoption() {
    let mut vm = new_parsed_test_vm(
        "https://form-wrapper-adoption.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
      const detached = document.implementation.createHTMLDocument('forms');
      const parsed = new DOMParser().parseFromString('<form></form>', 'text/html');
      const frame = document.body.appendChild(document.createElement('iframe'));
      const child = frame.contentDocument;
      const forms = [document.createElement('form'), detached.createElement('form'),
        parsed.querySelector('form'), child.createElement('form')];
      const keys = ['name', 'elements', 'length', 'submit', 'reset', 'nodeType', 'nodeName', 'ownerDocument', 'matches'];
      for (const [mode, form] of forms.entries()) {
        const doc = form.ownerDocument;
        const values = keys.map(() => ({}));
        for (const [i, key] of keys.entries()) {
          if (Object.hasOwn(form, key)) throw Error(mode + ': native own ' + key);
          Object.defineProperty(form, key, {value: values[i], writable: true, configurable: true, enumerable: true});
          const control = form.appendChild(doc.createElement('input'));
          control.name = key;
        }
        const first = form[0];
        for (const target of [detached, child, document, parsed]) {
          if (target.adoptNode(form) !== form) throw Error('adopt replaced form');
          target.body.appendChild(form);
          if (form[0] !== first || Object.getOwnPropertyDescriptor(form, '0').value !== first) throw Error('control identity');
          for (const [i, key] of keys.entries()) {
            const d = Object.getOwnPropertyDescriptor(form, key);
            if (!d || d.value !== values[i] || !d.writable || !d.enumerable || !d.configurable || form[key] !== values[i])
              throw Error(mode + ': adoption changed own ' + key);
          }
        }
        for (const [i, key] of keys.entries()) {
          if (!delete form[key] || form[key] !== form[i]) throw Error('own deletion must reveal control: ' + key);
        }
      }
      return 'ok';
    })()"#).unwrap(), "ok");
}

#[test]
fn form_wrappers_return_collections_in_receiver_realm_for_borrowed_accessors() {
    let mut vm = new_parsed_test_vm(
        "https://form-wrapper-realms.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
      const frame = document.body.appendChild(document.createElement('iframe'));
      const realm = frame.contentWindow;
      const detached = realm.document.implementation.createHTMLDocument('forms');
      const source = detached.createElement('form');
      const markup = '<input name="group"><input name="group"><img name="photos"><img name="photos">';
      source.innerHTML = markup;
      const forms = [source, source.cloneNode(true), realm.document.importNode(source, true),
        realm.document.createElement('form')];
      const parentElements = Object.getOwnPropertyDescriptor(HTMLFormElement.prototype, 'elements').get;
      for (const [mode, form] of forms.entries()) {
        if (!form.firstChild) form.innerHTML = markup;
        const inputs = form.querySelectorAll('input');
        const photos = form.querySelectorAll('img');
        const elements = parentElements.call(form);
        if (elements !== form.elements || !(elements instanceof realm.HTMLFormControlsCollection) || elements[0] !== inputs[0])
          throw Error('borrowed elements accessor used caller realm');
        for (const [key, items] of [['group', inputs], ['photos', photos]]) {
          const list = form[key];
          const described = Object.getOwnPropertyDescriptor(form, key).value;
          if (list !== described || !(list instanceof realm.RadioNodeList) || list.length !== 2 || list[0] !== items[0])
            throw Error(mode + ':' + key + ': named result identity/realm ' + [list === described, list instanceof realm.RadioNodeList, list.length, list[0] === items[0]].join(','));
        }
        document.body.appendChild(document.adoptNode(form));
        if (parentElements.call(form) !== elements || form[0] !== inputs[0]) throw Error('adoption changed wrapper cache');
        inputs[1].remove();
        if (form.group !== inputs[0] || Object.getOwnPropertyDescriptor(form, 'group').value !== inputs[0]) throw Error('single identity');
      }
      return 'ok';
    })()"#).unwrap(), "ok");
}

#[test]
fn form_wrappers_share_reset_events_and_keep_inert_submissions_inert() {
    let mut vm = new_parsed_test_vm(
        "https://form-wrapper-methods.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
      const frame = document.body.appendChild(document.createElement('iframe'));
      const child = frame.contentWindow;
      const docs = [document.implementation.createHTMLDocument('forms'),
        new DOMParser().parseFromString('<body></body>', 'text/html'),
        child.document.implementation.createHTMLDocument('forms')];
      for (const [mode, doc] of docs.entries()) {
        const form = doc.body.appendChild(doc.createElement('form'));
        form.innerHTML = '<input name="field" value="initial">';
        const input = form.field;
        let resets = 0;
        form.addEventListener('reset', event => {
          if (event.target !== form || event.currentTarget !== form || !event.bubbles || !event.cancelable ||
              event.constructor.constructor !== form.constructor.constructor) throw Error(mode + ': reset identity/realm');
          if (++resets === 1) event.preventDefault();
        });
        input.value = 'edited';
        HTMLFormElement.prototype.reset.call(form);
        if (input.value !== 'edited' || resets !== 1) throw Error('reset cancellation');
        child.HTMLFormElement.prototype.reset.call(form);
        if (input.value !== 'initial' || resets !== 2) throw Error('shared reset default action');
        form.action = '/should-not-navigate';
        for (const method of ['get', 'post']) {
          form.method = method;
          HTMLFormElement.prototype.submit.call(form);
        }
      }
      return 'ok';
    })()"#).unwrap(), "ok");
    assert!(vm.take_pending_location_navigation_with_seed().is_none());
}

#[tokio::test]
async fn form_wrappers_submit_get_to_lightweight_popup() {
    assert_lightweight_popup_form_submission("get").await;
}

#[tokio::test]
async fn form_wrappers_submit_post_to_lightweight_popup() {
    assert_lightweight_popup_form_submission("post").await;
}

async fn assert_lightweight_popup_form_submission(method: &str) {
    let cases = [
        ("", "form.submit()"),
        ("_self", "HTMLFormElement.prototype.submit.call(form)"),
        ("_parent", "form.submit()"),
        ("_top", "form.requestSubmit()"),
    ];
    // Submitting to the current URL must still load a new document every time.
    let submissions = 2;
    let server = StaticHttpServer::spawn_with_responder(
        cases.len() * submissions,
        "Content-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\n",
        |_, _| "<!doctype html><body>child fixture</body>".to_owned(),
    )
    .await;
    let loader = static_http_loader([]);
    let opener_url = server.base_url().join("opener.html").unwrap();
    let expected_target = if method == "get" {
        "/submit?field=a+b%2Bc"
    } else {
        "/submit?existing=1"
    };
    let expected_url = server.base_url().join(expected_target).unwrap();
    for (target, invocation) in cases {
        let mut vm = new_page_task_executor_test_vm_with_loader(opener_url.as_str(), &loader);
        vm.eval("globalThis.popup = open(); globalThis.originalOpenerDocument = document;")
            .unwrap();
        for _ in 0..submissions {
            vm.eval(&format!(
                r#"(() => {{
              globalThis.originalPopupDocument = popup.document;
              const form = popup.document.body.appendChild(popup.document.createElement('form'));
              form.action = '/submit?existing=1';
              form.method = '{method}';
              form.target = '{target}';
              form.innerHTML = '<input name="field" value="a b+c">';
              {invocation};
            }})()"#
            ))
            .expect("popup form submission should evaluate");
            assert!(
                vm.take_pending_location_navigation_with_seed().is_none(),
                "{method} target={target:?} must not navigate the opener"
            );
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(popup.document !== originalPopupDocument && popup.document.body.textContent === 'child fixture')",
                "true",
                "popup form response should replace the popup document",
            )
            .await;
            assert_eq!(
                vm.eval("popup.location.href").unwrap(),
                expected_url.as_str()
            );
            assert_eq!(vm.eval("location.href").unwrap(), opener_url.as_str());
            assert_eq!(
                vm.eval("document === originalOpenerDocument").unwrap(),
                "true"
            );
        }
        vm.eval("popup.close()").unwrap();
    }
    let requests = server.finish().await;
    assert_eq!(requests.len(), cases.len() * submissions);
    for request in requests {
        assert_eq!(request.method, method.to_ascii_uppercase());
        assert_eq!(request.target, expected_target);
        if method == "post" {
            assert_eq!(request.body, b"field=a+b%2Bc");
            assert_eq!(
                request.header_value("content-type"),
                Some("application/x-www-form-urlencoded")
            );
        } else {
            assert!(request.body.is_empty());
        }
    }
}

#[tokio::test]
async fn form_wrappers_popup_submission_respects_cancellation_and_closed_documents() {
    let server = StaticHttpServer::spawn(1).await;
    let loader = static_http_loader([]);
    let opener_url = server.base_url().join("opener.html").unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(opener_url.as_str(), &loader);
    vm.eval("globalThis.popup = open('/initial');").unwrap();
    // Navigation API entries and events are disabled on initial about:blank.
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(popup.document.URL.endsWith('/initial') && popup.document.readyState === 'complete')",
        "true",
        "popup cancellation target must have a committed document",
    )
    .await;
    assert_eq!(
        vm.eval("popup.navigation.currentEntry !== null").unwrap(),
        "true"
    );
    vm.eval(r#"
          const original = popup.document;
          const originalURL = popup.location.href;
          const form = original.body.appendChild(original.createElement('form'));
          form.action = '/submit';
          form.innerHTML = '<input name="field" value="data">';
          const events = [];
          popup.navigation.addEventListener('navigate', event => {
            events.push([event.sourceElement === form, event.formData && event.formData.get('field')]);
            event.preventDefault();
          });
        "#).unwrap();
    let mut previous_events = "[]";
    for (method, expected_events) in [
        ("get", "[[true,null]]"),
        ("post", "[[true,null],[true,\"data\"]]"),
    ] {
        vm.eval(&format!(
            "form.method = '{method}'; HTMLFormElement.prototype.submit.call(form);"
        ))
        .unwrap();
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            previous_events,
            "navigate waits for the form's DOM-manipulation task"
        );
        assert!(
            vm.run_one_dom_manipulation_body_for_test(
                crate::runtime::PageDomManipulationTestFamily::FormNavigation,
            )
            .await
            .unwrap()
        );
        assert_eq!(vm.eval("JSON.stringify(events)").unwrap(), expected_events);
        assert_eq!(
            vm.eval("popup.document === original && popup.location.href === originalURL")
                .unwrap(),
            "true"
        );
        assert!(vm.take_pending_location_navigation_with_seed().is_none());
        assert!(
            !vm._context_host
                .borrow()
                .has_pending_lightweight_popup_document_loads()
        );
        previous_events = expected_events;
    }
    vm.eval(
        r#"
        popup.close();
        for (const method of ['get', 'post']) {
          form.method = method;
          HTMLFormElement.prototype.submit.call(form);
        }
    "#,
    )
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        "[[true,null],[true,\"data\"]]"
    );
    assert!(!vm.has_ready_dom_manipulation_family_for_test(
        crate::runtime::PageDomManipulationTestFamily::FormNavigation,
    ));
    assert!(vm.take_pending_location_navigation_with_seed().is_none());
    assert!(
        !vm._context_host
            .borrow()
            .has_pending_lightweight_popup_document_loads()
    );
    assert_eq!(server.finish_targets().await, vec!["/initial"]);
}
