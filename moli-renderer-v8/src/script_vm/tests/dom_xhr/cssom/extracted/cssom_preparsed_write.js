(() => {
  const reactions = [], records = [];
  class CssomWriteProbe extends HTMLElement {
    static observedAttributes = ['style'];
    attributeChangedCallback(name, oldValue, newValue) {
      if (this.style.cssText !== newValue || this.getAttribute(name) !== newValue) {
        throw new Error('reaction observed stale style');
      }
      reactions.push({oldValue, newValue, left:this.style.marginLeft});
    }
  }
  customElements.define('cssom-write-probe', CssomWriteProbe);
  const element = document.createElement('cssom-write-probe');
  const observer = new MutationObserver(batch => records.push(...batch));
  observer.observe(element, {attributes:true, attributeOldValue:true});
  element.style.setProperty('margin', '1px 2px', 'important');
  element.style.marginLeft = '3px';
  element.style.width = 'calc(2px + 3px)';
  element.style.setProperty('--gap', '4px');
  element.style.padding = 'var(--gap)';
  records.push(...observer.takeRecords());
  observer.disconnect();
  if (reactions.length !== 5 || records.length !== 5) throw new Error('wrong mutation count');
  if (JSON.stringify(records.map(r=>r.oldValue)) !== JSON.stringify(reactions.map(r=>r.oldValue))) {
    throw new Error('mutation old values differ from reactions');
  }
  if (reactions[0].oldValue !== null || reactions[0].left !== '2px' || reactions[1].left !== '3px') {
    throw new Error('wrong intermediate shorthand value');
  }
  if (element.style.getPropertyPriority('margin-left') !== '' ||
      element.style.getPropertyPriority('margin-right') !== 'important' ||
      element.style.width !== 'calc(5px)' || element.style.padding !== 'var(--gap)') {
    throw new Error('wrong resulting values or importance');
  }
  const beforeInvalid = element.style.cssText;
  element.style.width = 'not-a-length';
  if (element.style.cssText !== beforeInvalid) throw new Error('invalid write changed style');
  return 'ok';
})()
