use super::*;

// Match the real-input Chromium probe: Escape itself does not grant history
// activation, so establish it with an ordinary trusted key first.
#[cfg(feature = "devtools")]
fn press_dialog_escape(vm: &mut ScriptVm) {
    for (event, key, code) in [
        ("keydown", "a", "KeyA"),
        ("keyup", "a", "KeyA"),
        ("keydown", "Escape", "Escape"),
        ("keyup", "Escape", "Escape"),
    ] {
        vm.dispatch_key_event(event, key, code, "", 0, false, false)
            .unwrap();
    }
}

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
            !crate::native_bridge::element::post_parse_autofocus_is_pending(
                unsafe { &*runtime_ptr },
                unsafe { &*runtime_ptr }.document_handle()
            )
        );
        assert!(
            !crate::native_bridge::element::process_post_parse_autofocus(
                scope,
                runtime_ptr,
                unsafe { &*runtime_ptr }.document_handle()
            )
        );
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
            !crate::native_bridge::element::post_parse_autofocus_is_pending(
                unsafe { &*runtime_ptr },
                unsafe { &*runtime_ptr }.document_handle()
            )
        );
        assert!(
            !crate::native_bridge::element::process_post_parse_autofocus(
                scope,
                runtime_ptr,
                unsafe { &*runtime_ptr }.document_handle()
            )
        );
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

    vm.eval("globalThis.__dialogTimerControl = setTimeout(() => {}, 0); true")
        .expect("callback timer control should evaluate");
    assert!(vm.has_ready_callback_timer());
    vm.eval("clearTimeout(__dialogTimerControl); requestAnimationFrame(() => {}); true")
        .expect("rendering wake control should evaluate");
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert!(vm.has_ready_timeout(), "the rendering wake must be ready");

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
        !vm.has_ready_callback_timer(),
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
async fn dialog_commands_preserve_invoker_sources_through_close_requests() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://dialog-command-source.test/",
        &loader,
    );

    let opened = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const dialog = document.createElement('dialog');
  const opener = document.createElement('button');
  const closer = document.createElement('button');
  host.append(opener, dialog);
  dialog.appendChild(closer);
  opener.id = 'opener';
  opener.commandForElement = dialog;
  opener.command = 'show-modal';
  closer.id = 'closer';
  closer.commandForElement = dialog;
  closer.command = 'request-close';
  closer.value = 'initial';
  dialog.returnValue = 'seed';

  const events = [];
  for (const type of ['beforetoggle', 'toggle']) {
    dialog.addEventListener(type, event => {
      events.push([event.type, event.oldState, event.newState, event.source?.id]);
    });
  }
  dialog.addEventListener('cancel', () => events.push(['cancel']));
  dialog.addEventListener('close', () => events.push(['close', dialog.returnValue]));
  globalThis.dialogCommandProbe = {dialog, closer, events};
  opener.click();
  return dialog.open && dialog.matches(':modal');
})()
"#,
        )
        .expect("opening command should evaluate");
    assert_eq!(opened, "true");
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle,
            &loader,
        )
        .await
        .expect("opening toggle should run")
    );

    let requested = vm
        .eval(
            r#"
(() => {
  const {dialog, closer} = dialogCommandProbe;
  dialog.addEventListener('cancel', event => event.preventDefault(), {once: true});
  closer.click();
  const canceled = [dialog.open, dialog.returnValue];
  dialog.addEventListener('command', () => {
    closer.command = 'close';
    closer.value = 'from-command';
  }, {once: true});
  dialog.addEventListener('cancel', () => closer.value = 'from-cancel', {once: true});
  closer.click();
  return JSON.stringify([canceled, dialog.open, dialog.returnValue]);
})()
"#,
        )
        .expect("request-close command should evaluate");
    assert_eq!(requested, r#"[[true,"seed"],false,"from-command"]"#);
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle,
            &loader,
        )
        .await
        .expect("closing toggle should run")
    );
    assert!(
        vm.run_one_user_interaction_executor_turn(&loader)
            .await
            .expect("close event should run")
    );
    assert_eq!(
        vm.eval("JSON.stringify(dialogCommandProbe.events)")
            .expect("dialog command events should evaluate"),
        r#"[["beforetoggle","closed","open","opener"],["toggle","closed","open","opener"],["cancel"],["cancel"],["beforetoggle","open","closed","closer"],["toggle","open","closed","closer"],["close","from-command"]]"#
    );
}

#[test]
fn dialog_close_command_keeps_the_result_of_a_reentrant_close() {
    let mut vm = new_storage_test_vm("https://dialog-close-command-reentry.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const dialog = document.createElement('dialog');
  const button = document.createElement('button');
  host.appendChild(dialog);
  dialog.appendChild(button);
  button.commandForElement = dialog;
  button.command = 'close';
  button.value = 'outer';
  dialog.show();
  dialog.addEventListener('beforetoggle', () => dialog.close('inner'), {once: true});
  button.click();
  return JSON.stringify([dialog.open, dialog.returnValue]);
})()
"#,
        )
        .expect("reentrant close should evaluate");
    assert_eq!(result, r#"[false,"inner"]"#);
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
        !vm.has_ready_callback_timer(),
        "dialog requestClose must not create a callback timer; focus rendering wakes are separate"
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
        !vm.has_ready_callback_timer(),
        "dialog close must not create a callback timer; focus rendering wakes are separate"
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

#[cfg(feature = "devtools")]
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

    press_dialog_escape(&mut vm);

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

#[cfg(feature = "devtools")]
#[test]
fn dialog_escape_keydown_respects_prevent_default() {
    let mut vm = new_storage_test_vm("https://dialog-escape-prevent-default.test/");
    vm.eval("const root = document.documentElement || document.appendChild(document.createElement('html')); if (!document.body) root.appendChild(document.createElement('body'))")
        .unwrap();

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
    press_dialog_escape(&mut vm);
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
    press_dialog_escape(&mut vm);
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

#[cfg(feature = "devtools")]
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

    press_dialog_escape(&mut vm);

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

#[cfg(feature = "devtools")]
#[test]
fn dialog_escape_follows_modal_open_order() {
    for (options, expected) in [
        (r#"{}"#, r#"{"open":[true,false],"cancels":["second"]}"#),
        (
            r#"{"openOrder":["second","first"]}"#,
            r#"{"open":[false,true],"cancels":["first"]}"#,
        ),
        (
            r#"{"creationOrder":["second","first"]}"#,
            r#"{"open":[true,false],"cancels":["second"]}"#,
        ),
        (
            r#"{"creationOrder":["second","first"],"openOrder":["second","first"]}"#,
            r#"{"open":[false,true],"cancels":["first"]}"#,
        ),
        (
            r#"{"reopen":"first"}"#,
            r#"{"open":[false,true],"cancels":["first"]}"#,
        ),
        (
            r#"{"creationOrder":["second","first"],"openOrder":["second","first"],"reopen":"second"}"#,
            r#"{"open":[true,false],"cancels":["second"]}"#,
        ),
        (
            r#"{"repeat":"first"}"#,
            r#"{"open":[true,false],"cancels":["second"]}"#,
        ),
        (
            r#"{"openOrder":["second","first"],"repeat":"second"}"#,
            r#"{"open":[false,true],"cancels":["first"]}"#,
        ),
    ] {
        let mut vm = new_storage_test_vm("https://dialog-modal-order.test/");
        vm.eval(&format!(
            "globalThis.dialogProbe = ({})({options})",
            include_str!("dialog_escape_order.js")
        ))
        .unwrap();
        press_dialog_escape(&mut vm);
        assert_eq!(
            vm.eval("JSON.stringify(dialogProbe.state())").unwrap(),
            expected,
            "options {options}"
        );
    }
}

#[cfg(feature = "devtools")]
#[test]
fn dialog_escape_cancellation_preserves_modal_order() {
    let mut vm = new_storage_test_vm("https://dialog-cancel-modal-order.test/");
    vm.eval(&format!(
        "globalThis.dialogProbe = ({})({{openOrder:['second','first'], blockKeydown:true}})",
        include_str!("dialog_escape_order.js")
    ))
    .unwrap();
    for (prepare, expected) in [
        ("", r#"{"open":[true,true],"cancels":[]}"#),
        (
            "dialogProbe.blockKeydown=false;dialogProbe.blockCancel=true",
            r#"{"open":[true,true],"cancels":["first"]}"#,
        ),
        (
            "dialogProbe.blockCancel=false",
            r#"{"open":[false,true],"cancels":["first","first"]}"#,
        ),
        (
            "",
            r#"{"open":[false,false],"cancels":["first","first","second"]}"#,
        ),
    ] {
        vm.eval(prepare).unwrap();
        press_dialog_escape(&mut vm);
        assert_eq!(
            vm.eval("JSON.stringify(dialogProbe.state())").unwrap(),
            expected
        );
    }
}

#[cfg(feature = "devtools")]
#[test]
fn dialog_escape_is_scoped_to_keyboard_dispatch_document() {
    for owner in 0..2 {
        let mut vm = new_storage_test_vm("https://dialog-escape-documents.test/");
        vm.eval(
            r#"
const root = document.documentElement || document.appendChild(document.createElement('html'));
if (!document.body) root.appendChild(document.createElement('body'));
document.body.innerHTML = '<dialog id=parent><button type=button>Continue</button><iframe id=child></iframe></dialog>';
const realms = [window, document.getElementById('child').contentWindow];
globalThis.dialogs = [];
globalThis.cancels = [];
for (let i = 0; i < realms.length; i++) {
  const dialog = i === 0 ? document.getElementById('parent')
    : realms[i].document.createElement('dialog');
  if (i !== 0) {
    dialog.innerHTML = '<button type=button>Continue</button>';
    realms[i].document.body.appendChild(dialog);
  }
  dialog.addEventListener('cancel', () => cancels.push(i));
  dialog.showModal();
  dialogs.push(dialog);
}
"#,
        )
        .unwrap();
        vm.eval(&format!("dialogs[{owner}].querySelector('button').focus()"))
            .unwrap();
        press_dialog_escape(&mut vm);
        assert_eq!(
            vm.eval("JSON.stringify([dialogs.map(dialog => dialog.open), cancels])")
                .unwrap(),
            if owner == 0 {
                "[[false,true],[0]]"
            } else {
                "[[true,false],[1]]"
            }
        );
    }
}

#[cfg(feature = "devtools")]
#[test]
fn dialog_escape_untrusted_keydown_keeps_modal_order() {
    let mut vm = new_storage_test_vm("https://dialog-untrusted-escape.test/");
    vm.eval(&format!(
        "globalThis.dialogProbe = ({})({{openOrder:['second','first']}})",
        include_str!("dialog_escape_order.js")
    ))
    .unwrap();
    assert_eq!(
        vm.eval(
            "dialogProbe.dialogs.first.dispatchEvent(new KeyboardEvent('keydown', {key:'Escape', code:'Escape', bubbles:true, cancelable:true})); JSON.stringify(dialogProbe.state())"
        )
        .unwrap(),
        r#"{"open":[true,true],"cancels":[]}"#
    );
    press_dialog_escape(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify(dialogProbe.state())").unwrap(),
        r#"{"open":[false,true],"cancels":["first"]}"#
    );
}

#[cfg(feature = "devtools")]
#[test]
fn dialog_escape_uses_internal_keyboard_key() {
    for replace_prototype in [false, true] {
        for (key, code, spoof, expected_open, expected_cancels) in [
            ("a", "KeyA", "Escape", true, 0),
            ("Escape", "Escape", "x", false, 1),
        ] {
            let mut vm = new_storage_test_vm("https://dialog-escape-internal-key.test/");
            vm.eval(&format!(
                r#"
globalThis.dialogProbe = ({fixture})({{openOrder:['first']}});
globalThis.keyReads = 0;
document.addEventListener('keydown', event => {{
  const target = {replace_prototype} ? KeyboardEvent.prototype : event;
  Object.defineProperty(target, 'key', {{configurable:true, get() {{
    keyReads++;
    return '{spoof}';
  }}}});
}});
"#,
                fixture = include_str!("dialog_escape_order.js"),
            ))
            .unwrap();
            vm.dispatch_key_event("keydown", key, code, "", 0, false, false)
                .unwrap();
            assert_eq!(
                vm.eval("JSON.stringify([dialogProbe.dialogs.first.open, dialogProbe.state().cancels.length, keyReads])")
                    .unwrap(),
                format!("[{expected_open},{expected_cancels},0]"),
                "key {key}, public key {spoof}, prototype override {replace_prototype}"
            );
        }
    }
}

#[cfg(feature = "devtools")]
#[tokio::test]
async fn dialog_escape_cancel_reentry_is_bound_to_original_opening() {
    for (action, expected, queues_close) in [
        ("dialog.close(); dialog.showModal()", "[true,false]", true),
        ("dialog.close(); dialog.show()", "[true,false]", true),
        ("dialog.close()", "[false,false]", true),
        (
            "dialogProbe.dialogs.second.showModal()",
            "[false,true]",
            true,
        ),
        ("dialog.showModal()", "[false,false]", true),
        (
            "dialog.removeAttribute('open'); dialog.setAttribute('open', '')",
            "[true,false]",
            false,
        ),
        (
            "dialog.setAttribute('open', 'still-open')",
            "[false,false]",
            true,
        ),
        (
            "dialog.setAttributeNS('urn:probe', 'x:open', ''); dialog.removeAttributeNS('urn:probe', 'open')",
            "[false,false]",
            true,
        ),
    ] {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(
            "https://dialog-escape-reentrant-cancel.test/",
            &loader,
        );
        vm.eval(&format!(
            r#"
globalThis.dialogProbe = ({fixture})({{openOrder:['first']}});
dialogProbe.dialogs.first.addEventListener('cancel', () => {{
  const dialog = dialogProbe.dialogs.first;
  {action};
}}, {{once:true}});
"#,
            fixture = include_str!("dialog_escape_order.js"),
        ))
        .unwrap();
        press_dialog_escape(&mut vm);
        assert_eq!(
            vm.eval(
                "JSON.stringify([dialogProbe.dialogs.first.open, dialogProbe.dialogs.second.open])"
            )
            .unwrap(),
            expected,
            "cancel action: {action}"
        );
        assert_eq!(
            vm.run_one_user_interaction_executor_turn(&loader)
                .await
                .unwrap(),
            queues_close,
            "cancel action: {action}"
        );
        assert_eq!(
            vm.eval("JSON.stringify(dialogProbe.events.map(event => event.type))")
                .unwrap(),
            if queues_close {
                r#"["cancel","close"]"#
            } else {
                r#"["cancel"]"#
            },
            "cancel action: {action}"
        );
        assert!(
            !vm.run_one_user_interaction_executor_turn(&loader)
                .await
                .unwrap(),
            "cancel action must not queue a second close: {action}"
        );
    }
}

#[cfg(feature = "devtools")]
#[test]
fn dialog_escape_after_adoption_uses_destination_registration() {
    for explicit_removal in [false, true] {
        let mut vm = new_storage_test_vm("https://dialog-escape-adoption.test/");
        vm.eval(&format!(
            r#"
globalThis.dialogProbe = ({fixture})({{openOrder:['first']}});
const frame = document.body.appendChild(document.createElement('iframe'));
const dialog = dialogProbe.dialogs.first;
if ({explicit_removal}) dialog.remove();
frame.contentDocument.body.appendChild(dialog);
dialog.removeAttribute('open');
dialog.showModal();
dialog.querySelector('button').focus();
"#,
            fixture = include_str!("dialog_escape_order.js"),
        ))
        .unwrap();
        press_dialog_escape(&mut vm);
        assert_eq!(
            vm.eval("JSON.stringify(dialogProbe.state())").unwrap(),
            r#"{"open":[false,false],"cancels":["first"]}"#
        );
    }
}

#[cfg(feature = "devtools")]
#[test]
fn dialog_escape_after_atomic_move_keeps_original_opening_order() {
    let mut vm = new_storage_test_vm("https://dialog-escape-atomic-move.test/");
    vm.eval(&format!(
        r#"
globalThis.dialogProbe = ({fixture})({{}});
const destination = document.body.appendChild(document.createElement('div'));
destination.moveBefore(dialogProbe.dialogs.first, null);
dialogProbe.dialogs.second.querySelector('button').focus();
"#,
        fixture = include_str!("dialog_escape_order.js"),
    ))
    .unwrap();
    press_dialog_escape(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify(dialogProbe.state())").unwrap(),
        r#"{"open":[true,false],"cancels":["second"]}"#
    );
    vm.eval("dialogProbe.dialogs.first.querySelector('button').focus()")
        .unwrap();
    press_dialog_escape(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify(dialogProbe.state())").unwrap(),
        r#"{"open":[false,false],"cancels":["second","first"]}"#
    );
}

#[cfg(feature = "devtools")]
#[test]
fn dialog_escape_respects_closedby_policy() {
    for (attribute, expected_open) in [
        (None, false),
        (Some("none"), true),
        (Some("NONE"), true),
        (Some(" none "), false),
        (Some("closerequest"), false),
        (Some("any"), false),
        (Some("invalid"), false),
    ] {
        let mut vm = new_storage_test_vm("https://dialog-escape-closedby.test/");
        vm.eval(&format!(
            r#"
globalThis.dialogProbe = ({fixture})({{openOrder:['first']}});
const closedby = {attribute};
if (closedby !== null) dialogProbe.dialogs.first.setAttribute('closedby', closedby);
"#,
            fixture = include_str!("dialog_escape_order.js"),
            attribute = serde_json::to_string(&attribute).unwrap(),
        ))
        .unwrap();
        press_dialog_escape(&mut vm);
        assert_eq!(
            vm.eval("JSON.stringify([dialogProbe.dialogs.first.open, dialogProbe.state().cancels.length])").unwrap(),
            format!("[{expected_open},{}]", u8::from(!expected_open)),
            "closedby={attribute:?}"
        );
        // A policy that disables user closure does not disable script closure.
        vm.eval("dialogProbe.dialogs.first.close()").unwrap();
        assert_eq!(vm.eval("dialogProbe.dialogs.first.open").unwrap(), "false");
    }
}

#[cfg(feature = "devtools")]
#[test]
fn dialog_escape_rechecks_closedby_after_keyboard_and_cancel_listeners() {
    for (event, initial, action, expected_open, expected_cancels) in [
        (
            "keydown",
            "closerequest",
            "setAttribute('closedby', 'none')",
            true,
            0,
        ),
        ("keydown", "none", "removeAttribute('closedby')", false, 1),
        (
            "cancel",
            "closerequest",
            "setAttribute('closedby', 'none')",
            true,
            1,
        ),
    ] {
        let mut vm = new_storage_test_vm("https://dialog-escape-dynamic-closedby.test/");
        vm.eval(&format!(
            r#"
globalThis.dialogProbe = ({fixture})({{openOrder:['first']}});
const dialog = dialogProbe.dialogs.first;
dialog.setAttribute('closedby', '{initial}');
const target = '{event}' === 'keydown' ? document : dialog;
target.addEventListener('{event}', () => dialog.{action});
"#,
            fixture = include_str!("dialog_escape_order.js"),
        ))
        .unwrap();
        press_dialog_escape(&mut vm);
        assert_eq!(
            vm.eval("JSON.stringify([dialogProbe.dialogs.first.open, dialogProbe.state().cancels.length])").unwrap(),
            format!("[{expected_open},{expected_cancels}]"),
            "{event}: {action}"
        );
    }
}

#[cfg(feature = "devtools")]
#[test]
fn dialog_escape_closedby_ignores_namespaced_attributes() {
    for (on_cancel, policy, expected_open) in [
        (
            false,
            "dialog.setAttributeNS('urn:probe', 'closedby', 'none');",
            false,
        ),
        (
            false,
            "dialog.setAttributeNS('urn:probe', 'closedby', 'any');
             dialog.setAttributeNS(null, 'closedby', 'none');",
            true,
        ),
        (
            false,
            "dialog.setAttributeNS(null, 'closedby', 'none');
             dialog.setAttributeNS('urn:probe', 'closedby', 'any');",
            true,
        ),
        (
            true,
            "dialog.setAttributeNS('urn:probe', 'closedby', 'none');",
            false,
        ),
        (
            true,
            "dialog.setAttributeNS('urn:probe', 'closedby', 'any');
             dialog.setAttributeNS(null, 'closedby', 'none');",
            true,
        ),
    ] {
        let mut vm = new_storage_test_vm("https://dialog-escape-closedby-namespace.test/");
        vm.eval(&format!(
            r#"
globalThis.dialogProbe = ({fixture})({{openOrder:['first']}});
const dialog = dialogProbe.dialogs.first;
const setPolicy = () => {{ {policy} }};
if ({on_cancel}) dialog.addEventListener('cancel', setPolicy);
else setPolicy();
"#,
            fixture = include_str!("dialog_escape_order.js"),
        ))
        .unwrap();
        press_dialog_escape(&mut vm);
        let expected_cancels = u8::from(on_cancel || !expected_open);
        assert_eq!(
            vm.eval("JSON.stringify([dialogProbe.dialogs.first.open, dialogProbe.state().cancels.length])").unwrap(),
            format!("[{expected_open},{expected_cancels}]"),
            "on_cancel={on_cancel}: {policy}"
        );
    }
}

#[cfg(feature = "devtools")]
#[test]
fn dialog_escape_does_not_bypass_top_modal_closedby_none() {
    // This covers the current modal-only close-request selection. Chromium can
    // skip a disabled watcher and reach another enabled watcher in the same
    // group; CloseWatcher registration order and grouping remain follow-ups.
    let mut vm = new_storage_test_vm("https://dialog-escape-disabled-top.test/");
    vm.eval(&format!(
        r#"
globalThis.dialogProbe = ({fixture})({{}});
dialogProbe.dialogs.second.setAttribute('closedby', 'none');
"#,
        fixture = include_str!("dialog_escape_order.js"),
    ))
    .unwrap();
    press_dialog_escape(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify(dialogProbe.state())").unwrap(),
        r#"{"open":[true,true],"cancels":[]}"#
    );
}
