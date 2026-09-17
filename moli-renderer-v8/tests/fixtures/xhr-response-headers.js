async function xhrResponseHeadersProbe(url, expected) {
  const checks = [];
  const check = (label, actual, wanted) => checks.push({
    label, actual, wanted, pass: actual === wanted
  });
  const block = expected.map(([name, value]) => `${name}: ${value}\r\n`).join('');
  const forbidden = ['set-cookie', 'SET-COOKIE', 'SeT-CoOkIe', 'set-cookie2', 'SET-COOKIE2'];
  function empty(xhr, phase) {
    check(`${phase}/all`, xhr.getAllResponseHeaders(), '');
    check(`${phase}/ordinary`, xhr.getResponseHeader('foo-test'), null);
    for (const name of forbidden) check(`${phase}/${name}`, xhr.getResponseHeader(name), null);
  }
  function visible(xhr, phase) {
    check(`${phase}/all`, xhr.getAllResponseHeaders(), block);
    for (const [name, value] of expected) {
      check(`${phase}/${name}`, xhr.getResponseHeader(name), value);
      check(`${phase}/${name}/uppercase`, xhr.getResponseHeader(name.toUpperCase()), value);
    }
    for (const name of [...forbidden, 'missing', ' foo-test', 'foo-test ', 'foo:test', 'foo-test\n']) {
      check(`${phase}/${name}/absent`, xhr.getResponseHeader(name), null);
    }
  }
  for (const async of typeof document === 'undefined' ? [true, false] : [true]) {
    for (const credentials of [false, true]) {
      const xhr = new XMLHttpRequest();
      const label = `${async}/${credentials}`;
      empty(xhr, `${label}/unsent`);
      xhr.open('GET', url, async);
      xhr.withCredentials = credentials;
      empty(xhr, `${label}/opened`);
      const seen = new Set();
      xhr.onreadystatechange = () => {
        if (xhr.readyState >= 2 && !seen.has(xhr.readyState)) {
          seen.add(xhr.readyState);
          visible(xhr, `${label}/state${xhr.readyState}`);
        }
      };
      if (async) {
        await new Promise((resolve, reject) => {
          xhr.onload = resolve;
          xhr.onerror = () => reject(new Error(`${label}: network error`));
          xhr.send();
        });
        check(`${label}/states`, Array.from(seen).join(','), '2,3,4');
      } else {
        xhr.send();
        visible(xhr, `${label}/sync-done`);
      }
      check(`${label}/done`, xhr.readyState, 4);
      check(`${label}/body`, xhr.responseText, 'body');
      xhr.open('GET', url, async);
      empty(xhr, `${label}/reopened`);
    }
  }
  return {state: checks.every(entry => entry.pass) ? 'pass' : 'fail', checks};
}
