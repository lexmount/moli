(() => {
  const ns = 'http://www.w3.org/2000/svg';
  const results = [];
  const snapshot = length => ({text: length.valueAsString, value: length.value,
    specified: length.valueInSpecifiedUnits, unit: length.unitType});
  for (const [realmIndex, realm] of [window, document.querySelector('iframe').contentWindow].entries()) {
    const doc = realm.document;
    const svg = doc.createElementNS(ns, 'svg');
    for (const kind of ['standalone', 'rect', 'list']) {
      for (const unit of ['cm', 'in', 'px']) {
        for (const input of [0, 0.1, 1, 2.54, 10, 96, -10, 16777217, 1e-7]) {
          for (const field of ['value', 'valueInSpecifiedUnits']) {
            const owner = kind === 'standalone' ? null : doc.createElementNS(ns, kind === 'rect' ? 'rect' : 'text');
            if (owner) owner.setAttribute('x', '1' + unit);
            const length = !owner ? svg.createSVGLength() : kind === 'rect' ? owner.x.baseVal : owner.x.baseVal.getItem(0);
            if (!owner) length.valueAsString = '1' + unit;
            length[field] = input;
            const first = snapshot(length);
            const serialized = owner ? owner.getAttribute('x') : null;
            const parsed = svg.createSVGLength();
            parsed.valueAsString = first.text;
            const reparsed = snapshot(parsed);
            length.convertToSpecifiedUnits(realm.SVGLength.SVG_LENGTHTYPE_PX);
            const pixels = snapshot(length);
            const unitType = {cm: realm.SVGLength.SVG_LENGTHTYPE_CM,
              in: realm.SVGLength.SVG_LENGTHTYPE_IN, px: realm.SVGLength.SVG_LENGTHTYPE_PX}[unit];
            length.convertToSpecifiedUnits(unitType);
            results.push({realm: realmIndex, kind, unit, input, field, first, serialized,
              reparsed, pixels, roundtrip: snapshot(length)});
          }
        }
      }
    }
  }
  return results;
})()
