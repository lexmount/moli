(() => {
  const child = document.getElementById('child').contentWindow;
  const result = globalThis.__errorEventProbe([window, child]);
  globalThis.__uiEventResults = result;
  return result.errors.length === 0 && result.rows.every(row => Object.values(row.checks).every(value => value === true));
})()
