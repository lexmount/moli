globalThis.installSvgViewportSizes = () => {
  const cases = [
    ['positive', '80', '40', '', [80, 40]],
    ['negative-width', '-80', '40', '', [0, 40]],
    ['negative-height', '80', '-40', '', [80, 0]],
    ['negative-both', '-80', '-40', '', [0, 0]],
    ['negative-pixels', '-80px', '40', '', [0, 40]],
    ['negative-relative', '-1em', '40', '', [0, 40]],
    ['negative-exponent', '-1e2', '40', '', [0, 40]],
    ['negative-percent', '-10%', '40', '', [300, 40]],
    ['negative-zero', '-0', '40', '', [0, 40]],
    ['css-width', '-80', '40', 'width:80px', [80, 40]],
    ['css-height', '80', '-40', 'height:40px', [80, 40]],
    ['css-both', '-80', '-40', 'width:80px;height:40px', [80, 40]],
    ['css-percent', '-80', '40', 'width:40%', [80, 40]],
    ['css-auto', '-80', '40', 'width:auto', [0, 40]],
    ['css-invalid', '-80', '40', 'width:-80px', [0, 40]],
    ['css-min-width', '-80', '40', 'min-width:80px', [80, 40]],
    ['css-max-width', '-80', '40', 'max-width:80px', [0, 40]],
    ['stylesheet', '-80', '40', '', [80, 40]],
    ['viewbox-negative-width', '-80', '40', '', [80, 40], '0 0 80 40'],
    ['viewbox-negative-height', '80', '-40', '', [80, 40], '0 0 80 40'],
    ['viewbox-negative-both', '-80', '-40', '', [0, 0], '0 0 80 40'],
    ['viewbox-zero-width', '0', '40', '', [0, 40], '0 0 80 40'],
    ['viewbox-css-auto', '-80', '40', 'width:auto', [80, 40], '0 0 80 40'],
  ].map(([name, width, height, style, size, viewBox], index) => ({
    name, width, height, style, size, viewBox, top: index * 48,
  }));
  document.head.innerHTML = `<style>
    html,body{margin:0;background:white}svg{display:block}
    #svg-viewport-stylesheet{width:80px}
  </style>`;
  document.body.innerHTML = cases.map(({name, width, height, style, viewBox, top}) => `
    <div style="position:absolute;left:0;top:${top}px;width:200px">
      <svg id="svg-viewport-${name}" width="${width}" height="${height}" style="${style}" ${viewBox ? `viewBox="${viewBox}"` : ''}>
        <rect width="40" height="20" fill="red"/>
      </svg>
    </div>`).join('');
  return {width: 320, height: cases.length * 48, cases};
};

globalThis.readSvgViewportSize = name => {
  const rect = document.getElementById(`svg-viewport-${name}`).getBoundingClientRect();
  return [rect.x, rect.y, rect.width, rect.height];
};
