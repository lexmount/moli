(() => {
    const assert = (condition, message) => {
        if (!condition) throw Error(message);
    };
    const primary = document.body.appendChild(document.createElement('form'));
    primary.id = 'owner';
    const primaryInput = document.body.appendChild(document.createElement('input'));
    primaryInput.setAttribute('form', 'owner');
    primaryInput.name = 'primary';

    const doc = document.implementation.createHTMLDocument('secondary');
    const form = doc.body.appendChild(doc.createElement('form'));
    form.id = 'owner';
    const inside = form.appendChild(doc.createElement('input'));
    inside.name = 'inside';
    inside.value = 'inside-value';
    const excluded = form.appendChild(doc.createElement('input'));
    excluded.name = 'excluded';
    excluded.value = 'excluded-value';
    excluded.setAttribute('form', '');
    const outside = doc.body.appendChild(doc.createElement('input'));
    outside.setAttribute('form', 'owner');
    outside.name = 'outside';
    outside.value = 'outside-value';
    const elements = form.elements;
    const check = (expected, label) => {
        assert(JSON.stringify(Array.from(elements, control => control.name)) === JSON.stringify(expected), label + ': live elements');
        assert(JSON.stringify(Array.from(new FormData(form))) === JSON.stringify(expected.map(name => [name, name + '-value'])), label + ': FormData');
        assert(primaryInput.form === primary && primary.elements.length === 1, label + ': primary document isolation');
    };
    assert(outside.isConnected && outside.form === form && excluded.form === null, 'secondary owners');
    check(['inside', 'outside'], 'initial');
    form.id = 'renamed';
    assert(outside.form === null, 'id change');
    check(['inside'], 'renamed');
    form.id = 'owner';
    check(['inside', 'outside'], 'id restored');
    form.remove();
    assert(outside.form === null && excluded.form === form, 'detached owners');
    check(['inside', 'excluded'], 'detached');
    doc.body.append(form);
    assert(outside.form === form && excluded.form === null, 'reconnected owners');
    check(['outside', 'inside'], 'reconnected');
    document.body.append(form);
    assert(form.ownerDocument === document && outside.form === null, 'adopted owners');
    check(['inside'], 'adopted');
    return true;
})()
