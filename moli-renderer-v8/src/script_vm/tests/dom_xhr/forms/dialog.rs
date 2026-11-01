use super::*;

#[tokio::test]
async fn dialog_toggle_events_cancel_opening_and_coalesce_state_changes() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://dialog-toggle-events.test/",
        &loader,
    );

    let before_toggle = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const dialog = document.createElement('dialog');
  host.appendChild(dialog);
  globalThis.__dialogToggleEvents = [];
  globalThis.__dialogToggleTarget = dialog;

  dialog.addEventListener('beforetoggle', event => {
    __dialogToggleEvents.push(`cancel:${event.oldState}->${event.newState}:${dialog.open}:${event.cancelable}`);
    event.preventDefault();
  }, { once: true });
  dialog.show();
  const canceledOpen = dialog.open;

  dialog.addEventListener('beforetoggle', event => {
    __dialogToggleEvents.push(`before:${event.oldState}->${event.newState}:${dialog.open}:${event.cancelable}`);
  });
  dialog.addEventListener('toggle', event => {
    __dialogToggleEvents.push(`toggle:${event.oldState}->${event.newState}:${dialog.open}:${event.cancelable}`);
  });
  dialog.show();
  dialog.close();

  return JSON.stringify({
    canceledOpen,
    open: dialog.open,
    events: __dialogToggleEvents
  });
})()
"#,
        )
        .expect("dialog toggle setup should evaluate");

    assert_eq!(
        before_toggle,
        r#"{"canceledOpen":false,"open":false,"events":["cancel:closed->open:false:true","before:closed->open:false:true","before:open->closed:true:false"]}"#
    );

    assert!(
        !vm.has_ready_timeout(),
        "dialog toggle events must not create synthetic Page timers"
    );
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle,
            &loader,
        )
        .await
        .expect("coalesced dialog toggle event should run")
    );
    let after_toggle = vm
        .eval("JSON.stringify(__dialogToggleEvents)")
        .expect("coalesced dialog toggle state should evaluate");
    assert_eq!(
        after_toggle,
        r#"["cancel:closed->open:false:true","before:closed->open:false:true","before:open->closed:true:false","toggle:closed->closed:false:false"]"#
    );
}

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
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle,
            &loader,
        )
        .await
        .expect("first coalesced dialog toggle event should run")
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
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle,
            &loader,
        )
        .await
        .expect("second coalesced dialog toggle event should run")
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
