async function(mode, token) {
  const frame = document.createElement('iframe');
  const loaded = new Promise(resolve => frame.onload = resolve);
  frame.src = '/child-import-probe-policy?mode=' + mode + '&token=' + token;
  document.body.append(frame);
  if (mode === 'late') {
    await Promise.all(['branch', 'real'].map(name => fetch('/probe-started?token=' +
      encodeURIComponent(token + ':/child-import-probe-' + name + '.css'))));
    const doc = frame.contentDocument;
    const meta = doc.createElement('meta');
    meta.httpEquiv = 'Content-Security-Policy';
    meta.content = "style-src 'self'";
    doc.head.append(meta);
    await fetch('/probe-release?token=' + encodeURIComponent(token + ':/child-import-probe-branch.css'));
  }
  await loaded;
  const child = frame.contentWindow;
  const violation = await child.firstViolation;
  const style = child.getComputedStyle(frame.contentDocument.documentElement);
  const result = {
    first: style.getPropertyValue('--redirect-first').trim(),
    style: style.getPropertyValue('--nested-redirect').trim(),
    script: child.afterSiblingStylesheet === true,
    violation
  };
  frame.remove();
  return result;
}
