use super::*;

#[test]
fn document_implementation_realm() {
    let mut vm = new_parsed_test_vm(
        "https://factory-realms.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
        const assert = (ok,label) => { if (!ok) throw Error(label); };
        const frame = document.body.appendChild(document.createElement('iframe'));
        const child = frame.contentWindow;

        for (const [owner,caller] of [[window,child],[child,window]]) {
            const getter = Object.getOwnPropertyDescriptor(caller.Document.prototype,'implementation').get;
            const detached = owner.document.implementation.createHTMLDocument('detached');
            for (const doc of [owner.document,detached]) {
                const implementation = getter.call(doc);
                assert(Object.getPrototypeOf(implementation) === owner.DOMImplementation.prototype,'implementation realm');
                assert(getter.call(doc) === implementation && doc.implementation === implementation,'SameObject');
                const type = implementation.createDocumentType('html','','');
                assert(owner.DocumentType.prototype.isPrototypeOf(type) &&
                    !caller.DocumentType.prototype.isPrototypeOf(type),'created DocumentType realm');
                assert(type.ownerDocument === doc,'created DocumentType owner');
            }
        }
        return true;
    })()"#).unwrap(), "true");
}
