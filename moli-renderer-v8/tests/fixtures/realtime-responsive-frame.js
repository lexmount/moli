(async () => {
  const tick = () => new Promise(resolve => requestAnimationFrame(resolve));
  const frame = document.createElement('iframe');
  frame.style.cssText = 'width:320px;height:100px;border:0';
  document.body.append(frame);
  const child = frame.contentWindow;
  child.document.open();
  child.document.write('<!doctype html><html style="overflow:hidden"><body style="margin:0"></body>');
  child.document.close();
  frame.getBoundingClientRect();
  for (let i = 0; i < 4; ++i) await tick();
  const rows = [];
  child.addEventListener('resize', () => rows.push(['window', child.innerWidth]));
  child.visualViewport.addEventListener('resize', () => rows.push(['visual', child.visualViewport.width]));
  const query = child.matchMedia('(min-width:400px)');
  await new Promise(resolve => {
    let changes = 0;
    query.onchange = event => {
      rows.push(['media', event.matches]);
      if (++changes === 3) resolve();
      else frame.style.width = event.matches ? '320px' : '480px';
    };
    // From here, only lifecycle notifications drive the next update. There
    // are no polling getters, timers, captures or further rAF registrations.
    frame.style.width = '480px';
  });
  frame.remove();
  return {rows, failures: []};
})()
