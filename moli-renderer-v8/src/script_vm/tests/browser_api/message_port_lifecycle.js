function retiredChannelProbe() {
  const failures = [], rows = [];
  const check = (value, label) => { if (!value) failures.push(label); };
  for (const reinsert of [false, true]) {
    const frame = document.createElement('iframe');
    document.body.appendChild(frame);
    const realm = frame.contentWindow;
    const C = realm.MessageChannel, P = realm.MessagePort, T = realm.TypeError;
    const getters = ['port1', 'port2'].map(name =>
      Object.getOwnPropertyDescriptor(MessageChannel.prototype, name).get);
    frame.remove();
    if (reinsert) document.body.appendChild(frame);
    class Subchannel extends C {}
    for (const construct of [() => new C(), () => new Subchannel(),
                            () => Reflect.construct(C, [], function Custom() {})]) {
      try {
        const channel = construct(), ports = getters.map(get => get.call(channel));
        check(ports[0] !== ports[1], 'distinct retired ports');
        Object.setPrototypeOf(channel, null);
        Object.freeze(channel);
        for (const [index, port] of ports.entries()) {
          check(Object.getPrototypeOf(port) === P.prototype, 'retired port realm');
          check(getters[index].call(channel) === port, 'retired channel brand');
          let calls = 0;
          port.addEventListener('probe', () => calls++);
          const event = new Event('probe');
          check(port.dispatchEvent(event) === true && calls === 1, 'detached port dispatch');
          check(event.target === port && event.currentTarget === null && event.eventPhase === 0,
                'detached port event cleanup');
          try { structuredClone(port, {transfer:[port]}); failures.push('transferred born-detached port'); }
          catch (error) { check(error.name === 'DataCloneError', 'born-detached transfer error'); }
          port.start(); port.postMessage('ignored'); port.close(); port.close();
        }
        rows.push([reinsert, true]);
      } catch (error) { failures.push('retained constructor: ' + error.name); }
    }
    try { C(); failures.push('constructor called without new'); }
    catch (error) { check(error instanceof T, 'retired constructor error realm'); }
    if (reinsert) {
      const fresh = new frame.contentWindow.MessageChannel();
      let calls = 0;
      fresh.port1.addEventListener('probe', () => calls++);
      check(fresh.port1.dispatchEvent(new Event('probe')) === true && calls === 1,
            'reinserted frame has a fresh active realm');
      check(Object.getPrototypeOf(fresh.port1) !== P.prototype, 'fresh port realm');
      fresh.port1.close(); fresh.port2.close();
    }
    frame.remove();
  }
  return {failures, rows};
}

function retiredTargetProbe() {
  const failures = [], rows = [];
  const check = (value, label) => { if (!value) failures.push(label); };
  // Synthetic dispatch follows each listener's realm lifetime, even when the
  // retained target's own realm has been destroyed.
  for (const kind of ['EventTarget', 'MessagePort', 'FileReader', 'AbortSignal']) {
    for (const state of ['live', 'removed', 'reinserted']) {
      const frame = document.createElement('iframe');
      document.body.appendChild(frame);
      const w = frame.contentWindow;
      const T = w.TypeError, childDispatch = w.EventTarget.prototype.dispatchEvent;
      let channel;
      const target = kind === 'EventTarget' ? new w.EventTarget() :
        kind === 'MessagePort' ? (channel = new w.MessageChannel()).port1 :
        kind === 'FileReader' ? new w.FileReader() : new w.AbortController().signal;
      const methods = [target.dispatchEvent];
      if (kind !== 'AbortSignal') methods.push(EventTarget.prototype.dispatchEvent);
      let calls = 0;
      const childCalls = [];
      const childListener = w.Function('calls', 'return function() { calls.push(true); };')(childCalls);
      target.addEventListener('probe', childListener);
      target.addEventListener('probe', () => calls++);
      if (state !== 'live') frame.remove();
      if (state === 'reinserted') document.body.appendChild(frame);
      for (const [index, dispatch] of methods.entries()) {
        const label = kind + ':' + state + ':' + index;
        const before = calls, childBefore = childCalls.length, event = new Event('probe');
        try {
          const returned = dispatch.call(target, event);
          check(returned === true, label + ' return');
          check(calls - before === 1, label + ' live listener');
          check(childCalls.length - childBefore === (state === 'live' ? 1 : 0), label + ' child listener lifetime');
          check(event.target === target, label + ' event target');
          check(event.currentTarget === null && event.eventPhase === 0, label + ' event state');
          for (const value of [null, {}, new Proxy(event, {})]) {
            try { dispatch.call(target, value); failures.push(label + ' accepted invalid Event'); }
            catch (error) { check(error instanceof (index === 0 ? T : TypeError), label + ' error realm'); }
          }
          try { dispatch.call(target, document.createEvent('Event')); failures.push(label + ' uninitialized'); }
          catch (error) { check(error.name === 'InvalidStateError', label + ' event initialization'); }
          if (state !== 'live') {
            const active = new EventTarget(), reused = new Event('probe');
            active.dispatchEvent(reused);
            check(dispatch.call(target, reused) === true && reused.target === target,
                  label + ' update previous target');
            active.addEventListener('probe', event => {
              try { dispatch.call(target, event); failures.push(label + ' accepted dispatching Event'); }
              catch (error) { check(error.name === 'InvalidStateError', label + ' active dispatch flag'); }
            });
            active.dispatchEvent(new Event('probe'));
            if (kind !== 'AbortSignal') {
              let parentCalls = 0;
              active.addEventListener('parent', () => parentCalls++);
              check(childDispatch.call(active, new Event('parent')) === true && parentCalls === 1,
                    label + ' receiver realm governs borrowed dispatch');
            }
          }
          rows.push(label);
        } catch (error) { failures.push(label + ': ' + error.name); }
      }
      channel?.port1.close(); channel?.port2.close(); frame.remove();
    }
  }
  return {failures, rows};
}

async function retiredPortTransferProbe() {
  const rows = [];
  for (const mode of ['transfer-before-removal', 'transfer-after-removal',
                      'remove-during-transfer', 'post-after-removal', 'remove-during-options',
                      'remove-during-post', 'close-during-post', 'remove-and-transfer-during-post']) {
    const frame = document.createElement('iframe'); document.body.appendChild(frame);
    const channel = new frame.contentWindow.MessageChannel();
    const ports = [channel.port1, channel.port2];
    let clones = [], messages = [], extra, error = null;
    try {
      if (mode === 'transfer-before-removal') {
        clones = structuredClone(ports, {transfer:ports}); frame.remove();
      } else if (mode === 'transfer-after-removal') {
        frame.remove(); clones = structuredClone(ports, {transfer:ports});
      } else if (mode === 'remove-during-transfer') {
        clones = structuredClone({get ports() {frame.remove(); return ports;}}, {transfer:ports}).ports;
      } else {
        clones = [ports[0], structuredClone(ports[1], {transfer:[ports[1]]})];
        if (mode === 'post-after-removal') frame.remove();
      }
      await new Promise(resolve => {
        setTimeout(resolve, 200);
        clones[1].onmessage = event => {messages.push(event.data.value ?? event.data);};
        const reentrant = ['remove-during-post', 'close-during-post', 'remove-and-transfer-during-post'].includes(mode);
        const data = reentrant ? {get value() {
          if (mode === 'close-during-post') clones[0].close();
          else frame.remove();
          if (mode === 'remove-and-transfer-during-post') {
            extra = structuredClone(clones[0], {transfer:[clones[0]]});
          }
          return 'first';
        }} : 'first';
        if (mode === 'remove-during-options') {
          clones[0].postMessage(data, {get transfer() {frame.remove(); return [];}});
        } else clones[0].postMessage(data);
        if (reentrant) clones[0].postMessage('second');
      });
    } catch (caught) {error = caught.name;}
    finally {for (const port of [...ports, ...clones]) port.close(); extra?.close(); frame.remove();}
    rows.push({mode, messages, error});
  }
  return rows;
}
