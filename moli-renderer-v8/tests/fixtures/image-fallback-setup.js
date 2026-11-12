(async () => {
  document.body.innerHTML = '<style>body{margin:8px;font:16px monospace;line-height:20px;width:200px}section{display:block;width:180px;min-height:24px}img{vertical-align:baseline}</style><span id="alt-reference" style="display:inline-block">abc</span>';
  const cases = [
    ['no-source', {}], ['no-source-empty-alt', {alt:''}], ['no-source-alt', {alt:'abc'}],
    ['empty-source', {src:''}], ['empty-source-alt', {src:'',alt:'abc'}],
    ['broken', {broken:true}], ['broken-empty-alt', {broken:true,alt:''}],
    ['broken-alt', {broken:true,alt:'abc'}], ['broken-title', {broken:true,title:'abc'}],
    ['broken-size', {broken:true,width:40,height:30}],
    ['broken-size-empty-alt', {broken:true,width:40,height:30,alt:''}],
    ['broken-size-alt', {broken:true,width:40,height:30,alt:'abc'}],
    ['broken-width', {broken:true,width:40}], ['broken-height', {broken:true,height:30}],
    ['broken-width-alt', {broken:true,width:40,alt:'abc'}],
    ['broken-height-alt', {broken:true,height:30,alt:'abc'}],
    ['broken-small', {broken:true,width:10,height:10}],
    ['broken-css-size', {broken:true,style:'width:40px;height:30px'}],
    ['broken-css-size-alt', {broken:true,alt:'abc',style:'width:40px;height:30px'}],
    ['broken-inline-block-alt', {broken:true,alt:'abc',style:'display:inline-block;width:40px;height:30px'}],
    ['broken-block-alt', {broken:true,alt:'abc',style:'display:block;width:40px;height:30px'}],
    ['broken-aspect', {broken:true,style:'width:40px;aspect-ratio:2/1'}],
    ['broken-border', {broken:true,style:'border:3px solid red;padding:2px'}],
    ['broken-empty-alt-border', {broken:true,alt:'',style:'border:3px solid red;padding:2px'}],
  ];
  for (const [id, style, alt] of [
    ['block', 'display:block'], ['block-alt', 'display:block', 'abc'],
    ['block-width', 'display:block;width:40px'],
    ['block-width-alt', 'display:block;width:40px', 'abc'],
    ['flex-alt', 'display:flex', 'abc'], ['inline-flex-alt', 'display:inline-flex', 'abc'],
    ['grid-alt', 'display:grid', 'abc'], ['inline-grid-alt', 'display:inline-grid', 'abc'],
    ['rtl-alt', 'direction:rtl', 'abc'], ['float', 'float:right', 'abc'],
    ['minimum', 'min-width:50px'], ['maximum', 'max-width:10px', 'abc'],
    ['percent', 'width:50%'], ['percent-pair', 'width:50%;height:30px'],
    ['aspect-three', 'width:40px;aspect-ratio:3/1'],
    ['borderbox', 'width:40px;height:30px;padding:2px;border:3px solid red;box-sizing:border-box'],
    ['zoom', 'zoom:2'], ['zoom-alt', 'zoom:2', 'abc'],
    ['contents', 'display:contents', 'abc'], ['none', 'display:none', 'abc'],
  ]) cases.push([id, {broken:true, style, ...(alt === undefined ? {} : {alt})}]);
  const pending = [];
  for (const [id, attributes] of cases) {
    const section = document.createElement('section');
    const image = document.createElement('img');
    image.id = id;
    if (attributes.broken || Object.hasOwn(attributes, 'src')) {
      pending.push(new Promise(resolve => {
        image.addEventListener('load', resolve, {once:true});
        image.addEventListener('error', resolve, {once:true});
      }));
    }
    for (const [name,value] of Object.entries(attributes)) {
      if (name !== 'broken') image.setAttribute(name,value);
    }
    if (attributes.broken) image.src = 'data:image/png;base64,YmFk';
    section.append('A', image, 'B');
    document.body.append(section);
  }
  await Promise.all(pending);
  return cases.length;
})()
