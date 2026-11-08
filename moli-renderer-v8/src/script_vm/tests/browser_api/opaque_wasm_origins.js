(async function opaqueWasmProbe() {
  const checks = [];
  const channels = [];
  const frames = [];
  const urls = [];
  const module = () =>
    new WebAssembly.Module(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]));
  function resultOf(target, send) {
    return new Promise((resolve) => {
      const onmessage = (event) =>
        finish({
          type: "message",
          wasm: event.data instanceof WebAssembly.Module,
          ports: event.ports.length,
        });
      const onerror = (event) =>
        finish({
          type: "messageerror",
          wasm: false,
          ports: event.ports.length,
        });
      function finish(result) {
        target.removeEventListener("message", onmessage);
        target.removeEventListener("messageerror", onerror);
        resolve(result);
      }
      target.addEventListener("message", onmessage);
      target.addEventListener("messageerror", onerror);
      if (target instanceof MessagePort) target.start();
      send();
    });
  }
  async function workerModuleProbe(nested = false) {
    const inner =
      "onmessage=e=>postMessage(e.data);onmessageerror=()=>postMessage('messageerror');";
    const code = nested
      ? `onmessage=e=>{const url=URL.createObjectURL(new Blob([${JSON.stringify(inner)}],{type:'text/javascript'}));const child=new Worker(url);child.onmessage=event=>{postMessage(event.data);child.terminate();URL.revokeObjectURL(url);};child.onmessageerror=()=>postMessage('nested-messageerror');child.postMessage(e.data);};onmessageerror=()=>postMessage('messageerror');`
      : inner;
    const url = URL.createObjectURL(
      new Blob([code], { type: "text/javascript" }),
    );
    const worker = new Worker(url);
    try {
      return await resultOf(worker, () => worker.postMessage(module()));
    } finally {
      worker.terminate();
      URL.revokeObjectURL(url);
    }
  }
  const record = (name, actual, expected) =>
    checks.push({
      name,
      actual,
      passed: JSON.stringify(actual) === JSON.stringify(expected),
    });
  function frameProbe(mode, operations) {
    return new Promise((resolve) => {
      const frame = document.createElement("iframe");
      frames.push(frame);
      const token = "probe-" + frames.length;
      function receive(event) {
        if (event.data && event.data.token === token) {
          removeEventListener("message", receive);
          resolve(event.data.result);
        }
      }
      addEventListener("message", receive);
      const code = `const resultOf=${resultOf.toString()}; const module=${module.toString()}; const workerModuleProbe=${workerModuleProbe.toString()};
        (async()=>{${operations}})().then(result=>parent.postMessage({token:${JSON.stringify(token)},result},'*'))
        .catch(error=>parent.postMessage({token:${JSON.stringify(token)},result:{error:String(error)}},'*'));`;
      const html = "<!doctype html><body><script>" + code + "<\/script>";
      if (mode === "credentialless") frame.setAttribute("credentialless", "");
      if (mode === "sandbox") frame.setAttribute("sandbox", "allow-scripts");
      if (mode === "blob") {
        const url = URL.createObjectURL(
          new Blob([html], { type: "text/html" }),
        );
        urls.push(url);
        frame.src = url;
      } else frame.srcdoc = html;
      (document.body || document.documentElement).appendChild(frame);
    });
  }
  const accepted = { type: "message", wasm: true, ports: 0 };
  try {
    const local = new MessageChannel();
    channels.push(local);
    record(
      "opaque-root-channel",
      await resultOf(local.port1, () => local.port2.postMessage(module())),
      accepted,
    );
    record(
      "opaque-root-window",
      await resultOf(window, () => window.postMessage(module(), "*")),
      accepted,
    );
    record("opaque-root-blob-worker", await workerModuleProbe(), accepted);
    record(
      "opaque-root-nested-blob-worker",
      await workerModuleProbe(true),
      accepted,
    );
    const popup = window.open("about:blank");
    if (!popup) throw new Error("about:blank popup was not created");
    try {
      popup.onmessage = (event) => window.postMessage(event.data, "*");
      popup.onmessageerror = () =>
        window.postMessage("popup-messageerror", "*");
      record(
        "inherited-blank-popup-roundtrip",
        await resultOf(window, () => popup.postMessage(module(), "*")),
        accepted,
      );
    } finally {
      popup.close();
    }
    for (const mode of ["srcdoc", "credentialless", "blob", "sandbox"]) {
      const result = await frameProbe(
        mode,
        `const c=new MessageChannel();
        const channel=await resultOf(c.port1,()=>c.port2.postMessage(module()));c.port1.close();c.port2.close();
        const self=await resultOf(window,()=>postMessage(module(),'*'));
        const worker=await workerModuleProbe(); return {channel,self,worker};`,
      );
      record(mode + "-channel", result.channel, accepted);
      record(mode + "-window-self", result.self, accepted);
      record(mode + "-blob-worker", result.worker, accepted);
    }
    for (const mode of ["srcdoc", "credentialless", "blob"]) {
      // Register the parent listener before the child posts its module.
      const incoming = resultOf(window, () => {});
      const done = frameProbe(
        mode,
        `parent.postMessage(module(),'*'); return null;`,
      );
      record(
        mode + "-to-parent",
        await incoming,
        mode === "blob"
          ? { type: "messageerror", wasm: false, ports: 0 }
          : accepted,
      );
      await done;
    }
  } finally {
    for (const c of channels) {
      c.port1.close();
      c.port2.close();
    }
    for (const frame of frames) frame.remove();
    for (const url of urls) URL.revokeObjectURL(url);
  }
  return {
    origin: location.origin,
    total: checks.length,
    failures: checks.filter((row) => !row.passed),
    checks,
  };
});
