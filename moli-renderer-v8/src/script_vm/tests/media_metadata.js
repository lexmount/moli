(async () => {
  const checks = [], observations = [];
  const check = (name, ok, detail = null) => checks.push({name, passed: !!ok, detail});
  const run = (name, fn) => {
    try { check(name, fn()); } catch (error) { check(name, false, String(error)); }
  };
  const capture = fn => { try { fn(); } catch (error) { return error; } };
  const frame = document.querySelector('iframe');
  if (frame.contentDocument.readyState !== 'complete') await new Promise(resolve => frame.addEventListener('load', resolve, {once: true}));
  const other = frame.contentWindow;
  for (const w of [window, other]) {
    const label = w === window ? 'main' : 'child';
    const C = w.MediaMetadata, p = C.prototype;
    const getter = name => Object.getOwnPropertyDescriptor(p, name).get;
    const setter = name => Object.getOwnPropertyDescriptor(p, name).set;
    run(label + '/constructor-metadata', () => C.name === 'MediaMetadata' && C.length === 0 && p.constructor === C && Object.getPrototypeOf(p) === w.Object.prototype);
    for (const name of ['title', 'artist', 'album', 'artwork', 'chapterInfo']) {
      run(label + '/descriptor/' + name, () => {
        const d = Object.getOwnPropertyDescriptor(p, name);
        return d.enumerable && d.configurable && d.get.length === 0 && d.get.name === 'get ' + name &&
          (name === 'chapterInfo' ? d.set === undefined : d.set.length === 1 && d.set.name === 'set ' + name);
      });
    }
    for (const [index, init] of [undefined, null, {}].entries()) run(label + '/default/' + index, () => {
      const value = new C(init);
      return value.title === '' && value.artist === '' && value.album === '' && value.artwork.length === 0 && value.chapterInfo.length === 0 && value.chapterInfo === value.chapterInfo;
    });
    run(label + '/missing-init', () => new C().title === '');
    run(label + '/requires-new', () => capture(() => C()) instanceof w.TypeError);
    for (const [i, init] of [1, 'text', true, Symbol(), 0n].entries()) run(label + '/dictionary-type/' + i, () => capture(() => new C(init)) instanceof w.TypeError);
    run(label + '/utf16', () => {
      const value = new C({title: '\ud800T', artist: 'A\udfff', album: '\ud800'});
      value.title = '\udfff'; value.artist = '\ud800'; value.album = 'x\udfff';
      return value.title === '\udfff' && value.artist === '\ud800' && value.album === 'x\udfff';
    });
    run(label + '/inherited-members', () => {
      const image = Object.create({src: '../inherited', sizes: '12x12', type: 'image/png'});
      const value = new C(Object.create({album: 'album', artist: 'artist', title: 'title', artwork: [image]}));
      return value.album === 'album' && value.artist === 'artist' && value.title === 'title' && value.artwork[0].src === new URL('../inherited', document.baseURI).href;
    });
    run(label + '/dictionary-order', () => {
      const log = [], init = {};
      for (const name of ['title', 'chapterInfo', 'artwork', 'artist', 'album']) Object.defineProperty(init, name, {get() {log.push(name); return undefined;}});
      new C(init); return log.join(',') === 'album,artist,artwork,chapterInfo,title';
    });
    run(label + '/nullish-does-not-read-object-prototype', () => {
      const d = Object.getOwnPropertyDescriptor(w.Object.prototype, 'title'); let reads = 0;
      Object.defineProperty(w.Object.prototype, 'title', {configurable: true, get() {reads++; throw 42;}});
      try { new C(null); new C(); return reads === 0; }
      finally { if (d) Object.defineProperty(w.Object.prototype, 'title', d); else delete w.Object.prototype.title; }
    });
    run(label + '/constructor-prototype-order', () => {
      const log = [], N = new Proxy(w.Function(), {get(target, key, receiver) {
        if (key === 'prototype') {log.push('prototype'); return w.Object.prototype;}
        return Reflect.get(target, key, receiver);
      }});
      const value = Reflect.construct(C, [{get title() {log.push('title'); return 't';}}], N);
      return log.join(',') === 'title,prototype' && Object.getPrototypeOf(value) === w.Object.prototype && getter('title').call(value) === 't';
    });
    run(label + '/conversion-exception-before-prototype', () => {
      const sentinel = {}, N = new Proxy(w.Function(), {get(target, key, receiver) {if (key === 'prototype') throw 42; return Reflect.get(target, key, receiver);}});
      return capture(() => Reflect.construct(C, [{get title() {throw sentinel;}}], N)) === sentinel;
    });
    for (const nw of [window, other]) run(label + '/fallback/' + (nw === window ? 'main' : 'child'), () => {
      let reads = 0;
      const N = new Proxy(nw.Function(), {get(target, key, receiver) {if (key === 'prototype') {reads++; return 42;} return Reflect.get(target, key, receiver);}});
      const value = Reflect.construct(C, [], N);
      return reads === 1 && Object.getPrototypeOf(value) === nw.MediaMetadata.prototype && getter('title').call(value) === '';
    });
    run(label + '/artwork-copy-freeze-cache', () => {
      const image = {src: '../cover', sizes: '40x40\ud800', type: 'image/png\udfff', ignored: true}, input = [image];
      const value = new C({artwork: input}), before = value.artwork;
      image.src = '/changed'; input.push({src: '/another'});
      value.artwork = [{src: '/replacement'}]; const after = value.artwork;
      return before === before && Object.isFrozen(before) && Object.isFrozen(before[0]) && !('ignored' in before[0]) &&
        Object.keys(before[0]).join(',') === 'sizes,src,type' && before.length === 1 && before[0].src === new URL('../cover', document.baseURI).href &&
        before[0].sizes === '40x40\ud800' && before[0].type === 'image/png\udfff' && before !== after && after === value.artwork &&
        after[0].src === new URL('/replacement', document.baseURI).href;
    });
    run(label + '/artwork-url-usv-utf8', () => {
      const value = new C({artwork: [{src: '?icon=아이콘&bad=\ud800'}]});
      return value.artwork[0].src === new URL('?icon=아이콘&bad=\ufffd', document.baseURI).href;
    });
    for (const [i, input] of [null, undefined, '', {}, [null], [{}], [{src: Symbol()}], [{src: 'http://[broken]'}]].entries()) run(label + '/artwork-atomic/' + i, () => {
      const value = new C({artwork: [{src: '/original'}]}), old = value.artwork;
      return capture(() => {value.artwork = input;}) instanceof w.TypeError && value.artwork === old;
    });
    run(label + '/iterator-and-dictionary-order', () => {
      const log = [], image = {};
      for (const name of ['type', 'src', 'sizes']) Object.defineProperty(image, name, {get() {log.push(name); return name === 'src' ? '/cover' : '';}});
      const input = {[Symbol.iterator]() {log.push('iterator'); let i = 0; return {next() {log.push('next'); return i++ ? {done: true} : {done: false, value: image};}};}};
      const value = new C(); value.artwork = input;
      observations.push({name: label + '/setter-conversion-order', value: log});
      return log.join(',') === 'iterator,next,next,sizes,src,type';
    });
    run(label + '/constructor-dictionary-before-next-item', () => {
      const log = [], image = {};
      for (const name of ['type', 'src', 'sizes']) Object.defineProperty(image, name, {get() {log.push(name); return name === 'src' ? '/cover' : '';}});
      const input = {[Symbol.iterator]() {log.push('iterator'); let i = 0; return {next() {log.push('next'); return i++ ? {done: true} : {done: false, value: image};}};}};
      new C({artwork: input}); return log.join(',') === 'iterator,next,sizes,src,type,next';
    });
    run(label + '/exception-identity', () => {
      const value = new C({title: 'old'}), sentinel = {};
      return capture(() => {value.title = {toString() {throw sentinel;}};}) === sentinel && value.title === 'old' &&
        capture(() => {value.artwork = [{get src() {throw sentinel;}}];}) === sentinel;
    });
    run(label + '/chapters', () => {
      const input = {title: 'one\ud800', startTime: '2.5', artwork: [{src: '/chapter', sizes: '12x12'}]};
      const value = new C({chapterInfo: [input]}), chapters = value.chapterInfo, chapter = chapters[0];
      input.title = 'changed'; input.artwork.length = 0;
      return chapters === value.chapterInfo && Object.isFrozen(chapters) && Object.isFrozen(chapter) &&
        Object.getPrototypeOf(chapter) === w.ChapterInformation.prototype && chapter.title === 'one\ud800' && chapter.startTime === 2.5 &&
        Object.isFrozen(chapter.artwork) && chapter.artwork === chapter.artwork && chapter.artwork[0].src === new URL('/chapter', document.baseURI).href;
    });
    for (const [i, time] of [-1, NaN, Infinity, -Infinity, Symbol(), 1n].entries()) run(label + '/invalid-start-time/' + i, () => capture(() => new C({chapterInfo: [{startTime: time}]})) instanceof w.TypeError);
    run(label + '/chapter-defaults', () => {
      const chapter = new C({chapterInfo: [null, undefined, {}]}).chapterInfo;
      return chapter.length === 3 && chapter.every(item => item.title === '' && item.startTime === 0 && item.artwork.length === 0);
    });
    run(label + '/chapter-illegal-constructor', () => capture(() => new w.ChapterInformation()) instanceof w.TypeError && capture(() => w.ChapterInformation()) instanceof w.TypeError);
    run(label + '/native-brand-independent-of-prototype', () => {
      const value = new C(); Object.setPrototypeOf(value, null); Object.freeze(value);
      setter('title').call(value, 'changed'); return getter('title').call(value) === 'changed';
    });
    let real;
    try { real = new C(); check(label + '/native-receiver-fixture', true); }
    catch (error) { real = Object.create(p); check(label + '/native-receiver-fixture', false, String(error)); }
    const revoked = Proxy.revocable(real, {}); revoked.revoke();
    for (const [i, receiver] of [{}, p, Object.create(real), Object.create(p), new Proxy(real, {}), revoked.proxy].entries()) {
      for (const name of ['title', 'artist', 'album', 'artwork', 'chapterInfo']) run(label + '/invalid-receiver/' + i + '/' + name, () => {
        const descriptor = Object.getOwnPropertyDescriptor(p, name);
        if (!descriptor || typeof descriptor.get !== 'function') return false;
        let reads = 0, conversions = 0;
        const author = new Proxy(receiver, {get() {reads++; throw 42;}});
        const error = capture(() => getter(name).call(author));
        const setterError = name === 'chapterInfo' ? null : capture(() => setter(name).call(receiver, {toString() {conversions++; return '';}, [Symbol.iterator]() {conversions++; return [][Symbol.iterator]();}}));
        return error instanceof w.TypeError && (name === 'chapterInfo' || setterError instanceof w.TypeError) && reads === 0 && conversions === 0;
      });
    }
    run(label + '/session-producer-sameobject', () => {
      const session = w.navigator.mediaSession;
      return session === w.navigator.mediaSession && Object.getPrototypeOf(session) === w.MediaSession.prototype && session.metadata === null;
    });
    run(label + '/session-metadata-native-brand', () => {
      const session = w.navigator.mediaSession, value = new C({title: 'before'});
      session.metadata = value; value.title = 'after';
      const same = session.metadata === value && session.metadata.title === 'after';
      const error = capture(() => {session.metadata = new Proxy(value, {});});
      const kept = session.metadata === value; session.metadata = undefined;
      return same && error instanceof w.TypeError && kept && session.metadata === null;
    });
  }
  run('cross-realm/borrowed-title-setter', () => {
    const value = new MediaMetadata(), desc = Object.getOwnPropertyDescriptor(other.MediaMetadata.prototype, 'title');
    desc.set.call(value, 'cross\ud800'); return value.title === 'cross\ud800' && desc.get.call(value) === 'cross\ud800';
  });
  run('cross-realm/artwork-first-get', () => {
    const value = new MediaMetadata({artwork: [{src: '/cover'}]}), getter = Object.getOwnPropertyDescriptor(other.MediaMetadata.prototype, 'artwork').get;
    const array = getter.call(value);
    observations.push({name: 'artwork-first-get-realm', value: {arrayInGetterRealm: Object.getPrototypeOf(array) === other.Array.prototype, dictionaryInGetterRealm: Object.getPrototypeOf(array[0]) === other.Object.prototype}});
    return array === value.artwork && Object.getPrototypeOf(array) === other.Array.prototype && Object.getPrototypeOf(array[0]) === other.Object.prototype;
  });
  run('entry-settings/base-url', () => {
    const parentBase = document.createElement('base'), childBase = other.document.createElement('base');
    parentBase.href = 'https://parent-base.test/a/'; childBase.href = 'https://child-base.test/b/';
    document.head.appendChild(parentBase); other.document.head.appendChild(childBase);
    try {
      const parentEntry = new other.MediaMetadata({artwork: [{src: 'relative'}]});
      const childEntry = other.eval('new parent.MediaMetadata({artwork:[{src:"relative"}]})');
      return parentEntry.artwork[0].src === 'https://parent-base.test/a/relative' && childEntry.artwork[0].src === 'https://child-base.test/b/relative';
    } finally {parentBase.remove(); childBase.remove();}
  });
  globalThis.__uiEventResults = {checks, observations, complete: true, passed: checks.filter(row => row.passed).length, total: checks.length};
  return true;
})()
