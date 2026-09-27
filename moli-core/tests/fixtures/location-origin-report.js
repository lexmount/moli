addEventListener('load', () => {
  const token = globalThis.originToken || new URL(location.href).searchParams.get('token');
  const result = {
    token,
    href: location.href,
    locationOrigin: location.origin,
    windowOrigin: self.origin,
    ownGetter: Object.getOwnPropertyDescriptor(location, 'origin').get.call(location),
  };
  (opener || parent).postMessage(result, '*');
});
