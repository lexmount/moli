use super::*;

#[test]
fn command_interfaces_apply_reflection_and_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://command-interface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const container = document.createElement('div');
  const button = document.createElement('button');
  const target = document.createElement('div');
  container.append(button, target);
  host.appendChild(container);

  const initial = button.command;
  button.setAttribute('command', 'ToGgLe-PoPoVeR');
  const builtin = button.command;
  button.setAttribute('command', '--Preserve-Me');
  const custom = button.command;
  button.command = [1, 2, 3];
  const invalid = [button.getAttribute('command'), button.command];

  const defaultEvent = new CommandEvent('command');
  const initialized = new CommandEvent('command', {
    command: null,
    source: target,
  });
  const commandDescriptor = Object.getOwnPropertyDescriptor(
    CommandEvent.prototype,
    'command',
  );
  const sourceDescriptor = Object.getOwnPropertyDescriptor(
    CommandEvent.prototype,
    'source',
  );
  const descriptors = [
    typeof commandDescriptor.get === 'function',
    commandDescriptor.set === undefined,
    typeof sourceDescriptor.get === 'function',
    sourceDescriptor.set === undefined,
  ];
  const rejected = [false, true, {}, new XMLHttpRequest()].map(source => {
    try {
      new CommandEvent('command', { source });
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  });

  return JSON.stringify({
    initial,
    builtin,
    custom,
    invalid,
    event: [defaultEvent.command, defaultEvent.source, initialized.command,
            initialized.source === target],
    descriptors,
    rejected,
  });
})()
"#,
        )
        .expect("command interface probe should evaluate");

    assert_eq!(
        result,
        r#"{"initial":"","builtin":"toggle-popover","custom":"--Preserve-Me","invalid":["1,2,3",""],"event":["",null,"null",true],"descriptors":[true,true,true,true],"rejected":[true,true,true,true]}"#
    );
}

#[test]
fn button_commands_validate_targets_and_apply_popover_actions_once() {
    let mut vm = new_storage_test_vm("https://button-command-activation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const container = document.createElement('div');
  const popover = document.createElement('div');
  popover.id = 'command-popover';
  popover.setAttribute('popover', '');
  const button = document.createElement('button');
  button.type = 'button';
  button.setAttribute('commandfor', popover.id);
  container.append(popover, button);
  host.appendChild(container);

  const states = [];
  button.command = 'toggle-popover';
  button.click();
  states.push(popover.matches(':popover-open'));
  button.click();
  states.push(popover.matches(':popover-open'));

  button.command = 'show-popover';
  button.click();
  states.push(popover.matches(':popover-open'));
  button.click();
  states.push(popover.matches(':popover-open'));
  popover.hidePopover();

  popover.addEventListener('command', event => event.preventDefault(), { once: true });
  button.click();
  states.push(popover.matches(':popover-open'));

  popover.addEventListener('command', () => {
    button.command = 'hide-popover';
  }, { once: true });
  button.command = 'show-popover';
  button.click();
  states.push(popover.matches(':popover-open'));

  popover.addEventListener('command', event => event.preventDefault(), { once: true });
  button.command = 'hide-popover';
  button.click();
  states.push(popover.matches(':popover-open'));
  button.click();
  states.push(popover.matches(':popover-open'));

  let invalidEvents = 0;
  const recordInvalidEvent = () => invalidEvents++;
  popover.addEventListener('command', recordInvalidEvent);
  button.setAttribute('command', 'not-a-command');
  button.click();
  const invalidCommandIgnored = invalidEvents === 0;
  popover.removeEventListener('command', recordInvalidEvent);

  const plainTarget = document.createElement('div');
  plainTarget.id = 'plain-command-target';
  container.appendChild(plainTarget);
  let plainEvent = null;
  plainTarget.addEventListener('command', event => plainEvent = event);
  button.setAttribute('commandfor', plainTarget.id);
  button.command = 'show-popover';
  button.click();

  const svgTarget = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svgTarget.id = 'svg-command-target';
  container.appendChild(svgTarget);
  const svgCommands = [];
  svgTarget.addEventListener('command', event => svgCommands.push(event.command));
  button.setAttribute('commandfor', svgTarget.id);
  button.command = '--MiXeD';
  button.click();
  button.command = 'show-popover';
  button.click();

  button.setAttribute('commandfor', popover.id);
  button.command = 'show-popover';
  let disconnectedSafe = true;
  popover.addEventListener('command', () => popover.remove(), { once: true });
  try {
    button.click();
  } catch {
    disconnectedSafe = false;
  }

  const targetActionPopover = document.createElement('div');
  targetActionPopover.id = 'target-action-popover';
  targetActionPopover.setAttribute('popover', '');
  const targetActionButton = document.createElement('button');
  targetActionButton.setAttribute('popovertarget', targetActionPopover.id);
  targetActionButton.setAttribute('popovertargetaction', 'show');
  container.append(targetActionPopover, targetActionButton);
  targetActionButton.click();
  const targetActionStates = [targetActionPopover.matches(':popover-open')];
  targetActionButton.click();
  targetActionStates.push(targetActionPopover.matches(':popover-open'));
  targetActionButton.setAttribute('popovertargetaction', 'hide');
  targetActionButton.click();
  targetActionStates.push(targetActionPopover.matches(':popover-open'));

  return JSON.stringify({
    states,
    invalidCommandIgnored,
    plainEvent: [plainEvent instanceof CommandEvent, plainEvent.command,
                 plainEvent.source === button],
    svgCommands,
    disconnectedSafe,
    disconnectedOpen: popover.matches(':popover-open'),
    targetActionStates,
  });
})()
"#,
        )
        .expect("button command activation probe should evaluate");

    assert_eq!(
        result,
        r#"{"states":[true,false,true,true,false,true,true,false],"invalidCommandIgnored":true,"plainEvent":[true,"show-popover",true],"svgCommands":["--MiXeD"],"disconnectedSafe":true,"disconnectedOpen":false,"targetActionStates":[true,true,false]}"#
    );
}

#[test]
fn button_auto_type_state_tracks_commands_form_owner_and_select_parent() {
    let mut vm = new_storage_test_vm("https://button-auto-type-state.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const form = document.createElement('form');
  const button = document.createElement('button');
  const target = document.createElement('div');
  target.id = 'target';
  form.append(button, target);
  host.appendChild(form);

  const states = [];
  const events = [];
  const capture = label => states.push(
    `${label}:${button.type}:${button.willValidate}:${button.matches(':valid')}`
  );
  form.addEventListener('submit', event => {
    event.preventDefault();
    events.push('submit');
  });
  target.addEventListener('command', () => events.push('command'));

  capture('initial');
  button.click();

  button.setAttribute('command', '--run');
  button.setAttribute('commandfor', 'target');
  capture('auto-command');
  button.click();
  let requestSubmitError = '';
  try {
    form.requestSubmit(button);
  } catch (error) {
    requestSubmitError = error.name;
  }

  button.type = 'submit';
  capture('explicit-submit');
  button.click();

  button.type = 'button';
  capture('explicit-button');
  button.click();

  button.type = ' submit ';
  capture('invalid-command');
  button.click();

  button.removeAttribute('command');
  button.removeAttribute('commandfor');
  capture('invalid-no-command');
  button.click();

  const select = document.createElement('select');
  const selectButton = document.createElement('button');
  select.appendChild(selectButton);
  host.appendChild(select);

  return JSON.stringify({
    states,
    events,
    requestSubmitError,
    selectButton: `${selectButton.type}:${selectButton.willValidate}:${selectButton.matches(':valid')}`
  });
})()
"#,
        )
        .expect("button Auto type-state probe should evaluate");

    assert_eq!(
        result,
        r#"{"states":["initial:submit:true:true","auto-command:button:false:false","explicit-submit:submit:true:true","explicit-button:button:false:false","invalid-command:button:false:false","invalid-no-command:submit:true:true"],"events":["submit","submit","command","submit"],"requestSubmitError":"TypeError","selectButton":"button:false:false"}"#
    );
}

#[test]
fn dispatched_bubbling_child_click_uses_ancestor_button_activation_behavior() {
    let mut vm = new_storage_test_vm("https://button-child-dispatched-click.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const form = document.createElement('form');
  const button = document.createElement('button');
  const child = document.createElement('span');
  button.appendChild(child);
  form.appendChild(button);
  host.appendChild(form);
  const submits = [];
  form.addEventListener('submit', event => {
    event.preventDefault();
    submits.push(event.submitter === button);
  });
  const allowed = child.dispatchEvent(new MouseEvent('click', {
    bubbles: true,
    cancelable: true
  }));
  const nonBubblingAllowed = child.dispatchEvent(new MouseEvent('click', {
    bubbles: false,
    cancelable: true
  }));
  return JSON.stringify({ allowed, nonBubblingAllowed, submits });
})()
"#,
        )
        .expect("bubbling child click activation probe should evaluate");

    assert_eq!(
        result,
        r#"{"allowed":true,"nonBubblingAllowed":true,"submits":[true]}"#
    );
}
