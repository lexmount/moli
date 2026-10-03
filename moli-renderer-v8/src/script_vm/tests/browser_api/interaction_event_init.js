(() => {
  const result = globalThis.__interactionEventProbe([window, document.getElementById('child').contentWindow]);
  globalThis.__uiEventResults = result;
  return result.errors.length === 0 && result.rows.every(row => Object.values(row.checks).every(value => value === true));
})()
