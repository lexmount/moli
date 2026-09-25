(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const reset = body.appendChild(document.createElement('input'));
  const failures = [];
  const svgNS = 'http://www.w3.org/2000/svg';
  const mathNS = 'http://www.w3.org/1998/Math/MathML';
  function check(condition, description) {
    if (!condition) failures.push(description);
  }
  function checkFocus(element, expected, description) {
    reset.focus();
    element.focus({preventScroll: true});
    check((document.activeElement === element) === expected, description);
  }
  const tokens = [
    [null, -1, false], ['', -1, false], ['invalid', -1, false],
    ['\u00a00', -1, false], ['\u000b0', -1, false], ['+ 1', -1, false],
    ['2147483648', -1, false], ['-2147483649', -1, false],
    ['99999999999999999999999', -1, false],
    ['0', 0, true], ['-1', -1, true], ['+1tail', 1, true],
    [' \t\n\r\f0tail', 0, true],
    ['2147483647', 2147483647, true], ['-2147483648', -2147483648, true]
  ];
  for (const [namespace, containerTag, tag] of [
    ['http://www.w3.org/1999/xhtml', 'div', 'div'],
    [svgNS, 'svg', 'rect'], [mathNS, 'math', 'mrow']
  ]) {
    const container = body.appendChild(document.createElementNS(namespace, containerTag));
    const element = container.appendChild(document.createElementNS(namespace, tag));
    for (const [value, tabIndex, focusable] of tokens) {
      if (value === null) element.removeAttribute('tabindex');
      else element.setAttribute('tabindex', value);
      check(element.tabIndex === tabIndex, `${tag} tabIndex ${JSON.stringify(value)}`);
      checkFocus(element, focusable, `${tag} focus ${JSON.stringify(value)}`);
    }
    element.tabIndex = 0;
    const events = [];
    reset.focus();
    for (const type of ['focus', 'focusin', 'blur', 'focusout']) {
      element.addEventListener(type, () => events.push(type));
    }
    element.focus();
    check(element.matches(':focus'), `${tag} :focus after focus`);
    element.blur();
    check(document.activeElement === body && !element.matches(':focus'), `${tag} blur`);
    check(events.join(',') === 'focus,focusin,blur,focusout', `${tag} events: ${events}`);
    container.remove();
  }

  const samples = body.appendChild(document.createElement('div'));
  samples.innerHTML = `
    <a id="nonlink"></a><a id="link" href="#" tabindex="invalid"></a>
    <button id="button" tabindex="invalid"></button><object id="object"></object>
    <input id="hidden" type="hidden" tabindex="0" style="display:block">
    <button id="disabled" disabled tabindex="0"></button>
    <audio id="audio" controls></audio><video id="video" controls></video>
    <audio id="audio-no-controls"></audio><video id="video-no-controls"></video>
    <summary id="outside-summary"></summary>
    <details open><summary id="first-summary"></summary><summary id="second-summary"></summary></details>
    <div id="editing" contenteditable></div><a id="editing-nonlink" contenteditable></a>
    <map name="focus-map"><area id="area" tabindex="0"></map><img usemap="#focus-map">
    <map name="unreferenced"><area id="unreferenced" href="#" tabindex="0"></map>
    <svg><a id="svg-nonlink"></a><a id="svg-link" href="#"></a></svg>
    <math><a id="math-nonlink"></a><a id="math-link" href="#"></a></math>`;
  for (const id of ['link', 'button', 'object', 'audio', 'video', 'first-summary', 'editing', 'editing-nonlink', 'area', 'svg-link', 'math-link']) {
    checkFocus(samples.querySelector(`#${id}`), true, `${id} default focus`);
  }
  for (const id of ['nonlink', 'hidden', 'disabled', 'audio-no-controls', 'video-no-controls', 'outside-summary', 'second-summary', 'unreferenced', 'svg-nonlink', 'math-nonlink']) {
    checkFocus(samples.querySelector(`#${id}`), false, `${id} no default focus`);
  }
  check(samples.querySelector('#nonlink').tabIndex === 0, 'nonlink reflection is separate from focusability');
  for (const id of ['audio', 'video']) {
    const media = samples.querySelector(`#${id}`);
    media.removeAttribute('controls');
    checkFocus(media, false, `${id} controls removal`);
  }
  const first = samples.querySelector('#first-summary');
  const second = samples.querySelector('#second-summary');
  first.parentNode.insertBefore(second, first);
  checkFocus(first, false, 'summary losing first position');
  checkFocus(second, true, 'summary gaining first position');
  const svgLink = samples.querySelector('#svg-link');
  svgLink.removeAttribute('href');
  svgLink.setAttributeNS('http://www.w3.org/1999/xlink', 'xlink:href', '#');
  checkFocus(svgLink, true, 'SVG xlink:href focus');
  samples.remove();

  const svg = body.appendChild(document.createElementNS(svgNS, 'svg'));
  for (const tag of ['clipPath', 'defs', 'desc', 'linearGradient', 'marker', 'mask', 'metadata', 'pattern', 'radialGradient', 'script', 'style', 'title', 'symbol', 'filter', 'stop', 'feGaussianBlur', 'animate', 'animateMotion', 'set', 'view', 'unknown', 'mesh']) {
    const container = svg.appendChild(document.createElementNS(svgNS, tag));
    container.setAttribute('tabindex', '0');
    container.style.display = 'block';
    checkFocus(container, false, `${tag} is never directly rendered`);
    const child = container.appendChild(document.createElementNS(svgNS, 'rect'));
    child.setAttribute('tabindex', '0');
    checkFocus(child, false, `${tag} excludes SVG descendants`);
    const foreign = container.appendChild(document.createElementNS(svgNS, 'foreignObject'));
    const input = foreign.appendChild(document.createElement('input'));
    checkFocus(input, false, `${tag} excludes HTML descendants`);
    container.remove();
  }
  const rect = svg.appendChild(document.createElementNS(svgNS, 'rect'));
  rect.tabIndex = 0;
  checkFocus(rect, true, 'renderable SVG descendant');
  const fakeInput = svg.appendChild(document.createElementNS(svgNS, 'input'));
  checkFocus(fakeInput, false, 'foreign input does not inherit HTML focus behavior');
  svg.remove();
  reset.remove();
  return failures;
})()
