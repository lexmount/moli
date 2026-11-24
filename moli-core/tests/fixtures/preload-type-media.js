(async () => {
  const resource = new URL(globalThis.preloadProbeResourcePath || '/static', location.href);
  const mediaType = globalThis.preloadProbeMediaType || 'screen';
  const narrowViewport = globalThis.preloadProbeNarrowViewport || false;
  const cases = [
    ['style', 'text/html', null, false],
    ['fetch', 'not-a-mime', 'not all', false]
  ];
  for (const [as, mime] of [
    ['fetch', 'application/octet-stream'], ['font', 'font/ttf'], ['image', 'image/png'],
    ['script', 'text/javascript'], ['style', 'text/css'], ['track', 'text/vtt']
  ]) {
    for (const [type, supported] of [
      [null, true], ['', true], [mime, true], [mime.toUpperCase() + '; charset=UTF-8', true],
      ['application/x-unknown', false], ['not-a-mime', false], [' ', false],
      ['\u00a0' + mime, false], ['*/*', false]
    ]) cases.push([as, type, null, as === 'fetch' || supported]);
    for (const [media, supported] of [
      ['screen', mediaType === 'screen'], ['print', mediaType === 'print'], ['not all', false],
      ['all and (min-width: 0px)', true], ['(max-width: 0px)', false],
      ['(max-width: 300px)', narrowViewport], ['(min-width: 500px)', !narrowViewport],
      ['(min-width: 100000px)', false], ['invalid-media-type', false]
    ]) cases.push([as, mime, media, supported]);
  }
  cases.push(
    ['image', 'image/unknown', null, false], ['font', 'font/not-a-font', null, false],
    ['font', 'application/font-woff', null, true], ['font', 'font/collection', null, true],
    ['font', 'font/sfnt', null, true], ['style', 'text/css;garbage', null, true],
    ['style', '\ttext/css \r\n', null, true], ['style', 'text/css, text/css', null, false],
    ['style', 'text/html', null, false, 'stylesheet preload']
  );
  const href = (context, index) => {
    const url = new URL(resource);
    url.searchParams.set('preload_type_media', context + '-' + index);
    return url.href;
  };
  const escape = value => value.replaceAll('&', '&amp;').replaceAll('"', '&quot;');
  const parsed = Array.from(document.querySelectorAll('link[data-preload-probe]'));
  const context = parsed.length ? 'top-parser' : 'top-dynamic';
  const records = cases.map(([as, type, media, _, rel = 'preload'], index) => {
    if (parsed.length) return {link: parsed[index], events: window.preloadEvents[index]};
    const link = document.createElement('link');
    Object.assign(link, {rel, as, href: href(context, index)});
    if (type !== null) link.type = type;
    if (media !== null) link.media = media;
    const events = [];
    link.onload = link.onerror = event => events.push(event.type);
    document.head.append(link);
    return {link, events};
  });
  const parserMarkup = '<!doctype html><head><script>window.preloadEvents = ' +
    JSON.stringify(cases.map(() => [])) + ';<' + '/script>' +
    cases.map(([as, type, media, _, rel = 'preload'], index) =>
      '<link data-preload-probe rel="' + rel + '" ' +
      'as="' + as + '" href="' + escape(href('top-parser', index)) + '" ' +
      (type === null ? '' : 'type="' + escape(type) + '" ') +
      (media === null ? '' : 'media="' + escape(media) + '" ') +
      'onload="preloadEvents[' + index + '].push(event.type)" ' +
      'onerror="preloadEvents[' + index + '].push(event.type)">').join('') +
    '</head><body>preload type and media';
  await Promise.race([
    Promise.all(records.filter((_, index) => cases[index][3]).map(({link, events}) =>
      events.length ? undefined : new Promise(resolve => {
        link.addEventListener('load', resolve, {once: true});
        link.addEventListener('error', resolve, {once: true});
      }))),
    new Promise(resolve => setTimeout(resolve, 5000))
  ]);
  await new Promise(resolve => setTimeout(resolve, 100));
  const failures = [];
  const observations = records.map(({link, events}, index) => {
    const [as, type, media, supported] = cases[index];
    const count = performance.getEntriesByName(link.href).length;
    if (count !== Number(supported) || events.length !== Number(supported))
      failures.push({context, as, type, media, supported, count, events});
    return {as, type, media, supported, count, events};
  });
  const recoveries = await Promise.all(records.slice(0, 2).map(({link}, index) =>
    new Promise(resolve => {
      const timer = setTimeout(() => resolve(false), 5000);
      link.onload = link.onerror = () => {
        clearTimeout(timer);
        resolve(performance.getEntriesByName(link.href).length === 1);
      };
      if (index === 0) link.type = 'text/css';
      else link.media = 'all';
    })));
  const recovered = recoveries.every(Boolean);
  if (!recovered) failures.push({recoveries});
  records.forEach(({link}) => link.remove());
  return {context, recovered, observations, failures, parserMarkup};
})()
