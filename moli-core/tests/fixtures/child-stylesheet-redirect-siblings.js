function () {
  const sheet = document.styleSheets[0];
  const root = sheet.href ? sheet : sheet.cssRules[0].styleSheet;
  const imports = Array.from(root.cssRules);
  const base = new URL(root.href, document.baseURI);
  const aliasRule = imports.find(rule => new URL(rule.href, base).pathname.endsWith('-alias.css'));
  const branchRule = imports.find(rule => new URL(rule.href, base).pathname.endsWith('-branch.css'));
  const alias = aliasRule.styleSheet;
  const branch = branchRule.styleSheet;
  const directRule = branch.cssRules[0];
  const direct = directRule.styleSheet;
  const readableRules = sheet => {
    try {
      return Array.from(sheet.cssRules);
    } catch (error) {
      if (error.name !== 'SecurityError') throw error;
      return [];
    }
  };
  const value = sheet => readableRules(sheet)[0]?.style.getPropertyValue('--sibling-load').trim() ?? '';
  return {
    alias: value(alias),
    direct: value(direct),
    directRules: readableRules(direct).length,
    distinct: alias !== direct,
    owners: alias.ownerRule === aliasRule && alias.parentStyleSheet === root &&
      branch.ownerRule === branchRule && branch.parentStyleSheet === root &&
      direct?.ownerRule === directRule && direct.parentStyleSheet === branch,
    cascade: getComputedStyle(document.documentElement).getPropertyValue('--sibling-load').trim()
  };
}
