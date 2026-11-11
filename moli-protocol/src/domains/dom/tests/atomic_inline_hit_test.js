() => {
  const samples = [];
  const boxes = ['input', 'atomic', 'inner', 'outer-atomic', 'inner-atomic', 'nested-inner'];
  for (const id of boxes) {
    const rect = document.getElementById(id).getBoundingClientRect();
    if (rect.width <= 4 || rect.height <= 4) {
      throw new Error(`${id} has no usable hit-test area`);
    }
    const xs = [Math.ceil(rect.left) + 1, Math.round((rect.left + rect.right) / 2), Math.floor(rect.right) - 1];
    const ys = [Math.ceil(rect.top) + 1, Math.round((rect.top + rect.bottom) / 2), Math.floor(rect.bottom) - 1];
    for (let row = 0; row < ys.length; ++row) {
      for (let column = 0; column < xs.length; ++column) {
        // Wrapper samples cover its border, while descendant samples cover
        // its contents. Do not assume their centers coincide across fonts.
        if (id.includes('atomic') && row === 1 && column === 1) continue;
        const x = xs[column], y = ys[row];
        samples.push({
          expected: id,
          x,
          y,
          elementFromPoint: document.elementFromPoint(x, y)?.id,
          elementsFromPoint: document.elementsFromPoint(x, y)[0]?.id,
        });
      }
    }
  }
  return samples;
}
