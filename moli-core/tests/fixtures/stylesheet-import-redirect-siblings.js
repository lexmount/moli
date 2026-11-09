async function(owner, kind, token) {
  const href = '/child-import-probe-root.css?token=' + token;
  let frame;
  let element;
  let target = window;
  if (owner === 'child') {
    frame = document.createElement('iframe');
    const loaded = new Promise(resolve => frame.onload = resolve);
    const style = kind === 'linked' ? `<link rel="stylesheet" href="${href}">`
      : `<style>@import url('${href}');</style>`;
    frame.srcdoc = `<!doctype html>${style}<script>globalThis.afterSiblingStylesheet=true;</script><body>Sibling aliases`;
    document.body.append(frame);
    await loaded;
    target = frame.contentWindow;
    element = frame.contentDocument.querySelector('link,style');
  } else {
    element = document.createElement(kind === 'linked' ? 'link' : 'style');
    const loaded = new Promise(resolve => element.onload = resolve);
    if (kind === 'linked') {
      element.rel = 'stylesheet';
      element.href = href;
    } else {
      element.textContent = `@import url('${href}');`;
    }
    document.head.append(element);
    await loaded;
  }
  const sheet = kind === 'linked' ? element.sheet : element.sheet.cssRules[0].styleSheet;
  const firstRule = sheet.cssRules[0];
  const branch = sheet.cssRules[2].styleSheet;
  const lastRule = token.includes('-shared-') ? branch?.cssRules[0]?.styleSheet?.cssRules[0]
    : branch?.cssRules[0];
  const snapshot = rule => {
    const child = rule?.styleSheet;
    try {
      let cycle = child;
      let depth = 0;
      while (cycle?.cssRules[0]?.type === 3 && depth < 4) {
        cycle = cycle.cssRules[0].styleSheet;
        depth++;
      }
      return {
        text: child ? Array.from(child.cssRules).find(rule => rule.type === 1)?.cssText ?? null : null,
        cycleRules: cycle?.cssRules.length ?? 0,
        depth,
        owner: !!child && child.ownerRule === rule && child.parentStyleSheet === rule.parentStyleSheet
      };
    } catch (error) {
      return {error: error.name};
    }
  };
  const result = {
    style: target.getComputedStyle(target.document.documentElement).getPropertyValue('--nested-redirect').trim(),
    first: snapshot(firstRule), last: snapshot(lastRule),
    distinct: !!lastRule?.styleSheet && firstRule.styleSheet !== lastRule.styleSheet,
    script: owner === 'main' || target.afterSiblingStylesheet === true
  };
  (frame ?? element).remove();
  return result;
}
