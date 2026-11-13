async function runInstallEventConstructorChecks() {
    const check = (value, name) => {
        if (!value) throw Error(name);
    };
    const throws = (callback, name) => {
        let caught;
        try { callback(); } catch (error) { caught = error; }
        check(caught && caught.name === name, 'expected ' + name);
    };
    check(InstallEvent.name === 'InstallEvent' && InstallEvent.length === 1, 'constructor');
    check(Object.getPrototypeOf(InstallEvent) === ExtendableEvent, 'constructor inheritance');
    check(Object.getPrototypeOf(InstallEvent.prototype) === ExtendableEvent.prototype, 'prototype inheritance');
    check(Object.getOwnPropertyDescriptor(InstallEvent, 'prototype').writable === false, 'readonly prototype');
    const basic = new InstallEvent('install');
    check(basic instanceof InstallEvent && basic instanceof ExtendableEvent && basic instanceof Event, 'brands');
    check(Object.prototype.toString.call(basic) === '[object InstallEvent]', 'toStringTag');
    check(basic.type === 'install' && !basic.bubbles && !basic.cancelable && !basic.composed && !basic.isTrusted, 'defaults');
    check(basic.target === null && basic.currentTarget === null && basic.eventPhase === Event.NONE, 'initial state');
    check(new InstallEvent('\ud800').type === '\ud800', 'DOMString preserves UTF-16');
    check(new InstallEvent(undefined, null).type === 'undefined', 'undefined argument and null dictionary');
    check(new InstallEvent('x', undefined).bubbles === false, 'undefined dictionary');
    throws(() => InstallEvent('x'), 'TypeError');
    throws(() => new InstallEvent(), 'TypeError');
    throws(() => new InstallEvent(Symbol()), 'TypeError');
    for (const init of [0, 1, true, 'x']) throws(() => new InstallEvent('x', init), 'TypeError');
    const reads = [];
    const event = new InstallEvent({toString() { reads.push('type'); return 'custom'; }}, {
        get bubbles() { reads.push('bubbles'); return 1; },
        get cancelable() { reads.push('cancelable'); return {}; },
        get composed() { reads.push('composed'); return 'yes'; }
    });
    check(reads.join(',') === 'type,bubbles,cancelable,composed', 'conversion order');
    check(event.bubbles && event.cancelable && event.composed, 'init flags');
    for (const attribute of ['type', 'bubbles', 'cancelable', 'composed']) {
        const getter = Object.getOwnPropertyDescriptor(Event.prototype, attribute).get;
        check(getter.call(event) === event[attribute], 'private state for ' + attribute);
        check(!Object.hasOwn(event, attribute), 'inherited attribute ' + attribute);
        check(Reflect.set(event, attribute, null) === false, 'readonly attribute ' + attribute);
    }
    const marker = {};
    let caught;
    try { new InstallEvent('x', { get bubbles() { throw marker; } }); } catch (error) { caught = error; }
    check(caught === marker, 'getter exception identity');
    class Derived extends InstallEvent {}
    const derived = new Derived('derived');
    check(derived instanceof Derived && derived instanceof InstallEvent, 'subclass');
    check(Object.getPrototypeOf(derived) === Derived.prototype, 'new.target prototype');
    throws(() => basic.waitUntil(Promise.resolve()), 'InvalidStateError');
    for (const fake of [{}, Object.create(InstallEvent.prototype), new Proxy(basic, {})]) {
        throws(() => ExtendableEvent.prototype.waitUntil.call(fake, Promise.resolve()), 'TypeError');
    }
    const target = new EventTarget();
    let received;
    target.addEventListener('custom', value => { received = value; value.preventDefault(); });
    check(target.dispatchEvent(event) === false && received === event && event.defaultPrevented, 'dispatch and cancellation');
    check(!event.isTrusted && event.currentTarget === null && event.eventPhase === Event.NONE, 'dispatch cleanup');
    return 'checked';
}
