(() => {
  const textWidth = document.getElementById('alt-reference').getBoundingClientRect().width;
  const altSize = [16 + textWidth, 20];
  const expected = {
    'no-source':[0,0], 'no-source-empty-alt':[0,0], 'no-source-alt':altSize,
    'empty-source':[0,0], 'empty-source-alt':altSize,
    'broken':[16,16], 'broken-empty-alt':[0,0], 'broken-alt':altSize, 'broken-title':altSize,
    'broken-size':[40,30], 'broken-size-empty-alt':[40,30], 'broken-size-alt':altSize,
    'broken-width':[16,16], 'broken-height':[16,16],
    'broken-width-alt':altSize, 'broken-height-alt':altSize, 'broken-small':[10,10],
    'broken-css-size':[40,30], 'broken-css-size-alt':altSize,
    'broken-inline-block-alt':[40,30], 'broken-block-alt':[40,30],
    'broken-aspect':[40,20], 'broken-border':[26,26], 'broken-empty-alt-border':[10,10],
    'block':[180,16], 'block-alt':[180,20], 'block-width':[40,16], 'block-width-alt':[40,36],
    'flex-alt':[180,20], 'inline-flex-alt':altSize, 'grid-alt':[180,20], 'inline-grid-alt':altSize,
    'rtl-alt':altSize, 'float':altSize, 'minimum':[50,16], 'maximum':[10,36],
    'percent':[16,16], 'percent-pair':[90,30], 'aspect-three':[40,20], 'borderbox':[40,30],
    'zoom':[32,32], 'zoom-alt':[2*altSize[0],40], 'contents':[0,0], 'none':[0,0],
  };
  const failures = [];
  for (const [id, size] of Object.entries(expected)) {
    const image = document.getElementById(id);
    const rect = image.getBoundingClientRect();
    const actual = [rect.width,rect.height];
    if (actual.some((value, axis) => Math.abs(value-size[axis]) > 1/64)) {
      failures.push({id, kind:'border-box', expected:size, actual});
    }
    const inset = id === 'borderbox' || id.includes('border') ? 10 : 0;
    const zoom = id.startsWith('zoom') ? 2 : 1;
    const dimensions = size.map(value => Math.round((value-inset)/zoom));
    if (image.width !== dimensions[0] || image.height !== dimensions[1]) {
      failures.push({id, kind:'IDL content dimensions', expected:dimensions, actual:[image.width,image.height]});
    }
    if (image.naturalWidth !== 0 || image.naturalHeight !== 0 || image.textContent !== '') {
      failures.push({id, kind:'fallback must not manufacture intrinsic content or DOM text'});
    }
  }
  return {cases:Object.keys(expected).length, failures};
})()
