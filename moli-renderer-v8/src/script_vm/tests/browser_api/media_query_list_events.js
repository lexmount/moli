function mediaQueryListEventConstructorProbe() {
  const failures = [];
  let checks = 0;
  const check = (value, message) => { ++checks; if (!value) failures.push(message); };
  const throws = (operation, expected, message) => {
    try { operation(); check(false, message); }
    catch (error) { check(error === expected || (typeof expected === "function" && error instanceof expected), message); }
  };
  for (const init of [undefined, null, {}]) {
    const event = new MediaQueryListEvent("custom", init);
    check(event instanceof MediaQueryListEvent && event instanceof Event, "constructor inheritance");
    check(event.type === "custom" && event.media === "" && !event.matches &&
      !event.bubbles && !event.cancelable && !event.composed && !event.isTrusted, "event defaults");
  }
  const order = [];
  const init = {};
  for (const name of ["bubbles", "cancelable", "composed", "matches", "media"]) {
    Object.defineProperty(init, name, {get() {
      order.push(name);
      return name === "media" ? {toString() { order.push("media:string"); return "\ud800"; }} : true;
    }});
  }
  const event = new MediaQueryListEvent({toString() { order.push("type"); return "\udfff"; }}, init);
  check(order.join() === "type,bubbles,cancelable,composed,matches,media,media:string", "dictionary order " + order);
  check(event.type === "\udfff" && event.media === "\ud800" && event.matches &&
    event.bubbles && event.cancelable && event.composed, "converted event fields");
  for (const name of ["media", "matches"]) {
    const descriptor = Object.getOwnPropertyDescriptor(MediaQueryListEvent.prototype, name);
    check(descriptor.enumerable && descriptor.configurable && typeof descriptor.get === "function" &&
      descriptor.set === undefined && !Object.hasOwn(event, name), "readonly prototype attribute " + name);
    check(!Reflect.set(event, name, "changed"), "readonly value " + name);
    throws(() => descriptor.get.call(new Proxy(event, {})), TypeError, "proxy getter " + name);
  }
  check(new MediaQueryListEvent(undefined, {media: null, matches: ""}).type === "undefined", "undefined type");
  check(new MediaQueryListEvent("x", {media: null}).media === "null", "null media");
  check(new MediaQueryListEvent("x", {matches: []}).matches, "boolean conversion");
  for (const value of [false, 1, "", Symbol()]) {
    throws(() => new MediaQueryListEvent("x", value), TypeError, "invalid dictionary");
  }
  throws(() => MediaQueryListEvent("x"), TypeError, "requires new");
  throws(() => new MediaQueryListEvent(), TypeError, "required type");
  throws(() => new MediaQueryListEvent(Symbol()), TypeError, "symbol type");
  throws(() => new MediaQueryListEvent("x", {media: Symbol()}), TypeError, "symbol media");
  const marker = {};
  const members = ["bubbles", "cancelable", "composed", "matches", "media"];
  for (const [index, member] of members.entries()) {
    const reads = [];
    const dictionary = new Proxy({}, {get(_, key) {
      reads.push(key);
      if (key === member) throw marker;
      return undefined;
    }});
    throws(() => new MediaQueryListEvent("x", dictionary), marker, "getter exception " + member);
    check(reads.join() === members.slice(0, index + 1).join(), "stopped dictionary conversion " + member);
  }
  class SubEvent extends MediaQueryListEvent {}
  const subclass = new SubEvent("sub", {media: "all", matches: true});
  check(subclass instanceof SubEvent && subclass.media === "all" && subclass.matches, "subclass brand");
  const frame = document.body.appendChild(document.createElement("iframe"));
  try {
    const realm = frame.contentWindow;
    const getter = Object.getOwnPropertyDescriptor(realm.MediaQueryListEvent.prototype, "media").get;
    check(getter.call(event) === "\ud800", "cross-realm getter");
    throws(() => getter.call(new Event("x")), realm.TypeError, "callee realm error");
  } finally { frame.remove(); }
  return {checks, failures};
}
