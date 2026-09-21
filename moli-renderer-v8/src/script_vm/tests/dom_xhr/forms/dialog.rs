use super::*;

#[test]
fn dialog_show_methods_enforce_requested_state_and_active_document() {
    let mut vm = new_storage_test_vm("https://dialog-requested-state.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const outcome = callback => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return `${error.name}:${error.code}:${error instanceof DOMException}`;
    }
  };

  const disconnected = document.createElement("dialog");
  const disconnectedModal = [outcome(() => disconnected.showModal()), disconnected.open];
  disconnected.show();
  const disconnectedNonModal = [
    outcome(() => disconnected.show()),
    outcome(() => disconnected.showModal()),
    disconnected.open
  ];
  disconnected.close();

  const connected = document.createElement("dialog");
  (document.body || document.documentElement || document).appendChild(connected);
  connected.show();
  const nonModal = [
    outcome(() => connected.show()),
    outcome(() => connected.showModal()),
    connected.open
  ];
  connected.close();

  connected.showModal();
  const modal = [
    outcome(() => connected.showModal()),
    outcome(() => connected.show()),
    connected.open
  ];
  connected.close();

  const inactiveDocument = document.implementation.createHTMLDocument("");
  const inactive = inactiveDocument.createElement("dialog");
  inactiveDocument.body.appendChild(inactive);
  const inactiveModal = [outcome(() => inactive.showModal()), inactive.open];

  return JSON.stringify({
    disconnectedModal,
    disconnectedNonModal,
    nonModal,
    modal,
    inactiveModal
  });
})()
"#,
        )
        .expect("dialog show state transitions should evaluate");

    assert_eq!(
        result,
        r#"{"disconnectedModal":["InvalidStateError:11:true",false],"disconnectedNonModal":["ok","InvalidStateError:11:true",true],"nonModal":["ok","InvalidStateError:11:true",true],"modal":["ok","InvalidStateError:11:true",true],"inactiveModal":["InvalidStateError:11:true",false]}"#
    );
}

#[tokio::test]
async fn dialog_form_submission_closes_with_submitter_result_and_queues_reentrant_close_events() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://dialog-form-submission.test/",
        &loader,
    );

    let before_close_events = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const dialog = document.createElement('dialog');
  const form = document.createElement('form');
  form.method = 'dialog';
  const goodbye = document.createElement('input');
  goodbye.type = 'submit';
  goodbye.setAttribute('value', 'Goodbye');
  const hello = document.createElement('input');
  hello.type = 'submit';
  hello.setAttribute('value', 'Hello');
  form.append(goodbye, hello);
  dialog.appendChild(form);
  host.appendChild(dialog);

  globalThis.__dialogCloseEvents = [];
  dialog.returnValue = 'seed';
  dialog.close('ignored');
  const closedNoop = [dialog.returnValue, dialog.hasAttribute('returnvalue')];

  dialog.addEventListener('close', event => {
    __dialogCloseEvents.push([
      dialog.returnValue,
      event.isTrusted,
      event.bubbles,
      event.cancelable
    ]);
    if (__dialogCloseEvents.length === 1) {
      dialog.show();
      hello.click();
    }
  });

  dialog.show();
  goodbye.click();
  globalThis.__dialogProbe = {dialog, closedNoop};
  return JSON.stringify({
    closedNoop,
    open: dialog.open,
    returnValue: dialog.returnValue,
    contentAttribute: dialog.getAttribute('returnvalue'),
    events: __dialogCloseEvents
  });
})()
"#,
        )
        .expect("dialog form submission setup should evaluate");

    assert_eq!(
        before_close_events,
        r#"{"closedNoop":["seed",false],"open":false,"returnValue":"Goodbye","contentAttribute":null,"events":[]}"#
    );

    assert!(
        !vm.has_ready_timeout(),
        "dialog close must not create a synthetic Page timer"
    );
    assert!(
        vm.run_one_user_interaction_executor_turn(&loader)
            .await
            .expect("first queued dialog close event should run")
    );
    let after_first_close_event = vm
        .eval(
            r#"JSON.stringify({
  open: __dialogProbe.dialog.open,
  returnValue: __dialogProbe.dialog.returnValue,
  events: __dialogCloseEvents
})"#,
        )
        .expect("first dialog close event state should evaluate");
    assert_eq!(
        after_first_close_event,
        r#"{"open":false,"returnValue":"Hello","events":[["Goodbye",true,false,false]]}"#
    );

    assert!(
        vm.run_one_user_interaction_executor_turn(&loader)
            .await
            .expect("second queued dialog close event should run")
    );
    let after_second_close_event = vm
        .eval("JSON.stringify(__dialogCloseEvents)")
        .expect("second dialog close event state should evaluate");
    assert_eq!(
        after_second_close_event,
        r#"[["Goodbye",true,false,false],["Hello",true,false,false]]"#
    );
}

#[test]
fn dialog_form_submission_distinguishes_absent_and_empty_submitter_values() {
    let mut vm = new_storage_test_vm("https://dialog-valueless-submitter.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const dialog = document.createElement('dialog');
  const form = document.createElement('form');
  form.method = 'dialog';
  const submitter = document.createElement('button');
  submitter.type = 'submit';
  submitter.textContent = 'Close';
  form.appendChild(submitter);
  dialog.appendChild(form);
  host.appendChild(dialog);

  dialog.returnValue = 'previous';
  dialog.show();
  submitter.click();

  const absentValue = {
    open: dialog.open,
    returnValue: dialog.returnValue,
    valueAttribute: submitter.getAttribute('value')
  };

  dialog.returnValue = 'second';
  submitter.setAttribute('value', '');
  dialog.show();
  submitter.click();

  return JSON.stringify({
    absentValue,
    emptyValue: {
      open: dialog.open,
      returnValue: dialog.returnValue,
      valueAttribute: submitter.getAttribute('value')
    }
  });
})()
"#,
        )
        .expect("valueless dialog submitter probe should evaluate");

    assert_eq!(
        result,
        r#"{"absentValue":{"open":false,"returnValue":"previous","valueAttribute":null},"emptyValue":{"open":false,"returnValue":"","valueAttribute":""}}"#
    );
}

#[test]
fn dialog_escape_keydown_light_dismisses_modal_dialog() {
    let mut vm = new_storage_test_vm("https://dialog-escape-modal.test/");

    vm.eval(
        r#"
(() => {
  const d = document.createElement('dialog');
  d.id = 'modal';
  d.innerHTML = '<p>content</p>';
  (document.body || document.documentElement || document).appendChild(d);
  window.__events = [];
  d.addEventListener('cancel', () => { window.__events.push('cancel'); });
  d.addEventListener('close', () => { window.__events.push('close'); });
  document.addEventListener('keydown', e => { window.__key = e.key; });
  d.showModal();
  return d.open;
})()
"#,
    )
    .expect("modal dialog setup should evaluate");

    vm.dispatch_key_event("keydown", "Escape", "Escape", "", 0, false, false)
        .expect("Escape keydown should dispatch");

    let result = vm
        .eval(
            r#"
(() => JSON.stringify({
  open: document.getElementById('modal').open,
  key: window.__key,
  events: window.__events
}))()
"#,
        )
        .expect("modal Escape result should evaluate");
    let value: serde_json::Value = serde_json::from_str(&result).expect("json");
    assert_eq!(value["open"], serde_json::json!(false));
    assert_eq!(value["key"], serde_json::json!("Escape"));
    assert!(
        value["events"]
            .as_array()
            .expect("events array")
            .contains(&serde_json::json!("cancel")),
        "cancel must fire before Escape closes a modal dialog; got {result}"
    );
}

#[test]
fn dialog_escape_keydown_respects_prevent_default() {
    let mut vm = new_storage_test_vm("https://dialog-escape-prevent-default.test/");

    vm.eval(
        r#"
(() => {
  const host = document.body || document.documentElement || document;
  window.__cancels = [];
  document.addEventListener('keydown', e => {
    window.__key = e.key;
    if (window.__blockEscape) { e.preventDefault(); }
  });
  function make(id, preventCancel) {
    const d = document.createElement('dialog');
    d.id = id;
    host.appendChild(d);
    if (preventCancel) {
      d.addEventListener('cancel', e => { window.__cancels.push(id); e.preventDefault(); });
    } else {
      d.addEventListener('cancel', () => { window.__cancels.push(id); });
    }
    d.showModal();
    return d;
  }
  window.__make = make;
})()
"#,
    )
    .expect("prevent-default dialog setup should evaluate");

    // A keydown preventDefault suppresses the whole light-dismiss.
    vm.eval("window.__blockEscape = true; window.__make('kd', false);")
        .expect("blocked dialog");
    vm.dispatch_key_event("keydown", "Escape", "Escape", "", 0, false, false)
        .expect("blocked Escape keydown should dispatch");
    let blocked = vm
        .eval(
            r#"(JSON.stringify({ open: document.getElementById('kd').open, cancels: window.__cancels }))"#,
        )
        .expect("blocked result should evaluate");
    assert_eq!(
        blocked, r#"{"open":true,"cancels":[]}"#,
        "preventing the keydown must suppress the dialog light-dismiss"
    );

    // A cancel preventDefault fires cancel but keeps the dialog open.
    vm.eval("window.__blockEscape = false; window.__make('cc', true);")
        .expect("cancel-catch dialog");
    vm.dispatch_key_event("keydown", "Escape", "Escape", "", 0, false, false)
        .expect("cancel-caught Escape keydown should dispatch");
    let cancel_caught = vm
        .eval(
            r#"(JSON.stringify({ open: document.getElementById('cc').open, cancels: window.__cancels }))"#,
        )
        .expect("cancel-caught result should evaluate");
    assert_eq!(
        cancel_caught, r#"{"open":true,"cancels":["cc"]}"#,
        "a prevented cancel must keep the dialog open"
    );
}

#[test]
fn dialog_escape_keydown_does_not_close_non_modal_dialog() {
    let mut vm = new_storage_test_vm("https://dialog-escape-non-modal.test/");

    vm.eval(
        r#"
(() => {
  const d = document.createElement('dialog');
  d.id = 'plain';
  (document.body || document.documentElement || document).appendChild(d);
  window.__cancels = 0;
  d.addEventListener('cancel', () => { window.__cancels += 1; });
  d.show();
  return d.open;
})()
"#,
    )
    .expect("non-modal dialog setup should evaluate");

    vm.dispatch_key_event("keydown", "Escape", "Escape", "", 0, false, false)
        .expect("Escape keydown should dispatch");

    let result = vm
        .eval(
            r#"(JSON.stringify({ open: document.getElementById('plain').open, cancels: window.__cancels }))"#,
        )
        .expect("non-modal Escape result should evaluate");
    assert_eq!(
        result, r#"{"open":true,"cancels":0}"#,
        "Escape must not light-dismiss a non-modal dialog"
    );
}
