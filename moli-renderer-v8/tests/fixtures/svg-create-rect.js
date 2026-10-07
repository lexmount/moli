(() => {
  const check = (condition, message) => {
    if (!condition) throw new Error(`createSVGRect: ${message}`);
  };
  const throwsTypeError = operation => {
    try { operation(); } catch (error) { return error instanceof TypeError; }
    return false;
  };
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  check(typeof svg.createSVGRect === 'function' && svg.createSVGRect.length === 0, 'interface');
  const rect = svg.createSVGRect();
  const values = value => [value.x, value.y, value.width, value.height];
  check(JSON.stringify(values(rect)) === '[0,0,0,0]', 'zero initialization');
  check(SVGRect === DOMRect && rect instanceof DOMRect, 'SVGRect aliases DOMRect');
  check(Object.prototype.toString.call(rect) === '[object DOMRect]', 'prototype tag');
  check(Object.getOwnPropertyNames(rect).length === 0, 'private value storage');
  check(Object.getPrototypeOf(rect) === SVGRect.prototype, 'prototype identity');
  check(Object.getPrototypeOf(SVGRect.prototype) === DOMRectReadOnly.prototype, 'prototype hierarchy');
  check(new SVGRect(1,2,3,4).height === 4, 'alias constructor');
  check(throwsTypeError(() => svg.createSVGRect.call({})), 'method receiver');
  check(throwsTypeError(() => svg.createSVGRect.call(
    document.createElementNS('http://www.w3.org/2000/svg', 'g'))), 'non-root SVG receiver');
  check(throwsTypeError(() => svg.createSVGRect.call(Object.create(SVGSVGElement.prototype))),
        'forged SVG receiver');
  for (const name of ['x', 'y', 'width', 'height']) {
    const descriptor = Object.getOwnPropertyDescriptor(SVGRect.prototype, name);
    check(descriptor.enumerable && descriptor.configurable, `${name} descriptor`);
    check(typeof descriptor.get === 'function' && typeof descriptor.set === 'function', `${name} accessor`);
    check(throwsTypeError(() => descriptor.get.call({})), `${name} getter receiver`);
    check(throwsTypeError(() => descriptor.set.call({}, 1)), `${name} setter receiver`);
    rect[name] = '1.2';
    check(rect[name] === 1.2, `${name} preserves double precision`);
    for (const value of [NaN, Infinity, -Infinity, 1e40, undefined, -0]) {
      rect[name] = value;
      check(Object.is(rect[name], Number(value)), `${name} accepts unrestricted double`);
    }
    for (const invalid of [Symbol(), 1n]) {
      check(throwsTypeError(() => { rect[name] = invalid; }), `${name} conversion TypeError`);
      check(Object.is(rect[name], -0), `${name} rejected write preserves value`);
    }
    rect[name] = -5;
    check(rect[name] === -5, `${name} accepts negative values`);
    rect[name] = null;
    check(rect[name] === 0, `${name} numeric conversion`);
  }
  const second = svg.createSVGRect();
  rect.width = 50;
  check(second !== rect && second.width === 0, 'independent detached values');
  check(!svg.hasAttribute('width'), 'no live attribute binding');
  return 'svg-create-rect:ok';
})()
