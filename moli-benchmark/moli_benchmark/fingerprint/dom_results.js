// Passive projections of result containers, independent of layout and paint.
// Return null ONLY for an unsupported site; missing results never select a fallback.
({site}) => {
  const supported = ['browserscan-bot', 'browserscan-tls', 'creepjs'];
  if (!supported.includes(site)) return null;

  const report = {readyState: document.readyState, valid: false, collector: 'dom-results-v1'};
  const text = node => (node?.textContent || '').trim().replace(/\s+/g, ' ');
  // Exclude explicitly inactive markup without consulting computed style or geometry.
  const active = node => !node.closest('script, style, template, [hidden], [aria-hidden="true"]');
  const labelled = (selector, label) => Array.from(document.querySelectorAll(selector))
    .filter(node => active(node) && text(node) === label);

  if (site === 'browserscan-bot') {
    const labels = labelled('strong', 'Test Results:');
    report.verdict = null;
    report.state = labels.length === 0 ? 'missing' : labels.length > 1 ? 'ambiguous' : 'pending';
    if (labels.length === 1) {
      // The label and verdict are sibling <strong>s; do not scan explanatory prose.
      const values = Array.from(labels[0].parentElement.querySelectorAll(':scope > strong'))
        .filter(node => node !== labels[0] && active(node));
      if (values.length === 1 && ['Normal', 'Robot'].includes(text(values[0]))) {
        report.verdict = text(values[0]);
        report.state = 'complete';
        report.valid = true; // Complete is not the same as a passing verdict.
      } else if (values.length > 1) {
        report.state = 'ambiguous';
      }
    }
    return report;
  }

  if (site === 'browserscan-tls') {
    const fields = {
      akamaiHash: ['Akamai Hash', /^[a-f0-9]{32}$/i],
      ja3: ['JA3', /^\d+,(?:\d+(?:-\d+)*)?,(?:\d+(?:-\d+)*)?,(?:\d+(?:-\d+)*)?,(?:\d+(?:-\d+)*)?$/],
      ja4: ['JA4', /^[tqd][a-z0-9]{9}_[a-f0-9]{12}_[a-f0-9]{12}$/i],
    };
    report.kind = 'report-only';
    report.reportLabels = ['HTTP/2 Fingerprint', 'Akamai Hash', 'JA3', 'JA4']
      .filter(label => labelled('h3', label).length === 1);
    report.fields = Object.fromEntries(Object.entries(fields).map(([key, [label, pattern]]) => {
      const labels = labelled('h3', label);
      if (labels.length !== 1) return [key, labels.length ? 'ambiguous' : 'missing'];
      // A report card has a heading wrapper followed by a value wrapper. Avoid
      // hashed CSS classes and never accept hashes from unrelated cards/examples.
      const heading = labels[0].parentElement?.parentElement;
      const body = heading?.nextElementSibling;
      const values = body ? Array.from(body.querySelectorAll(':scope > div > p')).filter(active) : [];
      if (values.length > 1) return [key, 'ambiguous'];
      // The scalar is direct text in <p>. Ad annotations can append child DOM
      // (e.g. "Convert Data Files"); those are not part of the fingerprint.
      const value = Array.from(values[0]?.childNodes || []).filter(node => node.nodeType === Node.TEXT_NODE)
        .map(node => node.textContent).join('').trim();
      const state = !value || /^(?:loading[.\u2026]*|pending|-+)$/i.test(value) ? 'pending'
        : pattern.test(value) ? 'populated' : 'unrecognized';
      return [key, state]; // Do not retain the fingerprint or network identity.
    }));
    report.valid = report.reportLabels.length === 4
      && Object.values(report.fields).every(state => state === 'populated');
    return report;
  }

  // CreepJS publishes its completed measurements here. Rating fields are the
  // source for the DOM percentages; neither a layout snapshot nor modal text is needed.
  const fingerprint = window.Fingerprint;
  const count = value => Number.isInteger(value) && value >= 0 ? value : null;
  const boolean = value => typeof value === 'boolean' ? value : null;
  const percent = value => typeof value === 'number' && Number.isFinite(value)
    && value >= 0 && value <= 100 ? value : null;
  report.metrics = {
    totalLies: count(fingerprint?.lies?.totalLies),
    capturedErrorCount: Array.isArray(fingerprint?.capturedErrors?.data)
      ? fingerprint.capturedErrors.data.length : null,
    audioLied: boolean(fingerprint?.offlineAudioContext?.lied),
    svgLied: boolean(fingerprint?.svg?.lied),
  };
  const names = {likeHeadless: 'like headless', headless: 'headless', stealth: 'stealth'};
  report.ratings = Object.fromEntries(Object.keys(names)
    .map(key => [key, percent(fingerprint?.headless?.[key + 'Rating'])]));
  report.percentages = Object.entries(report.ratings)
    .filter(([, value]) => value !== null).map(([key, value]) => `${value}% ${names[key]}`);
  report.ratingsComplete = Object.values(report.ratings).every(value => value !== null);
  report.headlessSignals = Object.fromEntries(Object.keys(names).map(key => {
    const signals = fingerprint?.headless?.[key];
    return [key, signals && typeof signals === 'object' ? Object.fromEntries(Object.entries(signals)
      .filter(([name, value]) => /^[A-Za-z0-9_]{1,80}$/.test(name) && typeof value === 'boolean')) : null];
  }));
  // Retain the survey's core-result criterion, but expose partial ratings separately.
  report.valid = report.metrics.totalLies !== null && report.metrics.capturedErrorCount !== null;
  return report;
}
