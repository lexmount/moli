(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const fixture = body.appendChild(document.createElement('div'));
  const reset = fixture.appendChild(document.createElement('button'));
  const seed = fixture.appendChild(document.createTextNode('retained selection'));
  const selection = getSelection();
  const failures = [];
  const check = (condition, label) => { if (!condition) failures.push(label); };
  function at(range, node, offset) {
    return range && range.startContainer === node && range.startOffset === offset &&
      range.endContainer === node && range.endOffset === offset;
  }
  // Paths and offsets are native DOM boundaries, including UTF-16 text offsets.
  const cases = [
    ['empty', '', [], 0],
    ['text', 'abc', [0], 0],
    ['nested text', '<span>abc</span>', [0, 0], 0],
    ['collapsed prefix', '<p> \t\nabc</p>', [0, 0], 3],
    ['pre', '<p style="white-space:pre"> \nabc</p>', [0, 0], 0],
    ['pre-wrap', '<p style="white-space:pre-wrap"> \nabc</p>', [0, 0], 0],
    ['pre-line', '<p style="white-space:pre-line"> \nabc</p>', [0, 0], 1],
    ['break-spaces', '<p style="white-space:break-spaces"> abc</p>', [0, 0], 0],
    ['nonbreaking space', '\u00a0abc', [0], 0],
    ['inline leading space', ' abc', [0], 1, 0],
    ['surrogate pair', ' \ud83d\ude00abc', [0], 1, 0],
    ['lone surrogate', ' \ud800abc', [0], 1, 0],
    ['only collapsed space', ' \n', [], 0],
    ['empty inline', '<span></span>', [], 0],
    ['empty inline before text', '<span></span>abc', [1], 0],
    ['empty blocks', '<p><!--comment--></p><div><p> </p></div><p>abc</p>', [2, 0], 0],
    ['sized empty block', '<div style="height:30px"></div><p>abc</p>', [0], 0],
    ['padded empty block', '<div style="padding:10px"></div><p>abc</p>', [0], 0],
    ['bordered empty block', '<div style="border:2px solid"></div><p>abc</p>', [0], 0],
    ['empty inline block', '<span style="display:inline-block"></span>abc', [1], 0],
    ['line break', '<span><br></span>', [0], 0],
    ['atomic boundary', '<span></span><input>abc', [], 1],
    ['nested atomic boundary', '<span><b></b><input></span>abc', [0], 1],
    ['image boundary', '<span></span><img alt="image">abc', [], 1],
    ['noneditable island', '<span></span><span contenteditable="false">abc</span>def', [], 0],
    ['inert island', '<span></span><span inert>abc</span>def', [], 0],
    ['noneditable control', '<span></span><input contenteditable="false">abc', [], 0],
    ['nested noneditable island', '<div><span></span><span contenteditable="false">abc</span>def</div>', [0], 0],
    ['hidden subtree', '<span style="display:none">hidden</span><p>shown</p>', [1, 0], 0],
    ['hidden text', '<span style="visibility:hidden">hidden</span><span>shown</span>', [1, 0], 0],
    ['visibility override', '<p style="visibility:hidden"><span style="visibility:visible">shown</span></p>', [0, 0, 0], 0],
    ['list', '<ul><li>abc</li></ul>', [0, 0, 0], 0],
    ['table boundary', '<table><tr><td>abc</td></tr></table>', [], 0],
    ['SVG text', '<svg><text> abc</text></svg>def', [0, 0, 0], 1],
    ['SVG foreignObject', '<svg><foreignObject width="100" height="100"><span> abc</span></foreignObject></svg>def', [0, 0, 0, 0], 1],
    ['SVG graphics before text', '<svg><rect width="10" height="10"/></svg>def', [1], 0],
    ['MathML text', '<math><mi> abc</mi></math>def', [0, 0, 0], 1],
    ['display contents', '<p style="display:contents"><span>abc</span></p>', [0, 0, 0], 0],
    ['offscreen text', '<p style="position:absolute;top:-100px">abc</p><p>def</p>', [0, 0], 0],
  ];
  for (const tag of ['div', 'span']) {
    for (const [name, html, path, blockOffset, inlineOffset = blockOffset] of cases) {
      const offset = tag === 'span' ? inlineOffset : blockOffset;
      const editor = fixture.appendChild(document.createElement(tag));
      editor.contentEditable = 'true';
      editor.innerHTML = html;
      const node = path.reduce((node, index) => node.childNodes[index], editor);
      reset.focus();
      selection.setBaseAndExtent(seed, 1, seed, 4);
      const previous = selection.getRangeAt(0);
      let observed = false;
      editor.addEventListener('focus', () => {
        observed = selection.rangeCount === 1 && at(selection.getRangeAt(0), node, offset);
      }, {once: true});
      editor.focus();
      const label = tag + ': ' + name;
      check(document.activeElement === editor && observed, label + ': updated before focus');
      check(selection.rangeCount === 1, label + ': range exists');
      if (selection.rangeCount) {
        const range = selection.getRangeAt(0);
        check(at(range, node, offset) && selection.isCollapsed && selection.direction === 'none',
          label + ': initial caret');
        check(selection.anchorNode === node && selection.anchorOffset === offset &&
          selection.focusNode === node && selection.focusOffset === offset, label + ': endpoints');
        check(at(selection.getComposedRanges()[0], editor, 0), label + ': composed initial boundary');
        check(range !== previous && previous.startContainer === seed && previous.startOffset === 1 &&
          previous.endContainer === seed && previous.endOffset === 4, label + ': old range unchanged');
        reset.focus();
        editor.focus();
        check(selection.getRangeAt(0) === range, label + ': refocus retains range');
      }
      editor.remove();
    }
  }
  for (const [before, html, after, path, offset] of [
    ['abc', '   abc', '', [0], 0],
    ['abc ', '   abc', '', [0], 3],
    ['<span>abc</span><!--x--><span></span>', '<span> abc</span>', '', [0, 0], 0],
    ['<p>abc</p>', ' abc', '', [0], 1],
    ['<br>', ' abc', '', [0], 1],
    ['<input>', ' abc', '', [0], 0],
    ['<span style="display:none">abc</span>', ' abc', '', [0], 1],
    ['<span style="visibility:hidden">abc</span>', ' abc', '', [0], 0],
    ['<span style="display:inline-block">abc </span>', ' abc', '', [0], 0],
    ['abc', '<span> </span><span>abc</span>', '', [0, 0], 0],
    ['abc ', '<span> </span><span>abc</span>', '', [1, 0], 0],
    ['abc', ' ', 'def', [0], 0],
    ['abc', ' ', '', [], 0],
    ['abc', '<span style="white-space:pre-line"> \nabc</span>', '', [0, 0], 1],
    ['abc', '<span style="white-space:pre-line"> </span><span style="white-space:pre-line">\nabc</span>', '', [1, 0], 0],
    ['<span style="white-space:pre">abc </span>', ' abc', '', [0], 0],
    ['<svg width="10" height="10"><rect width="10" height="10"/></svg>', ' abc', '', [0], 0],
  ]) {
    const block = fixture.appendChild(document.createElement('div'));
    block.innerHTML = before + '<span data-editor contenteditable>' + html + '</span>' + after;
    const editor = block.querySelector('[data-editor]');
    const node = path.reduce((node, index) => node.childNodes[index], editor);
    selection.removeAllRanges();
    editor.focus();
    check(selection.rangeCount === 1 && at(selection.getRangeAt(0), node, offset),
      'inline whitespace: ' + before + ' / ' + html + ' / ' + after);
    block.remove();
  }
  fixture.remove();
  return failures.join('\n');
})()
