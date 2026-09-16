function mediaQueryListEventTargetProbe() {
  const failures = [];
  const rows = [];
  const check = (value, label) => { if (!value) failures.push(label); };
  const frame = document.body.appendChild(document.createElement("iframe"));
  try {
    for (const [name, realm, methods] of [
      ["main", window, frame.contentWindow.EventTarget.prototype],
      ["child", frame.contentWindow, EventTarget.prototype],
    ]) {
      const target = realm.matchMedia("all");
      const calls = [];
      check(target instanceof realm.EventTarget, name + ":brand inheritance");
      check(Object.getPrototypeOf(realm.MediaQueryList.prototype) === realm.EventTarget.prototype,
        name + ":prototype inheritance");
      for (const method of ["addEventListener", "removeEventListener", "dispatchEvent"]) {
        check(!Object.hasOwn(realm.MediaQueryList.prototype, method), name + ":inherited " + method);
        check(target[method] === realm.EventTarget.prototype[method], name + ":shared " + method);
      }
      const legacy = {handleEvent(event) {
        check(this === legacy && event.target === target && event.currentTarget === target,
          name + ":callback receiver");
        check(event.eventPhase === Event.AT_TARGET && window.event === event,
          name + ":callback dispatch state");
        calls.push("legacy");
      }};
      target.addListener(legacy);
      methods.addEventListener.call(target, "change", legacy, {once: true});
      target.onchange = () => calls.push("old-handler");
      methods.addEventListener.call(target, "change", () => calls.push("modern"));
      target.onchange = () => { calls.push("handler"); return false; };
      methods.addEventListener.call(target, "change", () => calls.push("capture"), true);
      const event = new Event("change", {cancelable: true});
      check(methods.dispatchEvent.call(target, event) === false, name + ":cancellation");
      check(calls.join() === "capture,legacy,handler,modern", name + ":handler order " + calls);
      check(event.currentTarget === null && event.eventPhase === 0 && event.target === target,
        name + ":dispatch cleanup");
      target.onchange = null;
      target.onchange = () => calls.push("last-handler");
      calls.length = 0;
      target.dispatchEvent(new Event("change"));
      check(calls.join() === "capture,legacy,modern,last-handler", name + ":handler re-add " + calls);
      methods.removeEventListener.call(target, "change", legacy);
      target.onchange = 12;
      check(target.onchange === null, name + ":handler conversion");
      calls.length = 0;
      target.dispatchEvent(new Event("change"));
      check(calls.join() === "capture,modern", name + ":legacy removal " + calls);
      let once = 0;
      const controller = new AbortController();
      methods.addEventListener.call(target, "probe", () => ++once, {once: true});
      methods.addEventListener.call(target, "probe", () => failures.push(name + ":aborted listener"),
        {signal: controller.signal});
      controller.abort();
      target.dispatchEvent(new Event("probe"));
      target.dispatchEvent(new Event("probe"));
      check(once === 1, name + ":once");
      rows.push(name);
    }
  } finally { frame.remove(); }
  return {rows, failures};
}

function mediaQueryListReceiverProbe() {
  const failures = [];
  let checks = 0;
  const frame = document.body.appendChild(document.createElement("iframe"));
  try {
    for (const [name, realm] of [["main", window], ["child", frame.contentWindow]]) {
      const target = realm.matchMedia("all");
      const revoked = Proxy.revocable(target, {});
      revoked.revoke();
      const invalid = [{}, Object.create(realm.MediaQueryList.prototype), Object.create(target),
        new Proxy(target, {}), revoked.proxy];
      let conversions = 0;
      const type = {toString() { ++conversions; return "probe"; }};
      const methods = realm.EventTarget.prototype;
      const prototype = realm.MediaQueryList.prototype;
      const operations = [
        value => methods.addEventListener.call(value, type, () => {}),
        value => methods.removeEventListener.call(value, type, () => {}),
        value => methods.dispatchEvent.call(value, new Event("probe")),
        value => prototype.addListener.call(value, () => {}),
        value => prototype.removeListener.call(value, () => {}),
        ...["media", "matches", "onchange"].map(property =>
          value => Object.getOwnPropertyDescriptor(prototype, property).get.call(value)),
        value => Object.getOwnPropertyDescriptor(prototype, "onchange").set.call(value, () => {}),
      ];
      for (const [index, operation] of operations.entries()) for (const value of invalid) {
        ++checks;
        try { operation(value); failures.push(name + ":accepted receiver " + index); }
        catch (error) {
          if (!(error instanceof realm.TypeError)) failures.push(name + ":wrong exception realm " + index);
        }
      }
      if (conversions !== 0) failures.push(name + ":converted before receiver check");
      let reads = 0;
      const fake = {get type() { ++reads; return "probe"; }};
      try { methods.dispatchEvent.call(target, fake); failures.push(name + ":accepted fake event"); }
      catch (error) { if (!(error instanceof realm.TypeError)) failures.push(name + ":fake event error"); }
      if (reads !== 0) failures.push(name + ":read unbranded event");
      target.addEventListener("probe", event => {
        try { methods.dispatchEvent.call(target, event); failures.push(name + ":redispatched active event"); }
        catch (error) { if (error.name !== "InvalidStateError") failures.push(name + ":active event error"); }
      });
      target.dispatchEvent(new Event("probe"));
    }
  } finally { frame.remove(); }
  return {checks, failures};
}

function mediaQueryListLifetimeProbe() {
  const failures = [];
  const frame = document.createElement("iframe");
  frame.width = "200";
  document.body.appendChild(frame);
  const realm = frame.contentWindow;
  const target = realm.matchMedia("(max-width: 200px)");
  const parentMatches = Object.getOwnPropertyDescriptor(MediaQueryList.prototype, "matches").get;
  if (!target.matches || !parentMatches.call(target)) failures.push("borrowed matches used caller viewport");
  const childMatches = Object.getOwnPropertyDescriptor(realm.MediaQueryList.prototype, "matches").get;
  if (childMatches.call(matchMedia("(max-width: 200px)"))) failures.push("child getter used child viewport for parent");
  let calls = 0;
  target.addListener(() => ++calls);
  const dispatchers = [EventTarget.prototype.dispatchEvent, realm.EventTarget.prototype.dispatchEvent];
  frame.remove();
  for (const dispatch of dispatchers) {
    const event = new Event("change");
    try {
      if (dispatch.call(target, event) !== false) failures.push("retired target returned true");
      if (event.target !== null || event.isTrusted) failures.push("retired dispatch mutated event");
    } catch (error) { failures.push("retired dispatch: " + error.name); }
    for (const [value, expected] of [[null, "TypeError"], [{}, "TypeError"],
      [document.createEvent("Event"), "InvalidStateError"]]) {
      try { dispatch.call(target, value); failures.push("retired target accepted invalid event"); }
      catch (error) { if (error.name !== expected) failures.push("retired argument validation: " + error.name); }
    }
  }
  if (calls !== 0) failures.push("retired target invoked listeners");
  return {calls, failures};
}
