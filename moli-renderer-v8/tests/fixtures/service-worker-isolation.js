function facts() {
  return {secure: isSecureContext, isolated: crossOriginIsolated};
}
if ('registration' in self) {
  self.oninstall = event => event.waitUntil(skipWaiting());
  self.onmessage = event => event.ports[0].postMessage(facts());
} else {
  self.onconnect = event => {
    const port = event.ports[0];
    port.onmessage = () => port.postMessage(facts());
    port.start();
  };
}
