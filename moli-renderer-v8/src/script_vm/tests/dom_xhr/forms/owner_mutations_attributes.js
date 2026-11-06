(() => {
    const radio = document.getElementById('radio');
    const form = document.getElementById('owner');
    const assertOwner = message => {
        if (radio.form !== form || form.elements[0] !== radio) throw Error(message);
    };
    assertOwner('initial parser owner');
    for (const namespace of [null, '', 'urn:form-owner-test']) {
        radio.setAttributeNS(namespace, 'FORM', 'missing');
        assertOwner('uppercase namespaced attribute must preserve parser owner');
        radio.removeAttributeNS(namespace, 'FORM');
        assertOwner('removing uppercase namespaced attribute must preserve parser owner');
    }
    radio.setAttributeNS('urn:form-owner-test', 'x:form', 'missing');
    assertOwner('non-null namespace must preserve parser owner');
    radio.removeAttributeNS('urn:form-owner-test', 'form');
    assertOwner('removing a non-null namespace must preserve parser owner');
    radio.setAttributeNS('', 'form', 'missing');
    if (radio.form !== null || form.elements.length !== 0) throw Error('lowercase form must reset owner');
    radio.removeAttributeNS('', 'form');
    if (radio.form !== null) throw Error('removing form must not resurrect parser owner');
    return true;
})()
