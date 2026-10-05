(() => {
  const checks = [];
  const check = (name, ok, detail = null) => checks.push({name, passed: !!ok, detail});
  const run = (name, fn) => {try {check(name, fn());} catch (error) {check(name, false, String(error));}};
  const caught = fn => {try {fn();} catch (error) {return error;}};
  const other = document.querySelector('iframe').contentWindow;
  for (const w of [window, other]) {
    const label = w === window ? 'main' : 'child';
    const C = w.VideoPlaybackQuality, p = C.prototype, method = w.HTMLVideoElement.prototype.getVideoPlaybackQuality;
    run(label + '/constructor', () => C.name === 'VideoPlaybackQuality' && C.length === 0 && p.constructor === C && Object.getPrototypeOf(p) === w.Object.prototype && caught(() => new C()) instanceof w.TypeError && caught(() => C()) instanceof w.TypeError);
    run(label + '/method-descriptor', () => {
      const d = Object.getOwnPropertyDescriptor(w.HTMLVideoElement.prototype, 'getVideoPlaybackQuality');
      return d.value === method && d.enumerable && d.configurable && d.writable && method.name === 'getVideoPlaybackQuality' && method.length === 0;
    });
    for (const name of ['creationTime', 'totalVideoFrames', 'droppedVideoFrames', 'corruptedVideoFrames']) {
      run(label + '/descriptor/' + name, () => {
        const d = Object.getOwnPropertyDescriptor(p, name);
        return d.enumerable && d.configurable && d.get.name === 'get ' + name && d.get.length === 0 && d.set === undefined;
      });
    }
    run(label + '/fresh-unloaded-snapshot', () => {
      const video = w.document.createElement('video'), before = w.performance.now(), first = method.call(video), second = method.call(video), after = w.performance.now();
      return first !== second && Object.getPrototypeOf(first) === p && Object.getPrototypeOf(second) === p &&
        first.creationTime >= before && first.creationTime <= after && second.creationTime >= first.creationTime && second.creationTime <= after &&
        first.totalVideoFrames === 0 && first.droppedVideoFrames === 0 && first.corruptedVideoFrames === 0 && Object.keys(first).length === 0 &&
        Object.prototype.toString.call(first) === '[object VideoPlaybackQuality]';
    });
    run(label + '/detached-document-video', () => {
      const video = w.document.implementation.createHTMLDocument('').createElement('video'), result = method.call(video);
      return Object.getPrototypeOf(result) === p && result.totalVideoFrames === 0 && result.creationTime === 0;
    });
    run(label + '/receiver-realm-on-cross-realm-video', () => {
      const owner = w === window ? other : window, receiver = owner.document.createElement('video');
      const before = owner.performance.now(), result = method.call(receiver), after = owner.performance.now();
      return Object.getPrototypeOf(result) === owner.VideoPlaybackQuality.prototype && result.creationTime >= before && result.creationTime <= after;
    });
    run(label + '/adopted-video-document-clock', () => {
      const original = w === window ? other : window, video = original.document.createElement('video');
      w.document.adoptNode(video);
      const before = w.performance.now(), result = method.call(video), after = w.performance.now();
      return Object.getPrototypeOf(result) === original.VideoPlaybackQuality.prototype && result.creationTime >= before && result.creationTime <= after;
    });
    run(label + '/synthetic-window-does-not-create-document-clock', () => {
      const outer = w.document.implementation.createHTMLDocument(''), iframe = outer.createElement('iframe');
      outer.body.appendChild(iframe); const document = iframe.contentDocument;
      iframe.contentWindow;
      return method.call(document.createElement('video')).creationTime === 0;
    });
    run(label + '/snapshot-slots', () => {
      const result = method.call(w.document.createElement('video'));
      const getter = Object.getOwnPropertyDescriptor(p, 'creationTime').get, original = getter.call(result);
      Object.defineProperty(result, 'creationTime', {value: 900}); result.__moliVideoPlaybackQualityCreationTime = 900;
      Object.setPrototypeOf(result, null); Object.freeze(result);
      return getter.call(result) === original;
    });
    let result;
    try {result = method.call(w.document.createElement('video'));} catch (_) {result = Object.create(p);}
    const revoked = Proxy.revocable(result, {}); revoked.revoke();
    for (const [index, receiver] of [{}, p, Object.create(p), Object.create(result), new Proxy(result, {}), revoked.proxy].entries()) {
      for (const name of ['creationTime', 'totalVideoFrames', 'droppedVideoFrames', 'corruptedVideoFrames']) run(label + '/getter-receiver/' + index + '/' + name, () => {
        const d = Object.getOwnPropertyDescriptor(p, name); if (!d || typeof d.get !== 'function') return false;
        return caught(() => d.get.call(receiver)) instanceof w.TypeError;
      });
    }
    const video = w.document.createElement('video'), author = new Proxy(video, {}), revokedVideo = Proxy.revocable(video, {}); revokedVideo.revoke();
    for (const [index, receiver] of [{}, w.document.createElement('audio'), Object.create(video), author, revokedVideo.proxy].entries()) run(label + '/method-receiver/' + index, () => typeof method === 'function' && caught(() => method.call(receiver)) instanceof w.TypeError);
  }
  globalThis.__uiEventResults = {checks, complete: true, passed: checks.filter(row => row.passed).length, total: checks.length};
  return true;
})()
