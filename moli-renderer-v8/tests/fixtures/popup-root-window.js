function popupRootFrame(hosts, index) {
  if (index < hosts.length - 1) {
    addEventListener('message', event => {
      (index === 0 ? opener : parent).postMessage(event.data, '*');
    });
    addEventListener('load', () => {
      const frame = document.createElement('iframe');
      const url = new URL('/frame-' + (index + 1), location.href);
      url.hostname = hosts[index + 1];
      frame.src = url.href;
      document.body.appendChild(frame);
    });
    return;
  }

  addEventListener('load', () => {
    const read = callback => {
      try { return callback(); } catch (error) { return error.name; }
    };
    parent.postMessage({
      topHasOpener: top.opener !== null,
      topIsParent: top === parent,
      parentTopIsTop: parent.top === top,
      topParentIsTop: top.parent === top,
      topSelfIsTop: top.self === top,
      topDocumentPath: read(() => new URL(top.document.URL).pathname),
      parentDocumentPath: read(() => new URL(parent.document.URL).pathname),
      openerDocumentPath: read(() => new URL(top.opener.document.URL).pathname),
    }, '*');
  });
}
