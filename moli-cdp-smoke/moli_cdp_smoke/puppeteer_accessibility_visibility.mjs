import assert from 'node:assert/strict';

function buttonNames(snapshot) {
  const names = [];
  const pending = [snapshot];
  while (pending.length) {
    const node = pending.pop();
    if (node.role === 'button') names.push(node.name ?? '');
    pending.push(...(node.children ?? []));
  }
  return names.sort();
}

export async function runPuppeteerAccessibilityVisibilitySmoke(page) {
  await page.goto('about:blank');
  await page.evaluate(() => {
    document.body.innerHTML = `
      <style id="visibility-sheet">.hidden-control { display:none }</style>
      <button id="visible">Visible action</button>
      <button hidden>Hidden action</button>
      <button class="hidden-control">Stylesheet hidden action</button>
      <button id="override" hidden style="display:block">CSS override action</button>
      <span id="hidden-label" style="display:none">Hidden root<span aria-hidden="true">hidden aria child</span></span>
      <button aria-labelledby="hidden-label"></button>
      <div style="visibility:hidden"><button>Invisible action</button>
        <button style="visibility:visible">Visibility override action</button></div>
      <div inert><button>Inert action</button></div>
      <div role="button" style="content-visibility:hidden">Hidden contents<button>CV hidden action</button></div>
      <div hidden="until-found"><button>Until found action</button></div>
      <div hidden="until-found" style="content-visibility:visible"><button>Until found override action</button></div>
      <div style="display:contents;content-visibility:hidden"><button>CV contents action</button></div>
      <button style="opacity:0">Transparent action</button>
      <div id="visibility-host" hidden><button slot="action" style="visibility:visible">Slotted visible action</button></div>
    `;
    const root = document.querySelector('#visibility-host').attachShadow({mode: 'closed'});
    root.innerHTML = `
      <style>:host { display:block } .hidden-control { display:none } slot { visibility:hidden }</style>
      <button class="hidden-control">Shadow hidden action</button>
      <slot name="action"><button>Assigned fallback action</button></slot>
      <button>Shadow visible action</button>
    `;
    globalThis.__visibilityRoot = root;
    globalThis.__visibilityClicks = [];
    for (const id of ['visible', 'override']) {
      document.getElementById(id).addEventListener('click',
        event => __visibilityClicks.push({id, trusted: event.isTrusted}));
    }
  });
  const expected = [
    '', 'CSS override action', 'CV contents action', 'Hidden root hidden aria child', 'Shadow visible action',
    'Slotted visible action', 'Transparent action', 'Until found override action',
    'Visibility override action', 'Visible action',
  ];
  const client = await page.createCDPSession();
  try {
    // Resolve an AX-external hidden label before any full AX snapshot. A local
    // request must observe its dependencies and discard them between commands.
    const remote = (await client.send('Runtime.evaluate', {
      expression: 'document.querySelector("button[aria-labelledby]")',
    })).result;
    assert.ok(remote.objectId);
    try {
      const partial = async () => (await client.send('Accessibility.getPartialAXTree', {
        objectId: remote.objectId, fetchRelatives: false,
      })).nodes[0];
      const first = await partial();
      assert.equal(first.ignored, false);
      assert.equal(first.name.value, 'Hidden root hidden aria child');
      await page.evaluate(() => {
        document.getElementById('hidden-label').firstChild.nodeValue = 'Updated root';
      });
      const updated = await partial();
      assert.equal(updated.name.value, 'Updated root hidden aria child');
      assert.equal(updated.nodeId, first.nodeId);
      assert.equal(updated.backendDOMNodeId, first.backendDOMNodeId);
      await page.evaluate(() => {
        document.getElementById('hidden-label').firstChild.nodeValue = 'Hidden root';
      });
    } finally {
      await client.send('Runtime.releaseObject', {objectId: remote.objectId});
    }
    for (const interestingOnly of [true, false]) {
      assert.deepEqual(buttonNames(await page.accessibility.snapshot({interestingOnly})), expected);
    }
    const initial = (await client.send('Accessibility.getFullAXTree')).nodes
      .find(node => !node.ignored && node.name?.value === 'Visible action');
    assert.ok(initial?.backendDOMNodeId);
    const override = await page.$('aria/CSS override action[role="button"]');
    assert.ok(override, 'CSS override of hidden must be selectable');
    try { await override.click(); } finally { await override.dispose(); }

    await page.evaluate(() => { document.getElementById('visible').style.display = 'none'; });
    assert.ok(!buttonNames(await page.accessibility.snapshot()).includes('Visible action'));
    assert.equal(await page.$('aria/Visible action[role="button"]'), null);
    const partial = await client.send('Accessibility.getPartialAXTree', {
      backendNodeId: initial.backendDOMNodeId, fetchRelatives: false,
    });
    assert.equal(partial.nodes[0].ignored, true);
    assert.equal(partial.nodes[0].backendDOMNodeId, initial.backendDOMNodeId);
    assert.ok(partial.nodes[0].ignoredReasons.some(reason => reason.name === 'notRendered'));

    await page.evaluate(() => { document.getElementById('visible').style.removeProperty('display'); });
    const restored = (await client.send('Accessibility.getFullAXTree')).nodes
      .find(node => !node.ignored && node.name?.value === 'Visible action');
    assert.equal(restored.backendDOMNodeId, initial.backendDOMNodeId);
    assert.equal(restored.nodeId, initial.nodeId);
    const visible = await page.$('aria/Visible action[role="button"]');
    assert.ok(visible);
    try { await visible.click(); } finally { await visible.dispose(); }
    assert.deepEqual(await page.evaluate(() => __visibilityClicks), [
      {id: 'override', trusted: true}, {id: 'visible', trusted: true},
    ]);

    await page.evaluate(() => {
      document.getElementById('visibility-sheet').sheet.cssRules[0].style.display = 'block';
      __visibilityRoot.querySelector('style').sheet.cssRules[1].style.display = 'block';
    });
    const changed = buttonNames(await page.accessibility.snapshot());
    assert.ok(changed.includes('Stylesheet hidden action'));
    assert.ok(changed.includes('Shadow hidden action'));
    assert.ok(!changed.includes('Hidden action'));
    return {trustedClicks: 2, stableReference: true, shadowMode: 'closed', cssomRuleChanges: 2, coldPartial: true};
  } finally {
    await client.detach();
  }
}
