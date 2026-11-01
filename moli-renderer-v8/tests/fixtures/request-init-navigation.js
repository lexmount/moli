(() => {
    const assert = (value, message) => {
        if (!value) throw new Error(message);
    };
    const inherited = request => {
        assert(request.mode === 'navigate', 'empty init preserves navigation mode');
        assert(request.isReloadNavigation && request.isHistoryNavigation,
               'empty init preserves both native navigation flags');
        assert(request.referrer === nativeRequest.referrer &&
               request.referrerPolicy === nativeRequest.referrerPolicy,
               'empty init preserves inherited referrer metadata');
        assert(request.signal !== nativeRequest.signal && request.signal instanceof AbortSignal,
               'constructor still creates its own following signal');
    };
    for (const init of [undefined, null, {}, {cache: undefined}, {body: undefined},
                        {signal: undefined}, {unrecognized: true}]) {
        inherited(new Request(nativeRequest, init));
    }
    inherited(new Request(nativeRequest));
    inherited(new Request(nativeRequest, Object.create({cache: undefined})));

    const members = [
        ['body', null], ['signal', null], ['method', 'GET'], ['cache', 'no-store'],
        ['mode', 'cors'], ['redirect', 'manual'], ['referrer', ''],
        ['referrerPolicy', 'no-referrer'], ['integrity', ''], ['duplex', 'half'],
        ['headers', {}], ['credentials', 'include'], ['priority', 'auto'],
        ['keepalive', false],
    ];
    for (const [member, value] of members) {
        let reads = 0;
        const init = new Proxy({}, {
            get(target, name) {
                if (name === member) {
                    reads++;
                    return value;
                }
            },
            ownKeys() { throw new Error('dictionary presence must not enumerate properties'); },
        });
        const request = new Request(nativeRequest, init);
        assert(reads === 1, member + ': dictionary getter is read once');
        assert(request.mode === (member === 'mode' ? 'cors' : 'same-origin'),
               member + ': defined member resets inherited navigation mode');
        assert(!request.isReloadNavigation && !request.isHistoryNavigation,
               member + ': defined member resets native navigation flags');
        assert(request.referrer === (member === 'referrer' ? '' : 'about:client'),
               member + ': referrer is reset before explicit overrides');
        assert(request.referrerPolicy === (member === 'referrerPolicy' ? 'no-referrer' : ''),
               member + ': referrer policy is reset before explicit overrides');
    }
    const cached = new Request(nativeRequest, {cache: 'only-if-cached'});
    assert(cached.mode === 'same-origin' && cached.cache === 'only-if-cached',
           'navigation mode is reset before cache-mode validation');
    const overrides = new Request(nativeRequest, {
        referrer: '/override', referrerPolicy: 'origin', mode: 'no-cors',
    });
    assert(overrides.referrer === 'https://request-init-navigation.test/override' &&
           overrides.referrerPolicy === 'origin' && overrides.mode === 'no-cors',
           'explicit init values are applied after resetting inherited state');
    assert(nativeRequest.mode === 'navigate' && nativeRequest.isReloadNavigation &&
           nativeRequest.isHistoryNavigation && nativeRequest.referrerPolicy === 'strict-origin' &&
           nativeRequest.headers.get('x-native') === 'original',
           'constructing requests leaves the source native request unchanged');
    return true;
})()
