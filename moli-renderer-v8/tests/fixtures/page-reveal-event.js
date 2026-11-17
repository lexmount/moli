(() => {
  function check(value, message) { if (!value) throw new Error(message); }
  function typeError(callback, message) {
    try { callback(); } catch (error) { check(error instanceof TypeError, message); return; }
    throw new Error(message);
  }
  const event = new PageRevealEvent('pagereveal');
  check(event instanceof Event && event instanceof PageRevealEvent, 'inheritance');
  check(Object.prototype.toString.call(event) === '[object PageRevealEvent]', 'tag');
  check(PageRevealEvent.length === 1, 'constructor arity');
  check(event.viewTransition === null, 'default transition');
  check(!event.bubbles && !event.cancelable && !event.composed && !event.isTrusted, 'flags');
  check(new PageRevealEvent('x', null).viewTransition === null, 'null dictionary');
  check(new PageRevealEvent('x', {viewTransition: undefined}).viewTransition === null, 'undefined member');
  const descriptor = Object.getOwnPropertyDescriptor(PageRevealEvent.prototype, 'viewTransition');
  check(descriptor.enumerable && descriptor.configurable && !descriptor.set, 'readonly descriptor');
  check(!Object.hasOwn(event, 'viewTransition'), 'prototype attribute');
  typeError(() => descriptor.get.call({}), 'getter brand');
  typeError(() => descriptor.get.call(new Event('x')), 'Event is not PageRevealEvent');
  typeError(() => new PageRevealEvent(), 'required type');
  typeError(() => PageRevealEvent('x'), 'new required');
  typeError(() => new PageRevealEvent(Symbol()), 'DOMString conversion');
  for (const value of [{}, Object.create(ViewTransition.prototype), new Proxy(event, {}), 1]) {
    typeError(() => new PageRevealEvent('x', {viewTransition: value}), 'transition brand');
  }
  const transition = document.startViewTransition(() => {});
  const withTransition = new PageRevealEvent('custom', {
    bubbles: true, cancelable: true, composed: true, viewTransition: transition
  });
  check(withTransition.viewTransition === transition, 'actual transition identity');
  check(withTransition.bubbles && withTransition.cancelable && withTransition.composed, 'inherited flags');
  const order = [];
  new PageRevealEvent({toString() { order.push('type'); return 'x'; }}, {
    get bubbles() { order.push('bubbles'); return false; },
    get cancelable() { order.push('cancelable'); return false; },
    get composed() { order.push('composed'); return false; },
    get viewTransition() { order.push('viewTransition'); return transition; }
  });
  check(order.join() === 'type,bubbles,cancelable,composed,viewTransition', 'conversion order');
  const sentinel = {};
  try {
    new PageRevealEvent('x', { get viewTransition() { throw sentinel; } });
    throw new Error('getter must throw');
  } catch (error) { check(error === sentinel, 'getter exception identity'); }
  return true;
})()
