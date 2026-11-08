use super::*;

#[test]
fn document_range_factory_realm() {
    let mut vm = new_parsed_test_vm(
        "https://factory-realms.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
        const assert = (ok,label) => { if (!ok) throw Error(label); };
        const frame = document.body.appendChild(document.createElement('iframe'));
        const child = frame.contentWindow;

        for (const [owner,caller] of [[window,child],[child,window]]) {
            const doc = owner.document;
            const range = caller.Document.prototype.createRange.call(doc);
            const clone = caller.Range.prototype.cloneRange.call(range);
            assert(Object.getPrototypeOf(range) === owner.Range.prototype,'Range realm');
            assert(Object.getPrototypeOf(clone) === owner.Range.prototype,'cloned Range realm');
            assert(range.startContainer === doc && clone.startContainer === doc,'range document');
        }
        return true;
    })()"#
        )
        .unwrap(),
        "true"
    );
}
