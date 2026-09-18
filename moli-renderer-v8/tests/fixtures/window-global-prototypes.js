(() => {
  const failures = [];
  let checks = 0;
  const check = (value, label) => { checks++; if (!value) failures.push(label); };
  const throwsTypeError = (callback, constructor) => {
    try { callback(); } catch (error) { return error instanceof constructor; }
    return false;
  };
  if (!document.documentElement) document.appendChild(document.createElement('html'));
  if (!document.body) document.documentElement.appendChild(document.createElement('body'));
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const probe = (w, label) => {
    const wp = Object.getPrototypeOf(w.Window.prototype);
    const chain = [w, w.Window.prototype, wp, w.EventTarget.prototype, w.Object.prototype];
    for (let i = 0; i < chain.length; i++) {
      const value = chain[i];
      const original = Object.getPrototypeOf(value);
      const expected = chain[i + 1] || null;
      check(original === expected, label + ' parent ' + i);
      check(Object.isExtensible(value), label + ' extensible ' + i);
      check(!Reflect.setPrototypeOf(value, {}), label + ' Reflect rejects prototype ' + i);
      if (Object.getPrototypeOf(value) !== original) Reflect.setPrototypeOf(value, original);
      check(throwsTypeError(() => Object.setPrototypeOf(value, {}), TypeError), label + ' calling realm TypeError ' + i);
      if (Object.getPrototypeOf(value) !== original) Reflect.setPrototypeOf(value, original);
      check(throwsTypeError(() => w.Object.setPrototypeOf(value, {}), w.TypeError), label + ' target realm TypeError ' + i);
      if (Object.getPrototypeOf(value) !== original) Reflect.setPrototypeOf(value, original);
      const setter = Object.getOwnPropertyDescriptor(w.Object.prototype, '__proto__').set;
      check(throwsTypeError(() => setter.call(value, {}), w.TypeError), label + ' dunder rejects prototype ' + i);
      if (Object.getPrototypeOf(value) !== original) Reflect.setPrototypeOf(value, original);
      check(Reflect.setPrototypeOf(value, original), label + ' same prototype allowed ' + i);
      check(Object.setPrototypeOf(value, original) === value, label + ' Object same prototype ' + i);
      check(Object.getPrototypeOf(value) === original, label + ' prototype unchanged ' + i);
      if (value !== wp) {
        const key = Symbol('extensible prototype probe');
        check(Reflect.defineProperty(value, key, {value: 1, configurable: true}), label + ' property additions allowed ' + i);
        check(value[key] === 1, label + ' property visible ' + i);
        check(Reflect.deleteProperty(value, key), label + ' property deletion allowed ' + i);
      }
    }
    check(Object.getPrototypeOf(w.Window) === w.EventTarget, label + ' Window constructor inherits EventTarget');
    check(w.Window.prototype.constructor === w.Window, label + ' Window constructor identity');
    check(!Object.hasOwn(wp, 'constructor'), label + ' anonymous WindowProperties');
    check(wp.constructor === w.EventTarget, label + ' inherited constructor');
    check(Object.prototype.toString.call(wp) === '[object WindowProperties]', label + ' named properties tag');
    check(Reflect.ownKeys(wp).length === 1 && Reflect.ownKeys(wp)[0] === Symbol.toStringTag, label + ' named properties keys');
    check(!Reflect.preventExtensions(wp) && Object.isExtensible(wp), label + ' named properties stay extensible');
    check(w instanceof w.EventTarget, label + ' global EventTarget instance');
    check(throwsTypeError(() => w.EventTarget.prototype.dispatchEvent.call(wp, new w.Event('probe')), w.TypeError), label + ' named properties lack EventTarget brand');
    const named = w.document.createElement('div');
    named.id = 'immutablePrototypeNamedProbe';
    w.document.body.appendChild(named);
    try {
      check(w[named.id] === named && wp[named.id] === named, label + ' named access');
      check(Object.getOwnPropertyDescriptor(wp, named.id).value === named, label + ' named descriptor');
      w.EventTarget.prototype[named.id] = 23;
      try {
        check(w[named.id] === 23, label + ' inherited property shadows name');
        check(Object.getOwnPropertyDescriptor(wp, named.id) === undefined, label + ' shadowed descriptor absent');
      } finally { delete w.EventTarget.prototype[named.id]; }
      check(w[named.id] === named, label + ' name restored after deletion');
      for (const key of [named.id, 'missing', 0, Symbol.toStringTag, Symbol('missing')]) {
        check(!Reflect.defineProperty(wp, key, {}), label + ' named properties reject definition');
        check(!Reflect.deleteProperty(wp, key), label + ' named properties reject deletion');
        check(!Reflect.set(wp, key, 9), label + ' named properties reject direct write');
      }
      const receiver = Object.create(wp);
      check(Reflect.set(wp, named.id, 42, receiver), label + ' different receiver write succeeds');
      check(Object.hasOwn(receiver, named.id) && receiver[named.id] === 42, label + ' different receiver gets own property');
      check(wp[named.id] === named, label + ' receiver write preserves named value');
    } finally { named.remove(); }
    const key = Symbol('inherited setter');
    let received;
    Object.defineProperty(w.EventTarget.prototype, key, {
      configurable: true, set(value) { received = [this, value]; }
    });
    try {
      check(Reflect.set(wp, key, 7), label + ' inherited setter called directly');
      check(received[0] === wp && received[1] === 7, label + ' direct setter receiver');
      const receiver = {};
      check(Reflect.set(wp, key, 8, receiver), label + ' inherited setter called with receiver');
      check(received[0] === receiver && received[1] === 8, label + ' supplied setter receiver');
    } finally { delete w.EventTarget.prototype[key]; }
  };
  try {
    probe(window, 'main');
    probe(frame.contentWindow, 'child');
  } finally { frame.remove(); }
  return {checks, failures};
})()
