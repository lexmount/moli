(async () => {
  // A missing/invalid charset inherits from an iframe's UTF-8 container;
  // a top-level popup uses the browser default instead.
  const cases = [
    [['', 'text/plain'], 'text/plain', null, 'hi'],
    [['text/plain', ''], 'text/plain', null, 'hi'],
    [['text/html', 'text/plain'], 'text/plain', null, 'hi'],
    [['text/plain;charset=gbk', 'text/html'], 'text/html', null, 'hi'],
    [['text/plain;charset=gbk', 'text/html;charset=windows-1254'], 'text/html', 'windows-1254', 'Ğ'],
    [['text/plain;charset=gbk', 'text/plain'], 'text/plain', 'GBK', '家居'],
    [['text/html;charset=gbk', 'text/html;charset=utf-8', 'text/html'], 'text/html', 'GBK', '家居'],
    [['text/html;charset=gbk', 'text/plain', 'text/html'], 'text/html', null, 'hi'],
    [['text/plain', '*/*;charset=gbk'], 'text/plain', null, 'hi'],
    [['text/html', '*/*'], 'text/html', null, 'hi'],
    [['text/html;x="', 'text/plain'], 'text/html', null, 'hi'],
    [['text/html;"', '\\"', 'text/plain', '";charset=GBK'], 'text/html', 'GBK', '家居'],
    [['text/html;"', '"', 'text/plain'], 'text/plain', null, 'hi'],
    [['application/octet-stream', 'text/html', 'invalid'], 'text/html', null, 'hi'],
    [['text/html;charset=gbk', 'text/html;charset=unknown'], 'text/html', null, 'hi'],
    [['text/html;charset=gbk', 'text/html;charset=""'], 'text/html', null, 'hi'],
  ];
  const errors = [];
  let checked = 0;
  for (const kind of ['iframe', 'popup']) {
    for (const [values, type, charset, text] of cases) {
      for (const fields of [values, [values.join(',')]]) {
        const url = new URL('/mime', location.href);
        for (const field of fields) url.searchParams.append('value', field);
        url.searchParams.set('encoding', (charset || 'UTF-8').toLowerCase());
        let frame, popup;
        try {
          const doc = await new Promise((resolve, reject) => {
            const timer = setTimeout(() => reject(new Error('navigation timeout')), 5000);
            const loaded = document => { clearTimeout(timer); resolve(document); };
            if (kind === 'iframe') {
              frame = document.createElement('iframe');
              frame.onload = () => loaded(frame.contentDocument);
              frame.src = url.href;
              document.body.append(frame);
            } else {
              popup = open('about:blank', '_blank');
              popup.onload = () => {
                if (popup.location.href === url.href) loaded(popup.document);
              };
              popup.location.href = url.href;
            }
          });
          const observed = [doc.contentType, doc.characterSet.toUpperCase(),
            doc.body.firstChild.localName, doc.body.textContent];
          const expected = [type, (charset || (kind === 'iframe' ? 'UTF-8' : 'windows-1252')).toUpperCase(), type === 'text/plain' ? 'pre' : 'b',
            type === 'text/plain' ? `<b>${text}</b>\n` : `${text}\n`];
          if (JSON.stringify(observed) !== JSON.stringify(expected)) {
            errors.push({kind, fields, observed, expected});
          }
        } catch (error) {
          errors.push({kind, fields, error: String(error)});
        } finally {
          if (frame) frame.remove();
          if (popup) popup.close();
        }
        checked++;
      }
    }
  }
  return JSON.stringify({errors, checked});
})()
