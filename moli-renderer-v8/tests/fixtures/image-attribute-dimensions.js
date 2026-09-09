(() => {
  const image = globalThis.dimensionImage;
  if (image.naturalWidth !== 1 || image.naturalHeight !== 1) throw new Error('decoded 1x1 image required');
  const values = [null, '0', '0junk', '-0', '-00px', '-1', '+7px', ' \t\n8tail',
    '\v8', '\u00a08', '1.9', '2e2', '4294967295', '4294967296', '999999999999'];
  return values.map(value => {
    for (const name of ['width','height']) {
      if (value === null) image.removeAttribute(name);
      else image.setAttribute(name,value);
    }
    return [image.width,image.height];
  });
})()
