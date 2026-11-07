async function(mode, cross, token, kind) {
  const frame = document.createElement('iframe');
  const loaded = new Promise(resolve => frame.onload = resolve);
  frame.src = '/child-import-late-page?mode=' + mode + '&cross=' + cross + '&token=' + token + '&kind=' + kind;
  document.body.append(frame);
  if (mode !== 'before' || !cross) {
    const key = encodeURIComponent(token + ':/child-import-csp-root.css');
    await fetch('/probe-started?token=' + key);
    if (mode === 'dynamic') {
      const doc = frame.contentDocument;
      const meta = doc.createElement('meta');
      meta.httpEquiv = 'Content-Security-Policy';
      meta.content = "style-src 'self'";
      doc.head.append(meta);
    }
    await fetch('/probe-release?token=' + key);
  }
  await loaded;
  const child = frame.contentWindow;
  const style = child.getComputedStyle(frame.contentDocument.documentElement);
  const result = {
    root: style.getPropertyValue('--csp-root').trim(),
    allowed: style.getPropertyValue('--csp-allowed').trim(),
    blocked: style.getPropertyValue('--csp-blocked').trim(),
    data: style.getPropertyValue('--csp-data').trim(),
    script: child.afterLateStylesheet === true,
    ready: frame.contentDocument.readyState,
    violations: child.lateViolations
  };
  frame.remove();
  return result;
}
