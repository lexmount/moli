(() => {
  const rows = [], H = 'http://www.w3.org/1999/xhtml';
  const realms = [['main', globalThis], ['child', document.querySelector('#child').contentWindow]];
  const check = (name, body) => {
    try { rows.push({name, passed: body() === true}); }
    catch (error) {rows.push({name, passed: false, error: String(error)});}
  };
  const same = (left, right) => left.length === right.length && left.every((value, index) => value === right[index]);
  const documents = (owner, flavor) => flavor === 'live' ? owner.document : flavor === 'html' ?
    owner.document.implementation.createHTMLDocument('bulk removal') : flavor === 'parsed' ?
    new owner.DOMParser().parseFromString('<!doctype html><body>', 'text/html') :
    owner.document.implementation.createDocument(H, 'html');
  const modes = ['default', 'first', 'middle', 'dirty-middle', 'last', 'multiple', 'listbox'];
  for (const [ownerName, owner] of realms) for (const flavor of ['live', 'html', 'parsed', 'xml']) {
    const doc = documents(owner, flavor);
    for (const [calleeName, callee] of realms) for (const grouped of [false, true]) for (const mode of modes) {
      for (const sink of ['innerHTML', 'textContent', 'replaceChildren', 'setHTMLUnsafe']) for (const replacement of [false, true]) {
        const label = `${calleeName}/${ownerName}/${flavor}/${grouped}/${mode}/${sink}/${replacement}`;
        check(label, () => {
          const select = doc.createElementNS(H, 'select');
          select.multiple = mode === 'multiple';
          if (mode === 'listbox') select.size = 3;
          const group = grouped ? select.appendChild(doc.createElementNS(H, 'optgroup')) : select;
          const options = ['one', 'two', 'three'].map(value => {
            const option = doc.createElementNS(H, 'option');option.value = value;option.textContent = value;
            if ((mode === 'first' && value === 'one') ||
                (['middle', 'listbox'].includes(mode) && value === 'two') ||
                (mode === 'last' && value === 'three') ||
                (mode === 'multiple' && value !== 'three')) option.defaultSelected = true;
            group.appendChild(option);return option;
          });
          (doc.body || doc.documentElement).appendChild(select);
          if (mode === 'dirty-middle') select.value = 'two';
          const selected = options.map(option => option.selected);
          const original = [...select.childNodes];
          const live = select.selectedOptions;
          const observer = new owner.MutationObserver(() => {});
          observer.observe(select, {childList: true});
          try {
            if (sink === 'innerHTML') Object.getOwnPropertyDescriptor(callee.Element.prototype, 'innerHTML').set.call(select,
              replacement ? '<option selected="">new</option>' : '');
            else if (sink === 'textContent') Object.getOwnPropertyDescriptor(callee.Node.prototype, 'textContent').set.call(select,
              replacement ? 'new' : '');
            else if (sink === 'replaceChildren') {
              const method = callee.Element.prototype.replaceChildren;
              if (replacement) method.call(select, doc.createElementNS(H, 'option'));
              else method.call(select);
            } else callee.Element.prototype.setHTMLUnsafe.call(select,
              replacement ? '<option selected="">new</option>' : '');
            const records = observer.takeRecords();
            const removedSelectedness = options.map(option => option.selected);
            const added = [...select.childNodes];
            if (!same(removedSelectedness, selected) || !options.every(option => option.ownerDocument === doc) ||
                records.length !== 1 || records[0].target !== select || records[0].previousSibling !== null || records[0].nextSibling !== null ||
                !same([...records[0].removedNodes], original) || !same([...records[0].addedNodes], added)) return false;
            // Clear only the newly constructed children; the original options
            // are still detached and their selectedness must remain intact.
            while (select.firstChild) select.removeChild(select.firstChild);
            if (live.length !== 0) return false;
            if (grouped) {
              for (const option of options) if (option.parentNode !== group) return false;
              select.appendChild(group);
            } else for (const option of options) select.appendChild(option);
            return same(options.map(option => option.selected), selected) &&
              same([...live], options.filter((_, index) => selected[index]));
          } finally {observer.disconnect();select.remove();}
        });
      }
    }
  }
  // Individual removals remain observable operations: the select chooses a
  // new fallback after each removal. They must not inherit batch semantics.
  for (const [ownerName, owner] of realms) {
    check(ownerName+'/sequential removal preserves its separate fallback selections', () => {
      const select = owner.document.createElement('select');
      select.innerHTML = '<option>one</option><option selected>two</option><option>three</option>';
      const options = [...select.options];
      select.removeChild(options[0]);select.removeChild(options[1]);select.removeChild(options[2]);
      const flags = options.map(option => option.selected);
      options.forEach(option => select.appendChild(option));
      return same(flags, [false, true, true]) && select.value === 'three';
    });
  }
  globalThis.__uiEventResults = {complete: true, total: rows.length, passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
