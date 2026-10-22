(async () => {
  const checks = [];
  const realms = [['main', globalThis], ['child', document.querySelector('#child').contentWindow]];
  const check = (name, passed, details) => checks.push({name, passed: !!passed, ...(passed ? {} : {details})});
  const flush = async () => { await Promise.resolve(); await Promise.resolve(); };
  const htmlNS = 'http://www.w3.org/1999/xhtml';
  for (const [ownerName, owner] of realms) {
    const documents = [
      ['live', owner.document],
      ['html', owner.document.implementation.createHTMLDocument('')],
      ['parsed', new owner.DOMParser().parseFromString('<!doctype html><body>', 'text/html')],
      ['xml', new owner.DOMParser().parseFromString('<root/>', 'application/xml')],
    ];
    for (const [docName, doc] of documents) for (const [calleeName, callee] of realms) {
      const prefix = `${ownerName}/${docName}/${calleeName}`;
      const make = () => {
        const parent = doc.createElementNS(htmlNS, 'div');
        const branch = parent.appendChild(doc.createElementNS(htmlNS, 'section'));
        const leaf = branch.appendChild(doc.createElementNS(htmlNS, 'span'));
        leaf.setAttribute('data-value', 'before');
        const text = leaf.appendChild(doc.createTextNode('before'));
        return {parent, branch, leaf, text};
      };
      for (const sink of ['removeChild', 'replaceChild', 'innerHTML', 'textContent', 'replaceChildren', 'setHTMLUnsafe', 'appendChild', 'fragment']) {
        const {parent, branch, leaf, text} = make();
        const observer = new callee.MutationObserver(() => {});
        observer.observe(parent, {childList: true, attributes: true, attributeOldValue: true, characterData: true, characterDataOldValue: true, subtree: true});
        try {
          if (sink === 'removeChild') parent.removeChild(branch);
          if (sink === 'replaceChild') parent.replaceChild(doc.createElementNS(htmlNS, 'b'), branch);
          if (sink === 'innerHTML') parent.innerHTML = '';
          if (sink === 'textContent') parent.textContent = '';
          if (sink === 'replaceChildren') parent.replaceChildren();
          if (sink === 'setHTMLUnsafe') parent.setHTMLUnsafe('');
          if (sink === 'appendChild') doc.createElementNS(htmlNS, 'aside').appendChild(branch);
          if (sink === 'fragment') { const f = doc.createDocumentFragment(); f.appendChild(branch); doc.createElementNS(htmlNS, 'aside').appendChild(f); }
          observer.takeRecords();
          leaf.setAttribute('data-value', 'after'); text.data = 'after'; leaf.appendChild(doc.createComment('after'));
          const records = observer.takeRecords();
          check(`${prefix}/${sink}/retains-after-takeRecords`, records.length === 3 && records[0].target === leaf && records[0].oldValue === 'before' && records[1].target === text && records[1].oldValue === 'before' && records[2].type === 'childList', records.map(r => [r.type, r.oldValue]));
        } catch (error) {check(`${prefix}/${sink}/retains-after-takeRecords`, false, String(error));}
        finally {observer.disconnect();}
      }
      {
        const {parent, branch, leaf} = make(); let calls = 0;
        const observer = new callee.MutationObserver(() => {calls++;});
        observer.observe(parent, {attributes: true, subtree: true}); parent.removeChild(branch);
        await flush(); leaf.setAttribute('later', 'yes');
        check(`${prefix}/cleanup-without-records`, observer.takeRecords().length === 0 && calls === 0, calls); observer.disconnect();
      }
      {
        const {parent, branch, leaf, text} = make(); const observer = new callee.MutationObserver(() => {});
        observer.observe(parent, {characterData: true, subtree: true, characterDataOldValue: true});
        parent.removeChild(branch); branch.removeChild(leaf); leaf.removeChild(text); text.data = 'after';
        const records = observer.takeRecords();
        check(`${prefix}/nested-removal-propagates-source`, records.length === 1 && records[0].target === text && records[0].oldValue === 'before', records.map(r => r.oldValue)); observer.disconnect();
      }
      {
        const {parent, branch, leaf} = make(); const observer = new callee.MutationObserver(() => {});
        observer.observe(parent, {attributes: true, subtree: true}); parent.removeChild(branch);
        observer.observe(parent, {attributes: true, subtree: true}); leaf.setAttribute('later', 'yes');
        check(`${prefix}/reobserve-clears-own-source`, observer.takeRecords().length === 0); observer.disconnect();
      }
      {
        const {parent, branch, leaf} = make(); const observer = new callee.MutationObserver(() => {});
        observer.observe(parent, {attributes: true, subtree: true}); parent.removeChild(branch); observer.disconnect(); leaf.setAttribute('later', 'yes');
        check(`${prefix}/disconnect-clears-transients`, observer.takeRecords().length === 0);
      }
      for (const detached of [false, true]) {
        const {parent, branch, leaf, text} = make(); const observer = new callee.MutationObserver(() => {});
        observer.observe(parent, {attributes: true, attributeOldValue: true, characterData: true, characterDataOldValue: true, subtree: true});
        observer.observe(branch, {attributes: true, characterData: true, subtree: true});
        if (detached) parent.removeChild(branch);
        leaf.setAttribute('data-value', 'after'); text.data = 'after'; const records = observer.takeRecords();
        check(`${prefix}/overlap-oldValue/${detached}`, records.length === 2 && records.every(r => r.oldValue === 'before'), records.map(r => r.oldValue)); observer.disconnect();
      }
      {
        const {parent, branch, leaf} = make(); let calls = 0;
        const observer = new callee.MutationObserver(() => {calls++;});
        observer.observe(parent, {attributes: true, subtree: true}); parent.removeChild(branch);
        leaf.setAttribute('before-notification', 'yes');
        const before = observer.takeRecords(); await flush(); leaf.setAttribute('after-notification', 'yes');
        check(`${prefix}/takeRecords-keeps-until-notification`, before.length === 1 && observer.takeRecords().length === 0 && calls === 0, {before: before.length, calls}); observer.disconnect();
      }
      {
        const {parent, branch, leaf} = make(); const observer = new callee.MutationObserver(() => {});
        observer.observe(parent, {attributes: true, attributeOldValue: true, attributeFilter: ['data-value'], subtree: true}); parent.removeChild(branch);
        observer.observe(branch, {attributes: true, attributeFilter: ['own'], subtree: true});
        leaf.setAttribute('data-value', 'during'); const before = observer.takeRecords();
        await flush(); leaf.setAttribute('data-value', 'after'); leaf.setAttribute('own', 'after'); const after = observer.takeRecords();
        check(`${prefix}/observe-removed-node-keeps-distinct-source`, before.length === 1 && before[0].oldValue === 'before' && after.length === 1 && after[0].attributeName === 'own', {before: before.map(r => r.oldValue), after: after.map(r => r.attributeName)}); observer.disconnect();
      }
      {
        const {parent, branch, leaf} = make(); const observer = new callee.MutationObserver(() => {});
        leaf.setAttribute('right', 'before');
        observer.observe(parent, {attributes: true, attributeFilter: ['left'], subtree: true});
        observer.observe(branch, {attributes: true, attributeFilter: ['right'], attributeOldValue: true, subtree: true});
        parent.removeChild(branch); branch.removeChild(leaf);
        observer.observe(parent, {attributes: true, attributeFilter: ['new'], subtree: true});
        leaf.setAttribute('left', 'after'); leaf.setAttribute('right', 'after');
        const records = observer.takeRecords();
        check(`${prefix}/reobserve-keeps-other-source`, records.length === 1 && records[0].attributeName === 'right' && records[0].oldValue === 'before', records.map(r => [r.attributeName, r.oldValue])); observer.disconnect();
      }
      {
        const {parent, branch, leaf} = make(); const observer = new callee.MutationObserver(() => {});
        observer.observe(parent, {attributes: true, subtree: true}); parent.removeChild(branch);
        let threw = false; try {observer.observe(parent, {attributes: false, attributeOldValue: true});} catch (error) {threw = error instanceof callee.TypeError;}
        leaf.setAttribute('later', 'yes');
        check(`${prefix}/failed-observe-preserves-source`, threw && observer.takeRecords().length === 1, threw); observer.disconnect();
      }
      {
        const {parent, branch, leaf} = make(); const observer = new callee.MutationObserver(() => {});
        observer.observe(parent, {attributes: true, attributeFilter: ['data-value'], attributeOldValue: true, subtree: true}); parent.removeChild(branch);
        leaf.setAttributeNS('urn:observer', 'x:data-value', 'namespace'); leaf.setAttribute('ignored', 'yes'); leaf.setAttribute('data-value', 'after');
        const records = observer.takeRecords();
        check(`${prefix}/transient-attribute-filter-and-namespace`, records.length === 1 && records[0].attributeNamespace === null && records[0].oldValue === 'before', records.map(r => [r.attributeName, r.attributeNamespace, r.oldValue])); observer.disconnect();
      }
      for (const operation of ['deleteContents', 'extractContents', 'adoptNode', 'normalize']) {
        const {parent, branch, leaf, text} = make(); const observer = new callee.MutationObserver(() => {});
        observer.observe(parent, {characterData: true, characterDataOldValue: true, subtree: true});
        let removedText = text;
        if (operation === 'deleteContents' || operation === 'extractContents') {
          const range = doc.createRange(); range.selectNode(branch); range[operation]();
        } else if (operation === 'adoptNode') {
          owner.document.implementation.createHTMLDocument('').adoptNode(branch);
        } else {
          removedText = leaf.appendChild(doc.createTextNode('second')); leaf.normalize(); observer.takeRecords();
        }
        removedText.data = 'after'; const records = observer.takeRecords();
        check(`${prefix}/${operation}/retains-transient`, records.length === 1 && records[0].target === removedText && records[0].oldValue === (operation === 'normalize' ? 'second' : 'before'), records.map(r => r.oldValue)); observer.disconnect();
      }
      await flush();
    }
  }
  for (const [calleeName, callee] of realms) for (const action of ['mutate', 'disconnect', 'takeRecords']) {
    const target = document.createElement('div'), seen = []; let taken = -1;
    const first = new callee.MutationObserver(() => {
      seen.push('first');
      if (action === 'mutate') target.setAttribute('second', 'during');
      if (action === 'disconnect') second.disconnect();
      if (action === 'takeRecords') taken = second.takeRecords().length;
    });
    const second = new callee.MutationObserver(records => {seen.push('second:' + records.length);});
    first.observe(target, {attributes: true, attributeFilter: ['first']});
    second.observe(target, {attributes: true, attributeFilter: ['second']});
    target.setAttribute('second', 'queued'); target.setAttribute('first', 'queued');
    await flush();
    const expected = action === 'mutate' ? ['first', 'second:2'] : ['first'];
    check(`${calleeName}/delivery/${action}`, JSON.stringify(seen) === JSON.stringify(expected) && (action !== 'takeRecords' || taken === 1), {seen, taken});
    first.disconnect(); second.disconnect();
  }
  for (const [calleeName, callee] of realms) {
    {
      const parent = document.createElement('div'), branch = parent.appendChild(document.createElement('section')), leaf = branch.appendChild(document.createElement('span'));
      const trigger = document.createElement('div'), seen = [];
      const first = new callee.MutationObserver(() => {seen.push('first'); leaf.setAttribute('during', 'yes');});
      const second = new callee.MutationObserver(records => {seen.push('second:' + records.length);});
      first.observe(trigger, {attributes: true}); second.observe(parent, {attributes: true, subtree: true});
      trigger.setAttribute('start', 'yes'); parent.removeChild(branch);
      await flush();
      leaf.setAttribute('after', 'yes');
      check(`${calleeName}/delivery/transient-cleanup-at-own-turn`, JSON.stringify(seen) === JSON.stringify(['first', 'second:1']) && second.takeRecords().length === 0, seen);
      first.disconnect(); second.disconnect();
    }
    {
      const firstTarget = document.createElement('div'), secondTarget = document.createElement('div'), seen = [];
      const first = new callee.MutationObserver(() => {seen.push('first'); secondTarget.setAttribute('later', 'yes');});
      const second = new callee.MutationObserver(() => {seen.push('second');});
      first.observe(firstTarget, {attributes: true}); second.observe(secondTarget, {attributes: true});
      firstTarget.setAttribute('start', 'yes'); Promise.resolve().then(() => seen.push('marker'));
      await flush();
      check(`${calleeName}/delivery/newly-pending-waits-next-notification`, JSON.stringify(seen) === JSON.stringify(['first', 'marker', 'second']), seen);
      first.disconnect(); second.disconnect();
    }
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(c => c.passed).length, checks};
  return checks.every(c => c.passed);
})()
