async function probeCrossOriginWindowDomain(sameURL) {
  const failures = [];
  let checks = 0;
  const frame = document.createElement('iframe');
  const loaded = new Promise(resolve => frame.onload = resolve);
  frame.src = sameURL;
  document.body.appendChild(frame);
  await loaded;
  const child = frame.contentWindow;
  const read = child.Function(`
    const saved = parent;
    return () => {
      const probe = callback => { try { return callback(); } catch (error) { return error.name; } };
      return {
        sameParent: parent === saved,
        self: probe(() => parent[0] === window),
        cachedSelf: probe(() => saved[0] === window),
        length: probe(() => saved.length),
        frame: probe(() => frameElement === null),
        document: probe(() => typeof saved.document),
        keys: probe(() => Reflect.ownKeys(saved).filter(key => typeof key === 'string' && /^\\d+$/.test(key)))
      };
    };
  `)();
  const relaxChild = child.Function('document.domain = document.domain');
  const check = (phase, crossOrigin) => {
    const actual = read();
    const expected = {sameParent:true, self:true, cachedSelf:true, length:1, frame:crossOrigin,
      document:crossOrigin ? 'SecurityError' : 'object', keys:['0']};
    for (const key of Object.keys(expected)) {
      checks++;
      if (JSON.stringify(actual[key]) !== JSON.stringify(expected[key]))
        failures.push({phase, key, actual:actual[key], expected:expected[key]});
    }
  };
  check('before', false);
  relaxChild();
  check('child-relaxed', true);
  document.domain = document.domain;
  check('both-relaxed', false);
  frame.remove();
  return {checks, failures};
}
