(() => {
  const child = document.getElementById('child').contentWindow;
  const failures = [];
  const check = (name, run) => { try { if (!run()) failures.push(name); } catch (e) { failures.push(name + ': ' + e); } };
  const documents = [['main', document, window], ['iframe', child.document, child],
    ['windowless', document.implementation.createHTMLDocument(''), window],
    ['xml', document.implementation.createDocument('urn:root', 'root'), window]];
  for (const [label, doc, realm] of documents) for (const method of ['before', 'after', 'replaceWith']) {
    for (const kind of ['element', 'text', 'doctype']) {
      const make = () => kind === 'element' ? doc.createElement('div') : kind === 'text' ? doc.createTextNode('target') : doc.implementation.createDocumentType('html', '', '');
      const proto = kind === 'element' ? realm.Element.prototype : kind === 'text' ? realm.CharacterData.prototype : realm.DocumentType.prototype;
      for (const [binding, invoke] of [['own', (target, values) => target[method](...values)], ['borrowed', (target, values) => Reflect.apply(proto[method], target, values)]]) {
        const prefix = label + '/' + method + '/' + kind + '/' + binding;
        check(prefix + '/parentless-converts-all', () => {
          const target = make(), log = [];
          invoke(target, [{toString() { log.push(1); return 'one'; }}, {toString() { log.push(2); return 'two'; }}]);
          return log.join() === '1,2' && target.parentNode === null;
        });
        check(prefix + '/parentless-exception', () => {
          const target = make(), marker = new Error('conversion'); let caught;
          try { invoke(target, [doc.createAttribute('invalid'), {toString() { throw marker; }}]); } catch (e) { caught = e; }
          return caught === marker && target.parentNode === null;
        });
        check(prefix + '/parentless-symbol', () => {
          try { invoke(make(), [Symbol()]); } catch (e) { return e instanceof realm.TypeError; } return false;
        });
        if (kind === 'doctype') continue;
        check(prefix + '/no-adoption-before-conversion', () => {
          const target = make(), parent = doc.createElement('div'); parent.appendChild(target);
          const foreignDocument = doc === document ? child.document : document;
          const sourceParent = foreignDocument.createElement('div'), source = foreignDocument.createElement('span'); sourceParent.appendChild(source);
          const marker = new Error('conversion'); let caught;
          try { invoke(target, [source, {toString() { throw marker; }}]); } catch (e) { caught = e; }
          return caught === marker && source.ownerDocument === foreignDocument && source.parentNode === sourceParent && target.parentNode === parent;
        });
        check(prefix + '/conversion-before-invalid-node', () => {
          const target = make(), parent = doc.createElement('div'); parent.appendChild(target);
          const marker = new Error('conversion'); let caught;
          try { invoke(target, [doc.createAttribute('invalid'), {toString() { throw marker; }}]); } catch (e) { caught = e; }
          return caught === marker && parent.firstChild === target && parent.childNodes.length === 1;
        });
        check(prefix + '/reentrant-parent', () => {
          const target = make(), original = doc.createElement('div'), destination = doc.createElement('div'); original.appendChild(target);
          let conversions = 0;
          invoke(target, [{toString() { conversions++; destination.appendChild(target); return 'inserted'; }}]);
          return conversions === 1 && original.childNodes.length === 0 &&
            destination.textContent === (method === 'replaceWith' ? 'inserted' : method === 'before' ? 'inserted' + target.textContent : target.textContent + 'inserted');
        });
      }
    }
  }
  globalThis.__childConversionFailures = failures;
})();
