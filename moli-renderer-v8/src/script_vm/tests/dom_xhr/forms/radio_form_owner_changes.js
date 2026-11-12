(mutation => {
    const radio = document.getElementById('radio');
    const peer = document.getElementById('peer');
    const form = document.getElementById('owner');
    const carrier = document.getElementById('carrier');
    const destination = document.getElementById('destination');
    const assert = (condition, message) => {
        if (!condition) throw Error(mutation + ': ' + message);
    };
    if (mutation !== 'setFormAttribute' && mutation !== 'setFormAttributeNS') {
        radio.setAttribute('form', 'owner');
    }
    assert(radio.form === form && peer.form === null, 'distinct initial owners');
    assert(radio.checked && peer.checked, 'distinct initial checked groups');
    const actions = {
        setFormAttribute: () => radio.setAttribute('FORM', 'missing'),
        setFormAttributeNS: () => radio.setAttributeNS(null, 'form', 'missing'),
        removeFormAttribute: () => radio.removeAttribute('FORM'),
        removeFormAttributeNS: () => radio.removeAttributeNS('', 'form'),
        removeForm: () => form.remove(),
        removeCarrier: () => carrier.remove(),
        moveForm: () => destination.append(form),
        moveCarrier: () => destination.append(carrier),
        detachForm: () => document.createDocumentFragment().append(form),
        detachCarrier: () => document.createDocumentFragment().append(carrier),
        sameFormAttribute: () => radio.setAttributeNS(null, 'form', 'owner'),
    };
    actions[mutation]();
    const keepsOwner = ['moveForm', 'moveCarrier', 'sameFormAttribute'].includes(mutation);
    assert(radio.form === (keepsOwner ? form : null), 'owner after mutation');
    assert(radio.checked && peer.checked === (mutation === 'sameFormAttribute'), 'checkedness after mutation');
    assert(form.elements.length === (keepsOwner ? 1 : 0), 'form collection after mutation');
    assert(JSON.stringify(Array.from(new FormData(form))) === JSON.stringify(keepsOwner ? [['group', 'on']] : []), 'FormData after mutation');
    return true;
})
