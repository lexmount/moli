(() => {
  function exercise(w) {
    const d = w.document;
    d.open();
    d.write('<!doctype html><body style="margin:0"><style>#target{width:100px;height:10px}</style><div id=target></div>');
    const target = d.getElementById('target');
    const style = w.getComputedStyle(target);
    const log = [['initial', target.offsetWidth, style.width]];
    class Probe extends w.HTMLElement {
      static observedAttributes = ['data-width'];
      connectedCallback() {
        log.push(['connected', target.offsetWidth, style.width,
          target.getBoundingClientRect().width, this.offsetWidth]);
        target.style.width = '320px';
        log.push(['write', target.offsetWidth, style.width]);
      }
      attributeChangedCallback(name, oldValue, newValue) {
        if (!this.isConnected) return;
        target.style.width = newValue + 'px';
        log.push(['attribute', target.offsetWidth, style.width]);
      }
    }
    w.customElements.define('x-geometry-probe', Probe);
    d.write('<style>#target{width:210px}x-geometry-probe{display:block;width:75px;height:10px}</style><x-geometry-probe id=probe></x-geometry-probe>');
    d.getElementById('probe').setAttribute('data-width', '430');
    log.push(['after', target.offsetWidth, style.width]);
    d.close();
    return log;
  }
  const main = exercise(window);
  const frame = document.createElement('iframe');
  frame.style.cssText = 'width:400px;height:200px;border:0';
  document.body.append(frame);
  const child = exercise(frame.contentWindow);
  frame.remove();
  return [main, child];
})()
