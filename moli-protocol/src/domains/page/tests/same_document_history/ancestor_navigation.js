function ancestorNavigationProbe(operation, targetName, destination) {
  const target = targetName === '_top' || targetName === 'ancestor-top' ? top : parent;
  let conversions = 0;
  const value = {toString() { ++conversions; return destination; }};
  const result = {operation, targetName, before: [navigator.userActivation.isActive, navigator.userActivation.hasBeenActive]};
  try {
    if (operation === 'location') target.location = value;
    else if (operation === 'href') target.location.href = value;
    else if (operation === 'replace') target.location.replace(value);
    else if (operation === 'open') result.returnedTarget = open(destination, targetName) === target;
    else if (operation === 'anchor') {
      const a = document.body.appendChild(document.createElement('a'));
      a.href = destination; a.target = targetName; a.click();
    } else if (operation === 'form' || operation === 'post') {
      const form = document.body.appendChild(document.createElement('form'));
      form.action = destination; form.target = targetName;
      if (operation === 'post') form.method = 'POST';
      form.submit();
    }
    result.outcome = 'returned';
  } catch (error) {
    result.outcome = error.name;
    result.localException = error instanceof DOMException;
  }
  result.conversions = conversions;
  return result;
}
