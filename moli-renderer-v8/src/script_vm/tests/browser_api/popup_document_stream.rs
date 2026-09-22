use super::*;

#[test]
fn popup_document_stream_methods_use_native_document_receiver_checks() {
    let mut vm = new_storage_test_vm("https://popup-document-stream-brand.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const w = open();
  try {
    const d = w.document;
    const failures = [];
    for (const name of ['open','write','writeln','close']) {
      if (Object.hasOwn(d, name) || d[name] !== Object.getPrototypeOf(d)[name]) {
        failures.push([name, 'binding']);
      }
      let conversions = 0, traps = 0;
      const value = {toString() { ++conversions; return 'text'; }};
      const revoked = Proxy.revocable(d, {});
      revoked.revoke();
      const proxy = new Proxy(d, {get(target, key, receiver) {
        ++traps; return Reflect.get(target, key, receiver);
      }});
      for (const receiver of [{}, Object.create(d), proxy, revoked.proxy]) {
        let error;
        try { d[name].call(receiver, value); } catch (caught) { error = caught; }
        if (!(error instanceof TypeError) || conversions !== 0 || traps !== 0) {
          failures.push([name, String(error), conversions, traps]);
        }
      }
    }
    return JSON.stringify(failures);
  } finally { w.close(); }
})()
"#,
        )
        .expect("popup stream methods should use the shared Document binding");
    assert_eq!(result, "[]");
}

#[test]
fn popup_document_stream_methods_reject_xml_receivers_without_mutation() {
    let mut vm = new_storage_test_vm("https://popup-document-stream-xml.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const w = open();
  try {
    const failures = [];
    for (const name of ['open','write','writeln','close']) {
      const doc = new DOMParser().parseFromString('<root/>', 'text/xml');
      let error;
      try { w.document[name].call(doc, 'text'); } catch (caught) { error = caught; }
      if (!(error instanceof DOMException) || error.name !== 'InvalidStateError' ||
          new XMLSerializer().serializeToString(doc) !== '<root/>') {
        failures.push([name, String(error)]);
      }
    }
    return JSON.stringify(failures);
  } finally { w.close(); }
})()
"#,
        )
        .expect("borrowed popup stream methods should apply the XML document check");
    assert_eq!(result, "[]");
}

#[test]
fn borrowed_popup_document_stream_methods_preserve_the_opener_document_owner() {
    let mut vm = new_storage_test_vm("https://popup-document-stream-owner.test/");
    let owner = vm.current_main_document_task_owner();
    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.textContent = 'opener';
  const beforeReady = document.readyState;
  const first = open(), second = open();
  try {
    first.document.body.textContent = 'first';
    second.document.body.textContent = 'second';
    const target = second.document;
    const same = Document.prototype.open.call(target) === target;
    first.document.write.call(target, '<p>replacement</p>');
    first.document.writeln.call(target, 'tail');
    first.document.close.call(target);
    return JSON.stringify({same, sameDocument:second.document === target,
      first:first.document.body.textContent, second:target.body.textContent,
      openerBody:document.body === body && body.textContent,
      openerRoot:document.documentElement === root,
      openerReady:document.readyState === beforeReady});
  } finally { first.close(); second.close(); }
})()
"#,
        )
        .expect("borrowed methods must dispatch to the receiver's popup Document");
    assert_eq!(
        result,
        r#"{"same":true,"sameDocument":true,"first":"first","second":"replacementtail\n","openerBody":"opener","openerRoot":true,"openerReady":true}"#
    );
    assert_eq!(vm.current_main_document_task_owner(), owner);
}

#[test]
fn popup_document_write_uses_its_own_trusted_types_requirements() {
    let mut vm = new_storage_test_vm("https://popup-document-stream-policy.test/");
    vm.eval("globalThis.__unprotectedStreamPopup = open(); 'ready'")
        .expect("create popup before the opener requires TrustedHTML");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);
    let result = vm
        .eval(
            r#"
(() => {
  const protectedPopup = open();
  try {
    const plain = __unprotectedStreamPopup.document;
    const guarded = protectedPopup.document;
    Document.prototype.write.call(plain, 'plain');
    let blocked;
    try { guarded.write('blocked'); } catch (error) { blocked = error.name; }
    const untouched = guarded.body.textContent;
    const policy = trustedTypes.createPolicy('popup-stream', {createHTML:value => value});
    guarded.writeln(policy.createHTML('one'), policy.createHTML('two'));
    return JSON.stringify({plain:plain.body.textContent, blocked, untouched,
      trusted:guarded.body.textContent});
  } finally { protectedPopup.close(); __unprotectedStreamPopup.close(); }
})()
"#,
        )
        .expect("popup stream policy must follow the receiver instead of its wrapper realm");
    assert_eq!(
        result,
        r#"{"plain":"plain","blocked":"TypeError","untouched":"","trusted":"onetwo\n"}"#
    );
}

#[tokio::test]
async fn popup_document_write_load_callbacks_convert_before_mutation_and_follow_the_receiver() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://popup-document-stream-load.test/",
        "<!doctype html><body>opener</body>",
        &loader,
    );
    vm.eval(
        r#"
window.__streamMethodPopup = open();
window.__streamResults = [];
window.__streamPopups = ['conversion','borrowed'].map(mode => {
  const html = `<!doctype html><script>
    onload = () => {
      if (${JSON.stringify(mode)} === 'conversion') {
        const sentinel = {};
        let caught;
        try { document.write({toString() { throw sentinel; }}); } catch (error) { caught = error; }
        opener.__streamResults.push(['conversion', caught === sentinel, document.body.textContent]);
      } else {
        opener.__streamMethodPopup.document.writeln.call(document, 'replacement');
        opener.__streamResults.push(['borrowed', document.body.textContent]);
      }
    };
  </script><body>original`;
  return open(URL.createObjectURL(new Blob([html], {type:'text/html'})));
});
"#,
    )
    .expect("popup stream load fixtures should open");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__streamResults.length)",
        "2",
        "popup document.write receiver and conversion",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__streamResults.sort((a,b) => a[0].localeCompare(b[0])))")
            .unwrap(),
        r#"[["borrowed","replacement\n"],["conversion",true,"original"]]"#
    );
    assert_eq!(vm.eval("document.body.textContent").unwrap(), "opener");
    vm.eval("__streamPopups.forEach(w => w.close()); __streamMethodPopup.close();")
        .expect("close popup stream fixtures");
}

#[test]
fn popup_document_stream_policy_inherits_delivered_meta_at_creation() {
    let mut vm = new_storage_test_vm("https://popup-document-stream-meta.test/");
    vm.eval(
        r#"
const root = document.appendChild(document.createElement('html'));
const head = root.appendChild(document.createElement('head'));
const body = root.appendChild(document.createElement('body'));
globalThis.__beforeMetaPopup = open();
const meta = head.appendChild(document.createElement('meta'));
meta.httpEquiv = 'Content-Security-Policy';
meta.content = "require-trusted-types-for 'script'";
globalThis.__afterMetaPopup = open();
const anchor = body.appendChild(document.createElement('a'));
anchor.href = 'about:blank';
anchor.target = 'meta-anchor-popup';
anchor.click();
meta.remove();
"#,
    )
    .expect("create popups on either side of meta policy delivery");
    let activation = vm
        .take_pending_popup_activations()
        .into_iter()
        .find(|activation| activation.target_name() == "meta-anchor-popup")
        .expect("anchor should create a popup");
    let popup_id = activation.popup_id().expect("native popup id");
    vm.with_default_context_scope(|scope, host_ptr| {
        let popup = unsafe { &*host_ptr }
            .lightweight_popup_window(scope, popup_id)
            .expect("anchor popup Window");
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "__anchorMetaPopup").unwrap();
        assert_eq!(global.set(scope, key.into(), popup.into()), Some(true));
        Ok(())
    })
    .unwrap();
    let result = vm
        .eval(
            r#"
(() => {
  const popups = [__beforeMetaPopup, __afterMetaPopup, __anchorMetaPopup];
  try {
    return JSON.stringify(popups.map(w => {
      let error = null;
      try { w.document.write('plain'); } catch (caught) { error = caught.name; }
      return [error, w.document.body.textContent];
    }));
  } finally { popups.forEach(w => w.close()); }
})()
"#,
        )
        .expect("meta inheritance should preserve each popup's own policy snapshot");
    assert_eq!(
        result,
        r#"[[null,"plain"],["TypeError",""],["TypeError",""]]"#
    );
}

#[tokio::test]
async fn nested_popup_document_stream_inherits_its_creator_meta_policy() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://popup-document-stream-nested-meta.test/",
        "<!doctype html><body>opener</body>",
        &loader,
    );
    vm.eval(
        r#"
window.__nestedMetaResult = 'pending';
const html = `<!doctype html><head>
  <meta http-equiv="Content-Security-Policy" content="require-trusted-types-for 'script'">
  <script>
    onload = () => {
      const child = open();
      let error = null;
      try { child.document.write('blocked'); } catch (caught) { error = caught.name; }
      opener.__nestedMetaResult = JSON.stringify([error, child.document.body.textContent]);
      child.close();
    };
  </script></head><body>popup`;
window.__nestedMetaCreator = open(URL.createObjectURL(new Blob([html], {type:'text/html'})));
"#,
    )
    .expect("create popup with its own meta policy");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__nestedMetaResult",
        r#"["TypeError",""]"#,
        "nested popup meta policy inheritance",
    )
    .await;
    assert_eq!(vm.eval("document.body.textContent").unwrap(), "opener");
    vm.eval("__nestedMetaCreator.close();").unwrap();
}
