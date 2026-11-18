const parameters = new URL(location.href).searchParams;
const kind = parameters.get('kind') || document.body.dataset.kind;
const phase = parameters.get('phase') || document.body.dataset.phase;
function report(phase, probe = null) {
  parent.postMessage({
    kind, phase, probe,
    source: document.body.dataset.source,
    controlled: navigator.serviceWorker.controller !== null
  }, '*');
}
addEventListener('load', () => report(phase));
addEventListener('message', event => {
  if (event.data.action === 'navigate') {
    location.href = event.data.url;
  } else if (event.data.action === 'inspect') {
    fetch('/control/probe').then(response => response.text()).then(text => {
      report('claimed', text);
    }).catch(error => parent.postMessage({error: String(error)}, '*'));
  }
});
