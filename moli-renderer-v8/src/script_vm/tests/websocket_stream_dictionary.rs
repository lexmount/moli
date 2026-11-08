use super::*;

#[test]
fn websocket_stream_derived_options_convert_before_url_validation() {
    let mut vm = new_storage_html_test_vm("https://websocket-options-dictionary.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
                const assert = (ok, message) => {if (!ok) throw Error(message);};
                const url = 'ftp://invalid.test/';
                const failure = (options, realm = window) => {
                    try {new realm.WebSocketStream(url, options);}
                    catch (error) {return error;}
                    throw Error('invalid URL unexpectedly accepted');
                };
                const controller = new AbortController();
                const order = [];
                const options = {
                    get protocols() {
                        order.push('protocols');
                        return {[Symbol.iterator]() {
                            order.push('iterator');
                            return [{toString(){order.push('item'); return 'chat';}}][Symbol.iterator]();
                        }};
                    },
                    get signal() {order.push('signal'); return controller.signal;}
                };
                assert(failure(options).name === 'SyntaxError', 'valid dictionary reaches URL validation');
                assert(order.join() === 'protocols,iterator,item,signal', 'complete each member conversion before the next getter');
                const marker = {};
                let signalReads = 0;
                const throwing = {
                    protocols: {[Symbol.iterator](){throw marker;}},
                    get signal(){signalReads++; return controller.signal;}
                };
                assert(failure(throwing) === marker && signalReads === 0, 'iterator exception before signal getter');
                for (const protocols of [null, 1, 'chat', Symbol('protocols')]) {
                    const error = failure({protocols, get signal(){signalReads++; return controller.signal;}});
                    assert(error instanceof TypeError && signalReads === 0, 'invalid sequence before later members');
                }
                assert(failure({protocols:[], get signal(){throw marker;}}) === marker, 'signal getter exception identity');
                for (const signal of [null, {}, Object.create(AbortSignal.prototype)])
                    assert(failure({signal}) instanceof TypeError, 'native AbortSignal conversion');
                let traps = 0;
                const proxy = new Proxy(controller.signal, {get(){traps++; throw marker;}, getPrototypeOf(){traps++; throw marker;}});
                assert(failure({signal:proxy}) instanceof TypeError && traps === 0, 'author Proxy is not a signal');
                for (const options of [null, undefined, {}, {protocols:[], signal:controller.signal}])
                    assert(failure(options).name === 'SyntaxError', 'empty and native options');
                for (const options of [false, 1, 'options', 1n, Symbol('options')])
                    assert(failure(options) instanceof TypeError, 'primitive dictionary');
                Object.defineProperty(Object.prototype,'protocols',{configurable:true,get(){throw marker;}});
                try {
                    assert(failure(null).name === 'SyntaxError' && failure(undefined).name === 'SyntaxError', 'nullish dictionary ignores Object.prototype');
                    assert(failure({}) === marker, 'ordinary dictionary reads inherited members');
                } finally {delete Object.prototype.protocols;}
                document.body.innerHTML = '<iframe></iframe>';
                const other = document.querySelector('iframe').contentWindow;
                assert(Object.getPrototypeOf(failure({protocols:null}, other)) === other.TypeError.prototype, 'callee realm TypeError');
                assert(failure({protocols:['chat'], signal:controller.signal}, other).name === 'SyntaxError', 'cross-realm genuine signal');
                return 'ok';
            })()"#,
        )
        .unwrap(),
        "ok"
    );
}
