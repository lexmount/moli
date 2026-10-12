(async () => {
  const frame = document.createElement('iframe');
  frame.style.cssText = 'width:320px;height:100px;border:0';
  document.body.append(frame);
  const child = frame.contentWindow;
  child.document.open();
  child.document.write('<!doctype html><html style="overflow:hidden"><body style="margin:0"></body>');
  child.document.close();
  frame.getBoundingClientRect();
  const tick = () => new Promise(resolve => requestAnimationFrame(resolve));
  await tick();
  await tick();
  const events = [];
  const query = child.matchMedia('(min-width:400px)');
  child.addEventListener('resize', () => events.push(['window', child.innerWidth]));
  child.visualViewport.addEventListener('resize', () => events.push(['visual', child.visualViewport.width]));
  query.addEventListener('change', event => events.push(['media', event.matches]));
  frame.style.width = '480px';
  const immediate = query.matches;
  const synchronous = events.slice();
  for (let index = 0; index < 4; ++index) await tick();
  const rows = [['immediate', immediate], ['synchronous', synchronous], ['events', events]];
  const failures = [];
  if (!immediate) failures.push('immediate');
  if (synchronous.length) failures.push('synchronous');
  for (const [type, expected] of [['window', 480], ['visual', 480], ['media', true]]) {
    const matching = events.filter(event => event[0] === type);
    if (matching.length !== 1 || matching[0][1] !== expected) failures.push(type);
  }
  return {rows, failures};
})()
