addEventListener('message', async event => {
  const input = event.data;
  if (!['frames', 'popups', 'namedPopups'].includes(input?.mode)) return;
  const rows = [];
  const marker = frame => {
    try { return frame.contentDocument?.getElementById('marker')?.textContent ?? null; }
    catch { return null; }
  };
  const loadFrame = (url, mode) => new Promise(resolve => {
    const frame = document.createElement('iframe');
    let navigating = mode !== 'location';
    frame.onload = () => {
      if (!navigating) {
        navigating = true;
        frame.contentWindow.location.href = url;
        return;
      }
      resolve(marker(frame));
      frame.remove();
    };
    if (mode !== 'location') frame.src = url;
    document.body.appendChild(frame);
  });
  const clickPopupLink = (method, url, target) => {
    const link = document.createElement(method);
    link.href = url;
    link.target = target;
    link.rel = 'opener';
    if (method === 'area') {
      const image = document.createElement('img');
      const map = document.createElement('map');
      map.name = 'blob-navigation-map';
      image.useMap = '#' + map.name;
      document.body.append(image, map);
      map.appendChild(link);
      link.shape = 'rect';
      link.coords = '0,0,1,1';
    } else {
      document.body.appendChild(link);
    }
    link.click();
  };
  try {
    const own = URL.createObjectURL(new Blob(['<p id=marker>own</p>'], {type:'text/html'}));
    if (input.mode === 'frames') {
      for (const method of ['attribute', 'location']) {
        rows.push({method, foreignMarker:await loadFrame(input.foreign, method), ownMarker:await loadFrame(own, method)});
      }
    } else if (input.mode === 'namedPopups') {
      for (const method of ['open', 'a', 'area', 'frame']) {
        const name = 'blob-navigation-existing-' + method;
        if (method === 'frame') {
          const frame = document.createElement('iframe');
          frame.name = name;
          document.body.appendChild(frame);
        }
        const popup = open('about:blank', name);
        const loaded = new Promise(resolve => {
          const onLoaded = event => {
            if (event.source !== popup || event.data?.kind !== 'named-blob-loaded') return;
            removeEventListener('message', onLoaded);
            resolve(popup.document.getElementById('marker')?.textContent);
          };
          addEventListener('message', onLoaded);
        });
        const markup = '<p id=marker>own</p><script>opener.postMessage({kind:"named-blob-loaded"},"*");</scr' + 'ipt>';
        const url = URL.createObjectURL(new Blob([markup], {type:'text/html'}));
        let nullHandle = null;
        if (method === 'open' || method === 'frame') {
          nullHandle = open(url, name) === null;
        } else {
          clickPopupLink(method, url, name);
        }
        rows.push({method, nullHandle, marker:await loaded, openerPreserved:popup.opener === window});
      }
    } else {
      const cases = [ ['own', own], ['data', 'data:text/html,<p>data</p>'] ];
      if (!input.sameSiteSource) cases.unshift(['foreign', input.foreign]);
      for (const [name, url] of cases) {
        for (const method of ['open', 'a', 'area']) {
          let nullHandle = null;
          if (method === 'open') {
            nullHandle = window.open(url) === null;
          } else {
            clickPopupLink(method, url, '_blank');
          }
          rows.push({case:name, method, nullHandle});
        }
      }
    }
    parent.postMessage({kind:'blob-navigation-result', rows}, '*');
  } catch (error) {
    parent.postMessage({kind:'blob-navigation-result', rows, error:String(error.stack || error)}, '*');
  }
});
parent.postMessage({kind:'blob-navigation-ready'}, '*');
