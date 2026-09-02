use super::*;

#[test]
fn dialog_show_focuses_autofocus_descendant_only_once() {
    let mut vm = new_storage_test_vm("https://dialog-autofocus-once.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const host = document.body || root.appendChild(document.createElement('body'));
  const dialog = document.createElement('dialog');
  dialog.innerHTML = '<input><input id=target autofocus>';
  host.appendChild(dialog);
  dialog.show();
  const focusedOnShow = document.activeElement === dialog.querySelector('#target');
  document.activeElement.blur();
  return JSON.stringify({ focusedOnShow, activeAfterBlur: document.activeElement.localName });
})()
"#,
        )
        .expect("dialog autofocus-once probe should evaluate");

    assert_eq!(result, r#"{"focusedOnShow":true,"activeAfterBlur":"body"}"#);
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, runtime_ptr| {
        assert!(
            !crate::native_bridge::element::post_parse_autofocus_is_pending(unsafe {
                &*runtime_ptr
            })
        );
        assert!(!crate::native_bridge::element::process_post_parse_autofocus(scope, runtime_ptr));
        Ok(())
    })
    .expect("processed dialog autofocus should suppress post-parse autofocus");
    assert_eq!(
        vm.eval("document.activeElement.localName")
            .expect("active element should remain readable"),
        "body"
    );
}

#[test]
fn dialog_show_without_focus_delegate_consumes_document_autofocus() {
    let mut vm = new_storage_test_vm("https://dialog-autofocus-consumed.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const host = document.body || root.appendChild(document.createElement('body'));
  const dialog = document.createElement('dialog');
  host.appendChild(dialog);
  dialog.show();
  const focusedDialog = document.activeElement === dialog;
  dialog.close();

  const input = document.createElement('input');
  input.id = 'later';
  input.autofocus = true;
  host.insertBefore(input, dialog);
  return JSON.stringify({ focusedDialog, focusedLater: document.activeElement === input });
})()
"#,
        )
        .expect("dialog autofocus-consumption probe should evaluate");

    assert_eq!(result, r#"{"focusedDialog":true,"focusedLater":false}"#);
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, runtime_ptr| {
        assert!(
            !crate::native_bridge::element::post_parse_autofocus_is_pending(unsafe {
                &*runtime_ptr
            })
        );
        assert!(!crate::native_bridge::element::process_post_parse_autofocus(scope, runtime_ptr));
        Ok(())
    })
    .expect("dialog focusing should consume later post-parse autofocus");
    assert_eq!(
        vm.eval("document.activeElement === document.querySelector('#later')")
            .expect("later autofocus state should remain readable"),
        "false"
    );
}

#[test]
fn dialog_show_skips_hidden_and_nonsequential_descendants() {
    let mut vm = new_storage_test_vm("https://dialog-focus-delegate.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const host = document.body || root.appendChild(document.createElement('body'));
  const dialog = document.createElement('dialog');
  dialog.innerHTML = `
    <dialog><button autofocus></button></dialog>
    <button hidden autofocus></button>
    <button tabindex=-1></button>
    <button id=target></button>`;
  host.appendChild(dialog);
  dialog.show();
  const focusedSequential = document.activeElement === dialog.querySelector('#target');
  dialog.querySelector('#target').remove();
  dialog.close();
  dialog.show();
  return JSON.stringify({
    focusedSequential,
    focusedDialogFallback: document.activeElement === dialog
  });
})()
"#,
        )
        .expect("dialog focus-delegate probe should evaluate");

    assert_eq!(
        result,
        r#"{"focusedSequential":true,"focusedDialogFallback":true}"#
    );
}

#[test]
fn inert_modal_dialog_clears_focus_outside_its_subtree() {
    let mut vm = new_storage_test_vm("https://inert-modal-focus-fixup.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const host = document.body || root.appendChild(document.createElement('body'));
  const outer = document.createElement('input');
  const dialog = document.createElement('dialog');
  dialog.inert = true;
  dialog.innerHTML = '<input autofocus>';
  host.append(outer, dialog);
  outer.focus();
  dialog.showModal();
  return JSON.stringify({
    focusedBody: document.activeElement === document.body,
    dialogOpen: dialog.open
  });
})()
"#,
        )
        .expect("inert modal focus-fixup probe should evaluate");

    assert_eq!(result, r#"{"focusedBody":true,"dialogOpen":true}"#);
}

#[test]
fn closing_dialog_restores_its_previously_focused_element() {
    let mut vm = new_storage_test_vm("https://dialog-focus-restoration.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const before = document.createElement('button');
  const outside = document.createElement('button');
  const dialog = document.createElement('dialog');
  const shadowHost = document.createElement('div');
  const shadow = shadowHost.attachShadow({mode: 'open'});
  const shadowButton = document.createElement('button');
  shadow.appendChild(shadowButton);
  dialog.appendChild(shadowHost);
  body.append(before, outside, dialog);

  before.focus();
  dialog.show();
  shadowButton.focus();
  dialog.close();
  const shadowRestore = document.activeElement === before;

  before.focus();
  dialog.show();
  outside.focus();
  dialog.close();
  const outsidePreserved = document.activeElement === outside;

  const slotHost = document.createElement('div');
  const slotShadow = slotHost.attachShadow({mode: 'open'});
  slotShadow.innerHTML = '<dialog><slot></slot></dialog>';
  const slottedButton = document.createElement('button');
  slotHost.appendChild(slottedButton);
  body.appendChild(slotHost);
  const slotDialog = slotShadow.querySelector('dialog');
  before.focus();
  slotDialog.show();
  slottedButton.focus();
  slotDialog.close();
  const slottedRestore = document.activeElement === before;

  const modal = document.createElement('dialog');
  body.appendChild(modal);
  before.focus();
  modal.showModal();
  modal.blur();
  const blurredToBody = document.activeElement === body;
  modal.close();
  const modalRestore = document.activeElement === before;

  outside.focus();
  modal.show();
  const dialogFocused = document.activeElement === modal;
  modal.close();
  const dialogRestore = document.activeElement === outside;

  return [
    shadowRestore,
    outsidePreserved,
    slottedRestore,
    blurredToBody,
    modalRestore,
    dialogFocused,
    dialogRestore
  ].join('|');
})()
"#,
        )
        .expect("dialog focus restoration probe should evaluate");

    assert_eq!(result, "true|true|true|true|true|true|true");
}

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

#[tokio::test]
async fn dialog_request_close_ignores_recursive_cancel_and_queues_close() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://dialog-request-close-recursive.test/",
        &loader,
    );

    let before_close = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const dialog = document.createElement('dialog');
  host.appendChild(dialog);
  globalThis.__dialogRequestCloseEvents = [];
  let cancelCount = 0;
  dialog.addEventListener('cancel', event => {
    cancelCount += 1;
    __dialogRequestCloseEvents.push(`${event.type}:${event.bubbles}:${event.cancelable}`);
    dialog.requestClose('nested');
  });
  dialog.addEventListener('close', event => {
    __dialogRequestCloseEvents.push(`${event.type}:${event.bubbles}:${event.cancelable}`);
  });

  dialog.showModal();
  dialog.requestClose('outer');
  return JSON.stringify({
    length: dialog.requestClose.length,
    cancelCount,
    open: dialog.open,
    returnValue: dialog.returnValue,
    events: __dialogRequestCloseEvents
  });
})()
"#,
        )
        .expect("recursive dialog requestClose setup should evaluate");

    assert_eq!(
        before_close,
        r#"{"length":1,"cancelCount":1,"open":false,"returnValue":"outer","events":["cancel:false:true"]}"#
    );

    assert!(
        !vm.has_ready_timeout(),
        "dialog requestClose must not create synthetic Page timers"
    );
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle,
            &loader,
        )
        .await
        .expect("coalesced dialog toggle event should run")
    );
    assert!(
        vm.run_one_user_interaction_executor_turn(&loader)
            .await
            .expect("queued dialog close event should run")
    );
    let after_close = vm
        .eval("JSON.stringify(__dialogRequestCloseEvents)")
        .expect("dialog requestClose event log should evaluate");
    assert_eq!(after_close, r#"["cancel:false:true","close:false:false"]"#);
}

#[test]
fn dialog_request_close_honors_cancellation_and_active_document() {
    let mut vm = new_storage_test_vm("https://dialog-request-close-state.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const dialog = document.createElement('dialog');
  host.appendChild(dialog);
  dialog.returnValue = 'seed';
  dialog.show();
  dialog.addEventListener('cancel', event => event.preventDefault(), { once: true });
  dialog.requestClose('blocked');
  const canceled = [dialog.open, dialog.returnValue];
  dialog.requestClose('accepted');
  const accepted = [dialog.open, dialog.returnValue];

  const disconnected = document.createElement('dialog');
  disconnected.open = true;
  disconnected.requestClose('disconnected');

  const inactiveDocument = document.implementation.createHTMLDocument('');
  const inactive = inactiveDocument.createElement('dialog');
  inactiveDocument.body.appendChild(inactive);
  inactive.open = true;
  inactive.requestClose('inactive');

  return JSON.stringify({
    canceled,
    accepted,
    disconnected: [disconnected.open, disconnected.returnValue],
    inactive: [inactive.open, inactive.returnValue]
  });
})()
"#,
        )
        .expect("dialog requestClose state probe should evaluate");

    assert_eq!(
        result,
        r#"{"canceled":[true,"seed"],"accepted":[false,"accepted"],"disconnected":[true,""],"inactive":[true,""]}"#
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
