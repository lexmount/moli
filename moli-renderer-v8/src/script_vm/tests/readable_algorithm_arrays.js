(async () => {
  const checks = [];
  async function check(name, test) {
    try { checks.push({name, passed: (await test()) === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  }
  let frame;
  const owners = [['main', globalThis]];
  if (typeof document !== 'undefined') {
    frame = document.createElement('iframe');document.body.append(frame);
    owners.push(['child', frame.contentWindow]);
  }
  try {
    for (const [label, owner] of owners) for (const kind of ['default', 'bytes', 'response']) {
      for (const prototype of ['Array', 'Object']) for (const index of [0, 1, 2, 3]) {
        for (const poison of ['setter', 'throwing-setter', 'getter-only', 'readonly-data']) {
          await check(`${label}/${kind}/${prototype}/${index}/${poison}`, async () => {
            const target = owner[prototype].prototype;
            const previous = Object.getOwnPropertyDescriptor(target, String(index));
            const reason = {numericPoison: poison};
            let calls = 0, starts = 0, pulls = 0, stream, canceled, cancelReason;
            const descriptor = poison === 'readonly-data' ? {configurable: true, value: 42, writable: false}
              : poison === 'getter-only' ? {configurable: true, get() {calls++; throw reason;}}
              : {configurable: true, set() {calls++; if (poison === 'throwing-setter') throw reason;}};
            Object.defineProperty(target, String(index), descriptor);
            try {
              if (kind === 'response') stream = new owner.Response(new owner.Uint8Array([65, 66]));
              else {
                const source = {
                  start(controller) {starts++; controller.enqueue(kind === 'bytes' ? new owner.Uint8Array([65]) : 'first');},
                  pull(controller) {pulls++; controller.enqueue(kind === 'bytes' ? new owner.Uint8Array([66]) : 'second');controller.close();}
                };
                if (kind === 'bytes') source.type = 'bytes';
                stream = new owner.ReadableStream(source);
                canceled = new owner.ReadableStream({cancel(value) {cancelReason = value;}});
              }
            } finally {
              if (previous) Object.defineProperty(target, String(index), previous);
              else delete target[String(index)];
            }
            if (calls !== 0) return false;
            if (kind === 'response') return Object.getPrototypeOf(stream) === owner.Response.prototype && await stream.text() === 'AB';
            if (Object.getPrototypeOf(stream) !== owner.ReadableStream.prototype) return false;
            const reader = stream.getReader();
            const first = await reader.read(), second = await reader.read(), done = await reader.read();
            if (starts !== 1 || pulls !== 1 || first.done || second.done || !done.done) return false;
            if (kind === 'bytes' ? first.value[0] !== 65 || second.value[0] !== 66 : first.value !== 'first' || second.value !== 'second') return false;
            const cancelValue = {cancelValue: label};await canceled.cancel(cancelValue);
            return cancelReason === cancelValue;
          });
        }
      }
    }
  } finally {if (frame) frame.remove();}
  globalThis.__readableAlgorithmResults = {checks, total: checks.length, passed: checks.filter(row => row.passed).length, complete: true};
  return true;
})()
