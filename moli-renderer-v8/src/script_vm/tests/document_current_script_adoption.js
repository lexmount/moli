(() => {
  if (!document.documentElement) document.appendChild(document.createElement('html'));
  if (!document.body) document.documentElement.appendChild(document.createElement('body'));
  const source = document;
  const checks = [];
  const check = (actual, expected, label) => checks.push({label, passed: actual === expected});
  const missing = source.implementation.createHTMLDocument();
  const frame = source.createElement('iframe');
  source.body.appendChild(frame);
  const child = frame.contentDocument;
  const host = source.createElement('div');
  source.body.appendChild(host);
  const shadow = host.attachShadow({mode: 'open'});
  const callbacks = new Map();
  let next = 0;
  globalThis.__currentScriptCallback = id => callbacks.get(id)();
  const run = (parent, callback) => {
    const script = source.createElement('script');
    const id = ++next;
    script.text = `globalThis.__currentScriptCallback(${id})`;
    callbacks.set(id, () => callback(script));
    parent.appendChild(script);
    callbacks.delete(id);
    return script;
  };
  check(source.currentScript, null, 'initial source value');
  run(source.body, outer => {
    check(source.currentScript, outer, 'outer entry');
    for (const [label, target] of [['windowless', missing], ['child', child]]) {
      run(source.body, inner => {
        check(source.currentScript, inner, label + ' inner entry');
        target.body.appendChild(inner);
        check(inner.ownerDocument, target, label + ' adoption');
        check(source.currentScript, inner, label + ' source retains moved script');
        check(target.currentScript, null, label + ' destination stays empty');
        run(source.body, nested => {
          check(source.currentScript, nested, label + ' nested entry');
          check(target.currentScript, null, label + ' destination during nesting');
        });
        check(source.currentScript, inner, label + ' moved inner restored');
      });
      check(source.currentScript, outer, label + ' outer restored');
      check(target.currentScript, null, label + ' destination after return');
    }
    missing.body.appendChild(outer);
    check(source.currentScript, outer, 'outer source retains moved script');
    check(missing.currentScript, null, 'outer destination stays empty');
    run(source.body, nested => check(source.currentScript, nested, 'nested entry after outer adoption'));
    check(source.currentScript, outer, 'moved outer restored after nested entry');
  });
  check(source.currentScript, null, 'source empty after outer return');
  check(missing.currentScript, null, 'windowless empty after outer return');
  check(child.currentScript, null, 'child empty after outer return');
  delete globalThis.__currentScriptCallback;
  frame.remove();
  host.remove();
  return checks;
})()
