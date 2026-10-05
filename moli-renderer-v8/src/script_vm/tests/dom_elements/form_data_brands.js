(() => {
  const globals = [window, document.getElementById('child').contentWindow];
  const rows = [], errors = [];
  for (let realm = 0; realm < globals.length; ++realm) {
    const g = globals[realm], other = globals[1 - realm];
    for (const kind of ['live', 'windowless', 'parsed']) {
      const label = `${realm}/${kind}`;
      try {
        const d = kind === 'live' ? g.document : kind === 'windowless' ?
          g.document.implementation.createHTMLDocument('') :
          new g.DOMParser().parseFromString('<!doctype html><body>', 'text/html');
        const form = d.createElement('form'), input = d.createElement('input');
        input.setAttribute('name', 'input-name'); input.setAttribute('value', 'input-value');
        form.appendChild(input);
        const select = d.createElement('select'), option = d.createElement('option');
        select.setAttribute('name', 'select-name'); option.setAttribute('value', 'option-value');
        select.appendChild(option); form.appendChild(select);
        const button = d.createElement('button');
        button.setAttribute('type', 'submit'); button.setAttribute('name', 'send'); button.setAttribute('value', 'button');
        form.appendChild(button);
        let tagReads = 0;
        Object.defineProperty(form, 'tagName', {configurable: true, get() { ++tagReads; throw new Error('author tag getter'); }});
        let valueReads = 0;
        Object.defineProperty(input, 'value', {configurable: true, get() { ++valueReads; throw new Error('author value getter'); }});
        let eventCount = 0, eventRealm = true, eventTarget = true;
        form.addEventListener('formdata', event => {
          ++eventCount;
          eventRealm &&= event instanceof g.FormDataEvent && event.formData instanceof g.FormData;
          eventTarget &&= event.target === form;
          event.formData.append('from-handler', 'listener');
        });
        const data = new other.FormData(form);
        const checks = {constructorRealm: Object.getPrototypeOf(data) === other.FormData.prototype,
          input: data.get('input-name') === 'input-value',
          option: data.get('select-name') === 'option-value',
          event: eventCount === 1 && eventRealm && eventTarget,
          handlerMutation: data.get('from-handler') === 'listener',
          nativeProperties: tagReads === 0 && valueReads === 0,
          omitSubmitter: !data.has('send')};
        const withSubmitter = new other.FormData(form, button);
        checks.submitter = withSubmitter.get('send') === 'button' && eventCount === 2;
        let traps = 0;
        const author = new g.Proxy(form, {get() { ++traps; return undefined; }, getPrototypeOf() { ++traps; return null; }});
        const revoked = g.Proxy.revocable(form, {}); revoked.revoke();
        checks.invalidForms = true;
        for (const invalid of [null, {}, g.Object.create(form), author, revoked.proxy,
          d.createElement('div'), d.createElementNS('http://www.w3.org/2000/svg', 'form')]) {
          let error;
          try { new other.FormData(invalid); } catch (caught) { error = caught; }
          checks.invalidForms &&= error instanceof other.TypeError;
        }
        checks.invalidSubmitters = true;
        for (const invalid of [{}, g.Object.create(button), new g.Proxy(button, {get() { ++traps; }}),
          d.createElement('div'), other.document.createElement('div')]) {
          let error;
          try { new other.FormData(form, invalid); } catch (caught) { error = caught; }
          checks.invalidSubmitters &&= error instanceof other.TypeError;
        }
        const foreign = d.createElement('button'); foreign.type = 'submit';
        const crossDocument = other.document.createElement('button'); crossDocument.type = 'submit';
        checks.foreignSubmitter = true;
        for (const submitter of [foreign, crossDocument]) {
          let notFound;
          try { new other.FormData(form, submitter); } catch (caught) { notFound = caught; }
          checks.foreignSubmitter &&= notFound instanceof other.DOMException && notFound.name === 'NotFoundError';
        }
        let omittedSubmitterError;
        try { new other.FormData(undefined, {}); }
        catch (error) { omittedSubmitterError = error; }
        checks.omittedFormStillConvertsSubmitter = omittedSubmitterError instanceof other.TypeError;
        checks.noPublicReads = traps === 0 && tagReads === 0 && valueReads === 0;
        other.document.adoptNode(form);
        const adoptedData = new other.FormData(form);
        checks.adoptedFormRealm = eventCount === 3 && eventRealm && eventTarget &&
          Object.getPrototypeOf(adoptedData) === other.FormData.prototype &&
          adoptedData.get('from-handler') === 'listener';
        rows.push({label, checks});
      } catch (error) { errors.push({label, message: String(error)}); }
    }
  }
  globalThis.__uiEventResults = {rows, errors,
    passed: errors.length === 0 && rows.every(row => Object.values(row.checks).every(value => value === true))};
  return __uiEventResults.passed;
})()
