use super::*;

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
