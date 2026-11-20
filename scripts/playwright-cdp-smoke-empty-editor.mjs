export async function checkEmptyEditorFill(context, fixture) {
  const page = await context.newPage();
  page.setDefaultTimeout(3_000);
  const assertEqual = (actual, expected, label) => {
    if (actual !== expected) throw new Error(`${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  };
  try {
    await page.goto(`${fixture}/plain`);
    await page.setContent('<div id="editor" contenteditable style="font:16px/24px Arial;white-space:pre-wrap"></div>');
    const editor = page.locator('#editor');
    assertEqual(await editor.isVisible(), true, 'empty editor visible before focus');
    assertEqual((await editor.boundingBox()).height, 24, 'empty editor one-line box');
    await editor.fill('typed');
    assertEqual(await editor.textContent(), 'typed', 'fill empty editor');
    await editor.fill('');
    assertEqual((await editor.boundingBox()).height, 24, 'cleared editor keeps line');
    await editor.fill('first\nsecond');
    // Publish the changed rendered text before reading innerText.
    await page.screenshot();
    assertEqual(await editor.innerText(), 'first\nsecond', 'fill editor with new lines');
    await editor.evaluate(element => {
      const range = document.createRange();
      range.selectNodeContents(element);
      const selection = window.getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
    });
    await editor.fill('replacement');
    assertEqual(await editor.textContent(), 'replacement', 'fill replaces existing selection once');
    await editor.fill('');
    await editor.evaluate(element => element.addEventListener('beforeinput', event => event.preventDefault(), { once: true }));
    await editor.fill('canceled');
    assertEqual(await editor.textContent(), '', 'canceled beforeinput does not insert');
    await editor.fill('accepted');
    assertEqual(await editor.textContent(), 'accepted', 'subsequent fill inserts once');

    await page.setContent(`<div id="editor" contenteditable></div><iframe src="${fixture}/child"></iframe>`);
    await page.frameLocator('iframe').locator('body').evaluate(body => {
      const input = document.createElement('input');
      body.append(input);
      input.focus();
    });
    await page.locator('#editor').fill('parent editor');
    assertEqual(await page.locator('#editor').textContent(), 'parent editor', 'fill while another frame has focus');

    await page.setContent('<body contenteditable></body>');
    await page.locator('body').fill('body editor');
    assertEqual(await page.locator('body').textContent(), 'body editor', 'fill empty editable body');
  } finally {
    await page.close();
  }
}
