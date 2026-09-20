async function(expectedURL) {
  if (location.href !== new URL(expectedURL, location.href).href) {
    throw new Error('crashtest document changed before readiness check');
  }
  // Follow wptrunner's test-wait.js: load, fonts, two animation frames,
  // then test-wait removal. TestRendered is dispatched once after observing.
  const root = document.documentElement;
  if (document.readyState !== 'complete') {
    await new Promise(resolve => addEventListener('load', resolve, {once: true}));
  }
  if (Object.prototype.hasOwnProperty.call(Document.prototype, 'fonts')) {
    await document.fonts.ready;
  }
  await new Promise(resolve => {
    let observer = null;
    const paints = () => requestAnimationFrame(() => requestAnimationFrame(ready));
    const ready = () => {
      if (root && root.classList.contains('test-wait')) {
        if (observer === null) {
          observer = new MutationObserver(paints);
          observer.observe(root, {attributes: true});
          root.dispatchEvent(new Event('TestRendered', {bubbles: true}));
        }
        return;
      }
      if (observer !== null) observer.disconnect();
      resolve();
    };
    paints();
  });
  return {complete: true};
}
