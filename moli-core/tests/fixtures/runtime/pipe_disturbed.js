async function runPipeDisturbedProbe(scenario) {
  const errors = [];
  const check = (condition, label) => {
    if (!condition) errors.push(label);
  };
  for (const ownerKind of ['Response', 'Request']) {
    for (const byteStream of [false, true]) {
      for (const method of ['pipeTo', 'pipeThrough']) {
        const cases = scenario === 'start'
          ? ['backpressure', 'pre-aborted', 'source-closed', 'source-errored', 'sink-closed', 'sink-errored']
          : ['source-locked', 'sink-locked', 'invalid-sink', 'invalid-signal', 'throwing-options'];
        for (const state of cases) {
          const label = `${ownerKind}/${byteStream ? 'bytes' : 'default'}/${method}/${state}`;
          const reason = new Error(label);
          let sourceController, sinkController, pulls = 0, writes = 0;
          const source = new ReadableStream({
            ...(byteStream ? {type: 'bytes'} : {}),
            start(controller) { sourceController = controller; },
            pull() { ++pulls; }
          }, {highWaterMark: 0});
          const body = ownerKind === 'Response'
            ? new Response(source)
            : new Request('https://example.test/', {method: 'POST', body: source, duplex: 'half'});
          const sink = new WritableStream({
            start(controller) { sinkController = controller; },
            write() { ++writes; },
            abort() { check(body.bodyUsed, `${label}: abort observes disturbed source`); }
          }, {highWaterMark: 0});
          const output = new ReadableStream({}, {highWaterMark: 0});
          const outputBody = new Response(output);
          const abort = new AbortController();
          let options = {signal: abort.signal, preventCancel: true};
          let destination = sink, reader, writer;
          if (state === 'source-closed') sourceController.close();
          if (state === 'source-errored') sourceController.error(reason);
          if (state === 'sink-closed') await sink.close();
          if (state === 'sink-errored') sinkController.error(reason);
          if (state === 'pre-aborted') abort.abort(reason);
          if (state === 'source-locked') reader = source.getReader();
          if (state === 'sink-locked') writer = sink.getWriter();
          if (state === 'invalid-sink') destination = {};
          if (state === 'invalid-signal') options = {signal: {}};
          if (state === 'throwing-options') options = {get preventCancel() {throw reason;}};
          check(!body.bodyUsed, `${label}: initially undisturbed`);
          let completion, thrown;
          try {
            if (method === 'pipeTo') {
              completion = source.pipeTo(destination, options);
              completion.catch(() => {});
            } else {
              const result = source.pipeThrough({readable: output, writable: destination}, options);
              check(result === output, `${label}: returns output stream`);
            }
          } catch (error) {
            thrown = error;
          }
          if (scenario === 'start') {
            check(thrown === undefined, `${label}: accepted pipe does not throw`);
            check(body.bodyUsed, `${label}: synchronously disturbed without reading`);
            check(pulls === 0 && writes === 0, `${label}: no pull or write before return`);
            check(!outputBody.bodyUsed, `${label}: output remains undisturbed`);
            abort.abort(reason);
            if (completion) await completion.catch(() => {});
            for (let turn = 0; (source.locked || sink.locked) && turn < 30; ++turn) {
              await new Promise(resolve => setTimeout(resolve, 0));
            }
            check(!source.locked && !sink.locked, `${label}: pipe releases locks`);
            check(body.bodyUsed, `${label}: remains disturbed after shutdown`);
          } else {
            if (completion) {
              check(thrown === undefined, `${label}: pipeTo returns a rejection`);
              thrown = await completion.then(() => undefined, error => error);
            }
            check(state === 'throwing-options' ? thrown === reason : thrown instanceof TypeError,
              `${label}: validation rejects with original error`);
            reader?.releaseLock();
            writer?.releaseLock();
            check(!body.bodyUsed, `${label}: rejected pipe leaves source undisturbed`);
            check(!source.locked && !sink.locked, `${label}: rejected pipe acquires no locks`);
            check(pulls === 0 && writes === 0, `${label}: rejected pipe does not read or write`);
          }
          // Avoid invoking the observation above during test cleanup.
          await Promise.allSettled([source.cancel(), output.cancel()]);
        }
      }
    }
  }
  return {errors};
}
