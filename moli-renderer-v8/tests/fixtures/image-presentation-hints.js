(() => {
  // display:none exposes computed CSS values, without depending on image loading,
  // intrinsic geometry, or Moli's explicit rendering-checkpoint policy.
  const sheet = document.createElement('style');
  sheet.textContent = '.image-hint-auto {width:auto;height:auto} '
    + '.image-hint-author {width:17px;height:19px;aspect-ratio:7/9}';
  document.head.append(sheet);
  const failures = [];
  let checks = 0;
  const check = (name, image, expected) => {
    checks++;
    const style = getComputedStyle(image);
    const actual = [style.width, style.height, style.aspectRatio];
    if (JSON.stringify(actual) !== JSON.stringify(expected))
      failures.push({name, actual, expected});
  };
  const create = (width, height, tag = 'img', namespace) => {
    const image = namespace ? document.createElementNS(namespace, tag) : document.createElement(tag);
    image.setAttribute('style', 'display:none');
    if (width !== null) image.setAttribute('width', width);
    if (height !== null) image.setAttribute('height', height);
    document.body.append(image);
    return image;
  };
  for (const [name, width, height, expected] of [
    ['absolute', '40', '30', ['40px', '30px', 'auto 40 / 30']],
    ['width-only', '40', null, ['40px', 'auto', 'auto']],
    ['height-only', null, '30', ['auto', '30px', 'auto']],
    ['percent-width', '25%', '30', ['25%', '30px', 'auto']],
    ['percent-height', '40', '50%', ['40px', '50%', 'auto']],
    ['relative-width', '10*', '30', ['auto', '30px', 'auto']],
    ['relative-height', '40', '10*', ['40px', 'auto', 'auto']],
    ['decimal', '12.5', '2.5', ['12.5px', '2.5px', 'auto 12.5 / 2.5']],
    ['zero-width', '0', '30', ['0px', '30px', 'auto 0 / 30']],
    ['zero-height', '40', '0', ['40px', '0px', 'auto 40 / 0']],
    ['negative', '-10', '30', ['auto', '30px', 'auto']],
    ['leading-plus', '+10', '30', ['auto', '30px', 'auto']],
    ['empty-width', '', '30', ['auto', '30px', 'auto']],
    ['trailing-garbage', '10px', '3abc', ['10px', '3px', 'auto 10 / 3']],
    ['space-before-star', '10 *', '30', ['10px', '30px', 'auto 10 / 30']],
    ['decimal-percent', '10.%', '30', ['10%', '30px', 'auto']],
  ]) check(name, create(width, height), expected);

  check('not-an-image', create('40', '30', 'div'), ['auto', 'auto', 'auto']);
  check('non-html-image', create('40', '30', 'img', 'urn:moli-image-hints'), ['auto', 'auto', 'auto']);

  const image = create('40', '30');
  image.width = 50;
  check('width-idl-mutation', image, ['50px', '30px', 'auto 50 / 30']);
  image.removeAttribute('height');
  check('height-removal', image, ['50px', 'auto', 'auto']);
  image.height = 30;
  check('height-restored', image, ['50px', '30px', 'auto 50 / 30']);
  image.className = 'image-hint-auto';
  check('author-auto', image, ['auto', 'auto', 'auto 50 / 30']);
  image.className = 'image-hint-author';
  check('author-ratio', image, ['17px', '19px', '7 / 9']);
  image.style.width = '23px';
  check('inline-width', image, ['23px', '19px', '7 / 9']);
  image.className = '';
  check('ratio-restored', image, ['23px', '30px', 'auto 50 / 30']);
  image.style.removeProperty('width');
  check('hints-restored', image, ['50px', '30px', 'auto 50 / 30']);
  return {checks, failures};
})()
