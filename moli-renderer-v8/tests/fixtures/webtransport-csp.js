globalThis.runWebTransportCspProbe = async function(config, realm = globalThis) {
  const reports = [];
  let constructing = true;
  const listener = event => reports.push({
    directive: event.effectiveDirective,
    disposition: event.disposition,
    blockedURI: event.blockedURI,
    documentURI: event.documentURI,
    hasSourceLocation: event.lineNumber > 0 && event.columnNumber > 0,
    trusted: event.isTrusted,
    duringConstructor: constructing,
  });
  realm.addEventListener('securitypolicyviolation', listener);
  try {
    const options = config.validation === 'dictionary'
      ? {get serverCertificateHashes() {throw new realm.RangeError('conversion');}}
      : {};
    let transport;
    try {
      const url = config.validation === 'url' ? '/relative' : config.url;
      if (config.validation === 'new-target') {
        const NewTarget = function() {};
        NewTarget.prototype = realm.WebTransport.prototype;
        transport = Reflect.construct(realm.WebTransport, [url, options], NewTarget);
      } else {
        transport = new realm.WebTransport(url, options);
      }
    } catch (error) {
      constructing = false;
      await new Promise(resolve => setTimeout(resolve, 0));
      return {thrown: error.name, reports};
    }
    constructing = false;
    let settled = false;
    const ready = transport.ready.then(() => {settled = true; return null;}, error => {settled = true; return error;});
    const closed = transport.closed.catch(error => error);
    await Promise.resolve();
    const pendingAfterMicrotask = !settled;
    if (config.closeImmediately) transport.close();
    const readyError = await ready;
    const closedError = await closed;
    while (reports.length < config.reportCount) await new Promise(resolve => setTimeout(resolve, 0));
    // Drain older CSP tasks before checking for duplicate reports.
    await new Promise(resolve => setTimeout(resolve, 0));
    await new Promise(resolve => setTimeout(resolve, 0));
    return {
      name: readyError?.name,
      source: readyError?.source,
      streamErrorCode: readyError?.streamErrorCode,
      errorRealm: readyError instanceof realm.WebTransportError,
      sameError: readyError === closedError,
      pendingAfterMicrotask,
      reports,
    };
  } finally {
    realm.removeEventListener('securitypolicyviolation', listener);
  }
};
