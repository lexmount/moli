(async () => {
  const rows = [];
  const assert = (condition, message) => { if (!condition) throw Error(message); };
  const test = async (name, run) => {
    try { await run(); rows.push({name, passed: true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  };
  const load = (frame, assign, insert) => new Promise((resolve, reject) => {
    const loaded = () => { frame.removeEventListener('error', failed); resolve(); };
    const failed = () => { frame.removeEventListener('load', loaded); reject(Error('frame load failed')); };
    frame.addEventListener('load', loaded, {once: true});
    frame.addEventListener('error', failed, {once: true});
    assign();
    if (insert) insert();
  });
  let sequence = 0;
  for (const owner of ['main', 'child']) {
    let holder;
    const realm = owner === 'main' ? window :
      (holder = document.body.appendChild(document.createElement('iframe'))).contentWindow;
    const entries = key => realm.performance.getEntriesByType('resource').filter(entry => entry.name.includes(key));
    const url = (origin, key, tao) => `${__fetchOrigins[origin]}/frame-document?key=${key}&${tao ? 'tao=*' : ''}`;
    try {
      for (const setter of ['idl', 'attribute', 'namespace']) for (const from of [0, 1]) for (const to of [0, 1]) for (const tao of [true, false]) {
        await test(`${owner} ${setter} ${from}->${to} TAO ${tao}`, async () => {
          const key = `frame-provenance-${++sequence}`;
          const frame = realm.document.createElement('iframe');
          const write = value => {
            if (setter === 'idl') frame.src = value;
            else if (setter === 'attribute') frame.setAttribute('src', value);
            else frame.setAttributeNS(null, 'src', value);
          };
          try {
            const first = url(from, key, tao), second = url(to, key, tao);
            await load(frame, () => write(first), () => realm.document.body.appendChild(frame));
            assert(entries(key).length === 1, 'first entry precedes the owner load callback');
            await load(frame, () => write(second));
            const result = entries(key);
            assert(result.length === 2, 'each attribute navigation reports exactly one entry, including the same URL');
            assert(result[0].name === first && result[1].name === second, 'each entry retains its initiating URL');
            assert(result[0].responseEnd <= result[1].startTime, 'the second navigation has its own clock');
            for (const [index, origin] of [from, to].entries()) {
              const entry = result[index];
              assert(entry instanceof realm.PerformanceResourceTiming, 'entry belongs to the owner Document realm');
              assert(entry.initiatorType === 'iframe' && entry.responseEnd <= realm.performance.now(), 'owner initiator and completed timing');
              if (origin === 1 && !tao)
                assert(entry.connectStart === 0 && entry.requestStart === 0 && entry.transferSize === 0, 'cross-origin TAO filtering survives navigation');
              else assert(entry.requestStart >= entry.startTime && entry.responseStart >= entry.requestStart, 'native request timing survives navigation');
            }
          } finally { frame.remove(); }
        });
      }
      for (const operation of ['assign', 'reload']) {
        await test(`${owner} child Location.${operation} keeps parent timing empty`, async () => {
          const key = `frame-location-${++sequence}`;
          const frame = realm.document.createElement('iframe');
          try {
            await load(frame, () => { frame.src = url(0, key, true); }, () => realm.document.body.appendChild(frame));
            assert(entries(key).length === 1, 'initial attribute navigation is reported');
            await load(frame, () => {
              if (operation === 'assign') frame.contentWindow.location.assign(url(0, key + '-next', true));
              else frame.contentWindow.location.reload();
            });
            assert(entries(key).length === 1, 'browsing-context navigation does not acquire an owner-attribute initiator');
          } finally { frame.remove(); }
        });
      }
      for (const setter of ['attribute', 'namespace']) {
        await test(`${owner} ${setter} keeps UTF-16 while navigating a repeated URL`, async () => {
          const key = `frame-utf16-${++sequence}`;
          const target = url(0, key, true) + '&literal=' + String.fromCharCode(0xd800);
          const frame = realm.document.createElement('iframe');
          const assign = () => {
            if (setter === 'attribute') frame.setAttribute('src', target);
            else frame.setAttributeNS(null, 'src', target);
          };
          try {
            await load(frame, assign, () => realm.document.body.appendChild(frame));
            assert(frame.getAttribute('src') === target, 'first attribute keeps the original lone surrogate');
            await load(frame, assign);
            assert(frame.getAttribute('src') === target && entries(key).length === 2, 'repeated UTF-16 attribute also performs a new navigation');
            assert(entries(key).every(entry => entry.name === new realm.URL(target).href), 'network timing uses the parsed URL');
          } finally { frame.remove(); }
        });
      }
      await test(`${owner} src navigation inside owner load callback retains the new owner`, async () => {
        const key = `frame-reentrant-${++sequence}`;
        const frame = realm.document.createElement('iframe');
        try {
          let loads = 0;
          await new Promise((resolve, reject) => {
            frame.onerror = () => reject(Error('reentrant frame load failed'));
            frame.onload = () => {
              if (++loads === 1) frame.src = url(0, key + '-second', true);
              else resolve();
            };
            frame.src = url(0, key + '-first', true);
            realm.document.body.appendChild(frame);
          });
          assert(loads === 2 && entries(key).length === 2, 'old load completion cannot consume the next navigation timing');
        } finally { frame.remove(); }
      });
    } finally { if (holder) holder.remove(); }
  }
  return JSON.stringify({total: rows.length, failures: rows.filter(row => !row.passed)});
})();
