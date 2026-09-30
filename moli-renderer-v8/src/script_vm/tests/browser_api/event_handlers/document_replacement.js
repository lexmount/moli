(() => {
  const rows = [];
  function check(name, run) {
    let actual;
    try { actual = run(); }
    catch (error) { actual = {error: error.name, message: error.message}; }
    rows.push({name, actual, pass: actual === true});
  }
  function sameDescriptor(before, after) {
    if (!before || !after) return before === after;
    const keys = ['value', 'writable', 'get', 'set', 'enumerable', 'configurable'];
    return keys.every(key => Object.hasOwn(before, key) === Object.hasOwn(after, key) &&
      before[key] === after[key]);
  }
  function withWindow(owner, run) {
    let frame;
    let target;
    if (owner === 'root') target = window;
    else if (owner === 'popup') target = open();
    else {
      frame = document.createElement('iframe');
      document.body.appendChild(frame);
      target = frame.contentWindow;
    }
    try {
      // Finish a replacement stream first so the write case is destructive.
      target.document.open();
      target.document.close();
      return run(target);
    } finally {
      if (frame) frame.remove();
      if (owner === 'popup') target.close();
    }
  }
  function replace(target, operation) {
    if (operation === 'open') target.document.open();
    else target.document.write('<!doctype html><body>replacement');
    target.document.close();
  }
  for (const owner of ['child', 'popup', 'root']) {
    for (const operation of ['open', 'write']) {
      for (const mode of ['native', 'deleted', 'replaced', 'throwing-setter', 'read-only']) {
        check(`${owner}:${operation}:${mode}`, () => withWindow(owner, target => {
          let callbacks = 0, writes = 0;
          const native = Object.getOwnPropertyDescriptor(target, 'onresize');
          try {
            target.onresize = () => { ++callbacks; };
            target.addEventListener('resize', () => { ++callbacks; });
            if (mode === 'deleted') delete target.onresize;
            if (mode === 'replaced' || mode === 'throwing-setter') {
              Object.defineProperty(target, 'onresize', {set() {
                ++writes;
                if (mode === 'throwing-setter') throw new Error('author setter ran');
              }});
            }
            if (mode === 'read-only') {
              Object.defineProperty(target, 'onresize', {value: 17, writable: false});
            }
            const descriptor = Object.getOwnPropertyDescriptor(target, 'onresize');
            replace(target, operation);
            target.dispatchEvent(new Event('resize'));
            return sameDescriptor(descriptor, Object.getOwnPropertyDescriptor(target, 'onresize')) &&
              native.get.call(target) === null && callbacks === 0 && writes === 0;
          } finally { Object.defineProperty(target, 'onresize', native); }
        }));
      }
      check(`${owner}:${operation}:pending-content-cleared`, () => withWindow(owner, target => {
        target.document.body.setAttribute('onresize', '}');
        replace(target, operation);
        let errors = 0;
        try {
          target.onerror = () => { ++errors; return true; };
          return target.onresize === null && errors === 0;
        } finally { target.onerror = null; }
      }));
    }
  }
  const failures = rows.filter(row => !row.pass);
  globalThis.__nodeReplacementResults = {total: rows.length,
    passed: rows.length - failures.length, failures, rows};
  return failures.length === 0;
})()
