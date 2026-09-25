(() => {
  const outer = document.getElementById('outer');
  const container = outer.contentDocument;
  const nested = container.getElementById('nested');
  const doc = nested.contentDocument;
  const get = Object.getOwnPropertyDescriptor(HTMLIFrameElement.prototype, 'contentDocument').get;
  const checks = [doc !== null, get.call(nested) === doc];
  document.domain = document.domain;
  checks.push(outer.contentDocument === null,
              nested.contentDocument === doc,
              get.call(nested) === null);
  doc.domain = doc.domain;
  checks.push(nested.contentDocument === null, get.call(nested) === doc);
  container.domain = container.domain;
  checks.push(outer.contentDocument === container,
              nested.contentDocument === doc, get.call(nested) === doc);
  return checks;
})()
