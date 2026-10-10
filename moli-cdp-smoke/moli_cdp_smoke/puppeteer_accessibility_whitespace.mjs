import assert from 'node:assert/strict';

const cases = [
  {id: 'link', html: '<nav aria-label="Main menu"><a id="target" href="#a">A&nbsp;link</a></nav>', role: 'link', name: 'A\u00a0link', rejects: ['A link']},
  {id: 'nbsp-repeat', html: '<button id="target">A&nbsp;&nbsp;B</button>', name: 'A\u00a0\u00a0B', rejects: ['A B', 'A\u00a0B']},
  {id: 'nbsp-edges', html: '<button id="target">&nbsp;A&nbsp;</button>', name: '\u00a0A\u00a0', rejects: ['A', ' A ']},
  {id: 'nbsp-only', html: '<button id="target" title="Tooltip">&nbsp;</button>', name: '\u00a0', rejects: ['', 'Tooltip']},
  {id: 'ascii-only', html: '<button id="target" title="Tooltip"> \t\n\r\f </button>', name: 'Tooltip', rejects: ['']},
  {id: 'ascii-text', html: '<button id="target">A \t\n\r\f B</button>', name: 'A B', rejects: ['A  B', 'A\tB']},
  {id: 'ascii-label', html: '<button id="target" aria-label="A&#9;&#10;&#13;&#12;B">Fallback</button>', name: 'A B', rejects: ['A\tB']},
  {id: 'ascii-aria-fallback', html: '<button id="target" aria-label=" &#9;&#10;&#13;&#12; ">Fallback</button>', name: 'Fallback', rejects: ['']},
  {id: 'mixed', html: '<button id="target">A&nbsp; \t B</button>', name: 'A\u00a0 B', rejects: ['A B', 'A\u00a0B']},
  {id: 'aria-label', html: '<button id="target" aria-label="A&nbsp;B">Fallback</button>', name: 'A\u00a0B', rejects: ['A B']},
  {id: 'aria-only', html: '<button id="target" aria-label="&nbsp;">Fallback</button>', name: '\u00a0', rejects: ['', 'Fallback']},
  {id: 'aria-edges', html: '<button id="target" aria-label="&nbsp;A&nbsp;">Fallback</button>', name: '\u00a0A\u00a0', rejects: ['A']},
  {id: 'reference', html: '<span id="label">A&nbsp;B</span><button id="target" aria-labelledby="label">Fallback</button>', name: 'A\u00a0B', rejects: ['A B']},
  {id: 'hidden-reference', html: '<span id="label" hidden>A&nbsp;B</span><button id="target" aria-labelledby="label">Fallback</button>', name: 'A\u00a0B', rejects: ['A B']},
  {id: 'native-label', html: '<label for="target">A&nbsp;B</label><input id="target">', role: 'textbox', name: 'A\u00a0B', rejects: ['A B']},
  {id: 'alt', html: '<button id="target"><img alt="A&nbsp;B"></button>', name: 'A\u00a0B', rejects: ['A B']},
  {id: 'title', html: '<button id="target" title="A&nbsp;B"></button>', name: 'A\u00a0B', rejects: ['A B']},
  {id: 'placeholder', html: '<input id="target" placeholder="A&nbsp;B">', role: 'textbox', name: 'A\u00a0B', rejects: ['A B']},
  {id: 'value', html: '<input id="target" type="button" value="A&nbsp;B">', name: 'A\u00a0B', rejects: ['A B']},
  {id: 'descendant', html: '<button id="target"><span aria-label="A&nbsp;B"></span></button>', name: 'A\u00a0B', rejects: ['A B']},
  {id: 'inline', html: '<button id="target">A<span>&nbsp;</span>B</button>', name: 'A\u00a0B', rejects: ['A B', 'AB']},
  {id: 'inline-only', html: '<button id="target" title="Tooltip"><span>&nbsp;</span></button>', name: '\u00a0', rejects: ['', 'Tooltip']},
  {id: 'shadow', html: '<div id="target" role="button"></div>', shadow: 'A&nbsp;B', name: 'A\u00a0B', rejects: ['A B']},
  {id: 'slot', html: '<div id="target" role="button"><span slot="label">A&nbsp;B</span></div>', shadow: '<slot name="label"></slot>', name: 'A\u00a0B', rejects: ['A B']},
  ...['\u0085', '\u2002', '\u2003', '\u2009', '\u202f', '\u3000', '\u000b'].map(space => ({
    id: 'unicode-' + space.codePointAt(0).toString(16),
    html: `<button id="target" aria-label="A${space}B"></button>`, name: `A${space}B`, rejects: ['A B'],
  })),
];

async function assertAriaMatch(page, name, role, backend, matches) {
  const selectors = [`aria/[name="${name}"][role="${role}"]`];
  // Keep the original bare aria/A link reproduction alongside role filters.
  if (role === 'link') selectors.push(`aria/${name}`);
  for (const selector of selectors) {
    const handle = await page.$(selector);
    try {
      if (matches) {
        assert.ok(handle, `Missing ARIA match for ${JSON.stringify(selector)}`);
        assert.equal(await handle.backendNodeId(), backend);
      } else {
        assert.equal(handle, null, `Unexpected ARIA match for ${JSON.stringify(selector)}`);
      }
    } finally { await handle?.dispose(); }
  }
}

export async function runPuppeteerAccessibilityWhitespaceSmoke(page) {
  await page.goto('about:blank');
  const client = await page.createCDPSession();
  try {
    for (const fixture of cases) {
      await page.evaluate(({html, shadow}) => {
        document.body.innerHTML = html;
        if (shadow) document.getElementById('target').attachShadow({mode: 'closed'}).innerHTML = shadow;
      }, fixture);
      const target = await page.$('#target');
      assert.ok(target, fixture.id);
      try {
        const backend = await target.backendNodeId();
        const role = fixture.role ?? 'button';
        // A local name request must work before any full snapshot warms AX.
        const partial = await client.send('Accessibility.getPartialAXTree', {
          backendNodeId: backend, fetchRelatives: false,
        });
        const node = partial.nodes.find(node => node.backendDOMNodeId === backend);
        assert.equal(node?.name?.value, fixture.name, fixture.id);
        assert.equal(node?.role?.value, role, fixture.id);
        assert.equal(node?.ignored, false, fixture.id);
        const full = await client.send('Accessibility.getFullAXTree');
        assert.equal(full.nodes.find(node => node.backendDOMNodeId === backend)?.name?.value,
          fixture.name, fixture.id);
        for (const interestingOnly of [true, false]) {
          const snapshot = await page.accessibility.snapshot({root: target, interestingOnly});
          assert.equal(snapshot?.name, fixture.name, fixture.id);
        }
        await assertAriaMatch(page, fixture.name, role, backend, true);
        for (const rejected of fixture.rejects) {
          await assertAriaMatch(page, rejected, role, backend, false);
        }
      } finally { await target.dispose(); }
    }

    // Keep both names present to catch overmatching and preserve DOM refs.
    await page.evaluate(() => {
      document.body.innerHTML = '<nav aria-label="Main menu"><a id="target" href="#a">A&nbsp;link</a><a id="plain" href="#b">A link</a></nav>';
    });
    const target = await page.$('#target');
    const plain = await page.$('#plain');
    assert.ok(target);
    assert.ok(plain);
    const {result} = await client.send('Runtime.evaluate', {expression: 'document'});
    const objectId = result.objectId;
    assert.ok(objectId);
    try {
      const backend = await target.backendNodeId();
      const plainBackend = await plain.backendNodeId();
      const {root} = await client.send('DOM.getDocument');
      for (const reference of [
        {nodeId: root.nodeId}, {backendNodeId: root.backendNodeId},
        {objectId},
      ]) {
        for (const [accessibleName, expected] of [
          ['A\u00a0link', [backend]], ['A link', [plainBackend]], ['A  link', []],
        ]) {
          const {nodes} = await client.send('Accessibility.queryAXTree', {
            ...reference, accessibleName, role: 'link',
          });
          assert.deepEqual(nodes.map(node => node.backendDOMNodeId), expected);
        }
      }
      await plain.evaluate(element => element.remove());
      let initialNodeId;
      for (const name of ['A\u00a0link', 'A link', 'A\u00a0link']) {
        await target.evaluate((element, name) => { element.textContent = name; }, name);
        const {nodes} = await client.send('Accessibility.getPartialAXTree', {
          backendNodeId: backend, fetchRelatives: false,
        });
        const node = nodes.find(node => node.backendDOMNodeId === backend);
        assert.equal(node?.name?.value, name);
        initialNodeId ??= node.nodeId;
        assert.equal(node.nodeId, initialNodeId);
        await assertAriaMatch(page, name, 'link', backend, true);
        await assertAriaMatch(page, name === 'A link' ? 'A\u00a0link' : 'A link', 'link', backend, false);
      }
    } finally {
      await client.send('Runtime.releaseObject', {objectId});
      await plain.dispose();
      await target.dispose();
    }
    return {cases: cases.length, queryReferences: 3, mutations: 3, stableReference: true};
  } finally { await client.detach(); }
}
