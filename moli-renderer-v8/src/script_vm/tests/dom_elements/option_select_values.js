(() => {
  const globals = [window, document.getElementById('child').contentWindow];
  const rows = [], errors = [];
  const values = ['prefix\uD800suffix', 'prefix\uDC01suffix', 'pair\uD800\uDC01',
    'replacement\uFFFD', '\t \0 \r\n X\u00A0 \f', '\u000Bv\u000B', ''];
  const normalize = value => value.replace(/[\t\n\f\r ]+/g, ' ').replace(/^ | $/g, '');
  const descriptor = (g, node, key) => {
    for (let object = node; object; object = g.Object.getPrototypeOf(object)) {
      const result = g.Object.getOwnPropertyDescriptor(object, key);
      if (result) return result;
    }
  };
  const record = (label, run) => {
    try { rows.push({label, checks: run()}); }
    catch (error) { errors.push({label, message: String(error)}); }
  };
  for (let realm = 0; realm < globals.length; ++realm) {
    const g = globals[realm], other = globals[1 - realm];
    for (const kind of ['live', 'windowless', 'parsed']) {
      const d = kind === 'live' ? g.document : kind === 'windowless' ?
        g.document.implementation.createHTMLDocument('') :
        new g.DOMParser().parseFromString('<!doctype html><body>', 'text/html');
      for (let index = 0; index < values.length; ++index) {
        const value = values[index], prefix = `${realm}/${kind}/${index}`;
        for (const property of ['value', 'label', 'text']) record(`${prefix}/${property}`, () => {
          const node = d.createElement('option');
          const desc = descriptor(other, other.document.createElement('option'), property);
          let conversions = 0;
          desc.set.call(node, {toString() { ++conversions; return value; }});
          const expected = property === 'text' ? normalize(value) : value;
          const checks = {convertOnce: conversions === 1, getter: desc.get.call(node) === expected,
            inheritedAccessor: !Object.hasOwn(node, property),
            nativeData: property === 'text' ? node.textContent === value : node.getAttribute(property) === value,
            clone: node.cloneNode(true)[property] === expected,
            import: other.document.importNode(node, true)[property] === expected};
          const sentinel = {};
          let error;
          try { desc.set.call(node, {toString() { throw sentinel; }}); } catch (caught) { error = caught; }
          checks.exception = error === sentinel && desc.get.call(node) === expected;
          error = undefined;
          try { desc.set.call(node, g.Symbol()); } catch (caught) { error = caught; }
          checks.symbol = error instanceof other.TypeError && desc.get.call(node) === expected;
          let traps = 0, invalidConversions = 0;
          const author = new g.Proxy(node, {get() { ++traps; }, set() { ++traps; }});
          const revoked = g.Proxy.revocable(node, {}); revoked.revoke();
          checks.receiver = true;
          for (const invalid of [{}, g.Object.create(node), author, revoked.proxy,
            d.createElement('div'), d.createTextNode('')]) {
            let getterError, setterError;
            try { desc.get.call(invalid); } catch (caught) { getterError = caught; }
            try { desc.set.call(invalid, {toString() { ++invalidConversions; return value; }}); }
            catch (caught) { setterError = caught; }
            checks.receiver &&= getterError instanceof other.TypeError && setterError instanceof other.TypeError;
          }
          checks.receiverOrder = traps === 0 && invalidConversions === 0;
          desc.set.call(node, null);
          checks.nullString = desc.get.call(node) === 'null';
          desc.set.call(node, undefined);
          checks.undefinedString = desc.get.call(node) === 'undefined';
          desc.set.call(node, value);
          other.document.adoptNode(node);
          checks.adoption = desc.get.call(node) === expected;
          return checks;
        });
        record(`${prefix}/fallback`, () => {
          const node = d.createElement('option'), expected = normalize(value);
          node.appendChild(d.createTextNode('\t ' + value));
          const span = d.createElement('span');
          span.appendChild(d.createTextNode(' \r\n'));
          node.appendChild(span);
          for (const namespace of ['http://www.w3.org/1999/xhtml', 'http://www.w3.org/2000/svg']) {
            const script = d.createElementNS(namespace, 'script');
            script.textContent = 'ignored\uD800'; node.appendChild(script);
          }
          const checks = {text: node.text === expected, value: node.value === expected,
            label: node.label === expected, clone: node.cloneNode(true).value === expected,
            import: other.document.importNode(node, true).value === expected};
          node.setAttribute('value', value); node.setAttribute('label', value);
          checks.attributeValue = node.value === value;
          checks.attributeLabel = node.label === value;
          node.setAttribute('value', ''); node.setAttribute('label', '');
          checks.emptyAttributes = node.value === '' && node.label === '';
          node.removeAttribute('value'); node.removeAttribute('label');
          checks.fallbackRestored = node.value === expected && node.label === expected;
          return checks;
        });
        for (const multiple of [false, true]) record(`${prefix}/select/${multiple}`, () => {
          const form = d.createElement('form'), select = d.createElement('select');
          select.name = 'control'; select.multiple = multiple; form.appendChild(select);
          const unique = ['\uD800', '\uDC00', '\uFFFD', value, value + 'x'];
          for (const entry of unique) {
            const option = d.createElement('option'); option.setAttribute('value', entry);
            option.textContent = entry; select.appendChild(option);
          }
          const duplicate = d.createElement('option'); duplicate.setAttribute('value', value);
          select.appendChild(duplicate);
          const desc = descriptor(other, other.document.createElement('select'), 'value');
          let conversions = 0;
          desc.set.call(select, {toString() { ++conversions; return value; }});
          const expectedIndex = unique.indexOf(value);
          const checks = {convertOnce: conversions === 1, getter: select.value === value,
            selectedIndex: select.selectedIndex === expectedIndex,
            selectedOptions: select.selectedOptions.length === 1 &&
              select.selectedOptions[0] === select.options[expectedIndex],
            cloneValues: Array.from(select.cloneNode(true).options, option => option.value).join('|') ===
              Array.from(select.options, option => option.value).join('|'),
            formData: new g.FormData(form).get('control') === value.toWellFormed()};
          for (let i = 0; i < 3; ++i) {
            desc.set.call(select, unique[i]);
            checks['distinct' + i] = select.selectedIndex === i && select.value === unique[i];
          }
          desc.set.call(select, 'not-in-the-list');
          checks.noMatch = select.selectedIndex === -1 && select.value === '' && select.selectedOptions.length === 0;
          return checks;
        });
      }
      record(`${realm}/${kind}/namespaced-attributes`, () => {
        const node = d.createElement('option');
        node.textContent = 'fallback';
        node.setAttributeNS('urn:value', 'value', 'ns-\uD800');
        node.setAttributeNS('urn:label', 'label', 'ns-\uDC00');
        const checks = {fallbackValue: node.value === 'fallback', fallbackLabel: node.label === 'fallback'};
        node.setAttributeNS(null, 'value', '\uD800');
        node.setAttributeNS(null, 'label', '\uDC00');
        checks.nullNamespaceValue = node.value === '\uD800';
        checks.nullNamespaceLabel = node.label === '\uDC00';
        checks.namespacedValuesRetained = node.getAttributeNS('urn:value', 'value') === 'ns-\uD800' &&
          node.getAttributeNS('urn:label', 'label') === 'ns-\uDC00';
        return checks;
      });
      record(`${realm}/${kind}/joined-text`, () => {
        const node = d.createElement('option');
        node.appendChild(d.createTextNode(' \uD83D'));
        const span = d.createElement('span'); span.appendChild(d.createTextNode('\uDE00 '));
        node.appendChild(span);
        return {text: node.text === '\uD83D\uDE00', value: node.value === '\uD83D\uDE00',
          label: node.label === '\uD83D\uDE00', raw: node.textContent === ' \uD83D\uDE00 '};
      });
    }
    for (let index = 0; index < values.length; ++index) record(`${realm}/constructor/${index}`, () => {
      const value = values[index], order = [];
      const node = new other.Option({toString() { order.push('text'); return value; }},
        {toString() { order.push('value'); return value; }}, true, false);
      const textOnly = new g.Option(value), omitted = new g.Option(undefined, undefined);
      return {order: order.join() === 'text,value', owner: node.ownerDocument === other.document,
        prototype: Object.getPrototypeOf(node) === other.HTMLOptionElement.prototype,
        rawText: node.textContent === value, text: node.text === normalize(value),
        value: node.value === value && node.getAttribute('value') === value,
        defaults: node.defaultSelected === true && node.selected === false,
        omittedValue: !textOnly.hasAttribute('value') && textOnly.value === normalize(value),
        omittedArguments: omitted.textContent === '' && !omitted.hasAttribute('value')};
    });
    record(`${realm}/cdata`, () => {
      const d = new g.DOMParser().parseFromString('<option xmlns="http://www.w3.org/1999/xhtml"/>', 'application/xml');
      const node = d.documentElement;
      node.appendChild(d.createCDATASection(' \uD83D'));
      node.appendChild(d.createTextNode('\uDE00\uD800 '));
      return {text: node.text === '\uD83D\uDE00\uD800', value: node.value === '\uD83D\uDE00\uD800',
        label: node.label === '\uD83D\uDE00\uD800'};
    });
    record(`${realm}/area-surface`, () => {
      const node = g.document.createElement('area');
      node.setAttribute('hreflang', 'retained'); node.setAttribute('type', 'retained');
      return {hreflangAbsent: !('hreflang' in node), typeAbsent: !('type' in node),
        attributesRetained: node.getAttribute('hreflang') === 'retained' && node.getAttribute('type') === 'retained'};
    });
  }
  globalThis.__uiEventResults = {rows, errors, values: values.length,
    passed: errors.length === 0 && rows.every(row => Object.values(row.checks).every(value => value === true))};
  return __uiEventResults.passed;
})()
