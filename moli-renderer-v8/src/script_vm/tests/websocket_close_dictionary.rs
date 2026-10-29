use super::*;

#[test]
fn websocket_close_info_dictionary_converts_members_before_validation() {
    let mut vm = new_storage_html_test_vm("https://websocket-close-dictionary.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
                const assert = (ok, message) => {if (!ok) throw Error(message);};
                const controller = new AbortController();
                controller.abort();
                // A pre-aborted signal creates a genuine stream without opening a socket.
                const stream = new WebSocketStream('wss://127.0.0.1:9/', {signal:controller.signal});
                stream.opened.catch(() => {});
                stream.closed.catch(() => {});
                const actions = [
                    info => new WebSocketError('message', info),
                    info => stream.close(info)
                ];
                const failure = action => {
                    try {action();} catch (error) {return error;}
                    throw Error('expected conversion or validation failure');
                };
                const marker = {};
                for (const action of actions) {
                    const order = [];
                    action({
                        get closeCode() {
                            order.push('closeCode');
                            return {valueOf(){order.push('number'); return 1000;}};
                        },
                        get reason() {
                            order.push('reason');
                            return {toString(){order.push('string'); return 'done';}};
                        }
                    });
                    assert(order.join() === 'closeCode,number,reason,string', 'complete member conversions in order');
                    assert(failure(() => action({closeCode:0, get reason(){throw marker;}})) === marker,
                        'reason conversion precedes close-code validation');
                    let reasonReads = 0;
                    assert(failure(() => action({get closeCode(){throw marker;}, get reason(){reasonReads++;}})) === marker,
                        'closeCode getter exception identity');
                    assert(reasonReads === 0, 'no later member after abrupt conversion');
                    assert(failure(() => action({closeCode:{valueOf(){throw marker;}}, get reason(){reasonReads++;}})) === marker,
                        'numeric conversion exception identity');
                    assert(reasonReads === 0, 'no reason after numeric conversion failure');
                    assert(failure(() => action({closeCode:1000, get reason(){throw marker;}})) === marker,
                        'reason getter exception identity');
                    assert(failure(() => action({closeCode:1000, reason:{toString(){throw marker;}}})) === marker,
                        'reason string exception identity');
                    for (const info of [false, 1, 'info', 1n, Symbol('info')])
                        assert(failure(() => action(info)) instanceof TypeError, 'primitive dictionary rejected');
                    assert(failure(() => action({reason:Symbol('reason')})) instanceof TypeError, 'USVString conversion rejects Symbol');
                    assert(failure(() => action({closeCode:0, reason:''})).name === 'InvalidAccessError', 'existing close-code validation');
                    assert(failure(() => action({closeCode:null})).name === 'InvalidAccessError', 'null numeric member converts to zero');
                    assert(failure(() => action({closeCode:1000, reason:'x'.repeat(124)})).name === 'SyntaxError', 'existing reason-byte limit');
                    action(undefined);
                    action(null);
                    action({});
                    const reads = [];
                    action(new Proxy({closeCode:1000, reason:'proxy'}, {
                        get(target, key, receiver){reads.push(key); return Reflect.get(target,key,receiver);}
                    }));
                    assert(reads.join() === 'closeCode,reason', 'dictionary Proxy members read once');
                    Object.defineProperty(Object.prototype, 'closeCode', {configurable:true,get(){throw marker;}});
                    try {
                        action(undefined);
                        action(null);
                        assert(failure(() => action({})) === marker, 'ordinary dictionaries read inherited members');
                    } finally {delete Object.prototype.closeCode;}
                }
                assert(new WebSocketError().closeCode === null && new WebSocketError().reason === '', 'empty dictionary defaults');
                assert(new WebSocketError('', {reason:null}).reason === 'null', 'null reason converts as USVString');
                assert(new WebSocketError('', {reason:'\uD800'}).reason === '\uFFFD', 'USVString replaces lone surrogate');
                const callable = function() {};
                callable.reason = 'callable';
                assert(new WebSocketError('', callable).reason === 'callable', 'callable dictionary accepted');
                let initReads = 0;
                assert(failure(() => new WebSocketError({toString(){throw marker;}}, {
                    get closeCode(){initReads++; return 1000;}
                })) === marker && initReads === 0, 'message conversion failure precedes dictionary reads');
                document.body.innerHTML = '<iframe></iframe>';
                const other = document.querySelector('iframe').contentWindow;
                const otherClose = other.WebSocketStream.prototype.close;
                assert(Object.getPrototypeOf(failure(() => new other.WebSocketError('', false))) === other.TypeError.prototype,
                    'constructor TypeError belongs to callee realm');
                assert(Object.getPrototypeOf(failure(() => otherClose.call(stream, false))) === other.TypeError.prototype,
                    'method TypeError belongs to callee realm');
                otherClose.call(stream, {closeCode:1000, reason:'cross-realm'});
                return 'ok';
            })()"#,
        )
        .unwrap(),
        "ok"
    );
}
