self.oninstall = event => event.waitUntil(skipWaiting());
self.onmessage = event => event.ports[0].postMessage({
  secure: isSecureContext,
  isolated: crossOriginIsolated,
});
