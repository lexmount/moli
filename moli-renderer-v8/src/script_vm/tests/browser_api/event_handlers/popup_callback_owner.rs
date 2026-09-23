use super::*;

async fn popup_callback_trace(source: &str) -> String {
    let loader = static_http_loader([]);
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://popup-callback-owner.test/", &loader);
    vm.exec(source, None).expect("popup callback setup");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupOwnerDone)",
        "true",
        "opener completion timer",
    )
    .await;
    vm.eval("JSON.stringify(__popupOwnerTrace.slice().sort())")
        .expect("popup callback trace")
}

#[tokio::test]
async fn popup_pagehide_preserves_opener_callback_timers() {
    let result = popup_callback_trace(
        r#"
globalThis.__popupOwnerTrace = [];
globalThis.__popupOwnerDone = false;
const popup = open('about:blank');
popup.onpagehide = function(event) {
  __popupOwnerTrace.push('handler:' + (this === popup && event.currentTarget === popup));
  setTimeout(function() { __popupOwnerTrace.push('function:' + (this === window)); }, 0);
  setTimeout("globalThis.__popupOwnerTrace.push('source')", 0);
  const interval = setInterval(function() {
    clearInterval(interval);
    __popupOwnerTrace.push('interval:' + (this === window));
  }, 0);
  Promise.resolve().then(() => setTimeout(() => __popupOwnerTrace.push('microtask'), 0));
  popup.setTimeout(() => __popupOwnerTrace.push('unexpected popup timer'), 0);
};
const listener = {get handleEvent() {
  setTimeout(() => __popupOwnerTrace.push('getter'), 0);
  return function() {
    setTimeout(() => __popupOwnerTrace.push('object:' + (this === listener)), 0);
  };
}};
popup.addEventListener('pagehide', listener);
popup.close();
setTimeout(() => { __popupOwnerDone = true; }, 100);
"#,
    )
    .await;
    assert_eq!(
        result,
        r#"["function:true","getter","handler:true","interval:true","microtask","object:true","source"]"#
    );
}

#[tokio::test]
async fn popup_pagehide_keeps_each_callback_window_owner() {
    let result = popup_callback_trace(
        r#"
globalThis.__popupOwnerTrace = [];
globalThis.__popupOwnerDone = false;
globalThis.__closingPopup = open('about:blank');
const sibling = open('about:blank');
__closingPopup.document.open();
__closingPopup.document.write(`<script>
  addEventListener('pagehide', function() {
    opener.__popupOwnerTrace.push('closing-handler');
    setTimeout(() => opener.__popupOwnerTrace.push('unexpected closing timer'), 0);
  });
<\/script>`);
__closingPopup.document.close();
sibling.document.open();
sibling.document.write(`<script>
  opener.__closingPopup.addEventListener('pagehide', function() {
    opener.__popupOwnerTrace.push('sibling-handler:' + (this === opener.__closingPopup));
    setTimeout(function() {
      opener.__popupOwnerTrace.push('sibling-timer:' + (this === window));
    }, 0);
  });
<\/script>`);
sibling.document.close();
__closingPopup.close();
setTimeout(() => { __popupOwnerDone = true; }, 100);
"#,
    )
    .await;
    assert_eq!(
        result,
        r#"["closing-handler","sibling-handler:true","sibling-timer:true"]"#
    );
}

#[tokio::test]
async fn popup_event_errors_follow_the_callback_window() {
    let result = popup_callback_trace(
        r#"
globalThis.__popupOwnerTrace = [];
globalThis.__popupOwnerDone = true;
const popup = open('about:blank');
window.addEventListener('error', e => {
  if (e.message.includes('owner-marker')) {
    __popupOwnerTrace.push('opener:' + e.error.message);
    e.preventDefault();
  }
});
popup.addEventListener('error', e => {
  __popupOwnerTrace.push('popup:' + e.error.message);
  e.preventDefault();
});
popup.addEventListener('probe', () => {throw new Error('owner-marker-opener')});
popup.dispatchEvent(new Event('probe'));
popup.document.open();
popup.document.write(`<script>
  addEventListener('probe', () => {throw new Error('owner-marker-popup')});
<\/script>`);
popup.document.close();
popup.addEventListener('error', e => {
  __popupOwnerTrace.push('popup:' + e.error.message);
  e.preventDefault();
});
popup.dispatchEvent(new Event('probe'));
popup.close();
"#,
    )
    .await;
    assert_eq!(
        result,
        r#"["opener:owner-marker-opener","popup:owner-marker-popup"]"#
    );
}
