// Allowlisted, passive DOM projections for the other nine sites. Only selected
// results cross CDP; no page HTML, API tokens, IP addresses or console text.
({site}) => {
  const text = node => (node?.textContent || '').trim();
  const parse = id => { try { return JSON.parse(text(document.getElementById(id))); } catch { return null; } };
  const bool = value => typeof value === 'boolean' ? value : null;
  const states = value => value && typeof value === 'object' ? Object.fromEntries(Object.entries(value)
    .filter(([key, v]) => /^[A-Za-z_]+$/.test(key) && ['OK', 'FAIL', 'WARN'].includes(v))) : null;
  const score = (value, maximum) => {
    if (typeof value === 'string' && /^\d+(?:\.\d+)?$/.test(value.trim())) value = Number(value);
    return typeof value === 'number' && Number.isFinite(value) && value >= 0 && value <= maximum ? value : null;
  };
  const report = {readyState: document.readyState, valid: false};
  if (site === 'fingerprint-pro') return {...report, source: 'network-result-required'};
  if (site === 'device-browser-static' || site === 'device-browser-behavior') {
    const value = parse('jsonResult');
    report.isBot = bool(value?.isBot);
    report.details = value?.details && typeof value.details === 'object' ? Object.fromEntries(
      Object.entries(value.details).filter(([k, v]) => /^(?:is|has|suspicious)[A-Za-z]+$/.test(k)
        && typeof v === 'boolean')) : null;
    report.valid = report.isBot !== null && report.details !== null && Object.keys(report.details).length > 0;
    return report;
  }
  if (site === 'incolumitas-bot') {
    const newer = parse('new-tests'), detection = parse('detection-tests');
    report.newTests = states(newer);
    report.intoli = states(detection?.intoli);
    report.fpscanner = states(detection?.fpscanner);
    report.valid = [report.newTests, report.intoli, report.fpscanner].every(v => v && Object.keys(v).length > 0);
    report.behavior = {jsonScore: score(newer?.behavioralClassificationScore, 1),
      domScore: score(text(document.getElementById('behavioralScore')), 1)};
    return report;
  }
  if (site === 'incolumitas-proxy') {
    // This page mixes several independent remote probes. Labels alone do not
    // establish a completed network report. Keep this explicitly partial until
    // the site exposes verifiable result values; never invent a pass from prose.
    const body = text(document.body);
    report.kind = 'report-only';
    report.certificateErrors = [...new Set(body.match(/(?:net::)?ERR_CERT_[A-Z_]+/g) || [])];
    report.reportLabels = ['WebRTC', 'TCP/IP', 'DNS', 'Timezone', 'Proxy'].filter(label => body.includes(label));
    report.completion = 'unverified';
    return report;
  }
  if (site === 'sannysoft') {
    const ids = ['user-agent-result', 'webdriver-result', 'advanced-webdriver-result', 'chrome-result',
      'permissions-result', 'plugins-length-result', 'plugins-type-result', 'languages-result',
      'webgl-vendor', 'webgl-renderer', 'broken-image-dimensions'];
    report.tests = Object.fromEntries(ids.map(id => {
      const node = document.getElementById(id);
      return [id, node ? {state: Array.from(node.classList).find(s => ['passed', 'failed', 'warn'].includes(s)) || null,
        value: text(node).slice(0, 250)} : null];
    }));
    report.fpscanner = Object.fromEntries(Array.from(document.querySelectorAll('tr'), row =>
      Array.from(row.querySelectorAll(':scope > th, :scope > td'), text)).filter(row =>
        /^[A-Z_]+$/.test(row[0] || '') && /^(ok|fail|warn)$/i.test(row[1] || '')).map(row => [row[0], row[1]]));
    report.valid = Object.values(report.tests).every(v => v?.state !== null && v?.state !== undefined);
    return report;
  }
  if (site === 'pixelscan') {
    const cards = Array.from(document.querySelectorAll('[checkervalue]'), text);
    const lower = cards.map(t => t.toLowerCase());
    report.masking = lower.includes('masking detected') ? true : lower.includes('no masking detected') ? false : null;
    report.automated = lower.includes('automated behavior detected') ? true
      : lower.includes('no automated behavior detected') ? false : null;
    report.browser = cards.find(t => /^(Chrome|Chromium|Firefox|Safari|Edge)\b/.test(t))?.slice(0, 100) || null;
    report.valid = location.pathname === '/fingerprint-check' && report.masking !== null && report.automated !== null;
    const selectors = {browser:'pxlscn-browser-integrity', fingerprint:'pxlscn-fingerprint-masking',
      automation:'pxlscn-bot-detection', location:'pxlscn-location-masking', proxy:'pxlscn-proxy',
      hardware:'pxlscn-hardware-fingerprints', navigator:'pxlscn-navigator', fonts:'pxlscn-fonts',
      timeLanguage:'pxlscn-time-language', connections:'pxlscn-connections'};
    const known = ['Masking detected', 'No masking detected', 'Automated behavior detected',
      'No automated behavior detected', 'Proxy detected', 'No proxy detected', 'Proxy check error',
      'Timezone spoofed', 'Location not detected'];
    report.sections = Object.fromEntries(Object.entries(selectors).map(([key, selector]) => {
      const node = document.querySelector(selector);
      if (!node) return [key, {present:false}];
      const cell = node.querySelector('[checkervalue]'), value = text(cell);
      const status = !cell ? 'absent' : !value ? 'empty' : /^Collecting Data/.test(value) ? 'collecting'
        : known.includes(value) ? value : 'populated';
      const details = Array.from(node.querySelectorAll('.checker-details-value, .checker-details-value--small'));
      return [key, {present:true, status, collecting:text(node).includes('Collecting Data'),
        loaders:node.querySelectorAll('pxlscn-loader').length, detailCells:details.length,
        populatedDetailCells:details.filter(e => text(e)).length}];
    }));
    return report;
  }
  if (site === 'browserleaks-js') {
    report.kind = 'report-only';
    report.fields = Object.fromEntries(['userAgent', 'platform', 'hardwareConcurrency', 'webdriver']
      .map(key => [key, text(document.getElementById('js-' + key)).slice(0, 250) || null]));
    report.valid = Object.values(report.fields).every(v => v !== null);
    return report;
  }
  if (site === 'browserleaks-webgl') {
    report.kind = 'report-only';
    const rows = Array.from(document.querySelectorAll('tr'), row =>
      Array.from(row.querySelectorAll(':scope > th, :scope > td'), text));
    const selected = label => {
      const row = rows.find(row => row[0] === label);
      return row?.length > 1 ? row.slice(1).join(' ').slice(0, 250) || null : null;
    };
    report.fields = Object.fromEntries(['WebGL Report Hash', 'WebGL Image Hash', 'WebGL Image',
      'Unmasked Vendor', 'Unmasked Renderer', 'Max Texture Size', 'Max Combined Texture Image Units',
      'Max Viewport Dimensions', 'Aliased Point Size Range'].map(key => [key, selected(key)]));
    report.unavailable = Array.from(document.querySelectorAll('p, .warning, .notice'), text)
      .some(t => /^(?:WebGL (?:is |seems to be )?(?:disabled or unavailable|not supported)|Your browser (?:does not support|doesn't support) WebGL)\b/i.test(t));
    report.valid = report.unavailable || report.fields['WebGL Report Hash'] !== null;
    const canvas = document.getElementById('gl-image-src')?.querySelector('canvas');
    const hash = text(document.getElementById('gl-image-hash'));
    report.imageProbe = {canvasInserted:!!canvas, dimensions:canvas ? [canvas.width, canvas.height] : null,
      hash:/^[a-f0-9]{32}$/i.test(hash) ? hash : null};
    return report;
  }
  throw new Error('Unknown fingerprint site');
}
