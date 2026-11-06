(async () => {
  const tick = () => new Promise(resolve => requestAnimationFrame(resolve));
  const frame = document.createElement('iframe');
  frame.style.cssText = 'width:300px;height:100px;border:0';
  document.body.append(frame);
  const child = frame.contentWindow;
  const events = [];
  child.addEventListener('resize', () => events.push(['resize', child.innerWidth]));
  child.visualViewport.addEventListener('resize', () => events.push(['visual', child.innerWidth]));
  const promise = child.document.fonts.ready;
  promise.then(() => events.push(['ready']));
  queueMicrotask(() => events.push(['microtask']));
  requestAnimationFrame(() => events.push(['raf']));
  const synchronous = events.slice();
  for (let i = 0; i < 4; ++i) await tick();
  const rows = [['synchronous', synchronous], ['initial', events.slice()]];
  const target = child.document.createElement('div');
  target.style.cssText = 'width:100px;height:20px;padding:0;border:0';
  child.document.body.append(target);
  const before = target.offsetWidth;
  target.style.width = '240px';
  const same = promise === child.document.fonts.ready;
  rows.push(['resolved-read', same, before, target.offsetWidth, child.document.fonts.status]);
  for (let i = 0; i < 3; ++i) await tick();
  events.length = 0;
  frame.style.width = '360px';
  requestAnimationFrame(() => events.push(['raf']));
  for (let i = 0; i < 4; ++i) await tick();
  rows.push(['later-width', events.slice()]);
  frame.remove();
  // A font-loading task and an already eligible rendering opportunity are
  // different sources. Their order is not a cross-source FIFO guarantee.
  const initial = rows[1][1];
  const later = rows[3][1];
  const failures = [];
  const check = (name, condition) => { if (!condition) failures.push(name); };
  check('initial readiness is asynchronous', synchronous.length === 0 &&
    initial.findIndex(e => e[0] === 'microtask') < initial.findIndex(e => e[0] === 'ready'));
  check('initial resize is delivered once',
    initial.filter(e => e[0] === 'resize').length === 1 && initial.some(e => e[0] === 'resize' && e[1] === 300));
  check('initial visual resize is delivered once',
    initial.filter(e => e[0] === 'visual').length === 1 && initial.some(e => e[0] === 'visual' && e[1] === 300));
  check('rAF progresses', initial.filter(e => e[0] === 'raf').length === 1);
  check('resolved read uses current geometry',
    JSON.stringify(rows[2]) === JSON.stringify(['resolved-read', true, 100, 240, 'loaded']));
  check('later viewport updates progress',
    later.filter(e => e[0] === 'resize').length === 1 && later.some(e => e[0] === 'resize' && e[1] === 360) &&
    later.filter(e => e[0] === 'visual').length === 1 && later.some(e => e[0] === 'visual' && e[1] === 360) &&
    later.filter(e => e[0] === 'raf').length === 1);
  return {rows, failures};
})()
