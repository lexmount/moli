use super::*;

#[test]
fn exception_arguments_keep_lone_surrogates_and_conversion_order() {
    let mut vm = new_parsed_test_vm(
        "https://exception-utf16.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
        function check(condition, label) { if (!condition) throw new Error(label); }
        for (const text of ["\ud800", "\udc00", "A\ud800B", "\ud83d\ude00", "\0"]) {
            const error = new DOMException(text, text);
            check(error.message === text, 'message');
            check(error.name === text, 'name');
            check(error.code === 0, 'unknown name code');
            check(new QuotaExceededError(text, {quota: 2, requested: 3}).message === text, 'quota message');
        }
        check(new DOMException().name === 'Error', 'default name');
        check(new DOMException(undefined, 'NotFoundError').code === 8, 'legacy code');
        const sentinel = {};
        const order = [];
        const message = {toString() { order.push('message'); return '\ud800'; }};
        const name = {toString() { order.push('name'); throw sentinel; }};
        try { new DOMException(message, name); throw new Error('must fail'); }
        catch (error) { check(error === sentinel, 'original exception'); }
        check(order.join(',') === 'message,name', 'conversion order');
        let optionReads = 0;
        try {
            new QuotaExceededError({toString() {throw sentinel;}}, {get quota() { optionReads++; return 1; }});
            throw new Error('must fail');
        } catch (error) { check(error === sentinel, 'quota original exception'); }
        check(optionReads === 0, 'message failure precedes options');
        return true;
    })()"#).unwrap(), "true");
}
