(() => {
  function assert(ok, message) { if (!ok) throw new Error(message); }
  const styles = [
    document.createElement('div').style,
    document.implementation.createHTMLDocument('').createElement('div').style
  ];
  for (const style of styles) for (const [property, alias] of [
    ['timeline-trigger', 'timelineTrigger'],
    ['-webkit-locale', 'webkitLocale']
  ]) {
    assert(property in style && alias in style, 'property accessors');
    for (const keyword of ['initial', 'inherit', 'unset', 'revert', 'revert-layer']) {
      assert(CSS.supports(property, keyword), 'CSS-wide keyword support');
      assert(CSS.supports(`(${property}: ${keyword})`), 'condition support');
      assert(CSS.supports(`(${property}: ${keyword} !important)`), 'condition priority');
      style.setProperty(property, keyword, 'important');
      assert(style.getPropertyValue(property) === keyword, 'stored keyword');
      assert(style[alias] === keyword, 'alias getter');
      assert(style.getPropertyPriority(property) === 'important', 'priority');
      style[alias] = keyword;
      assert(style.getPropertyPriority(property) === '', 'IDL setter priority');
    }
    for (const value of ['unsupported-value', 'initial inherit', 'var(--unimplemented)', 'initial; color: red']) {
      assert(!CSS.supports(property, value), 'unsupported grammar rejected');
      assert(!CSS.supports(`(${property}: ${value})`), 'unsupported condition rejected');
      style.setProperty(property, value);
      assert(style.getPropertyValue(property) === 'revert-layer', 'invalid write preserved value');
    }
    style[alias] = '/**/IN\\69 TIAL/**/';
    assert(style[property] === 'initial', 'shared CSS token parser');
    assert(CSS.supports(property, '/**/IN\\69 TIAL/**/'), 'escaped keyword support');
    assert(!CSS.supports(property, 'initial !important'), 'priority forbidden in value pair');
    assert(CSS.supports(`(not (${property}: unsupported-value)) and (${property}: inherit)`), 'condition boolean evaluation');
    assert(style.removeProperty(property) === 'initial', 'remove keyword');
    assert(style[property] === '', 'empty after remove');
  }
  return true;
})()
