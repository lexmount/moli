(async () => {
  const checks = [], sources = __mediaTimelineSources;
  const progress = globalThis.__mediaPendingLoadProgress = {stage: 'setup'};
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  const assert = (value, message) => {if (!value) throw new Error(message);};
  async function until(test, message) {
    const deadline = Date.now() + 3000;
    while (!await test()) {assert(Date.now() < deadline, message); await delay(5);}
  }
  const replace = (doc, loaded, opened) => {
    doc.open();
    if (loaded) doc.defaultView.addEventListener('load', loaded);
    if (opened) opened();
    doc.write('<!doctype html><title>replacement</title><body>replacement</body>');
    doc.close();
  };
  async function frame(html = '<!doctype html><body>child</body>', waitLoad = true) {
    const node = document.createElement('iframe');
    let loaded = false; node.addEventListener('load', () => loaded = true, {once: true});
    node.srcdoc = html; document.body.append(node);
    if (waitLoad) await until(() => loaded, 'child document must load');
    return node;
  }
  async function run(label, body) {
    progress.stage = label;
    try {checks.push({name: label, passed: true, facts: await body()});}
    catch (error) {checks.push({name: label, passed: false, error: String(error)});}
  }
  let gateId = 0;
  const gateUrl = (id, kind) => location.origin + '/gate/' + id + '/' + kind;
  const gate = (id, action = 'status') => fetch(location.origin + '/gate-' + action + '/' + id).then(response => response.json());
  for (const child of [false, true]) for (const kind of ['audio', 'video']) for (const transport of ['data', 'http']) {
    for (const connected of [false, true]) for (const phase of ['microtask', 'loadstart', 'loadedmetadata', 'canplay']) {
      await run([child ? 'child' : 'main', kind, transport, connected ? 'connected' : 'detached', phase].join('/'), async () => {
        let childFrame, media;
        try {
          if (child) childFrame = await frame();
          const doc = child ? childFrame.contentDocument : document, win = doc.defaultView;
          media = doc.createElement(kind); media.preload = 'auto'; media.muted = true;
          if (connected) doc.body.append(media);
          let opened = 0, oldTerminal = 0, newTerminal = 0;
          media.addEventListener('canplaythrough', () => oldTerminal++);
          const open = () => {
            opened++; replace(doc);
            media.addEventListener('canplaythrough', () => newTerminal++, {once: true});
          };
          if (phase !== 'microtask') media.addEventListener(phase, open, {once: true});
          media.src = sources[transport][kind];
          if (phase === 'microtask') {await Promise.resolve(); open();}
          await until(() => opened === 1 && newTerminal === 1, 'pending resource must complete after ' + phase);
          assert(oldTerminal === (connected ? 0 : 1), 'listeners must follow old tree membership');
          assert(media.readyState === 4 && Number.isFinite(media.duration) && media.error === null, 'real resource readiness and metadata');
          assert(media.ownerDocument === doc && doc.defaultView === win, 'Document and Window identities are retained');
          let seeked = 0; media.addEventListener('seeked', () => seeked++, {once: true});
          media.currentTime = 1;
          await until(() => seeked === 1 && !media.seeking, 'retained decoder must finish a seek');
          assert(media.currentTime === 1, 'seek position');
          return {opened, oldTerminal, newTerminal, duration: media.duration};
        } finally {
          if (media) {media.pause(); media.removeAttribute('src'); media.load(); media.remove();}
          if (childFrame) childFrame.remove();
        }
      });
    }
  }
  for (const child of [false, true]) for (const kind of ['audio', 'video']) for (const connected of [false, true]) {
    await run([child ? 'child' : 'main', kind, connected ? 'connected' : 'detached', 'in-flight HTTP and play promise'].join('/'), async () => {
      let childFrame, media; const id = String(++gateId);
      try {
        if (child) childFrame = await frame();
        const doc = child ? childFrame.contentDocument : document;
        media = doc.createElement(kind); media.preload = 'auto'; media.muted = true;
        if (connected) doc.body.append(media);
        let oldTerminal = 0, newTerminal = 0, play = 'pending', docLoads = 0, parentLoads = 0, dcls = 0, completes = 0;
        if (childFrame) childFrame.addEventListener('load', () => parentLoads++);
        media.addEventListener('canplaythrough', () => oldTerminal++);
        media.src = gateUrl(id, kind);
        media.play().then(() => play = 'fulfilled', error => play = error.name);
        await until(async () => (await gate(id)).accepted > 0, 'server must accept the blocked media request');
        assert(media.readyState === 0 && play === 'pending', 'resource and play request must still be pending');
        replace(doc, () => docLoads++, () => {
          doc.addEventListener('DOMContentLoaded', () => dcls++);
          doc.addEventListener('readystatechange', () => {if (doc.readyState === 'complete') completes++;});
        });
        media.addEventListener('canplaythrough', () => newTerminal++, {once: true});
        await until(() => doc.readyState === 'complete' && dcls === 1, 'replacement readiness must not wait for the old media resource');
        const before = await gate(id);
        assert(before.accepted > 0 && !before.released && media.readyState === 0, 'document.open cannot fabricate resource completion');
        const beforeReleaseDocument = {readyState: doc.readyState, loaded: docLoads > 0, parentLoads, dcls, completes};
        assert(docLoads === 0 && parentLoads === 0, 'Window and iframe load must wait for the media load flag');
        await gate(id, 'release');
        await until(() => newTerminal === 1 && play !== 'pending' && docLoads === 1 && (!childFrame || parentLoads === 1), 'media, play, and document load must settle');
        assert(dcls === 1 && completes === 1, 'load continuation must not repeat readiness events');
        assert(oldTerminal === (connected ? 0 : 1), 'old connected listeners must be cleared');
        assert(play === (connected ? 'AbortError' : 'fulfilled'), 'play promise follows removal, not parser generation: ' + play);
        assert(media.paused === connected && media.readyState === 4 && Number.isFinite(media.duration), 'playback intent and metadata');
        return {oldTerminal, newTerminal, play, beforeReleaseDocument};
      } finally {
        await gate(id, 'release');
        if (media) {media.pause(); media.removeAttribute('src'); media.load(); media.remove();}
        if (childFrame) childFrame.remove();
      }
    });
  }
  for (const kind of ['audio', 'video']) {
    await run(kind + '/static parser load delay and replacement document', async () => {
      const id = String(++gateId); let childFrame, media;
      try {
        childFrame = await frame('<!doctype html><body><' + kind + ' preload="auto" src="' + gateUrl(id, kind) + '"></' + kind + '>', false);
        await until(() => childFrame.contentDocument.querySelector(kind) !== null, 'static media must be parsed');
        const doc = childFrame.contentDocument; media = doc.querySelector(kind);
        await until(async () => (await gate(id)).accepted > 0, 'static request must remain in flight');
        const originalReadyState = doc.readyState;
        assert(originalReadyState !== 'complete', 'the static resource must delay its original parser');
        let loaded = 0, docLoads = 0, parentLoads = 0;
        childFrame.addEventListener('load', () => parentLoads++);
        replace(doc, () => docLoads++);
        media.addEventListener('canplaythrough', () => loaded++, {once: true});
        await delay(30);
        const beforeReleaseDocument = {readyState: doc.readyState, loaded: docLoads > 0, parentLoads};
        assert(docLoads === 0 && parentLoads === 0, 'static media still delays Window and iframe load');
        assert(doc.readyState === 'complete', 'replacement parser is independent of the old media delay');
        await gate(id, 'release');
        await until(() => loaded === 1 && docLoads === 1 && parentLoads === 1, 'static media and deferred document load survive parser replacement');
        assert(media.readyState === 4 && Number.isFinite(media.duration), 'static resource metadata');
        return {originalReadyState, beforeReleaseDocument, finalDocumentReadyState: doc.readyState};
      } finally {
        await gate(id, 'release');
        if (media) {media.removeAttribute('src'); media.load();}
        if (childFrame) childFrame.remove();
      }
    });
  }
  for (const child of [false, true]) for (const kind of ['audio', 'video']) for (const reset of [false, true]) {
    await run([child ? 'child' : 'main', kind, reset ? 'load reset releases barrier' : 'two parser replacements retain barrier'].join('/'), async () => {
      const id = String(++gateId); let childFrame, media;
      try {
        if (child) childFrame = await frame();
        const doc = child ? childFrame.contentDocument : document;
        media = doc.createElement(kind); media.preload = 'auto'; media.muted = true;
        media.src = gateUrl(id, kind);
        await until(async () => (await gate(id)).accepted > 0, 'retained resource must be in flight');
        let firstLoads = 0, finalLoads = 0, parentLoads = 0;
        if (childFrame) childFrame.addEventListener('load', () => parentLoads++);
        replace(doc, () => firstLoads++);
        await until(() => doc.readyState === 'complete', 'first replacement parser must complete');
        assert(firstLoads === 0 && parentLoads === 0, 'first parser cannot consume the resource flag');
        replace(doc, () => finalLoads++);
        await until(() => doc.readyState === 'complete', 'second replacement parser must complete');
        assert(firstLoads === 0 && finalLoads === 0 && parentLoads === 0, 'repeated document.open retains the resource flag');
        let terminal = 0; media.addEventListener('canplaythrough', () => terminal++);
        if (reset) {
          media.removeAttribute('src'); media.load();
          await until(() => finalLoads === 1 && (!childFrame || parentLoads === 1), 'load reset must release the old resource flag without its network response');
          assert(!(await gate(id)).released, 'old network response is still blocked when load fires');
        }
        await gate(id, 'release');
        await until(() => finalLoads === 1 && (!childFrame || parentLoads === 1) && (reset || terminal === 1), 'only the current parser receives deferred load');
        if (reset) {
          await until(async () => (await gate(id)).finished > 0, 'historical response must finish');
          await delay(30);
          assert(terminal === 0 && media.readyState === 0, 'reset resource cannot resume from old completion: ' + JSON.stringify({terminal, readyState: media.readyState, currentSrc: media.currentSrc}));
        }
        assert(firstLoads === 0 && finalLoads === 1, 'old parser load listener is cleared and new load fires once');
        return {firstLoads, finalLoads, parentLoads, terminal, currentSrc: media.currentSrc};
      } finally {
        await gate(id, 'release');
        if (media) {media.pause(); media.removeAttribute('src'); media.load();}
        if (childFrame) childFrame.remove();
      }
    });
  }
  for (const kind of ['audio', 'video']) for (const removed of [false, true]) {
    await run([kind, removed ? 'removed Window' : 'navigated Window', 'stale network completion'].join('/'), async () => {
      const id = String(++gateId); let childFrame, media;
      try {
        childFrame = await frame();
        const oldDocument = childFrame.contentDocument;
        media = oldDocument.createElement(kind); media.preload = 'auto'; oldDocument.body.append(media);
        let oldEvents = 0;
        for (const type of ['loadedmetadata', 'canplaythrough', 'error']) media.addEventListener(type, () => oldEvents++);
        media.src = gateUrl(id, kind);
        await until(async () => (await gate(id)).accepted > 0, 'old Window request must be accepted');
        if (removed) childFrame.remove();
        else {
          let loaded = false; childFrame.addEventListener('load', () => loaded = true, {once: true});
          childFrame.srcdoc = '<!doctype html><body>new Window</body>';
          await until(() => loaded && childFrame.contentDocument !== oldDocument, 'navigation must replace the Document');
        }
        await gate(id, 'release');
        await until(async () => (await gate(id)).finished > 0, 'server must finish the historical response');
        await delay(30);
        assert(oldEvents === 0, 'retired Window cannot deliver historical media callbacks');
        if (!removed) assert(childFrame.contentDocument.body.textContent === 'new Window', 'replacement document is unchanged');
        return {oldEvents};
      } finally {await gate(id, 'release'); if (childFrame) childFrame.remove();}
    });
  }
  globalThis.__uiEventResults = {total: checks.length, checks};
})().catch(error => {globalThis.__uiEventResults = {total: 1, checks: [{name: 'fixture setup', passed: false, error: String(error)}]};});
