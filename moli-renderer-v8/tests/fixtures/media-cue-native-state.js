(async () => {
  const checks = [];
  const progress = globalThis.__mediaCueNativeProgress = {stage: 'setup', checks};
  const assert = (value, message) => {if (!value) throw Error(message);};
  const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
  const realms = [['main', globalThis], ['iframe', document.querySelector('iframe').contentWindow]];
  async function until(condition, message) {
    const deadline = Date.now() + 5000;
    while (!condition()) {assert(Date.now() < deadline, message); await wait(5);}
  }
  for (const [ownerName, owner] of realms) for (const [calleeName, callee] of realms)
    for (const kind of ['audio', 'video']) for (const transport of ['data', 'http']) {
      const name = `${ownerName}/${calleeName}/${kind}/${transport}/native cue fields and list maintenance ignore author getters`;
      progress.stage = name;
      const media = owner.document.createElement(kind);
      owner.document.body.appendChild(media);
      try {
        let ready = false;
        media.addEventListener('canplaythrough', () => ready = true, {once: true});
        media.src = __mediaCueSources[transport][kind];
        await until(() => ready, 'real resource must become ready');
        assert(media.readyState >= 3 && media.error === null, 'valid media readiness');
        const track = callee.HTMLMediaElement.prototype.addTextTrack.call(media, 'metadata');
        track.mode = 'hidden';
        const first = new owner.VTTCue(0, 1, 'first'); first.id = 'first';
        const second = new owner.VTTCue(2, 3, 'second'); second.id = 'second';
        track.addCue(first); track.addCue(second);
        const cues = track.cues, active = track.activeCues;
        const start = Object.getOwnPropertyDescriptor(callee.TextTrackCue.prototype, 'startTime');
        const cueTrack = Object.getOwnPropertyDescriptor(callee.TextTrackCue.prototype, 'track').get;
        const length = Object.getOwnPropertyDescriptor(callee.TextTrackCueList.prototype, 'length').get;
        let reads = 0;
        const shadow = (object, key) => Object.defineProperty(object, key, {
          configurable: true,
          get() {reads++; throw Error('author getter: ' + key);}
        });
        for (const cue of [first, second])
          for (const key of ['startTime', 'endTime', 'track', 'id']) shadow(cue, key);
        shadow(cues, 'length'); shadow(active, 'length');
        start.set.call(second, -0.5);
        assert(start.get.call(second) === -0.5 && cueTrack.call(second) === track, 'native cue state');
        assert(cues[0] === second && cues[1] === first, 'native ordering after a timing change');
        await callee.HTMLMediaElement.prototype.play.call(media);
        let sought = false;
        media.addEventListener('seeked', () => sought = true, {once: true});
        media.currentTime = 0.5;
        await until(() => sought && !media.seeking, 'seek must finish');
        assert(active[0] === second && active[1] === first, 'native active cue timing and ordering');
        assert(cues.getCueById('second') === second, 'native cue identifier');
        track.removeCue(first);
        assert(length.call(cues) === 1 && cues[0] === second && cues[1] === undefined, 'no stale cue index');
        assert(length.call(active) === 1 && active[0] === second && active[1] === undefined, 'no stale active index');
        track.removeCue(second);
        assert(length.call(cues) === 0 && cues[0] === undefined && cues[1] === undefined, 'empty cue list');
        assert(length.call(active) === 0 && active[0] === undefined && active[1] === undefined, 'empty active list');
        assert(cues.getCueById('second') === null && cueTrack.call(second) === null, 'removed cue state');
        media.pause();
        assert(reads === 0, 'internal updates must not invoke author getters');
        checks.push({name, passed: true, reads});
      } catch (error) {checks.push({name, passed: false, detail: String(error)});}
      finally {media.pause(); media.removeAttribute('src'); media.load(); media.remove();}
    }
  globalThis.__uiEventResults = {checks, total: checks.length};
})()
