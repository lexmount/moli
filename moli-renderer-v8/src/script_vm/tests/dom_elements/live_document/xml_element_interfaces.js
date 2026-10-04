(() => {
  const check = (value, message) => { if (!value) throw new Error(message); };
  const source = '<root xmlns:s="http://www.w3.org/2000/svg" xmlns:m="http://www.w3.org/1998/Math/MathML" xmlns:h="http://www.w3.org/1999/xhtml">' +
    '<s:rect/><s:g/><s:unknown/><m:math/><m:mrow/><h:a/><plain/>';
  const constructors = [SVGRectElement, SVGGElement, SVGElement, MathMLElement, MathMLElement, HTMLAnchorElement, Element];
  // Cover both eager and lazy native imports of parsed documents.
  for (const padding of ['', '<filler/>'.repeat(1001)]) {
    const parsed = new DOMParser().parseFromString(source + padding + '</root>', 'application/xml');
    for (const doc of [parsed, parsed.cloneNode(true)]) {
      for (const [index, Constructor] of constructors.entries()) {
        const child = doc.documentElement.children[index];
        for (const element of [child, child.cloneNode(), document.importNode(child)]) {
          check(Object.getPrototypeOf(element) === Constructor.prototype, Constructor.name + ' native prototype');
          check(Object.prototype.toString.call(element) === '[object ' + Constructor.name + ']', Constructor.name + ' native tag');
        }
      }
      const rect = doc.documentElement.firstElementChild;
      rect.setAttribute('x', '3');
      check(rect.x.baseVal.value === 3, 'SVG length reads detached owner attribute');
      rect.x.baseVal.value = 17;
      check(rect.getAttribute('x') === '17', 'preserved SVG interface remains usable');
      const target = document.implementation.createDocument(null, 'target');
      target.adoptNode(rect);
      check(rect instanceof SVGRectElement && rect.ownerDocument === target && rect.x.baseVal.value === 17, 'adoption preserves native interface');
      rect.x.baseVal.value = 23;
      check(rect.getAttribute('x') === '23', 'adopted SVG length still reflects to owner');
    }
  }
  return true;
})()
