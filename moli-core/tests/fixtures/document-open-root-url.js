(async () => {
  const frame = document.createElement('iframe');
  frame.src = '/compat/child-dynamic-markup-document?markup=%3Cbody%3Esource#source-fragment';
  const loaded = new Promise(resolve => frame.onload = resolve);
  document.body.append(frame);
  await loaded;
  const child = frame.contentWindow;
  return new Promise(resolve => {
    child.finish = resolve;
    child.target = document;
    child.targetWindow = window;
    child.setTimeout(child.Function(`
      const done = finish, doc = target, win = targetWindow;
      const expected = document.URL.split('#')[0];
      const oldLength = win.history.length;
      const returned = doc.open();
      const observations = {url: doc.URL, uri: doc.documentURI, base: doc.baseURI,
        location: win.location.href, length: win.history.length, identity: returned === doc};
      const failures = [];
      for (const field of ['url', 'uri', 'base', 'location']) {
        if (observations[field] !== expected) failures.push({field, actual: observations[field], expected});
      }
      if (observations.length !== oldLength) failures.push({field: 'length', actual: observations.length, expected: oldLength});
      if (!observations.identity) failures.push({field: 'identity', actual: false, expected: true});
      doc.write('<!doctype html><body>replacement');
      doc.close();
      done({checks: 6, failures, observations});
    `), 0);
  });
})()
