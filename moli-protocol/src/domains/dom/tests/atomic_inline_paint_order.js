() => {
  const samples = [];
  for (const [expected, covered, pixel] of [
    ['floating-order', 'block-order', [0, 255, 0, 255]],
    ['later-order', 'atomic-under-order', [0, 255, 0, 255]],
    ['atomic-top-order', 'earlier-order', [0, 0, 255, 255]],
    ['bare-order', 'bare-atomic-order', null],
  ]) {
    let rect = document.getElementById(expected).getBoundingClientRect();
    if (expected === 'bare-order') {
      const range = document.createRange();
      range.selectNodeContents(document.getElementById(expected).lastChild);
      rect = range.getBoundingClientRect();
    }
    const under = document.getElementById(covered).getBoundingClientRect();
    for (const offset of [1, 10, 20]) {
      const x = Math.ceil(Math.max(rect.left, under.left)) + offset;
      const y = Math.ceil(Math.max(rect.top, under.top)) + offset;
      if (x >= Math.min(rect.right, under.right) || y >= Math.min(rect.bottom, under.bottom)) {
        throw new Error(`${expected} must overlap ${covered} at ${x},${y}`);
      }
      samples.push({
        expected, x, y,
        ...(offset === 1 && pixel ? { pixel } : {}),
        // The penetrating list ignores HTML text hits; a single hit retargets
        // that text to its parent instead. Chrome exposes this distinction.
        expectedElements: expected === 'bare-order' ? covered : expected,
        elementFromPoint: document.elementFromPoint(x, y)?.id,
        elementsFromPoint: document.elementsFromPoint(x, y)[0]?.id,
      });
    }
  }
  return samples;
}
