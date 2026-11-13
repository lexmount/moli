globalThis.installSvgLayoutPublication = () => {
  document.head.innerHTML = '<style>html,body{margin:0;background:white}#html{width:80px;height:20px;background:red}#scene{position:absolute;left:100px;top:40px}</style>';
  document.body.innerHTML = '<div id="html"></div><svg id="scene" width="200" height="100">' +
    '<rect id="shape" x="10" y="10" width="10" height="10" fill="blue"/>' +
    '<rect x="10" y="10" width="10" height="10" fill="blue"/>'.repeat(999) + '</svg>';
  return 'installed';
};

globalThis.readSvgLayoutPublication = () => {
  const rect = element => {
    const r = element.getBoundingClientRect();
    return [r.x, r.y, r.width, r.height];
  };
  const html = document.getElementById('html');
  const scene = document.getElementById('scene');
  const shape = document.getElementById('shape');
  const htmlBefore = html.getBoundingClientRect().width;
  const svg = rect(scene);
  const object = rect(shape);
  const shapeCount = shape.getClientRects().length;
  const range = document.createRange();
  range.selectNodeContents(scene);
  return {htmlBefore, svg, object, shapeCount, range: rect(range),
    rangeCount: range.getClientRects().length, htmlAfter: html.getBoundingClientRect().width};
};

globalThis.mutateSvgLayoutPublication = () => {
  document.getElementById('html').style.width = '300px';
  document.getElementById('scene').style.left = '300px';
  const shape = document.getElementById('shape');
  shape.setAttribute('x', '30');
  shape.setAttribute('width', '100');
  shape.setAttribute('fill', 'lime');
  return 'mutated';
};
