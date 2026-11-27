(() => {
  function assert(ok, message) { if (!ok) throw new Error(message); }
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div {}');
  const styles = [
    document.createElement('div').style,
    document.implementation.createHTMLDocument('').createElement('div').style,
    sheet.cssRules[0].style,
  ];
  for (const [property, alias] of [
    ['timeline-trigger', 'timelineTrigger'],
    ['row-rule-style', 'rowRuleStyle'],
    ['-webkit-locale', 'webkitLocale'],
  ]) {
    for (const keyword of ['initial', 'inherit', 'unset', 'revert', 'revert-layer']) {
      assert(!CSS.supports(property, keyword), `CSS.supports value for ${property}`);
      assert(!CSS.supports(`(${property}: ${keyword})`), `CSS.supports condition for ${property}`);
    }
    for (const style of styles) {
      assert(!(property in style) && !(alias in style), `unexpected accessor for ${property}`);
      style.cssText = 'color: green;';
      style.setProperty(property, 'initial', 'important');
      assert(style.length === 1, `unexpected declaration for ${property}`);
      style.cssText = `${property}: inherit; color: green;`;
      assert(style.getPropertyValue(property) === '', `unexpected value for ${property}`);
      assert(style.length === 1 && style.color === 'green', `lost color alongside ${property}`);
    }
  }
  return true;
})()
