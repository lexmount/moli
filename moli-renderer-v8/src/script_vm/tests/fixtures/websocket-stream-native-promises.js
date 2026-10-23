globalThis.probeWebSocketStreamNativePromises = async function(realm, mode, url) {
    const NativePromise = realm.Promise;
    const descriptor = Object.getOwnPropertyDescriptor(realm, 'Promise');
    let authorCalls = 0;
    let closedThenCalls = 0;
    let checks = 0;
    const check = (condition, message) => {
        if (!condition) throw new Error(`${mode}: ${message}`);
        checks++;
    };
    function withReplacedPromise(callback) {
        if (mode === 'getter') {
            Object.defineProperty(realm, 'Promise', {
                configurable: true,
                get() {
                    authorCalls++;
                    throw new Error('author Promise getter');
                }
            });
        } else if (mode === 'subclass') {
            const Replacement = class extends NativePromise {
                constructor(executor) {
                    authorCalls++;
                    super(executor);
                }
            };
            Object.defineProperty(realm, 'Promise', {
                configurable: true, writable: true, value: Replacement
            });
        } else {
            Object.defineProperty(realm, 'Promise', {
                configurable: true, writable: true, value: undefined
            });
        }
        try {
            return callback();
        } finally {
            Object.defineProperty(realm, 'Promise', descriptor);
        }
    }
    const controller = new realm.AbortController();
    if (!url) controller.abort();
    const stream = withReplacedPromise(() => new realm.WebSocketStream(
        url || 'wss://127.0.0.1:1/not-started', {signal: controller.signal}
    ));
    const opened = stream.opened;
    const closed = stream.closed;
    check(opened && Object.getPrototypeOf(opened) === NativePromise.prototype,
          'opened is an intrinsic Promise');
    check(closed && Object.getPrototypeOf(closed) === NativePromise.prototype,
          'closed is an intrinsic Promise');
    check(opened === stream.opened && closed === stream.closed && opened !== closed,
          'promise identity is cached and distinct');
    if (!url) {
        const errors = await NativePromise.all([
            opened.then(() => null, error => error),
            closed.then(() => null, error => error)
        ]);
        check(errors[0]?.name === 'AbortError', 'opened rejects after abort');
        check(errors[1]?.name === 'AbortError', 'closed rejects after abort');
        check(errors[0] === errors[1], 'abort rejects both promises with one error');
    } else {
        const closedResult = closed.then(value => ({value}), error => ({error}));
        const {readable, writable} = await opened;
        const writer = writable.getWriter();
        const write = withReplacedPromise(() => writer.write('intrinsic-promise'));
        check(Object.getPrototypeOf(write) === NativePromise.prototype,
              'write returns an intrinsic Promise');
        await write;
        const message = await readable.getReader().read();
        check(!message.done && message.value === 'intrinsic-promise',
              'native write reaches the echo server');
        Object.defineProperty(closed, 'then', {
            configurable: true,
            get() {
                closedThenCalls++;
                throw new Error('author closed.then getter');
            }
        });
        try {
            const close = withReplacedPromise(() => writer.close());
            check(Object.getPrototypeOf(close) === NativePromise.prototype,
                  'close returns an intrinsic Promise');
            check(await close === undefined, 'writer.close fulfills with undefined');
            const result = await closedResult;
            check(!result.error && result.value.closeCode === 1005 && result.value.reason === '',
                  'native close settles the cached closed promise');
        } finally {
            delete closed.then;
        }
    }
    check(authorCalls === 0, 'internal promises never invoke author Promise');
    check(closedThenCalls === 0, 'internal close never reads author then');
    return {checks, authorCalls, closedThenCalls};
};
