(async () => {
  const checks = [];
  const progress = globalThis.__mediaPlayedProgress = {stage: 'setup', checks};
  const realms = [['main', globalThis], ['iframe', document.querySelector('iframe').contentWindow]];
  const assert = (condition, message) => {if (!condition) throw Error(message);};
  const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
  const read = (callee, media) => Object.getOwnPropertyDescriptor(callee.HTMLMediaElement.prototype, 'played').get.call(media);
  const pairs = ranges => Array.from({length: ranges.length}, (_, i) => [ranges.start(i), ranges.end(i)]);
  const equal = (actual, expected, message) => assert(JSON.stringify(actual) === JSON.stringify(expected), message);
  const method = (callee, media, name) => callee.HTMLMediaElement.prototype[name].call(media);
  async function until(condition, message) {
    const deadline = Date.now() + 5000;
    while (!condition()) {assert(Date.now() < deadline, message); await wait(5);}
  }
  async function ready(media, source) {
    let done = false;
    media.addEventListener('canplaythrough', () => done = true, {once: true});
    media.src = source; media.load();
    await until(() => done, 'real resource must become ready');
    assert(media.error === null && media.readyState >= 3 && Number.isFinite(media.duration), 'finite resource readiness');
  }
  async function seek(media, time) {
    let done = false;
    media.addEventListener('seeked', () => done = true, {once: true});
    media.currentTime = time;
    await until(() => done && !media.seeking, 'seek must finish');
  }
  async function segment(callee, media, end) {
    await method(callee, media, 'play');
    await until(() => media.currentTime > end, 'playback must advance');
    method(callee, media, 'pause');
    const ranges = read(callee, media);
    assert(ranges.length > 0, 'real playback must produce a range');
    return ranges;
  }
  const cases = [
    ['cold, loaded, paused-seek and zero-rate states have no played span', async (owner, callee, media, source) => {
      assert(read(callee, media).length === 0, 'cold history');
      await ready(media, source);
      assert(read(callee, media).length === 0, 'loading does not play');
      await seek(media, 1);
      assert(read(callee, media).length === 0, 'paused seeking does not play');
      media.playbackRate = 0; await method(callee, media, 'play'); await wait(30);
      assert(read(callee, media).length === 0, 'zero-rate playback has no forward span');
      method(callee, media, 'pause');
      assert(read(callee, media).length === 0, 'pausing zero-rate playback has no span');
    }],
    ['snapshots use the published position and stay immutable', async (owner, callee, media, source) => {
      await ready(media, source); await method(callee, media, 'play');
      await until(() => media.currentTime > 0.05, 'playback must advance');
      const time = media.currentTime, snapshot = read(callee, media), again = read(callee, media);
      assert(media.currentTime === time, 'reading history does not advance the official position');
      assert(snapshot !== again && snapshot instanceof owner.TimeRanges, 'fresh snapshots in the receiver realm');
      assert(snapshot.length === 1 && snapshot.start(0) === 0 && snapshot.end(0) === time, 'history ends at the published position');
      const original = pairs(snapshot);
      method(callee, media, 'pause'); const paused = pairs(read(callee, media));
      assert(paused[0][1] === media.currentTime && paused[0][1] >= time, 'pause flushes the elapsed span');
      await wait(30); equal(pairs(snapshot), original, 'old snapshot stays immutable');
      equal(pairs(read(callee, media)), paused, 'paused history stays stable');
    }],
    ['paused seeks and zero rate preserve history without filling gaps', async (owner, callee, media, source) => {
      await ready(media, source); const before = pairs(await segment(callee, media, 0.05));
      await seek(media, 3); equal(pairs(read(callee, media)), before, 'a seek preserves only played spans');
      media.playbackRate = 0; await method(callee, media, 'play'); await wait(30);
      method(callee, media, 'pause');
      equal(pairs(read(callee, media)), before, 'stationary playback does not fill the skipped gap');
    }],
    ['playing seeks produce separate sorted ranges', async (owner, callee, media, source) => {
      await ready(media, source); const before = pairs(await segment(callee, media, 0.05));
      media.playbackRate = 0; await method(callee, media, 'play');
      const stationary = pairs(read(callee, media));
      assert(stationary.length === 1 && stationary[0][0] === before[0][0] && stationary[0][1] === media.currentTime, 'capture the published stationary position before seeking');
      await seek(media, 3);
      equal(pairs(read(callee, media)), stationary, 'the playing seek itself does not add a span');
      media.playbackRate = 1; const after = await segment(callee, media, 3.05);
      assert(after.length === 2 && after.start(0) === 0 && after.end(0) === stationary[0][1], 'the first span is retained');
      assert(after.start(1) === 3 && after.end(1) === media.currentTime && after.end(0) < after.start(1), 'the seek gap is not played');
    }],
    ['overlapping replay merges intervals without changing old snapshots', async (owner, callee, media, source) => {
      await ready(media, source); await seek(media, 2);
      const old = await segment(callee, media, 2.05), saved = pairs(old);
      await seek(media, 1); media.playbackRate = 4;
      const merged = await segment(callee, media, saved[0][1] + 0.05);
      assert(merged.length === 1 && merged.start(0) === 1 && merged.end(0) === media.currentTime, 'overlapping spans normalize to their union');
      equal(pairs(old), saved, 'merging does not mutate a prior snapshot');
    }],
    ['full loops retain one range covering the complete resource', async (owner, callee, media, source) => {
      await ready(media, source); media.playbackRate = 16; media.loop = true;
      let loops = 0; media.addEventListener('seeked', () => loops++);
      await method(callee, media, 'play'); await until(() => loops >= 2, 'two real loops must finish');
      method(callee, media, 'pause'); const history = read(callee, media);
      assert(history.length === 1 && history.start(0) === 0 && history.end(0) === media.duration, 'looping preserves exact full-resource coverage');
    }],
    ['natural completion retains exact finite coverage', async (owner, callee, media, source) => {
      await ready(media, source); media.playbackRate = 16;
      let ended = false; media.addEventListener('ended', () => ended = true, {once: true});
      await method(callee, media, 'play'); await until(() => ended && media.paused, 'finite playback must end');
      const history = read(callee, media);
      assert(history.length === 1 && history.start(0) === 0 && history.end(0) === media.duration, 'the end boundary is included');
    }],
    ['reload, cloning and import start fresh history', async (owner, callee, media, source) => {
      await ready(media, source); const old = await segment(callee, media, 0.05), saved = pairs(old);
      const target = owner.document.implementation.createHTMLDocument('');
      for (const copy of [media.cloneNode(), target.importNode(media, false)])
        assert(read(callee, copy).length === 0 && copy.readyState === 0, 'copies have fresh media resource state');
      media.remove(); owner.document.body.appendChild(media);
      equal(pairs(read(callee, media)), saved, 'moving within the Document keeps the resource history');
      let loaded = false; media.addEventListener('canplaythrough', () => loaded = true, {once: true});
      method(callee, media, 'load');
      assert(read(callee, media).length === 0, 'load clears history immediately');
      await until(() => loaded, 'the reloaded resource must become ready');
      assert(read(callee, media).length === 0, 'reload alone does not play');
      equal(pairs(old), saved, 'resetting the backend leaves old snapshots unchanged');
    }],
    ['timeupdate reentrant seek retains only the completed played span', async (owner, callee, media, source) => {
      await ready(media, source); let moved = false, sought = false, snapshot;
      media.addEventListener('seeked', () => sought = true);
      media.addEventListener('timeupdate', () => {
        if (!moved && !media.seeking && media.currentTime > 0) {
          moved = true; snapshot = read(callee, media);
          media.currentTime = 3; method(callee, media, 'pause');
        }
      });
      await method(callee, media, 'play'); await until(() => moved && sought && !media.seeking, 'reentrant seek must complete');
      const history = read(callee, media);
      assert(snapshot.length === 1 && history.length === 1 && history.start(0) === 0, 'the completed span remains');
      assert(snapshot.end(0) > 0 && history.end(0) >= snapshot.end(0) && history.end(0) < 3 && media.currentTime === 3, 'the reentrant seek does not create a synthetic span');
    }],
    ['receiver brands use the getter realm and snapshots use the receiver realm', async (owner, callee, media) => {
      const get = Object.getOwnPropertyDescriptor(callee.HTMLMediaElement.prototype, 'played').get;
      const history = get.call(media);
      assert(history instanceof owner.TimeRanges && Object.prototype.toString.call(history) === '[object TimeRanges]', 'native snapshot identity');
      for (const name of ['buffered', 'seekable']) {
        const getter = Object.getOwnPropertyDescriptor(callee.HTMLMediaElement.prototype, name).get;
        assert(getter.call(media) instanceof owner.TimeRanges, 'all media range snapshots use the receiver realm');
      }
      let traps = 0;
      const proxy = new Proxy(media, {get() {traps++; throw Error('get');}, getPrototypeOf() {traps++; throw Error('prototype');}});
      const revoked = Proxy.revocable(media, {}); revoked.revoke();
      for (const receiver of [{}, Object.create(media), proxy, revoked.proxy]) {
        let caught; try {get.call(receiver);} catch (error) {caught = error;}
        assert(caught instanceof callee.TypeError, 'invalid receiver error belongs to the getter realm');
      }
      assert(traps === 0, 'receiver validation does not execute author Proxy traps');
    }],
  ];
  for (const [ownerName, owner] of realms) for (const [calleeName, callee] of realms)
    for (const kind of ['audio', 'video']) for (const transport of ['data', 'http'])
      for (const [name, body] of cases) {
        const label = `${ownerName}/${calleeName}/${kind}/${transport}/${name}`; progress.stage = label;
        const media = owner.document.createElement(kind); owner.document.body.appendChild(media);
        try {await body(owner, callee, media, __mediaPlayedSources[transport][kind]); checks.push({name: label, passed: true});}
        catch (error) {checks.push({name: label, passed: false, detail: String(error)});}
        finally {media.pause(); media.removeAttribute('src'); method(callee, media, 'load'); media.remove();}
      }
  globalThis.__uiEventResults = {checks, total: checks.length};
})()
