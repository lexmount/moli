import assert from 'node:assert/strict';

function buttonNames(snapshot) {
  assert.ok(snapshot, 'Puppeteer accessibility snapshot must have a root');
  const names = [];
  const pending = [snapshot];
  while (pending.length) {
    const node = pending.pop();
    if (node.role === 'button') {
      names.push(node.name);
    }
    pending.push(...(node.children ?? []));
  }
  return names.sort();
}

function buttonNode(nodes, name) {
  const matches = nodes.filter(node => !node.ignored
    && node.role?.value === 'button' && node.name?.value === name);
  assert.equal(matches.length, 1, `AX tree must contain exactly one button named ${name}`);
  assert.ok(matches[0].backendDOMNodeId, `${name} must have a backend DOM ref`);
  return matches[0];
}

export async function runPuppeteerShadowAccessibilitySmoke(page) {
  await page.evaluate(() => {
    document.body.innerHTML = `
      <div id="shadow-open"><button id="shadow-slotted" slot="action">Slotted action</button><button>Unassigned action</button></div>
      <div id="shadow-closed"></div>
      <div id="shadow-widget" role="button"></div>
      <button>Outside action</button>
    `;
    const open = document.querySelector('#shadow-open').attachShadow({ mode: 'open' });
    open.innerHTML = `
      <button id="shadow-open-button">Open action</button>
      <slot name="action"><button id="shadow-assigned-fallback">Assigned fallback action</button></slot>
      <slot name="empty"><button>Empty fallback action</button></slot>
      <div id="shadow-nested"></div>
    `;
    const nested = open.querySelector('#shadow-nested').attachShadow({ mode: 'closed' });
    nested.innerHTML = '<button>Nested action</button>';
    const closed = document.querySelector('#shadow-closed').attachShadow({ mode: 'closed' });
    closed.innerHTML = '<button id="shadow-closed-button">Closed action</button>';
    document.querySelector('#shadow-widget').attachShadow({ mode: 'open' })
      .innerHTML = '<span>Shadow host name</span>';
    globalThis.__puppeteerShadowRoots = { open, closed };
    globalThis.__puppeteerShadowClicks = [];
    for (const button of [
      open.querySelector('#shadow-open-button'),
      closed.querySelector('#shadow-closed-button'),
      document.querySelector('#shadow-slotted'),
    ]) {
      button.addEventListener('click', event => {
        __puppeteerShadowClicks.push({ id: button.id, trusted: event.isTrusted });
        button.textContent = button.textContent.replace(' action', ' clicked');
      });
    }
  });

  const expectedNames = [
    'Closed action', 'Empty fallback action', 'Nested action', 'Open action',
    'Outside action', 'Shadow host name', 'Slotted action',
  ];
  assert.deepEqual(buttonNames(await page.accessibility.snapshot()), expectedNames);
  assert.deepEqual(
    buttonNames(await page.accessibility.snapshot({ interestingOnly: false })),
    expectedNames,
  );

  const client = await page.createCDPSession();
  try {
    const { nodes: initialNodes } = await client.send('Accessibility.getFullAXTree');
    await client.send('DOM.getDocument', { depth: -1, pierce: true });
    for (const [mode, names] of [
      ['open', ['Empty fallback action', 'Nested action', 'Open action', 'Slotted action']],
      ['closed', ['Closed action']],
    ]) {
      const evaluated = await client.send('Runtime.evaluate', {
        expression: `__puppeteerShadowRoots.${mode}`,
      });
      const objectId = evaluated.result.objectId;
      assert.ok(objectId, `${mode} ShadowRoot must expose a CDP object`);
      try {
        const { nodeId } = await client.send('DOM.requestNode', { objectId });
        const { node } = await client.send('DOM.describeNode', { nodeId });
        assert.equal(node.shadowRootType, mode);
        const expectedRefs = names.map(name => buttonNode(initialNodes, name).backendDOMNodeId).sort();
        for (const reference of [{ objectId }, { nodeId }, { backendNodeId: node.backendNodeId }]) {
          const { nodes } = await client.send('Accessibility.queryAXTree', {
            ...reference, role: 'button',
          });
          assert.deepEqual(nodes.map(node => node.name?.value).sort(), names,
            `${mode} ShadowRoot query must stay within its host subtree`);
          assert.deepEqual(nodes.map(node => node.backendDOMNodeId).sort(), expectedRefs,
            `${mode} ShadowRoot query must reuse full-snapshot refs`);
        }
      } finally {
        await client.send('Runtime.releaseObject', { objectId });
      }
    }

    const actions = [
      ['Open action', 'Open clicked', 'shadow-open-button'],
      ['Closed action', 'Closed clicked', 'shadow-closed-button'],
      ['Slotted action', 'Slotted clicked', 'shadow-slotted'],
    ];
    for (const [name, , id] of actions) {
      const snapshotNode = buttonNode(initialNodes, name);
      const { object } = await client.send('DOM.resolveNode', {
        backendNodeId: snapshotNode.backendDOMNodeId,
      });
      assert.ok(object.objectId, `${name} snapshot ref must resolve to a DOM object`);
      try {
        const resolved = await client.send('Runtime.callFunctionOn', {
          objectId: object.objectId,
          functionDeclaration: 'function() { return this.id; }',
          returnByValue: true,
        });
        assert.equal(resolved.result.value, id);
      } finally {
        await client.send('Runtime.releaseObject', { objectId: object.objectId });
      }

      const handle = await page.$(`aria/${name}[role="button"]`);
      assert.ok(handle, `Puppeteer ARIA selector must match ${name}`);
      try {
        assert.equal(await handle.backendNodeId(), snapshotNode.backendDOMNodeId);
        assert.equal(await handle.evaluate(element => element.id), id);
        await handle.click();
      } finally {
        await handle.dispose();
      }
    }
    assert.deepEqual(await page.evaluate(() => __puppeteerShadowClicks),
      actions.map(([, , id]) => ({ id, trusted: true })));

    const { nodes: updatedNodes } = await client.send('Accessibility.getFullAXTree');
    for (const [oldName, newName] of actions) {
      const before = buttonNode(initialNodes, oldName);
      const after = buttonNode(updatedNodes, newName);
      assert.equal(after.nodeId, before.nodeId, 'Name changes must preserve the AX ref');
      assert.equal(after.backendDOMNodeId, before.backendDOMNodeId);
      const stale = await page.$(`aria/${oldName}[role="button"]`);
      try {
        assert.equal(stale, null, `ARIA queries must observe the updated name of ${oldName}`);
      } finally {
        await stale?.dispose();
      }
    }
    const updatedNames = expectedNames.map(name => actions.find(action => action[0] === name)?.[1] ?? name);
    assert.deepEqual(buttonNames(await page.accessibility.snapshot()), updatedNames.sort());

    await page.evaluate(() => { document.querySelector('#shadow-slotted').slot = 'unmatched'; });
    const fallback = await page.$('aria/Assigned fallback action[role="button"]');
    assert.ok(fallback, 'Removing the slot assignment must expose fallback content');
    try {
      assert.equal(await fallback.evaluate(element => element.id), 'shadow-assigned-fallback');
    } finally {
      await fallback.dispose();
    }
    const withoutAssignment = updatedNames.filter(name => name !== 'Slotted clicked');
    withoutAssignment.push('Assigned fallback action');
    assert.deepEqual(buttonNames(await page.accessibility.snapshot()), withoutAssignment.sort());

    await page.evaluate(() => { document.querySelector('#shadow-slotted').slot = 'action'; });
    assert.deepEqual(buttonNames(await page.accessibility.snapshot()), updatedNames);
    const reassigned = await page.$('aria/Slotted clicked[role="button"]');
    assert.ok(reassigned, 'Restoring the slot assignment must expose the assigned control');
    try {
      assert.equal(await reassigned.backendNodeId(), buttonNode(initialNodes, 'Slotted action').backendDOMNodeId);
    } finally {
      await reassigned.dispose();
    }

    return {
      shadowModes: ['open', 'closed', 'nested-closed'],
      queryReferences: ['objectId', 'nodeId', 'backendNodeId'],
      trustedClicks: actions.length,
      slotAssignmentChanges: 2,
      hostName: 'Shadow host name',
    };
  } finally {
    await client.detach();
  }
}
