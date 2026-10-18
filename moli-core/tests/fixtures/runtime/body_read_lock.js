async function runBodyReadLockProbe(url) {
  const errors = [];
  for (const kind of ['Request', 'Response']) {
    for (const method of ['text', 'json', 'bytes', 'arrayBuffer', 'blob', 'formData']) {
      for (const fails of [false, true]) {
        const label = `${kind}/${method}/${fails ? 'failure' : 'success'}`;
        const reason = {label};
        const text = method === 'formData' ? 'name=value' : '"body"';
        const stream = new ReadableStream({
          start(controller) {
            if (fails) controller.error(reason);
            else {
              controller.enqueue(new TextEncoder().encode(text));
              controller.close();
            }
          }
        });
        const headers = {'Content-Type': method === 'formData'
          ? 'application/x-www-form-urlencoded' : 'application/json'};
        const body = kind === 'Request'
          ? new Request(url, {method: 'POST', body: stream, duplex: 'half', headers})
          : new Response(stream, {headers});
        if (body.bodyUsed || stream.locked) errors.push(label + ': initially unusable');
        try {
          await body[method]();
          if (fails) errors.push(label + ': fulfilled');
        } catch (error) {
          if (!fails || error !== reason) errors.push(label + ': wrong rejection');
        }
        if (!body.bodyUsed || !stream.locked) errors.push(label + ': reader ownership lost');
      }
    }
  }
  return {errors};
}
