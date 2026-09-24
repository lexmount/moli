async function formPlannedNavigation(base, target, method, scenario) {
  const failures = [], events = [];
  let checks = 0, frame, owner = window, formDocument = document;
  const check = (value, message) => { ++checks; if (!value) failures.push(message); };
  if (target === 'child' || target === 'named') {
    frame = document.createElement('iframe');
    frame.name = 'form-destination';
    frame.src = base + '/common/blank.html';
    await new Promise(resolve => { frame.onload = resolve; document.body.appendChild(frame); });
    owner = frame.contentWindow;
    if (target === 'child') formDocument = owner.document;
  } else if (target === 'popup') {
    owner = window.open(base + '/common/blank.html');
    await new Promise(resolve => owner.onload = resolve);
    formDocument = owner.document;
  }
  const action = scenario === 'same-url' ? owner.location.href : base + '/submitted';
  const makeForm = () => {
    const form = formDocument.createElement('form');
    form.action = action;
    form.method = method;
    if (target === 'named') form.target = frame.name;
    form.innerHTML = '<input name="value" value="first">' +
      '<button id="first" name="submitter" value="first">first</button>' +
      '<button id="second" name="submitter" value="second">second</button>';
    formDocument.body.appendChild(form);
    return form;
  };
  const form = makeForm(), first = form.querySelector('#first'), second = form.querySelector('#second');
  const field = form.querySelector('input');
  let returned = false, microtask = false, submits = 0, formdatas = 0, errors = 0;
  let expectedSource = first, finish;
  const done = new Promise(resolve => finish = resolve);
  form.addEventListener('submit', () => ++submits);
  form.addEventListener('formdata', () => ++formdatas);
  const expectedCount = scenario === 'reentrant' ? 2 : 1;
  owner.navigation.onnavigate = event => {
    event.preventDefault();
    check(returned && microtask, 'navigation runs after return and microtasks');
    check(event.sourceElement === expectedSource, 'captured submitter identity');
    check(!event.userInitiated, 'script submission is not user initiated');
    check(new URL(event.destination.url).pathname === new URL(action).pathname, 'captured action');
    check(!event.destination.sameDocument, 'submission loads a new document');
    check(!event.hashChange, 'submission is not a hash change');
    const data = method === 'post' ? event.formData : new URL(event.destination.url).searchParams;
    check(method === 'post' ? event.formData !== null : event.formData === null, 'formData only for POST');
    const expected = scenario === 'double' || scenario === 'different-forms' || events.length ? 'second' : 'first';
    check(data.get('value') === expected, 'captured entry values');
    check(data.get('submitter') === (scenario === 'submit' ? null : expected), 'captured successful submitter');
    events.push(data.get('value'));
    if (scenario === 'reentrant' && events.length === 1) {
      field.value = 'second';
      expectedSource = second;
      microtask = false;
      form.requestSubmit(second);
      check(events.length === 1, 'reentrant submission queues a later task');
      queueMicrotask(() => microtask = true);
    }
  };
  owner.navigation.onnavigateerror = event => {
    check(event.error.name === 'AbortError', 'cancellation error');
    if (++errors === expectedCount) finish();
  };
  if (scenario === 'submit') {
    expectedSource = form;
    form.submit();
    check(submits === 0, 'submit() skips submit event');
  } else {
    form.requestSubmit(first);
    check(submits === 1, 'requestSubmit() dispatches submit synchronously');
  }
  check(formdatas >= 1, 'entry list constructed synchronously');
  check(events.length === 0, 'navigation not dispatched synchronously');
  if (scenario === 'double') {
    field.value = 'second';
    expectedSource = second;
    form.requestSubmit(second);
    check(submits === 2, 'replacement submission still dispatches submit');
  } else if (scenario === 'different-forms') {
    const replacement = makeForm();
    replacement.querySelector('input').value = 'second';
    expectedSource = replacement.querySelector('#second');
    replacement.requestSubmit(expectedSource);
  }
  returned = true;
  queueMicrotask(() => microtask = true);
  if (scenario === 'snapshot') {
    form.action = base + '/wrong';
    form.method = method === 'post' ? 'get' : 'post';
    field.value = 'wrong';
    first.value = 'wrong';
    new Document().appendChild(form);
    if (frame && target === 'named') frame.name = 'renamed';
  }
  let timeout;
  await Promise.race([done, new Promise(resolve => timeout = setTimeout(() => {
    failures.push('missing asynchronous navigateerror'); resolve();
  }, 2000))]);
  clearTimeout(timeout);
  check(events.length === expectedCount, 'one navigation per surviving planned task');
  check(errors === expectedCount, 'one cancellation per navigation');
  owner.navigation.onnavigate = null;
  owner.navigation.onnavigateerror = null;
  if (frame) frame.remove();
  if (target === 'popup') owner.close();
  return {checks, failures, events};
}
