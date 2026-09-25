globalThis.iframeReinsertionCase = (() => {
  const cases = [];
  const failures = [];
  let checks = 0;
  const names = [
    'cross-document', 'same-document', 'insert-before', 'self-append',
    'self-insert', 'replace', 'subtree', 'shadow-subtree', 'fragment',
    'move-before', 'move-before-subtree', 'move-before-shadow', 'invalid-reference'
  ];
  function check(name, actual, expected) {
    checks++;
    if (actual !== expected) failures.push({name, actual, expected});
  }
  function setup() {
    for (const id of names) {
      const source = document.createElement('div');
      const destination = document.createElement('div');
      document.body.append(source, destination);
      let outer = null;
      let owner = document;
      if (id === 'cross-document') {
        outer = document.createElement('iframe');
        source.appendChild(outer);
        owner = outer.contentDocument;
      }
      const frame = owner.createElement('iframe');
      const code = 'window.localValue = 1; top.iframeReinsertionCase.record(' +
        JSON.stringify(id) + ', {parentIsTop: parent === top, ' +
        'ownerIsTop: frameElement.ownerDocument === top.document});';
      frame.srcdoc = '<!doctype html><body><script>' + code + '<' + '/script>';
      let moving = frame;
      if (id.includes('subtree') || id.includes('shadow')) {
        moving = owner.createElement('div');
        const holder = id.includes('shadow') ? moving.attachShadow({mode: 'open'}) : moving;
        holder.appendChild(frame);
      }
      cases.push({id, source, destination, outer, frame, moving, runs: []});
      (outer ? owner.body : source).appendChild(moving);
    }
  }
  function record(id, value) {
    cases.find(test => test.id === id).runs.push(value);
  }
  function move() {
    for (const test of cases) {
      const {id, frame, source, destination, moving} = test;
      const oldWindow = frame.contentWindow;
      const oldDocument = frame.contentDocument;
      check(id + ': initial script', test.runs.length, 1);
      check(id + ': initial local', oldWindow.localValue, 1);
      oldWindow.localValue = 42;
      const preserved = id.startsWith('move-before') || id === 'invalid-reference';
      let errorName = '';
      try {
        if (id === 'self-append') source.appendChild(frame);
        else if (id === 'self-insert') source.insertBefore(frame, frame);
        else if (id === 'insert-before' || id === 'replace') {
          const reference = document.createElement('b');
          destination.appendChild(reference);
          if (id === 'replace') destination.replaceChild(moving, reference);
          else destination.insertBefore(moving, reference);
        } else if (id === 'fragment') {
          const fragment = document.createDocumentFragment();
          fragment.appendChild(moving);
          destination.appendChild(fragment);
        } else if (id.startsWith('move-before')) destination.moveBefore(moving, null);
        else if (id === 'invalid-reference') destination.insertBefore(moving, document.createElement('b'));
        else destination.appendChild(moving);
      } catch (error) {
        errorName = error.name;
      }
      check(id + ': exception', errorName, id === 'invalid-reference' ? 'NotFoundError' : '');
      check(id + ': Window identity', frame.contentWindow === oldWindow, preserved);
      check(id + ': Document identity', frame.contentDocument === oldDocument, preserved);
      check(id + ': retired document retained', oldWindow.document === oldDocument, true);
      check(id + ': retired parent', oldWindow.parent === null, !preserved);
      check(id + ': retired top', oldWindow.top === null, !preserved);
      check(id + ': retired frameElement', oldWindow.frameElement === null, !preserved);
      check(id + ': retired globals retained', oldWindow.localValue, 42);
      check(id + ': new owner', frame.ownerDocument === document, true);
      check(id + ': new parent', frame.contentWindow.parent === window, true);
      test.preserved = preserved;
      test.newWindow = frame.contentWindow;
    }
  }
  function result() {
    for (const {id, frame, runs, preserved, newWindow} of cases) {
      check(id + ': script executions', runs.length, preserved ? 1 : 2);
      check(id + ': script observes parent', runs.at(-1)?.parentIsTop, true);
      check(id + ': script observes owner', runs.at(-1)?.ownerIsTop, true);
      check(id + ': new globals', frame.contentWindow.localValue, preserved ? 42 : 1);
      check(id + ': new Window survives load', frame.contentWindow === newWindow, true);
    }
    return {cases: cases.length, checks, failures};
  }
  function close() {
    for (const {source, destination} of cases) {
      source.remove();
      destination.remove();
    }
  }
  return {setup, record, move, result, close};
})();
