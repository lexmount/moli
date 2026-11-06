globalThis.runIdleBudgetProbe = variant => {
  const row = globalThis.__idleBudgetProbe = { variant, done: false };
  const child = document.querySelector('iframe').contentWindow;
  let animation;
  if (variant === 'pending-animation') animation = requestAnimationFrame(() => {});
  if (variant === 'child-animation') animation = child.requestAnimationFrame(() => {});
  requestIdleCallback(deadline => {
    row.didTimeout = deadline.didTimeout;
    row.before = deadline.timeRemaining();
    let timer;
    switch (variant) {
      case 'timeout': timer = setTimeout(() => {}, 10); break;
      case 'interval': timer = setInterval(() => {}, 10); break;
      case 'source': timer = setTimeout('void 0', 10); break;
      case 'animation': animation = requestAnimationFrame(() => {}); break;
      case 'child-timeout': timer = child.setTimeout(() => {}, 10); break;
      case 'borrowed-method': timer = setTimeout(() => {}, 10); break;
      case 'cancelled': timer = setTimeout(() => {}, 10); clearTimeout(timer); break;
      case 'cancelled-animation':
        cancelAnimationFrame(requestAnimationFrame(() => {}));
        break;
      case 'idle-only': {
        const idle = requestIdleCallback(() => {});
        cancelIdleCallback(idle);
        break;
      }
      case 'empty':
      case 'pending-animation':
      case 'child-animation': break;
      default: throw new Error(variant);
    }
    row.after = variant === 'borrowed-method'
      ? child.IdleDeadline.prototype.timeRemaining.call(deadline)
      : deadline.timeRemaining();
    if (timer !== undefined) {
      if (variant === 'child-timeout') child.clearTimeout(timer);
      else clearInterval(timer);
    }
    if (animation !== undefined) {
      if (variant === 'child-animation') child.cancelAnimationFrame(animation);
      else cancelAnimationFrame(animation);
    }
    row.done = true;
  });
  return 'scheduled';
};
