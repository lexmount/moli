(() => {
  const NativeObject = Object;
  const child = document.getElementById('child').contentWindow;
  const realms = [window, child].map(global => ({
    global, document: global.document, Event: global.Event, Submit: global.SubmitEvent,
    FormDataEvent: global.FormDataEvent, FormData: global.FormData, Function: global.Function,
    TypeError: global.TypeError, DOMException: global.DOMException,
    click: global.HTMLElement.prototype.click, reset: global.HTMLFormElement.prototype.reset,
    request: global.HTMLFormElement.prototype.requestSubmit,
    check: global.HTMLInputElement.prototype.checkValidity
  }));
  const rows = [], actions = [], restores = [];
  let reads = 0;
  function record(label, checks) { rows.push({label, checks}); }
  function makeForm(creation, targetDocument) {
    const form = creation.document.createElement('form');
    const input = form.appendChild(creation.document.createElement('input'));
    input.name = 'field'; input.defaultValue = 'initial'; input.value = 'value';
    const submit = form.appendChild(creation.document.createElement('button')); submit.type = 'submit';
    submit.name = 'submitter'; submit.value = 'chosen';
    const reset = form.appendChild(creation.document.createElement('button')); reset.type = 'reset';
    targetDocument.body.appendChild(form);
    return {form, input, submit, reset};
  }
  function observe(target, type, realm, label, after) {
    const opposite = realms[1 - realms.indexOf(realm)];
    const Constructor = type === 'submit' ? realm.Submit : type === 'formdata' ? realm.FormDataEvent : realm.Event;
    let captured;
    target.addEventListener(type, event => { captured = event; }, {capture: true});
    target.addEventListener(type, event => {
      const checks = {
        realm: event instanceof realm.Event && !(event instanceof opposite.Event),
        typed: event instanceof Constructor,
        prototype: NativeObject.getPrototypeOf(event) === Constructor.prototype,
        constructor: event.constructor === Constructor,
        constructorRealm: event.constructor instanceof realm.Function,
        identity: captured === event,
        target: event.target === target && event.currentTarget === target,
        trusted: event.isTrusted,
        type: event.type === type,
        bubbles: event.bubbles === (type !== 'invalid'),
        cancelable: event.cancelable === (type !== 'formdata'),
        composed: event.composed === false
      };
      if (type === 'formdata') {
        checks.payloadRealm = event.formData instanceof realm.FormData && !(event.formData instanceof opposite.FormData);
        checks.payloadIdentity = event.formData === event.formData;
      }
      record(label + ' ' + type, checks);
      if (after) after(event, checks);
    });
  }
  for (const creation of realms) {
    for (const owner of realms) {
      for (const caller of realms) {
        const label = `create ${realms.indexOf(creation)} / owner ${realms.indexOf(owner)} / caller ${realms.indexOf(caller)}`;
        const {form, input, submit, reset} = makeForm(creation, owner.document);
        let payload;
        observe(form, 'submit', creation, label, (event, checks) => {
          checks.submitter = event.submitter === submit;
          event.preventDefault(); checks.canceled = event.defaultPrevented;
        });
        observe(form, 'reset', creation, label, (event, checks) => {
          event.preventDefault(); checks.canceled = event.defaultPrevented;
        });
        observe(input, 'invalid', creation, label, (event, checks) => {
          event.preventDefault(); checks.canceled = event.defaultPrevented;
        });
        observe(form, 'formdata', creation, label, (event, checks) => {
          payload = event.formData;
          checks.entries = payload.get('field') === 'value' && payload.get('submitter') === 'chosen';
          payload.append('fromEvent', 'updated');
          event.preventDefault(); checks.notCanceled = !event.defaultPrevented;
        });
        actions.push(() => {
          caller.click.call(submit);
          caller.request.call(form, submit);
          caller.reset.call(form);
          caller.click.call(reset);
          const resetCanceled = input.value === 'value';
          input.required = true; input.value = '';
          const valid = caller.check.call(input);
          caller.request.call(form, submit);
          input.required = false; input.value = 'value';
          const result = new caller.FormData(form, submit);
          payload.append('afterReturn', 'late');
          record(label + ' result', {
            callerRealm: result instanceof caller.FormData,
            constructor: result.constructor === caller.FormData,
            distinct: result !== payload,
            mutation: result.get('fromEvent') === 'updated',
            cloned: result.get('afterReturn') === null,
            invalid: valid === false,
            resetCanceled
          });
        });
        poison(form, 'constructor');
      }
    }
  }
  for (const realm of realms) {
    const detached = realm.document.implementation.createHTMLDocument('');
    for (const caller of realms) {
      const label = `windowless ${realms.indexOf(realm)} / caller ${realms.indexOf(caller)}`;
      const {form, input} = makeForm(realm, detached);
      observe(input, 'invalid', realm, label);
      observe(form, 'formdata', realm, label, (event, checks) => {
        checks.entries = event.formData.get('field') === 'value';
        event.formData.append('fromEvent', 'updated');
      });
      actions.push(() => {
        input.required = true; input.value = '';
        const valid = caller.check.call(input);
        input.required = false; input.value = 'value';
        const result = new caller.FormData(form);
        record(label + ' result', {callerRealm: result instanceof caller.FormData,
          mutation: result.get('fromEvent') === 'updated', invalid: valid === false});
      });
      poison(form, 'constructor');
    }
  }
  // A nested foreign-form construction must restore the outer dispatch state;
  // reentering the same form must still throw in the invoking constructor realm.
  const outer = makeForm(realms[1], child.document);
  const inner = makeForm(realms[0], document);
  let outerEvent, nestedPayload;
  observe(inner.form, 'formdata', realms[0], 'nested parent', event => {
    nestedPayload = event.formData;
    event.formData.append('inner', 'nested');
  });
  observe(outer.form, 'formdata', realms[1], 'outer child', (event, checks) => {
    outerEvent = event;
    let error;
    try { new realms[0].FormData(outer.form); } catch (caught) { error = caught; }
    checks.reentrantError = error instanceof realms[0].DOMException && error.name === 'InvalidStateError';
    const nested = new realms[1].FormData(inner.form);
    checks.nestedResult = nested instanceof realms[1].FormData && nested.get('inner') === 'nested';
    checks.nestedPayload = nestedPayload instanceof realms[0].FormData;
    checks.retained = event === outerEvent && event.currentTarget === outer.form;
    event.formData.append('outer', 'retained');
  });
  actions.push(() => {
    class CustomData extends realms[0].FormData {}
    const result = new CustomData(outer.form);
    record('subclass result', {subclass: result instanceof CustomData,
      prototype: NativeObject.getPrototypeOf(result) === CustomData.prototype,
      callerRealm: result instanceof realms[0].FormData,
      entries: result.get('outer') === 'retained', distinct: result !== outerEvent.formData});
  });
  // Public constructor validation must retain callee-realm exceptions.
  for (const caller of realms) {
    actions.push(() => {
      let error;
      try { new caller.FormData({}); } catch (caught) { error = caught; }
      record('constructor error realm', {typeError: error instanceof caller.TypeError});
    });
  }
  const mixed = makeForm(realms[0], document);
  const foreignInput = child.document.createElement('input');
  foreignInput.required = true;
  mixed.form.insertBefore(foreignInput, mixed.input);
  mixed.input.required = true; mixed.input.value = '';
  observe(foreignInput, 'invalid', realms[1], 'foreign invalid target', () => {
    child.document.body.appendChild(mixed.form);
  });
  observe(mixed.input, 'invalid', realms[0], 'invalid after form adoption');
  actions.push(() => realms[1].request.call(mixed.form, mixed.submit));
  function poison(object, name) {
    restores.push([object, name, NativeObject.getOwnPropertyDescriptor(object, name)]);
    NativeObject.defineProperty(object, name, {configurable: true, get() {
      reads++;
      throw Error('author native form construction hook');
    }});
  }
  for (const realm of realms) {
    for (const name of ['Event', 'SubmitEvent', 'FormDataEvent', 'FormData']) poison(realm.global, name);
    for (const name of ['composed', 'submitter', 'formData']) poison(realm.global.Object.prototype, name);
  }
  try {
    for (const action of actions) action();
  } finally {
    for (let i = restores.length - 1; i >= 0; i--) {
      const [object, name, descriptor] = restores[i];
      if (descriptor) NativeObject.defineProperty(object, name, descriptor);
      else delete object[name];
    }
    globalThis.__uiEventResults = {rows, reads, expectedRows: 83, passed: false};
  }
  const expectedRows = 83;
  const required = ['realm', 'typed', 'prototype', 'constructor', 'constructorRealm', 'identity',
    'target', 'trusted', 'type', 'bubbles', 'cancelable', 'composed'];
  const passed = rows.length === expectedRows && reads === 0 && rows.every(row =>
    NativeObject.values(row.checks).every(value => value === true) &&
    (!NativeObject.hasOwn(row.checks, 'realm') || required.every(name => NativeObject.hasOwn(row.checks, name))));
  globalThis.__uiEventResults = {rows, reads, expectedRows, passed};
  return passed;
})()
