use super::*;

#[test]
fn popup_and_windowless_hyperlink_clicks_do_not_rewrite_reflected_attributes() {
    let mut vm = new_storage_test_vm("https://popup-hyperlink-attributes.test/path/index.html");
    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  if (!document.body) root.appendChild(document.createElement('body'));
  const popup = open();
  try {
    const docs = [document, popup.document,
      document.implementation.createHTMLDocument(''),
      new DOMParser().parseFromString('<!doctype html><body></body>', 'text/html')];
    const failures = [];
    for (const [index, doc] of docs.entries()) {
      for (const tag of ['a', 'area']) {
        for (const attributes of [[], [['href','../next']],
            [['href','../next'],['download','']], [['href','../next'],['download','saved.txt']]]) {
          const link = doc.createElement(tag);
          for (const [name, value] of attributes) link.setAttribute(name, value);
          doc.body.appendChild(link);
          const before = link.outerHTML;
          const observer = new MutationObserver(() => {});
          observer.observe(link, {attributes:true});
          let clicks = 0;
          link.addEventListener('click', event => {
            ++clicks;
            if (event.isTrusted || event.target !== link) failures.push([index,tag,'event']);
            event.preventDefault();
          });
          link.click();
          if (clicks !== 1 || link.outerHTML !== before || observer.takeRecords().length !== 0) {
            failures.push([index,tag,before,link.outerHTML,clicks]);
          }
          observer.disconnect();
        }
      }
    }
    return JSON.stringify(failures);
  } finally { popup.close(); }
})()
"#,
        )
        .expect("clicking a hyperlink should not change its attributes before event dispatch");
    assert_eq!(result, "[]");
    assert!(vm.take_pending_download_activations().is_empty());
}

#[test]
fn popup_hyperlink_activation_ignores_author_getters_and_uses_listener_updated_attributes() {
    let mut vm = new_storage_test_vm("https://popup-hyperlink-download.test/path/index.html");
    vm.eval("globalThis.__downloadPopup = open(); 'ready'")
        .expect("popup should open");
    vm.take_pending_popup_activations();
    let result = vm
        .eval(
            r#"
(() => {
  const doc = __downloadPopup.document;
  const states = [];
  for (const tag of ['a', 'area']) {
    for (const download of ['', 'initial.txt']) {
      const link = doc.createElement(tag);
      link.href = '/initial';
      link.download = download;
      doc.body.appendChild(link);
      let getterCalls = 0;
      for (const name of ['href', 'download']) {
        Object.defineProperty(link, name, {
          configurable:true, get() { ++getterCalls; throw new Error('author ' + name); }
        });
      }
      let clicks = 0;
      link.addEventListener('click', () => {
        ++clicks;
        link.setAttribute('href', tag + '.txt');
        if (download !== '') link.setAttribute('download', 'listener.txt');
      });
      link.click();
      states.push([getterCalls, clicks, link.getAttribute('href'), link.getAttribute('download')]);
    }
  }
  return JSON.stringify(states);
})()
"#,
        )
        .expect("activation should read native attributes after click listeners run");
    assert_eq!(
        result,
        r#"[[0,1,"a.txt",""],[0,1,"a.txt","listener.txt"],[0,1,"area.txt",""],[0,1,"area.txt","listener.txt"]]"#
    );
    let downloads = vm.take_pending_download_activations();
    assert_eq!(downloads.len(), 4);
    for (index, download) in downloads.iter().enumerate() {
        let tag = if index < 2 { "a" } else { "area" };
        assert_eq!(
            download.url,
            format!("https://popup-hyperlink-download.test/path/{tag}.txt")
        );
        assert_eq!(
            download.suggested_filename.as_deref(),
            (index % 2 == 1).then_some("listener.txt")
        );
    }
    assert!(vm.take_pending_popup_activations().is_empty());
    assert!(vm.take_pending_location_navigation_with_seed().is_none());
    vm.eval("__downloadPopup.close()").expect("close popup");
}

#[test]
fn popup_hyperlink_click_does_not_activate_an_expando_href_or_download() {
    let mut vm = new_storage_test_vm("https://popup-hyperlink-expando.test/");
    vm.eval("globalThis.__expandoPopup = open(); 'ready'")
        .expect("popup should open");
    vm.take_pending_popup_activations();
    let result = vm
        .eval(
            r#"
(() => {
  const doc = __expandoPopup.document;
  const results = [];
  for (const tag of ['a', 'area']) {
    const link = doc.createElement(tag);
    Object.defineProperty(link, 'href', {value:'https://popup-hyperlink-expando.test/fake'});
    Object.defineProperty(link, 'download', {value:'fake.txt'});
    let clicks = 0;
    link.addEventListener('click', () => ++clicks);
    doc.body.appendChild(link);
    link.click();
    results.push([clicks, link.hasAttribute('href'), link.hasAttribute('download')]);
  }
  return JSON.stringify(results);
})()
"#,
        )
        .expect("JS expando properties should not supply hyperlink activation attributes");
    assert_eq!(result, "[[1,false,false],[1,false,false]]");
    assert!(vm.take_pending_download_activations().is_empty());
    assert!(vm.take_pending_popup_activations().is_empty());
    assert!(vm.take_pending_location_navigation_with_seed().is_none());
    vm.eval("__expandoPopup.close()").expect("close popup");
}

#[tokio::test]
async fn popup_hyperlink_opens_a_nested_window_and_relays_messages_to_its_opener() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://popup-nested-hyperlink.test/",
        "<!doctype html><body></body>",
        &loader,
    );
    vm.eval(
        r#"
(() => {
  window.name = 'root';
  window.__nestedHyperlinkMessages = [];
  window.addEventListener('message', event => {
    __nestedHyperlinkMessages.push([event.data, event.source === __firstHyperlinkPopup]);
  });
  const childMarkup = `<!doctype html><script>
    window.opener.postMessage({name:window.name, openerName:window.opener.name,
      isTop:window.top === window}, '*');
  </script>`;
  const childUrl = URL.createObjectURL(new Blob([childMarkup], {type:'text/html'}));
  const firstMarkup = `<!doctype html><body><script>
    window.addEventListener('load', openNested, {once:true});
    function openNested() {
      window.addEventListener('message', event => {
        window.__nestedChild = event.source;
        window.opener.postMessage({child:event.data, name:window.name,
          openerName:window.opener.name, childOpener:event.source.opener === window}, '*');
      });
      const a = document.createElement('a');
      a.href = ${JSON.stringify(childUrl)};
      a.target = 'nested-child';
      document.body.appendChild(a);
      a.click();
      opener.__nestedHyperlinkMutated = a.hasAttribute('download');
    }
  </script>`;
  const firstUrl = URL.createObjectURL(new Blob([firstMarkup], {type:'text/html'}));
  window.__firstHyperlinkPopup = open(firstUrl, 'first-popup');
})()
"#,
    )
    .expect("nested popup fixture should open");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__nestedHyperlinkMessages.length)",
        "1",
        "nested popup hyperlink message",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify([__nestedHyperlinkMessages,__nestedHyperlinkMutated])")
            .unwrap(),
        r#"[[[{"child":{"name":"nested-child","openerName":"first-popup","isTop":true},"name":"first-popup","openerName":"root","childOpener":true},true]],false]"#,
    );
    assert!(vm.take_pending_download_activations().is_empty());
    vm.eval("__firstHyperlinkPopup.__nestedChild.close(); __firstHyperlinkPopup.close();")
        .expect("close nested popup fixtures");
}

#[test]
fn windowless_synthetic_clicks_have_null_views_without_changing_event_realms() {
    let mut vm = new_storage_test_vm("https://windowless-click-view.test/");
    vm.eval("const root = document.documentElement || document.appendChild(document.createElement('html')); if (!document.body) root.appendChild(document.createElement('body')); document.body.innerHTML='<iframe id=child></iframe>'")
        .unwrap();
    assert_eq!(
        vm.eval(&format!("({})()", include_str!("windowless_click_view.js")))
            .unwrap(),
        "[]"
    );
}

#[test]
fn synthetic_clicks_match_pointer_identity_across_realms_and_adoption() {
    let mut vm = new_storage_test_vm("https://synthetic-click-realms.test/");
    vm.eval("const root = document.documentElement || document.appendChild(document.createElement('html')); if (!document.body) root.appendChild(document.createElement('body')); document.body.innerHTML='<iframe id=child></iframe>'")
        .unwrap();
    assert_eq!(
        vm.eval(&format!(
            "({})()",
            include_str!("synthetic_click_realms.js")
        ))
        .unwrap(),
        "[]"
    );
}

#[test]
fn accesskey_clicks_use_the_focused_document_and_synthetic_pointer_identity() {
    let mut vm = new_storage_test_vm("https://accesskey-click-realms.test/");
    vm.eval(
        r#"
const root = document.documentElement || document.appendChild(document.createElement('html'));
if (!document.body) root.appendChild(document.createElement('body'));
document.body.innerHTML = '<button id=main accesskey=g>main</button><iframe id=child></iframe>';
const realms = [window, document.getElementById('child').contentWindow];
realms[1].document.body.innerHTML = '<button id=frame accesskey=g>frame</button>';
globalThis.accesskeyClicks = [];
for (let index = 0; index < realms.length; index++) {
  const w = realms[index];
  w.document.querySelector('button').addEventListener('click', event => {
    accesskeyClicks.push([
      index, event.view === w, Object.getPrototypeOf(event) === w.PointerEvent.prototype,
      event.pointerId, event.pointerType, event.isPrimary, event.isTrusted, event.detail,
      event.altKey, event.ctrlKey, event.shiftKey, event.metaKey
    ]);
  });
}
"#,
    )
    .unwrap();

    for owner in 0..2 {
        for modifiers in [1, 9, 3] {
            vm.eval(&format!(
                "accesskeyClicks=[];realms[{owner}].document.querySelector('button').focus()"
            ))
            .unwrap();
            vm.dispatch_key_event("keydown", "g", "KeyG", "", modifiers, false, false)
                .unwrap();
            let matches_platform = if cfg!(target_os = "macos") {
                modifiers == 3
            } else {
                modifiers != 3
            };
            let expected = if matches_platform {
                format!("[[{owner},true,true,-1,\"\",false,true,0,false,false,false,false]]")
            } else {
                "[]".to_owned()
            };
            assert_eq!(
                vm.eval("JSON.stringify(accesskeyClicks)").unwrap(),
                expected,
                "owner {owner}, modifiers {modifiers}"
            );
        }
    }

    assert_eq!(
        vm.eval(
            r#"(() => {
  accesskeyClicks = [];
  for (const w of realms) w.document.querySelector('button').dispatchEvent(
    new KeyboardEvent('keydown', {key: 'g', altKey: true, ctrlKey: true, shiftKey: true, bubbles: true})
  );
  return JSON.stringify(accesskeyClicks);
})()"#
        )
        .unwrap(),
        "[]"
    );
}

#[test]
fn accesskey_document_is_frozen_before_keydown_adoption() {
    for owner in 0..2 {
        for destination in ["other", "windowless"] {
            for phase in ["capture", "target", "bubble"] {
                let mut vm = new_storage_test_vm("https://accesskey-adoption.test/");
                vm.eval(&format!(
                    "globalThis.sourceOwner={owner};globalThis.destinationKind='{destination}';globalThis.listenerPhase='{phase}'"
                ))
                .unwrap();
                vm.eval(
                    r#"
const root = document.documentElement || document.appendChild(document.createElement('html'));
if (!document.body) root.appendChild(document.createElement('body'));
document.body.innerHTML = '<button accesskey=g>main</button><iframe id=child></iframe>';
const realms = [window, document.getElementById('child').contentWindow];
realms[1].document.body.innerHTML = '<button accesskey=g>child</button>';
const sourceDocument = realms[sourceOwner].document;
const destination = destinationKind === 'other' ? realms[1 - sourceOwner].document
  : sourceDocument.implementation.createHTMLDocument('');
const source = sourceDocument.body.appendChild(sourceDocument.createElement('input'));
globalThis.adopted = false;
globalThis.accesskeyClicks = [];
for (let index = 0; index < realms.length; index++) {
  const w = realms[index];
  w.document.querySelector('button').addEventListener('click', event => {
    accesskeyClicks.push([
      index, event.view === w, Object.getPrototypeOf(event) === w.PointerEvent.prototype
    ]);
  });
}
const listenerTarget = listenerPhase === 'target' ? source : sourceDocument;
listenerTarget.addEventListener('keydown', () => {
  destination.body.appendChild(destination.adoptNode(source));
  adopted = true;
}, {once: true, capture: listenerPhase === 'capture'});
source.focus();
"#,
                )
                .unwrap();
                let modifiers = if cfg!(target_os = "macos") { 3 } else { 9 };
                vm.dispatch_key_event("keydown", "g", "KeyG", "", modifiers, false, false)
                    .unwrap();
                assert_eq!(
                    vm.eval("JSON.stringify([adopted, source.ownerDocument === destination, accesskeyClicks])")
                        .unwrap(),
                    format!("[true,true,[[{owner},true,true]]]"),
                    "owner {owner}, destination {destination}, phase {phase}"
                );
            }
        }
    }
}
