function popupLocationFrame(hosts, index) {
  const inspect = target => {
    const read = callback => {
      try { return callback(); } catch (error) { return error.name; }
    };
    const conversion = callback => {
      let count = 0;
      const value = { toString() { ++count; throw new Error('conversion'); } };
      return read(() => { callback(value); return 'returned'; }) + ':' + count;
    };
    const result = {};
    for (const key of ['href', 'origin', 'protocol', 'host', 'hostname', 'port',
                       'pathname', 'search', 'hash', 'ancestorOrigins']) {
      result['get:' + key] = read(() => { void target[key]; return 'ok'; });
      result['descriptor:' + key] = read(() => {
        const descriptor = Object.getOwnPropertyDescriptor(target, key);
        return typeof descriptor.get + ':' + typeof descriptor.set;
      });
    }
    for (const key of ['href', 'protocol', 'host', 'hostname', 'port',
                       'pathname', 'search', 'hash']) {
      result['set:' + key] = conversion(value => { target[key] = value; });
    }
    result.path = read(() => target.pathname);
    result.toString = read(() => { target.toString(); return 'ok'; });
    result.assign = conversion(value => target.assign(value));
    result.replace = conversion(value => target.replace(value));
    result.hrefInvalidURL = read(() => { target.href = 'http://['; });
    result.replaceInvalidURL = read(() => target.replace('http://['));
    result.borrowedGetter = read(() => {
      Object.getOwnPropertyDescriptor(location, 'pathname').get.call(target);
      return 'ok';
    });
    result.borrowedSetter = conversion(value => {
      Object.getOwnPropertyDescriptor(location, 'pathname').set.call(target, value);
    });
    result.borrowedAssign = conversion(value => location.assign.call(target, value));
    result.borrowedToString = read(() => { location.toString.call(target); return 'ok'; });
    result.keys = read(() => Object.getOwnPropertyNames(target).sort());
    result.prototypeIsNull = read(() => Object.getPrototypeOf(target) === null);
    return result;
  };
  let popupResult;
  if (index < hosts.length - 1) {
    addEventListener('message', event => {
      const result = event.data;
      if (index === 0) result.popup = popupResult;
      (index === 0 ? opener : parent).postMessage(result, '*');
    });
  }
  addEventListener('load', () => {
    try {
      if (index === 0) {
        popupResult = { self: inspect(location), opener: inspect(opener.location) };
      }
      if (index < hosts.length - 1) {
        const frame = document.createElement('iframe');
        const url = new URL('/frame-' + (index + 1), location.href);
        url.hostname = hosts[index + 1];
        frame.src = url.href;
        document.body.appendChild(frame);
      } else {
        parent.postMessage({ top: inspect(top.location), self: inspect(location),
                             opener: inspect(top.opener.location) }, '*');
      }
    } catch (error) {
      (index === 0 ? opener : parent).postMessage({error: String(error), stack: error.stack, index}, '*');
    }
  });
}
