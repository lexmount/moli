export async function runPuppeteerAriaQuerySmoke(page) {
  await page.evaluate(async () => {
    document.body.innerHTML = `
      <main id="aria-root"><button id="aria-go">Go</button></main>
      <button id="aria-stay">Stay</button>
    `;
    globalThis.__puppeteerAriaClicks = 0;
    document.querySelector('#aria-go').addEventListener('click', () => {
      globalThis.__puppeteerAriaClicks += 1;
    });
    const iframe = document.createElement('iframe');
    iframe.id = 'aria-frame';
    iframe.srcdoc = '<button id="aria-child">Child Go</button>';
    await new Promise(resolve => {
      iframe.addEventListener('load', resolve, { once: true });
      document.body.append(iframe);
    });
  });

  for (const selector of ['aria/Go', 'aria/Go[role="button"]']) {
    const handle = await page.$(selector);
    if (!handle) {
      throw new Error(`Puppeteer ARIA selector did not match: ${selector}`);
    }
    try {
      const id = await handle.evaluate(element => element.id);
      if (id !== 'aria-go') {
        throw new Error(`Puppeteer ARIA selector returned ${id}: ${selector}`);
      }
    } finally {
      await handle.dispose();
    }
  }

  const buttons = await page.$$('aria/[role="button"]');
  try {
    const ids = (await Promise.all(buttons.map(handle => handle.evaluate(element => element.id)))).sort();
    if (JSON.stringify(ids) !== JSON.stringify(['aria-go', 'aria-stay'])) {
      throw new Error(`Puppeteer ARIA role query returned unexpected elements: ${JSON.stringify(ids)}`);
    }
  } finally {
    await Promise.all(buttons.map(handle => handle.dispose()));
  }

  if (await page.$('aria/Missing')) {
    throw new Error('Puppeteer ARIA query must return null for a missing name');
  }

  const root = await page.$('#aria-root');
  if (!root) {
    throw new Error('Puppeteer ARIA query fixture root is missing');
  }
  try {
    if (await root.$('aria/Stay')) {
      throw new Error('Puppeteer ARIA query escaped its element subtree');
    }
    const go = await root.waitForSelector('aria/Go[role="button"]', { timeout: 3000 });
    if (!go) {
      throw new Error('Puppeteer scoped ARIA wait returned no element');
    }
    try {
      await go.evaluate(element => element.click());
    } finally {
      await go.dispose();
    }
  } finally {
    await root.dispose();
  }
  const clicks = await page.evaluate(() => globalThis.__puppeteerAriaClicks);
  if (clicks !== 1) {
    throw new Error(`Puppeteer ARIA handle must activate the matched element once: ${clicks}`);
  }

  const iframe = await page.$('#aria-frame');
  if (!iframe) {
    throw new Error('Puppeteer ARIA query fixture iframe is missing');
  }
  try {
    const frame = await iframe.contentFrame();
    const child = await frame?.$('aria/Child Go[role="button"]');
    if (!child) {
      throw new Error('Puppeteer ARIA selector did not match in the child frame');
    }
    try {
      const id = await child.evaluate(element => element.id);
      if (id !== 'aria-child') {
        throw new Error(`Puppeteer child-frame ARIA selector returned ${id}`);
      }
    } finally {
      await child.dispose();
    }
  } finally {
    await iframe.dispose();
  }

  return { selectors: ['name', 'name-and-role', 'role', 'missing', 'scoped-wait', 'iframe'] };
}
