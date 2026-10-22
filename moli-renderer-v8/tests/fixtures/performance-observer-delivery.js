(() => new Promise((resolve, reject) => {
  const batches = [];
  const taken = [];
  let proxyCalls = 0;
  const check = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const callback = new Proxy(function(list, deliveredObserver, options) {
    try {
      check(this === observer && deliveredObserver === observer, 'callback receiver and observer');
      check(arguments.length === 3, 'callback argument count');
      check(list instanceof PerformanceObserverEntryList, 'native entry list');
      const descriptor = Object.getOwnPropertyDescriptor(options, 'droppedEntriesCount');
      check(Object.getPrototypeOf(options) === Object.prototype &&
            Object.keys(options).join() === 'droppedEntriesCount' &&
            descriptor.value === 0 && descriptor.writable && descriptor.enumerable && descriptor.configurable,
            'callback options after observe');
      check(observer.takeRecords().length === 0, 'pending batch cleared before callback');
      const entries = list.getEntries();
      batches.push(entries.map(entry => entry.name));
      if (batches.length === 1) {
        performance.mark('taken-during-callback');
        taken.push(...observer.takeRecords().map(entry => entry.name));
        performance.mark('cleared-by-disconnect');
        observer.disconnect();
        performance.mark('ignored-while-disconnected');
        observer.observe({ type: 'mark' });
        performance.mark('after-reobserve');
        check(list.getEntries().length === entries.length, 'delivered list remains a snapshot');
      } else {
        observer.disconnect();
        resolve({ batches, taken, proxyCalls });
      }
    } catch (error) {
      observer.disconnect();
      reject(error);
    }
  }, {
    apply(target, receiver, args) {
      proxyCalls++;
      return Reflect.apply(target, receiver, args);
    }
  });
  const observer = new PerformanceObserver(callback);
  observer.observe({ type: 'mark' });
  // Repeated observe must not register or deliver the observer twice.
  observer.observe({ type: 'mark' });
  performance.mark('first');
  performance.mark('second');
}))()
