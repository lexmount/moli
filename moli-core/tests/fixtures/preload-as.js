(async () => {
  const failures = [];
  const observations = [];
  const resource = new URL(globalThis.preloadProbeResourcePath || '/static', location.href);
  const cases = [
    [null, false], ['', false], [' ', false],
    ['garbagefoobar', false], ['audio', false], ['video', false],
    ['object', false], ['iframe', false], ['worklet', false],
    ['json', false], ['text', false], ['worker', false],
    ['audioworklet', false], ['paintworklet', false], ['serviceworker', false],
    ['sharedworker', false], ['document', false], ['embed', false],
    ['frame', false], ['manifest', false], ['report', false],
    ['webidentity', false], ['xslt', false],
    [' fetch', false], ['fetch ', false], ['\tfetch\n', false],
    ['\u00a0fetch', false], [' style ', false], [' script ', false],
    ['fetch', true], ['FETCH', true], ['script', true],
    ['style', true], ['image', true], ['font', true], ['track', true]
  ];
  const href = (context, index) => {
    const url = new URL(resource);
    url.searchParams.set('preload_probe', context + '-' + index);
    return url.href;
  };
  const escape = value => value.replaceAll('&', '&amp;').replaceAll('"', '&quot;');
  const install = (doc, context) => cases.map(([value], index) => {
    const link = doc.createElement('link');
    link.rel = 'preload';
    if (value !== null) link.setAttribute('as', value);
    link.href = href(context, index);
    const events = [];
    link.onload = event => events.push(event.type);
    link.onerror = event => events.push(event.type);
    doc.head.append(link);
    return {link, events};
  });

  const parsed = Array.from(document.querySelectorAll('link[data-preload-probe]'));
  const context = parsed.length ? 'top-parser' : 'top-dynamic';
  const topLinks = parsed.length
    ? parsed.map((link, index) => ({link, events: window.preloadEvents[index]}))
    : install(document, context);
  const parserMarkup = '<!doctype html><head><script>window.preloadEvents = ' +
    JSON.stringify(cases.map(() => [])) + ';<' + '/script>' +
    cases.map(([value], index) => '<link data-preload-probe rel="preload" ' +
      (value === null ? '' : 'as="' + escape(value) + '" ') +
      'href="' + escape(href('top-parser', index)) + '" ' +
      'onload="preloadEvents[' + index + '].push(event.type)" ' +
      'onerror="preloadEvents[' + index + '].push(event.type)">').join('') +
    '</head><body>parser';
  await new Promise(resolve => setTimeout(resolve, 1000));

  for (let index = 0; index < cases.length; ++index) {
    const [value, supported] = cases[index];
    const {link, events} = topLinks[index];
    const count = performance.getEntriesByName(link.href).length;
    observations.push({context, value, count, events});
    if (count !== Number(supported))
      failures.push([context, value, 'resource entries', count]);
    if (events.length !== Number(supported))
      failures.push([context, value, 'terminal events', events]);
  }

  // Unsupported input must not permanently mark a link as already processed.
  const recovery = topLinks[0];
  const recovered = await new Promise(resolve => {
    recovery.link.onload = () => resolve(true);
    recovery.link.onerror = () => resolve(false);
    recovery.link.as = 'FETCH';
    setTimeout(() => resolve(false), 2000);
  });
  if (!recovered || performance.getEntriesByName(recovery.link.href).length !== 1)
    failures.push(['missing-to-fetch', 'did not fetch exactly once']);
  for (const {link} of topLinks) link.remove();
  return {context, recovered, failures, observations, parserMarkup};
})()
