(() => {
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const other = frame.contentWindow;
  const checks = [];
  try {
    for (const name of ['brands', 'mobile', 'platform']) {
      const foreign = Object.getOwnPropertyDescriptor(other.NavigatorUAData.prototype, name)?.get;
      const local = Object.getOwnPropertyDescriptor(NavigatorUAData.prototype, name)?.get;
      let genuine = false, foreignError = false, localError = false;
      if (typeof foreign === 'function') {
        const value = foreign.call(navigator.userAgentData);
        genuine = name === 'brands'
          ? Object.isFrozen(value) && Object.getPrototypeOf(value) === Array.prototype &&
            Object.getPrototypeOf(value[0]) === Object.prototype
          : value === navigator.userAgentData[name];
        try { foreign.call(new Proxy(navigator.userAgentData, {})); }
        catch (error) { foreignError = error instanceof other.TypeError && !(error instanceof TypeError); }
      }
      if (typeof local === 'function') {
        try { local.call(new other.Proxy(other.navigator.userAgentData, {})); }
        catch (error) { localError = error instanceof TypeError && !(error instanceof other.TypeError); }
      }
      checks.push(
        {name: `${name} accepts a genuine foreign receiver and preserves FrozenArray realm`, pass: genuine},
        {name: `${name} rejects author proxy with foreign TypeError`, pass: foreignError},
        {name: `${name} rejects foreign proxy with local TypeError`, pass: localError}
      );
    }
  } finally { frame.remove(); }
  return {checks, passed: checks.filter(check => check.pass).length, total: checks.length};
})()
