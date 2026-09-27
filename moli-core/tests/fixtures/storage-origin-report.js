(function inspectStorage(target, token) {
  const read = callback => {
    try { return callback(); } catch (error) { return error.name; }
  };
  const storage = {};
  for (const name of ['localStorage', 'sessionStorage']) {
    const descriptor = Object.getOwnPropertyDescriptor(target, name);
    storage[name] = {
      descriptor: descriptor && [typeof descriptor.get, typeof descriptor.set,
        descriptor.enumerable, descriptor.configurable, 'value' in descriptor],
      access: read(() => { target[name].getItem('storage-origin-' + token); return 'accessible'; }),
      ownGetter: read(() => descriptor.get.call(target) === target[name] ? 'same' : 'different'),
      forged: read(() => { descriptor.get.call({}); return 'accepted'; }),
    };
  }
  return {token, origin: target.origin, storage};
})
