(() => {
  const rows = [], other = document.querySelector('#child').contentWindow, popup = open('about:blank', '_blank');
  if (!popup) throw Error('controlled eligibility probe requires a popup');
  const owners = [['top', globalThis], ['frame', other], ['popup', popup]];
  for (const [, owner] of owners) owner.__scriptEligibility = Object.create(null);
  const cases = ['disconnected-nonempty-inert', 'connected-empty-inert', 'connected-invalid-inert',
    'connected-valid-inert', 'legacy-for', 'legacy-event', 'nomodule'];
  for (const [kind, owner] of owners) for (const scenario of cases) {
    const label = `${kind}/${scenario}`;
    try {
      const doc = owner.document, inert = doc.implementation.createHTMLDocument('inert');
      const script = (scenario.startsWith('connected-valid') || scenario.startsWith('legacy') || scenario === 'nomodule' ? doc : inert).createElement('script');
      const source = `globalThis.__scriptEligibility[${JSON.stringify(label)}] = (globalThis.__scriptEligibility[${JSON.stringify(label)}] || 0) + 1;`;
      if (scenario === 'connected-invalid-inert') script.type = 'unsupported';
      if (scenario !== 'connected-empty-inert') script.textContent = source;
      if (scenario.startsWith('legacy')) {
        script.setAttribute('for', scenario === 'legacy-for' ? 'other' : 'window');
        script.setAttribute('event', scenario === 'legacy-event' ? 'onclick' : 'onload');
      }
      if (scenario === 'nomodule') script.noModule = true;
      if (scenario.startsWith('connected')) inert.body.appendChild(script);
      if (scenario.startsWith('legacy') || scenario === 'nomodule') doc.body.appendChild(script);
      const firstQuiet = owners.every(([, realm]) => realm.__scriptEligibility[label] === undefined);
      script.remove();
      if (scenario === 'connected-empty-inert') script.textContent = source;
      if (scenario === 'connected-invalid-inert') script.type = '';
      script.removeAttribute('for');script.removeAttribute('event');script.noModule = false;
      doc.body.appendChild(script);
      const expected = ['disconnected-nonempty-inert', 'connected-empty-inert', 'connected-invalid-inert'].includes(scenario) ? 1 : undefined;
      const observed = owners.map(([kind, realm]) => [kind, realm.__scriptEligibility[label] ?? null]);
      const passed = firstQuiet && owners.every(([, realm]) => realm.__scriptEligibility[label] === (realm === owner ? expected : undefined));
      rows.push({name: label, passed, observed});script.remove();
    } catch (error) {rows.push({name: label, passed: false, error: String(error)});}
  }
  for (const [kind, owner] of owners) for (const scenario of ['parser-valid', 'parser-empty', 'parser-invalid']) {
    const label = `${kind}/${scenario}`;
    try {
      const doc = owner.document, inert = doc.implementation.createHTMLDocument('parser');
      const source = `globalThis.__scriptEligibility[${JSON.stringify(label)}] = (globalThis.__scriptEligibility[${JSON.stringify(label)}] || 0) + 1;`;
      inert.write('<!doctype html><script id=probe' + (scenario === 'parser-invalid' ? ' type=unsupported' : '') + '>' +
        (scenario === 'parser-empty' ? '' : source) + '</script>');
      const script = inert.getElementById('probe');
      const parserAsync = script.async;
      const quiet = owners.every(([, realm]) => realm.__scriptEligibility[label] === undefined);
      script.remove();
      if (scenario === 'parser-invalid') script.type = '';
      if (scenario === 'parser-empty') script.textContent = source;
      doc.body.appendChild(script);
      const expected = scenario === 'parser-valid' ? undefined : 1;
      const passed = quiet && parserAsync === (scenario !== 'parser-valid') &&
        owners.every(([, realm]) => realm.__scriptEligibility[label] === (realm === owner ? expected : undefined));
      rows.push({name: label, passed, parserAsync, observed: owners.map(([kind, realm]) => [kind, realm.__scriptEligibility[label] ?? null])});
      script.remove();inert.close();
    } catch (error) {rows.push({name: label, passed: false, error: String(error)});}
  }
  popup.close();
  globalThis.__uiEventResults = {complete: true, total: rows.length, passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
