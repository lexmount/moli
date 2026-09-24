async function setupNavigationFocusProbe(crossOrigin, allow, deniedHeader = false) {
  document.body.innerHTML = `
    <input id="parent-input" style="position:absolute;left:300px">
    <button id="parent-autofocus" autofocus style="position:absolute;left:300px;top:30px">Parent</button>
  `;
  const frame = document.createElement("iframe");
  frame.id = "focus-frame";
  frame.style.cssText = "position:absolute;left:0;top:0;width:220px;height:220px;border:0";
  if (allow !== null) frame.allow = `focus-without-user-activation ${allow}`;
  const childURL = new URL(deniedHeader ? "/focus-denied.html" : "/focus-child.html", location.href);
  if (crossOrigin) childURL.hostname = childURL.hostname === "localhost" ? "127.0.0.1" : "localhost";
  frame.src = childURL.href;
  const messages = [];
  const waiters = new Map();
  addEventListener("message", event => {
    if (event.source !== frame.contentWindow) return;
    messages.push(event.data);
    const resolve = waiters.get(event.data?.phase);
    if (resolve) resolve(event.data);
  });
  globalThis.focusProbeWait = phase => {
    const message = messages.find(message => message.phase === phase);
    return message ? Promise.resolve(message) : new Promise(resolve => waiters.set(phase, resolve));
  };
  globalThis.focusProbeFrame = frame;
  globalThis.focusProbeResult = async () => {
    const done = await focusProbeWait("done");
    return {
      active: done.active,
      parent: document.activeElement.id,
      activation: done.activation,
      userInitiated: messages.find(message => message.phase === "started").userInitiated,
    };
  };
  document.body.append(frame);
  await focusProbeWait("ready");
  document.getElementById("parent-input").focus();
}

async function runNavigationFocusProbe(crossOrigin, allow, mode, deniedHeader = false) {
  await setupNavigationFocusProbe(crossOrigin, allow, deniedHeader);
  focusProbeFrame.contentWindow.postMessage({ action: "navigate", mode }, "*");
  return focusProbeResult();
}

async function runNavigationViewportFocusProbe() {
  const results = [];
  for (const mode of ["child-viewport", "parent-viewport"]) {
    const frame = document.createElement("iframe");
    frame.src = "/history.html?focus-child";
    const loaded = new Promise(resolve => frame.onload = resolve);
    document.body.append(frame);
    await loaded;
    const child = frame.contentWindow;
    child.document.body.innerHTML = '<button id="button">Button</button>';
    const button = child.document.getElementById("button");
    button.focus();
    if (mode === "child-viewport") {
      child.navigation.onnavigate = event => event.intercept({});
      await child.navigation.navigate("#reset").finished;
      child.navigation.onnavigate = event => event.intercept({ handler: () => button.focus() });
      await child.navigation.navigate("#author-focus").finished;
    } else {
      const autofocus = child.document.createElement("input");
      autofocus.id = "autofocus";
      autofocus.autofocus = true;
      child.document.body.append(autofocus);
      let finish;
      child.navigation.onnavigate = event => event.intercept({ handler: () => new Promise(resolve => finish = resolve) });
      const pending = child.navigation.navigate("#waiting");
      await pending.committed;
      navigation.onnavigate = event => event.intercept({});
      await navigation.navigate("#parent-viewport").finished;
      finish();
      await pending.finished;
    }
    results.push({ mode, active: child.document.activeElement.id, frameFocused: document.activeElement === frame });
    frame.remove();
  }
  return results;
}
