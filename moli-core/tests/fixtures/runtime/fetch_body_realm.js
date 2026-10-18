async function runFetchBodyRealmProbe(scenario, bodyURL, networkBytes) {
  if (document.readyState !== 'complete') {
    await new Promise(resolve => addEventListener('load', resolve, {once: true}));
  }
  const frame = document.createElement('iframe');
  const loaded = new Promise(resolve => frame.onload = resolve);
  frame.srcdoc = '<!doctype html><meta charset=utf-8><body>';
  document.body.append(frame);
  await loaded;
  const errors = [];
  let checked = 0;
  const check = (condition, label) => { if (!condition) errors.push(label); };
  try {
    for (const direction of ['parent-method', 'child-method']) {
      const methodRealm = direction === 'parent-method' ? self : frame.contentWindow;
      const receiverRealm = direction === 'parent-method' ? frame.contentWindow : self;
      const expectedBufferPrototype = receiverRealm.ArrayBuffer.prototype;
      const expectedBytesPrototype = receiverRealm.Uint8Array.prototype;
      for (const type of ['Request', 'Response']) {
        const sources = scenario === 'network'
          ? ['network', 'rewrapped-network', 'cloned-network']
          : ['null', 'buffered', 'receiver-stream', 'method-stream', 'async-stream', 'changed-prototype', 'cloned-stream'];
        for (const source of sources) {
          for (const method of ['arrayBuffer', 'bytes']) {
            const label = [direction, type, source, method].join('/');
            try {
              let expected = source === 'null' ? [] : [65, 0, 66, 255];
              let body = source === 'null' ? null : new receiverRealm.Uint8Array(expected);
              let object;
              if (scenario === 'network') {
                expected = networkBytes;
                const fetchRealm = source === 'rewrapped-network' ? methodRealm : receiverRealm;
                object = await fetchRealm.fetch(bodyURL);
                body = object.body;
                if (type === 'Request' || source === 'rewrapped-network') object = undefined;
              } else if (source.endsWith('stream')) {
                const streamRealm = source === 'receiver-stream' ? receiverRealm : methodRealm;
                body = new streamRealm.ReadableStream({start(controller) {
                  const send = () => {
                    controller.enqueue(new streamRealm.Uint8Array([9, ...expected, 9]).subarray(1, 5));
                    controller.close();
                  };
                  if (source === 'async-stream') setTimeout(send, 5); else send();
                }});
              }
              if (!object) {
                object = type === 'Request'
                  ? new receiverRealm.Request(bodyURL, {method: 'POST', body, duplex: 'half'})
                  : new receiverRealm.Response(body);
              }
              if (source.startsWith('cloned-')) object = object.clone();
              if (source === 'changed-prototype') {
                Object.setPrototypeOf(object, methodRealm[type].prototype);
              }
              const result = await methodRealm[type].prototype[method].call(object);
              const buffer = method === 'bytes' ? result.buffer : result;
              check(Object.getPrototypeOf(buffer) === expectedBufferPrototype, label + ': buffer realm');
              if (method === 'bytes') {
                check(Object.getPrototypeOf(result) === expectedBytesPrototype, label + ': bytes realm');
              }
              const bytes = new Uint8Array(buffer);
              check(bytes.length === expected.length && bytes.every((byte, index) => byte === expected[index]), label + ': contents');
              check(object.bodyUsed === (source !== 'null'), label + ': bodyUsed');
              checked++;
            } catch (error) {
              errors.push(label + ': ' + String(error));
            }
          }
        }
        if (scenario !== 'network') {
          for (const method of ['arrayBuffer', 'bytes']) {
            const reason = new receiverRealm.Error('stream failure');
            const body = new receiverRealm.ReadableStream({start(controller) {controller.error(reason)}});
            const object = type === 'Request'
              ? new receiverRealm.Request(bodyURL, {method: 'POST', body, duplex: 'half'})
              : new receiverRealm.Response(body);
            try {
              await methodRealm[type].prototype[method].call(object);
              errors.push(direction + '/' + type + '/' + method + ': error stream fulfilled');
            } catch (error) {
              check(error === reason, direction + '/' + type + '/' + method + ': error identity');
            }
            checked++;
          }
        }
      }
    }
  } finally {
    frame.remove();
  }
  return {errors, checked};
}
