(mutation => {
    const radio = document.getElementById('radio');
    const peer = document.getElementById('peer');
    const form = document.getElementById('owner');
    const carrier = document.getElementById('carrier');
    const cluster = document.getElementById('cluster');
    const destination = document.getElementById('destination');
    const assert = (condition, message) => {
        if (!condition) throw Error(mutation + ': ' + message);
    };
    assert(radio.form === form && !form.contains(radio), 'parser-associated external radio');
    assert(radio.checked && peer.checked, 'distinct initial radio groups');
    const actions = {
        moveForm: () => destination.append(form),
        removeForm: () => form.remove(),
        moveCarrier: () => destination.append(carrier),
        removeCarrier: () => carrier.remove(),
        moveTogether: () => destination.append(cluster),
        removeTogether: () => cluster.remove(),
    };
    actions[mutation]();
    const together = mutation === 'moveTogether' || mutation === 'removeTogether';
    assert(radio.form === (together ? form : null), 'owner after mutation');
    assert(radio.checked && peer.checked === together, 'checkedness after mutation');
    assert(form.elements.length === (together ? 1 : 0), 'form collection after mutation');
    assert(JSON.stringify(Array.from(new FormData(form))) === JSON.stringify(together ? [['group', 'on']] : []), 'FormData after mutation');
    return true;
})
