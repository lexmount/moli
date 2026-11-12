(async () => {
  async function probe(origin) {
    const popup = window.open('about:blank');
    const failures = [];
    let total = 0;
    try {
      for (const method of ['bytes', 'arrayBuffer', 'blob', 'text', 'json', 'formData']) {
        total++;
        try {
          const response = await popup.fetch(origin + '/seen?target=absent');
          let settled = false;
          const consumed = response[method]().then(
            value => { settled = true; return {value}; },
            error => { settled = true; return {error}; },
          );
          for (let i = 0; i < 8; i++) await Promise.resolve();
          if (settled) throw new Error('conversion did not wait for a fetch task');
          const output = await consumed;
          if (method === 'formData') {
            // Conversion errors belong to the invoked Body method's realm.
            const MethodTypeError = response.formData.constructor('return TypeError')();
            if (!(output.error instanceof MethodTypeError)) throw new Error('expected a conversion TypeError');
          } else {
            if ('error' in output) throw output.error;
            const actual = method === 'json' ? String(output.value)
              : method === 'text' ? output.value
              : method === 'blob' ? await output.value.text()
              : new TextDecoder().decode(output.value);
            if (actual !== 'false') throw new Error('unexpected body: ' + actual);
          }
        } catch (error) {
          failures.push(method + ': ' + error);
        }
      }
    } finally {
      popup.close();
    }
    return {total, failures};
  }
  const origin = location.origin;
  const main = await probe(origin);
  const frame = document.createElement('iframe');
  document.body.append(frame);
  try {
    const child = await frame.contentWindow.eval('(' + probe.toString() + ')(' + JSON.stringify(origin) + ')');
    return JSON.stringify({
      total: main.total + child.total,
      failures: main.failures.map(value => 'main: ' + value).concat(child.failures.map(value => 'child: ' + value)),
    });
  } finally {
    frame.remove();
  }
})();
