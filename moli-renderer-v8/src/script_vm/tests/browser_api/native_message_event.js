function nativeMessageEventProbe() {
  return new Promise((resolve, reject) => {
    const OriginalMessageEvent = MessageEvent;
    const constructorDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'MessageEvent');
    const iteratorDescriptor = Object.getOwnPropertyDescriptor(Array.prototype, Symbol.iterator);
    const names = ['bubbles', 'cancelable', 'composed', 'data', 'lastEventId', 'origin', 'ports', 'source'];
    const descriptors = names.map(name => Object.getOwnPropertyDescriptor(Object.prototype, name));
    const channel = new MessageChannel(), transfer = new MessageChannel();
    const sender = new BroadcastChannel('native-message-event-probe');
    const receiver = new BroadcastChannel('native-message-event-probe');
    const page = globalThis.window === globalThis;
    const rows = [];
    let remaining = page ? 4 : 3;
    let constructorReads = 0, dictionaryReads = 0, iteratorReads = 0;
    let timer;
    const restore = () => {
      Object.defineProperty(globalThis, 'MessageEvent', constructorDescriptor);
      Object.defineProperty(Array.prototype, Symbol.iterator, iteratorDescriptor);
      for (let i = 0; i < names.length; i++) {
        if (descriptors[i]) Object.defineProperty(Object.prototype, names[i], descriptors[i]);
        else delete Object.prototype[names[i]];
      }
      channel.port1.close(); channel.port2.close(); transfer.port2.close();
      sender.close(); receiver.close();
      if (page) removeEventListener('message', windowMessage);
      clearTimeout(timer);
    };
    const record = (kind, event, expectedOrigin, expectedSource, expectedPorts) => {
      rows.push({
        kind,
        branded: event instanceof OriginalMessageEvent,
        prototype: Object.getPrototypeOf(event) === OriginalMessageEvent.prototype,
        eventType: event.type === 'message',
        flags: !event.bubbles && !event.cancelable && !event.composed,
        payload: kind === 'undefined-port' ? event.data === undefined : event.data.marker === 'native-message-event-probe',
        eventOrigin: event.origin === expectedOrigin,
        eventId: event.lastEventId === '',
        eventSource: event.source === expectedSource,
        frozen: Array.isArray(event.ports) && Object.isFrozen(event.ports),
        arrayRealm: Object.getPrototypeOf(event.ports) === Array.prototype,
        portCount: event.ports.length === expectedPorts,
        portBrand: expectedPorts === 0 || event.ports[0] instanceof MessagePort,
        trusted: event.isTrusted,
      });
      if (event.ports.length) event.ports[0].close();
      if (--remaining === 0) {
        const result = {rows, constructorReads, dictionaryReads, iteratorReads};
        restore();
        resolve(result);
      }
    };
    const windowMessage = event => {
      if (event.data && event.data.marker === 'native-message-event-probe') {
        record('window', event, location.origin, globalThis, 0);
      }
    };
    channel.port1.onmessage = event => record(event.data === undefined ? 'undefined-port' : 'port', event, '', null, event.data === undefined ? 0 : 1);
    receiver.onmessage = event => record('broadcast', event, location.origin, null, 0);
    if (page) addEventListener('message', windowMessage);
    const payload = {marker: 'native-message-event-probe'};
    channel.port2.postMessage(payload, [transfer.port1]);
    channel.port2.postMessage(undefined);
    sender.postMessage(payload);
    if (page) postMessage(payload, '*');
    timer = setTimeout(() => {
      restore();
      reject(new Error('Native MessageEvent delivery did not settle: ' + JSON.stringify({rows, constructorReads, dictionaryReads, iteratorReads})));
    }, 5000);
    const poison = name => { throw new Error('Native MessageEvent reached author code: ' + name); };
    Object.defineProperty(globalThis, 'MessageEvent', {configurable: true, get() { constructorReads++; return poison; }});
    for (let i = 0; i < names.length; i++) {
      Object.defineProperty(Object.prototype, names[i], {configurable: true, get() { dictionaryReads++; return poison(names[i]); }});
    }
    Object.defineProperty(Array.prototype, Symbol.iterator, {configurable: true, get() { iteratorReads++; return poison('array iterator'); }});
  });
}
