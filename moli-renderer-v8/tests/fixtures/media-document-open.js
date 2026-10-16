(async () => {
  const checks = [];
  const sources = globalThis.__mediaTimelineSources;
  const progress = globalThis.__mediaDefaultStartProgress = {stage: 'starting'};
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  const check = (name, passed, facts) => checks.push({name, passed, facts});
  const once = (target, name) => new Promise((resolve, reject) => {
    const timer = setTimeout(() => {target.removeEventListener(name, done); reject(new Error('waiting for ' + name));}, 1500);
    const done = () => {clearTimeout(timer); resolve();};
    target.addEventListener(name, done, {once: true});
  });
  const replace = doc => {
    doc.open();
    doc.write('<!doctype html><title>replacement</title><body>replacement</body>');
    doc.close();
  };
  const ranges = media => Array.from({length: media.played.length}, (_, i) => [media.played.start(i), media.played.end(i)]);
  for (const child of [false, true]) for (const kind of ['audio', 'video']) {
    for (const transport of ['data', 'http']) for (const connected of [false, true]) {
      for (const reentrant of [false, true]) {
        const label = [child ? 'child' : 'main', kind, transport, connected ? 'connected' : 'detached', reentrant ? 'timeupdate' : 'pending'].join('/');
        progress.stage = label;
        let frame;
        let media;
        try {
          if (child) {
            frame = document.createElement('iframe');
            const loaded = once(frame, 'load');
            frame.srcdoc = '<!doctype html><body>child</body>';
            document.body.append(frame);
            await loaded;
          }
          const doc = child ? frame.contentDocument : document;
          const win = doc.defaultView;
          media = doc.createElement(kind);
          media.muted = true;
          if (connected) doc.body.append(media);
          const ready = once(media, 'canplaythrough');
          media.src = sources[transport][kind];
          await ready;
          const duration = media.duration;
          const src = media.currentSrc;
          let oldSeeked = 0, newSeeked = 0, opened = 0;
          media.addEventListener('seeked', () => oldSeeked++);
          let done;
          const finished = new Promise(resolve => {done = resolve;});
          const open = () => {
            opened++;
            replace(doc);
            media.addEventListener('seeked', () => {newSeeked++; done();}, {once: true});
          };
          if (reentrant) media.addEventListener('timeupdate', open, {once: true});
          media.currentTime = 1;
          if (!reentrant) open();
          await Promise.race([finished, delay(1500)]);
          check(label + '/queued notification', opened === 1 && newSeeked === 1, {opened, newSeeked});
          check(label + '/old listeners follow tree membership', oldSeeked === (connected ? 0 : 1), {oldSeeked});
          check(label + '/selected resource survives', media.ownerDocument === doc && doc.defaultView === win && media.duration === duration && media.currentSrc === src && media.currentTime === 1 && !media.seeking && media.readyState >= 1,
            {duration: media.duration, currentTime: media.currentTime, readyState: media.readyState, seeking: media.seeking});
          const next = once(media, 'seeked');
          media.currentTime = 2;
          await next;
          check(label + '/subsequent seek uses retained resource', media.currentTime === 2 && media.duration === duration && !media.seeking);
        } catch (error) {
          check(label + '/completion', false, String(error));
        } finally {
          if (media) {media.pause(); media.removeAttribute('src'); media.load(); media.remove();}
          if (frame) frame.remove();
        }
      }
    }
  }
  for (const child of [false, true]) for (const kind of ['audio', 'video']) for (const transport of ['data', 'http']) {
    const label = [child ? 'child' : 'main', kind, transport, 'history'].join('/');
    progress.stage = label;
    let frame, media;
    try {
      if (child) {
        frame = document.createElement('iframe');
        const loaded = once(frame, 'load');
        frame.srcdoc = '<!doctype html><body>child</body>';
        document.body.append(frame);
        await loaded;
      }
      const doc = child ? frame.contentDocument : document;
      media = doc.createElement(kind); media.muted = true;
      const ready = once(media, 'canplaythrough'); media.src = sources[transport][kind]; await ready;
      await media.play();
      await once(media, 'timeupdate');
      const paused = once(media, 'pause'); media.pause(); await paused;
      const before = JSON.stringify(ranges(media));
      const duration = media.duration;
      const snapshot = media.played;
      replace(doc);
      check(label + '/resource history retained', media.duration === duration && media.played.length > 0 && JSON.stringify(ranges(media)) === before && snapshot.length === media.played.length, {before, after: ranges(media), duration: media.duration});
      const tick = once(media, 'timeupdate'); await media.play(); await tick; media.pause();
      check(label + '/clock continues in same Window', media.currentTime > snapshot.end(snapshot.length - 1) && media.played.end(0) > snapshot.end(0));
    } catch (error) {check(label + '/completion', false, String(error));}
    finally {if (media) {media.pause(); media.removeAttribute('src'); media.load();} if (frame) frame.remove();}
  }
  globalThis.__uiEventResults = {total: checks.length, checks};
})().catch(error => {globalThis.__uiEventResults = {total: 1, checks: [{name: 'fixture completion', passed: false, facts: String(error)}]};});
