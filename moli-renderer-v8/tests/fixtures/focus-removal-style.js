(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const frame = body.appendChild(document.createElement('iframe'));
  const failures = [];
  for (const doc of [document, frame.contentDocument]) {
    for (const depth of [1, 2]) for (const operation of ['remove', 'ancestor', 'adopt']) {
      const container = doc.body.appendChild(doc.createElement('div'));
      const observed = [];
      let parent = container;
      for (let i = 0; i < depth; i++) {
        const style = doc.createElement('style');
        style.textContent = '.focus-host + span { color: rgb(0,0,0) } .focus-host:focus + span { color: rgb(255,0,0) } .focus-host:focus-within + span { background-color: rgb(0,0,255) }';
        const host = doc.createElement('div'); host.className = 'focus-host';
        const sibling = doc.createElement('span');
        parent.append(style, host, sibling);
        observed.push([host, sibling]);
        parent = host.attachShadow({mode:'open'});
      }
      const wrapper = doc.createElement('section'), input = doc.createElement('input');
      wrapper.append(input); parent.append(wrapper); input.focus();
      const label = `${doc === document ? 'main' : 'child'}/${depth}/${operation}`;
      for (const [host, sibling] of observed) {
        if (!host.matches(':focus') || getComputedStyle(sibling).color !== 'rgb(255, 0, 0)')
          failures.push(`${label}/initial host focus style`);
        getComputedStyle(sibling).backgroundColor;
      }
      if (operation === 'remove') input.remove();
      else if (operation === 'ancestor') wrapper.remove();
      else document.implementation.createHTMLDocument('').adoptNode(input);
      for (const [host, sibling] of observed) {
        if (host.matches(':focus') || getComputedStyle(sibling).color !== 'rgb(0, 0, 0)')
          failures.push(`${label}/stale host focus style`);
        if (host.matches(':focus-within') || getComputedStyle(sibling).backgroundColor === 'rgb(0, 0, 255)')
          failures.push(`${label}/stale host focus-within style`);
      }
      container.remove();
    }
  }
  frame.remove();
  return failures.length ? JSON.stringify(failures) : '';
})()
