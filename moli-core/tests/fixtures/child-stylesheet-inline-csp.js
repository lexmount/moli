async function() {
  const parentViolations = [];
  document.addEventListener('securitypolicyviolation', event => parentViolations.push(event.blockedURI));
  const meta = document.createElement('meta');
  meta.httpEquiv = 'Content-Security-Policy';
  meta.content = "style-src 'none'";
  document.head.append(meta);
  const rows = [];
  for (const [mode, nonce, imports] of [
    ['header', 'missing', true], ['meta', 'missing', true],
    ['header', 'wrong', true], ['header', 'allowed', true],
    ['meta', 'allowed', true], ['report', 'missing', true],
    ['header', 'missing', false], ['header', 'allowed', false]
  ]) {
    const frame = document.createElement('iframe');
    const loaded = new Promise(resolve => frame.onload = resolve);
    const token = 'inline-' + mode + '-' + nonce + '-' + imports;
    frame.src = '/child-import-inline-policy?mode=' + mode + '&nonce=' + nonce + '&imports=' + imports + '&token=' + token;
    document.body.append(frame);
    await loaded;
    const child = frame.contentWindow;
    const doc = frame.contentDocument;
    const style = child.getComputedStyle(doc.documentElement);
    rows.push({mode, nonce, imports, token,
      local:style.getPropertyValue('--inline-policy').trim(),
      imported:style.getPropertyValue('--inline-import').trim(),
      sheet:doc.querySelector('#policy-style').sheet !== null, sheetCount:doc.styleSheets.length,
      script:child.afterInlineStylesheet === true, ready:doc.readyState, violations:child.inlineViolations});
    frame.remove();
  }
  return {rows, parentViolations};
}
