(() => {
  const failures = [];
  let cases = 0;
  const check = (name, ok, actual) => { cases++; if (!ok) failures.push({name,actual}); };
  const close = (a,b) => Math.abs(a-b) < 0.002;
  const el = id => document.getElementById(id);
  const throws = (name, action, expected) => {
    let actual = 'no error'; try { action(); } catch (error) { actual = error.name; }
    check(name, actual === expected, actual);
  };
  const plain = el('plain'), part = el('part'), base = el('base'), moved = el('moved');
  // The baseline explicitly positions each glyph too. Blink rounds segmented
  // SVG runs to 1/64px differently from an unsegmented run; that is not dx or
  // textLength spacing becoming part of the typographic advance.
  check('root UTF-16 length', plain.getNumberOfChars() === 6, plain.getNumberOfChars());
  check('tspan provenance', part.getNumberOfChars() === 2, part.getNumberOfChars());
  const length = plain.getComputedTextLength();
  check('real advance', length > 0, length);
  check('real variable width', el('wide').getComputedTextLength() > 2 * el('narrow').getComputedTextLength());
  check('tspan substring advance', close(part.getComputedTextLength(), plain.getSubStringLength(2,2)));
  check('default textLength', close(plain.textLength.baseVal.value, length), plain.textLength.baseVal.value);
  check('clamped substring', close(plain.getSubStringLength(0,0xffffffff),length));
  check('zero substring', plain.getSubStringLength(0,0) === 0);
  check('first x', close(plain.getStartPositionOfChar(0).x,10));
  check('first baseline', close(plain.getStartPositionOfChar(0).y,40));
  check('tspan placement', close(part.getStartPositionOfChar(0).x,plain.getStartPositionOfChar(2).x));
  const extent = plain.getExtentOfChar(0);
  check('glyph cell', extent.width > 0 && extent.height > 0 && extent.y < 40);
  check('extent interface', Object.prototype.toString.call(extent) === '[object SVGRect]');
  check('bbox interface', Object.prototype.toString.call(plain.getBBox()) === '[object SVGRect]');
  const point = plain.getStartPositionOfChar(0);
  check('point interface', Object.prototype.toString.call(point) === '[object SVGPoint]' && point instanceof SVGPoint);
  const matrix = document.querySelector('svg').createSVGMatrix().translate(3,4);
  const transformed = point.matrixTransform(matrix);
  check('point transform', close(transformed.x,point.x+3) && close(transformed.y,point.y+4));
  point.x = 1/3;
  check('point float precision', point.x === Math.fround(1/3));
  check('detached point value', close(plain.getStartPositionOfChar(0).x,10));
  throws('point overflow',()=>{point.x=1e100},'TypeError');
  throws('point matrix receiver',()=>point.matrixTransform.call({},matrix),'TypeError');
  throws('point matrix argument',()=>point.matrixTransform({a:1}),'TypeError');
  check('hit-test', plain.getCharNumAtPosition({x:extent.x+extent.width/2,y:extent.y+extent.height/2}) === 0);
  check('nonfinite point', plain.getCharNumAtPosition({x:Infinity,y:0}) === -1);
  check('outside point', plain.getCharNumAtPosition({x:-1000,y:-1000}) === -1);
  check('position does not add advance', close(base.getComputedTextLength(),moved.getComputedTextLength()));
  check('dx placement', close(moved.getStartPositionOfChar(1).x,base.getStartPositionOfChar(1).x+40));
  check('rotation 30', close(moved.getRotationOfChar(1),30));
  check('rotation 60', close(moved.getRotationOfChar(2),60));
  check('spacing does not add advance', close(el('spaced').getComputedTextLength(),base.getComputedTextLength()));
  check('spacing bbox', close(el('spaced').getBBox().width,200), el('spaced').getBBox().width);
  check('glyph scaling does add advance', close(el('scaled').getComputedTextLength(),200));
  check('explicit textLength', el('scaled').textLength.baseVal.value === 200);
  check('whitespace collapse', el('collapsed').getNumberOfChars() === 3,el('collapsed').getNumberOfChars());
  check('whitespace preserve', el('preserved').getNumberOfChars() === 8,el('preserved').getNumberOfChars());
  for (const id of ['combining','astral']) {
    const text = el(id);
    check(id+' UTF-16 length', text.getNumberOfChars() === 3,text.getNumberOfChars());
    check(id+' shared grapheme', close(text.getSubStringLength(0,1),text.getSubStringLength(1,1)));
    check(id+' no double-counting', close(text.getSubStringLength(0,2),text.getSubStringLength(0,1)));
    check(id+' same position', close(text.getStartPositionOfChar(0).x,text.getStartPositionOfChar(1).x));
  }
  check('hidden has layout', el('hidden').getNumberOfChars() === 2);
  check('display-none has no layout', el('none').getNumberOfChars() === 0);
  check('CSS hidden has layout', el('css-hidden').getNumberOfChars() === 3);
  check('CSS display-none has no layout', el('css-none').getNumberOfChars() === 0);
  check('empty has no characters', el('empty').getNumberOfChars() === 0);
  check('duplicate IDs are not provenance', Array.from(document.querySelectorAll('.duplicate'),e=>e.getNumberOfChars()).join(',') === '1,4');
  throws('empty substring',()=>el('empty').getSubStringLength(0,0),'IndexSizeError');
  throws('index at end',()=>plain.getStartPositionOfChar(6),'IndexSizeError');
  throws('negative index',()=>plain.getExtentOfChar(-1),'IndexSizeError');
  throws('missing argument',()=>plain.getSubStringLength(),'TypeError');
  let coerced = false;
  throws('native receiver before coercion',()=>plain.getSubStringLength.call(document.body,{valueOf(){coerced=true;return 0}},1),'TypeError');
  check('no coercion on invalid receiver',!coerced);
  throws('prototype is not a receiver',()=>plain.getNumberOfChars.call(Object.create(plain)),'TypeError');
  return {cases,failures};
})()
