(() => {
  const other = document.getElementById('child').contentWindow;
  const failures = [];
  const check = (name, test) => { try { if (!test()) failures.push(name); } catch (error) { failures.push(name + ': ' + error); } };
  for (const [label, doc] of [['main', document], ['iframe', other.document], ['windowless', document.implementation.createHTMLDocument('')], ['xml', document.implementation.createDocument('urn:root', 'root')]]) {
    for (const [realm, global] of [['main', window], ['iframe', other]]) {
      for (const method of ['setAttributeNode', 'setAttributeNodeNS']) {
        const owner = doc.createElement('div'), peer = doc.createElement('div');
        const attr = doc.createAttributeNS('urn:attribute', 'p:key'); attr.value = 'original';
        const set = global.Element.prototype[method], remove = global.Element.prototype.removeAttributeNode;
        const prefix = label + '/' + realm + '/' + method;
        check(prefix + '/attach', () => Reflect.apply(set, owner, [attr]) === null && attr.ownerElement === owner && owner.getAttributeNS('urn:attribute', 'key') === 'original');
        const used = doc.createAttributeNS('urn:attribute', 'q:key'); used.value = 'elsewhere'; peer.setAttributeNodeNS(used);
        check(prefix + '/in-use-realm', () => { try { Reflect.apply(set, owner, [used]); } catch (error) { return error instanceof global.DOMException && error.name === 'InUseAttributeError'; } return false; });
        check(prefix + '/not-found-realm', () => { try { Reflect.apply(remove, owner, [used]); } catch (error) { return error instanceof global.DOMException && error.name === 'NotFoundError'; } return false; });
        const replacement = doc.createAttributeNS('urn:attribute', 'p:key'); replacement.value = 'replacement';
        check(prefix + '/replace', () => Reflect.apply(set, owner, [replacement]) === attr && attr.ownerElement === null && attr.value === 'original' && replacement.ownerElement === owner);
        let publicCalls = 0;
        for (const name of ['setAttribute', 'setAttributeNS', 'removeAttribute', 'removeAttributeNS']) Object.defineProperty(owner, name, {configurable: true, value() { publicCalls++; throw new Error('public method'); }});
        check(prefix + '/remove', () => Reflect.apply(remove, owner, [replacement]) === replacement && replacement.ownerElement === null && replacement.value === 'replacement' && publicCalls === 0);
      }
    }
  }
  globalThis.__attrDispatchFailures = failures;
})()
