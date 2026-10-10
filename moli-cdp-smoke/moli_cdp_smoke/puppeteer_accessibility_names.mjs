import assert from 'node:assert/strict';

const cases = [
  ['copy', '<svg role="img" aria-label="Copy"><use href="#clipboard"></use></svg>', 'Copy'],
  ['span', '<span aria-label="Copy">visual text</span>', 'Copy'],
  ['mixed', 'Save<span aria-label="document"></span>now', 'Save document now'],
  ['inline', 'foo<span>bar</span>baz', 'foobarbaz'],
  ['inline-space', 'foo<span> </span>bar', 'foo bar'],
  ['image', '<img alt="Copy">', 'Copy'],
  ['labelledby', '<span aria-labelledby="external" aria-label="Wrong">visual</span>', 'External'],
  ['empty', '<span aria-label="">Fallback</span>', 'Fallback'],
  ['presentational', '<span role="none" aria-label="Presentation">Fallback</span>', 'Presentation'],
  ['hidden', 'Start<span style="display:none" aria-label="Secret">Visual</span>End', 'StartEnd'],
  ['visibility', 'Start<span style="visibility:hidden" aria-label="Secret"><span style="visibility:visible" aria-label="Visible"></span></span>End', 'Start Visible End'],
  ['inert', 'Start<span inert aria-label="Secret">Visual</span>End', 'StartEnd'],
  ['cv', 'Start<span style="content-visibility:hidden" aria-label="CV">Visual</span>End', 'Start CV End'],
  ['cv-block', 'Before<span style="content-visibility:hidden;display:block">CV hidden</span>After', 'Before After'],
  ['ariahidden', 'Start<span aria-hidden="true" aria-label="Secret">Visual</span>End', 'StartEnd'],
  ['child-self', '<span id="self" aria-labelledby="self" aria-label="Copy">visual</span>', 'Copy'],
  ['child-parent', 'Prefix<span aria-labelledby="child-parent" aria-label="Fallback">visual</span>Suffix', 'Prefix Prefix Fallback Suffix'],
  ['reference', '', 'Inner', 'aria-labelledby="outer"'],
  ['explicit', '<svg aria-label="Copy"></svg>', 'Primary', 'aria-label="Primary"'],
  ['shared', '<span aria-labelledby="external"></span><span aria-labelledby="external"></span>', 'External External'],
  ['title-empty', '<span title="Tooltip"></span>', ''],
  ['title-content', '<span title="Tooltip">Contents</span>', 'Contents'],
  ['title-whitespace', '   ', 'Tooltip', 'title="Tooltip"'],
  ['title-wrapper-whitespace', '<span> </span>', 'Tooltip', 'title="Tooltip"'],
  ['title-empty-block', '<div></div>', 'Tooltip', 'title="Tooltip"'],
  ['reference-title', '', 'Tooltip', 'aria-labelledby="tooltip"'],
  ['reference-title-descendant', '', 'Tooltip child', 'aria-labelledby="tooltip-parent"'],
];

function buttonNames(snapshot) {
  const names = [];
  const pending = snapshot ? [snapshot] : [];
  while (pending.length) {
    const node = pending.pop();
    if (node.role === 'button') names.push(node.name ?? '');
    pending.push(...(node.children ?? []));
  }
  return names.sort();
}

export async function runPuppeteerAccessibilityNamesSmoke(page) {
  await page.goto('about:blank');
  await page.evaluate(cases => {
    document.body.innerHTML = `
      <span id="external" hidden>External</span><span id="end">End</span>
      <span id="outer" hidden><span aria-labelledby="end" aria-label="Inner">visual</span></span>
      <span id="tooltip" hidden title="Tooltip"></span>
      <span id="tooltip-parent" hidden><span title="Tooltip child"></span></span>
      ${cases.map(([id, html, , attrs = '']) => `<button id="${id}" ${attrs}>${html}</button>`).join('')}
      <div id="name-host" role="button"><span slot="action" aria-label="Slotted">visual</span></div>
    `;
    const shadow = document.getElementById('name-host').attachShadow({mode: 'closed'});
    shadow.innerHTML = '<svg role="img" aria-label="Shadow Copy"></svg><slot name="action" aria-label="Wrong slot"></slot>';
    globalThis.__nameClicks = [];
    for (const id of ['copy', 'span']) {
      document.getElementById(id).addEventListener('click', event => {
        __nameClicks.push({id, trusted: event.isTrusted});
      });
    }
  }, cases);

  const client = await page.createCDPSession();
  try {
    // Start with a local request: a full snapshot must not warm name inputs.
    const copy = await page.$('#copy');
    assert.ok(copy);
    const backend = await copy.backendNodeId();
    const partial = async () => (await client.send('Accessibility.getPartialAXTree', {
      backendNodeId: backend, fetchRelatives: false,
    })).nodes[0];
    const first = await partial();
    assert.equal(first.ignored, false);
    assert.equal(first.name?.value, 'Copy', 'The parent button must use its icon\'s text alternative');

    const expected = [...cases.map(([, , name]) => name), 'Shadow Copy Slotted'].sort();
    for (const interestingOnly of [true, false]) {
      assert.deepEqual(buttonNames(await page.accessibility.snapshot({interestingOnly})), expected);
    }
    for (const id of ['copy', 'span']) {
      const target = await page.$('#' + id);
      const expectedBackend = await target.backendNodeId();
      await target.dispose();
      const handles = await page.$$('aria/Copy[role="button"]');
      try {
        const matches = [];
        for (const handle of handles) if (await handle.backendNodeId() === expectedBackend) matches.push(handle);
        assert.equal(matches.length, 1, 'ARIA must resolve the parent button snapshot ref');
        await matches[0].click();
      } finally { await Promise.all(handles.map(handle => handle.dispose())); }
    }
    assert.deepEqual(await page.evaluate(() => __nameClicks), [
      {id: 'copy', trusted: true}, {id: 'span', trusted: true},
    ]);

    await page.evaluate(() => document.querySelector('#copy svg').setAttribute('aria-label', 'Copied'));
    const updated = await partial();
    assert.equal(updated.name.value, 'Copied');
    assert.equal(updated.nodeId, first.nodeId);
    assert.equal(updated.backendDOMNodeId, backend);
    const renamed = await page.$('aria/Copied[role="button"]');
    assert.ok(renamed);
    try { assert.equal(await renamed.backendNodeId(), backend); } finally { await renamed.dispose(); }
    const old = await page.$$('aria/Copy[role="button"]');
    try {
      for (const handle of old) assert.notEqual(await handle.backendNodeId(), backend);
    } finally { await Promise.all(old.map(handle => handle.dispose())); }

    await page.evaluate(() => { document.querySelector('#copy svg').style.display = 'none'; });
    const hidden = await partial();
    assert.equal(hidden.ignored, false, 'Hiding the icon must retain the parent button');
    assert.equal(hidden.name?.value ?? '', '');
    assert.equal(await page.$('aria/Copied[role="button"]'), null);
    await page.evaluate(() => { document.querySelector('#copy svg').style.removeProperty('display'); });
    assert.equal((await partial()).name.value, 'Copied');
    await copy.dispose();
    return {cases: cases.length + 1, trustedClicks: 2, stableReference: true, coldPartial: true};
  } finally { await client.detach(); }
}
