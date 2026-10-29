use super::*;

#[test]
fn document_traversal_factory_realms() {
    let mut vm = new_parsed_test_vm(
        "https://factory-realms.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
        const assert = (ok,label) => { if (!ok) throw Error(label); };
        const frame = document.body.appendChild(document.createElement('iframe'));
        const child = frame.contentWindow;

        for (const [owner,caller] of [[window,child],[child,window]]) {
            const doc = owner.document;
            const iterator = caller.Document.prototype.createNodeIterator.call(doc,doc);
            const walker = caller.Document.prototype.createTreeWalker.call(doc,doc);
            assert(Object.getPrototypeOf(iterator) === owner.NodeIterator.prototype,'NodeIterator realm');
            assert(Object.getPrototypeOf(walker) === owner.TreeWalker.prototype,'TreeWalker realm');
            assert(iterator.root === doc && walker.root === doc,'traversal roots');
        }
        return true;
    })()"#).unwrap(), "true");
}
