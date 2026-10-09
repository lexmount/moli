(() => {
  function assert(value, message) { if (!value) throw new Error(message); }
  function throws(expected, fn, message) {
    try { fn(); } catch(error) {
      assert(error instanceof expected, message + ': wrong exception ' + error);
      return;
    }
    throw new Error(message + ': did not throw');
  }
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const child = frame.contentWindow;
  try {
    for (const realm of [window, child]) {
      const blob = new realm.Blob(['value']);
      for (const [name, member, init, expected, order] of [
        ['AnimationEvent', 'animationName', {animationName:'name\uD800', elapsedTime:-0, pseudoElement:'::\uDC00'}, 'name\uD800',
          ['bubbles','cancelable','composed','animation','animationName','elapsedTime','pseudoElement']],
        ['TransitionEvent', 'propertyName', {propertyName:'name\uD800', elapsedTime:2.5, pseudoElement:'::\uDC00'}, 'name\uD800',
          ['bubbles','cancelable','composed','animation','elapsedTime','propertyName','pseudoElement']],
        ['BlobEvent', 'data', {data:blob, timecode:123}, blob,
          ['bubbles','cancelable','composed','data','timecode']]
      ]) {
        const C = realm[name];
        const gets = [];
        const values = {...init,bubbles:true,cancelable:true,composed:true};
        const event = new C('type\uD800', new Proxy(values, {
          get(target,key,receiver) { gets.push(key); return Reflect.get(target,key,receiver); }
        }));
        assert(gets.join(',') === order.join(','), name + ': dictionary order ' + gets);
        assert(event.type === 'type\uD800' && event[member] === expected, name + ': UTF-16/data');
        assert(event.bubbles && event.cancelable && event.composed && !event.isTrusted, name + ': flags');
        assert(event instanceof realm.Event && event instanceof C, name + ': inheritance');
        assert(Object.getPrototypeOf(C) === realm.Event && Object.getPrototypeOf(C.prototype) === realm.Event.prototype, name + ': prototype chain');
        assert(Object.prototype.toString.call(event) === '[object ' + name + ']', name + ': tag');
        const descriptor = Object.getOwnPropertyDescriptor(C.prototype,member);
        assert(descriptor.enumerable && descriptor.configurable && !descriptor.set &&
          descriptor.get.length === 0 && descriptor.get.name === 'get ' + member &&
          !Object.hasOwn(event,member), name + ': accessor');
        throws(realm.TypeError, () => C('x', init), name + ': new required');
        throws(realm.TypeError, () => new C(), name + ': arguments required');
        for(const bad of [true, 42, 'bad', Symbol('bad')]) {
          throws(realm.TypeError, () => new C('x', bad), name + ': dictionary required');
        }
        let traps = 0;
        const proxy = new Proxy(event, {get(){++traps;throw new Error('trap');},getPrototypeOf(){++traps;throw new Error('trap');}});
        const revoked = Proxy.revocable(event,{}); revoked.revoke();
        for(const fake of [{},Object.create(event),proxy,revoked.proxy,new realm.Event('x')]) {
          throws(realm.TypeError, () => descriptor.get.call(fake), name + ': native receiver');
        }
        assert(traps === 0, name + ': proxy traps');
        const other = new window[name]('x', init);
        assert(descriptor.get.call(other) === expected, name + ': cross-realm receiver');
        for (let index = 3; index < order.length; ++index) {
          const marker = {};
          const failedReads = [];
          try {
            new C('x', new Proxy(values,{get(target,key,receiver) {
              failedReads.push(key);
              if (key === order[index]) throw marker;
              return Reflect.get(target,key,receiver);
            }}));
            throw new Error('not thrown');
          } catch(error) { assert(error === marker, name + ': exception identity at ' + order[index]); }
          assert(failedReads.join(',') === order.slice(0,index+1).join(','), name + ': short-circuit at ' + order[index]);
        }
        if (name !== 'BlobEvent') {
          assert(event.animation === null, name + ': default nullable animation');
          assert(event.pseudoElement === '::\uDC00', name + ': pseudo-element UTF-16');
          assert(Object.is(event.elapsedTime,init.elapsedTime), name + ': signed zero');
          for(const value of [NaN,Infinity,-Infinity,1n,Symbol('bad')]) {
            throws(realm.TypeError, () => new C('x',{elapsedTime:value}), name + ': finite double');
          }
        }
        let dispatched = null;
        const target = document.createElement('div');
        target.addEventListener(event.type, value => { dispatched = value; value.preventDefault(); });
        assert(!target.dispatchEvent(event) && dispatched === event && event[member] === expected, name + ': dispatch');
      }
      for(const init of [null,undefined,{}, {data:null},{data:undefined},{data:{}},{data:Object.create(blob)}, {data:new Proxy(blob,{})}]) {
        throws(realm.TypeError, () => new realm.BlobEvent('x',init), 'BlobEvent: Blob brand required');
      }
      const file = new realm.File(['x'],'x.txt');
      assert(new realm.BlobEvent('x',{data:file}).data === file, 'BlobEvent: File is Blob');
      assert(Number.isNaN(new realm.BlobEvent('x',{data:blob}).timecode), 'BlobEvent: omitted timecode');
      for(const value of [NaN,Infinity,-Infinity]) {
        throws(realm.TypeError, () => new realm.BlobEvent('x',{data:blob,timecode:value}), 'BlobEvent: finite timecode');
      }
    }
    return true;
  } finally { frame.remove(); }
})()
