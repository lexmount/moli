// Compare CSS zoom with an equivalent outer transform using the same SVG
// contents. Chromium 145.0.7632.116 produces identical pixels for each pair.
globalThis.installSvgSceneInputs = (filter = 'all') => {
  const text = '<text font-size="10" x="5" y="25">Moli SVG</text>';
  const path = '<path d="M5 25 H85" stroke="black" stroke-width="2"/>';
  const units = [
    {name: 'font-zoom2', scale: 2, content: text},
    {name: 'font-zoom-half', scale: 0.5, content: text},
    {name: 'font-zoom-fractional', scale: 1.5, content: text},
    {name: 'font-unzoomed', scale: 1, content: text},
    {name: 'font-ancestor-zoom2', scale: 2, ancestor: true, content: text},
    {name: 'font-inherited', scale: 2, content: '<text x="5" y="25">Moli SVG</text>'},
    {name: 'font-relative', scale: 2, content: '<text font-size="1.5em" x="5" y="25">Moli SVG</text>'},
    {name: 'font-child-zoom', scale: 2, content: `<g style="zoom:2">${text}</g>`},
    {name: 'font-viewbox', scale: 2, viewbox: 'viewBox="0 0 50 35"', content: text},
    {name: 'stroke-width', scale: 2, content: path},
    {name: 'stroke-default', scale: 2, content: '<path d="M5 25 H85" stroke="black"/>'},
    {name: 'stroke-dashes', scale: 2, content: '<path d="M5 25 H85" stroke="black" stroke-width="2" stroke-dasharray="4 2" stroke-dashoffset="1"/>'},
    {name: 'stroke-percent', scale: 2, content: '<path d="M5 25 H85" stroke="black" stroke-width="2%" stroke-dasharray="4% 2%" stroke-dashoffset="1%"/>'},
  ].map(test => ({...test, kind: 'units'}));
  const templates = [
    {name: 'template-simple', content: '<template><span style="fill:lime">inert</span></template>'},
    {name: 'template-nested', content: '<template><span style="fill:lime">inert</span><template><span>nested</span></template></template>'},
  ].map(test => ({...test, kind: 'template'}));
  const cases = [...units, ...templates].filter(test => filter === 'all' || test.kind === filter);
  document.head.innerHTML = `<style>
    html,body{margin:0;background:white}
    .svg-input-case{position:absolute;left:0;width:480px;height:160px}
    .svg-input-case svg{position:absolute;left:0;top:0;display:block;font:10px "DejaVu Sans"}
    .svg-input-case .before{fill:red}.svg-input-case .after{fill:blue}
  </style>`;
  document.body.innerHTML = cases.map((test, index) => {
    test.top = index * 160;
    let content;
    if (test.kind === 'units') {
      const attributes = `width="100" height="70" ${test.viewbox || ''}`;
      let zoomed = `<svg class="zoomed" ${attributes} style="zoom:${test.ancestor ? 1 : test.scale}">${test.content}</svg>`;
      if (test.ancestor) {
        zoomed = `<div style="position:absolute;left:0;top:0;width:100px;height:70px;zoom:${test.scale}">${zoomed}</div>`;
      }
      content = zoomed + `<svg class="reference" ${attributes} style="left:240px;transform:scale(${test.scale});transform-origin:0 0">${test.content}</svg>`;
    } else {
      content = `<svg width="120" height="40"><rect class="before" width="20" height="20"/>
        <foreignObject x="0" y="20" width="20" height="20">${test.content}</foreignObject>
        <rect class="after" x="40" width="20" height="20"/></svg>`;
    }
    return `<div class="svg-input-case" id="${test.name}" style="top:${test.top}px">${content}</div>`;
  }).join('');
  return {cases: cases.map(({name, kind, top}) => ({name, kind, top})), width: 480, height: cases.length * 160};
};

globalThis.svgSceneInputRects = name => {
  const row = document.getElementById(name);
  const rect = element => {
    const r = element.getBoundingClientRect();
    return [r.x, r.y, r.width, r.height];
  };
  if (name.startsWith('template-')) {
    const template = row.querySelector('template');
    return {rects: [rect(row.querySelector('svg > .before')), rect(row.querySelector('svg > .after'))],
      children: template.childNodes.length, content: template.content.childNodes.length};
  }
  const target = rect(row.querySelector('.zoomed text,.zoomed path'));
  const reference = rect(row.querySelector('.reference text,.reference path'));
  reference[0] -= 240;
  return {target, reference};
};
