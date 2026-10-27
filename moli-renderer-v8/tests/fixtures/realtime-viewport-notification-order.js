(async () => {
  const tick = () => new Promise(resolve => requestAnimationFrame(resolve));
  const settle = async () => { for (let i = 0; i < 4; ++i) await tick(); };
  const rows = [];
  const create = async () => {
    const frame = document.createElement('iframe');
    frame.style.cssText = 'width:320px;height:100px;border:0';
    document.body.append(frame);
    const child = frame.contentWindow;
    child.document.open();
    child.document.write('<!doctype html><html style="overflow:hidden"><body style="margin:0"></body>');
    child.document.close();
    frame.getBoundingClientRect();
    await settle();
    const events = [];
    const query = child.matchMedia('(min-width:400px)');
    child.addEventListener('resize', () => events.push(['window', child.innerWidth]));
    child.visualViewport.addEventListener('resize', () => events.push(['visual', child.visualViewport.width]));
    query.addEventListener('change', event => events.push(['media', event.matches]));
    return {frame, child, events, query};
  };
  {
    const {frame, events} = await create();
    frame.style.width = '480px'; frame.style.width = '520px';
    await settle();
    rows.push(['coalesced', events]);
    frame.remove();
  }
  {
    const {frame, child, events, query} = await create();
    frame.style.width = '480px'; child.innerWidth;
    const first = query.matches;
    frame.style.width = '320px'; child.innerWidth;
    rows.push(['roundtrip-sync', first, query.matches, events.slice()]);
    await settle();
    rows.push(['roundtrip-events', events]);
    frame.remove();
  }
  {
    const {frame, child, events, query} = await create();
    child.addEventListener('resize', () => {
      if (child.innerWidth === 480) {
        frame.style.width = '320px';
        events.push(['reentrant-read', query.matches, child.innerWidth]);
        queueMicrotask(() => events.push(['microtask']));
      }
    });
    frame.style.width = '480px'; query.matches;
    await settle();
    rows.push(['reentrant', events]);
    frame.remove();
  }
  {
    const {frame, child, events} = await create();
    const inner = child.document.createElement('iframe');
    inner.style.cssText = 'width:50%;height:50px;border:0';
    child.document.body.append(inner);
    const nested = inner.contentWindow;
    nested.document.open();
    nested.document.write('<!doctype html><html style="overflow:hidden"><body style="margin:0"></body>');
    nested.document.close();
    inner.getBoundingClientRect();
    await settle();
    events.length = 0;
    nested.addEventListener('resize', () => events.push(['nested-window', nested.innerWidth]));
    nested.visualViewport.addEventListener('resize', () => events.push(['nested-visual', nested.visualViewport.width]));
    nested.matchMedia('(min-width:200px)').addEventListener('change', event => events.push(['nested-media', event.matches]));
    frame.style.width = '480px'; child.innerWidth;
    requestAnimationFrame(() => events.push(['raf']));
    await settle();
    rows.push(['nested', events]);
    frame.remove();
  }
  return {rows, failures: []};
})()
